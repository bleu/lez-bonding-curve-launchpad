//! The curve program: instruction enum, Borsh state, PDA derivation, and the handlers.
//!
//! Modelled on `lez/programs/amm/core/src/lib.rs` plus `lez/programs/amm/src/`, with one
//! deliberate difference. Upstream puts the handlers in the guest binary crate; here they
//! live in the host workspace and `methods/guest/src/bin/curve.rs` is a dispatch shim, so
//! the AMM-style tests run under `cargo test --workspace`. See `docs/adr/0002`.
//!
//! The handlers stay shallow. They deserialize accounts, call `pool`, and translate the
//! returned outcome into account post-states plus typed token-settlement values. The guest/token
//! adapter turns those settlements into chained calls.

use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::{
    account::{AccountId, Data},
    program::{PdaSeed, ProgramId},
};
use pool::Pool;
use serde::{Deserialize, Serialize};

pub mod dispatch;
pub mod pool_create;
pub mod pool_lifecycle;
pub mod pool_swap;
pub mod update_config;

#[cfg(test)]
mod tests;

/// Curve program instruction. Token accounts are supplied by the guest/token adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Instruction {
    /// Creates the config PDA on the first call, replaces it whole after.
    ///
    /// Required accounts:
    /// - Config PDA
    /// - Authority (authorized): the genesis admin on the first call, the stored
    ///   admin after
    UpdateConfig {
        admin: AccountId,
        pool_fee_bps: u16,
        protocol_fee_bps: u16,
        treasury: AccountId,
    },
    /// Creates a bounded pool over an ordered token pair.
    ///
    /// Required accounts:
    /// - Pool PDA (uninitialized)
    /// - Owner authority (authorized)
    /// - Token 0 definition
    /// - Token 1 definition
    /// - Owner's token 0 ATA
    /// - Owner's token 1 ATA
    /// - Pool PDA's token 0 ATA (uninitialized)
    /// - Pool PDA's token 1 ATA (uninitialized)
    CreatePool {
        token0_amount: u128,
        token1_amount: u128,
        virtual_reserve0: u128,
        virtual_reserve1: u128,
        close_timestamp: Option<u64>,
        owner: AccountId,
        /// Included in the wire instruction following the LEZ program convention;
        /// dispatch verifies it against the executing program id.
        curve_program_id: ProgramId,
    },
    /// Swaps an exact input amount; `token_in` selects the direction.
    ///
    /// Required accounts, in order:
    /// - Pool PDA
    /// - Config PDA
    /// - Participant (authorized)
    /// - Participant input ATA
    /// - Pool input ATA
    /// - Pool output ATA
    /// - Participant output ATA
    /// - Treasury input ATA
    SwapExactInput {
        amount_in: u128,
        min_amount_out: u128,
        token_in: AccountId,
    },
    /// Receives an exact output amount while capping fee-inclusive input.
    SwapExactOutput {
        amount_out: u128,
        max_amount_in: u128,
        token_in: AccountId,
    },
    /// Owner-only logical closure.
    ClosePool,
    /// Owner-only full withdrawal after manual closure or expiry.
    WithdrawReserves,
}

/// The admin key allowed to initialize the config. Compiled in, so it is part of the
/// risc0 image ID: changing it is a different program, and it cannot be front-run.
/// Replace with the operator's key before deploying. See the README, "Admin authority".
pub const GENESIS_ADMIN: AccountId = AccountId::new([0xAD; 32]);

/// The fee denominator. A combined fee above this would make `amount - fee`
/// underflow, so `update_config` rejects it.
pub const MAX_FEE_BPS: u16 = 10_000;

/// The ATA program shipped by the pinned LEZ revision. Program IDs are risc0
/// image IDs, so this binds custody calls to that exact guest implementation.
pub const ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID: ProgramId = [
    0xc81c_8495,
    0xd787_2cbd,
    0xc7c5_1b11,
    0x852a_aaf3,
    0xf790_5ed3,
    0xa382_7e21,
    0xac05_aa97,
    0xf800_15d5,
];

/// The protocol settings: one singleton PDA per deployment, read live by every trade.
/// Created and replaced whole by `update_config`. See `docs/adr/0003`.
#[derive(Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct Config {
    pub admin: AccountId,
    pub pool_fee_bps: u16,
    pub protocol_fee_bps: u16,
    pub treasury: AccountId,
}

impl TryFrom<&Data> for Config {
    type Error = std::io::Error;

    fn try_from(data: &Data) -> Result<Self, Self::Error> {
        Self::try_from_slice(data.as_ref())
    }
}

impl From<&Config> for Data {
    fn from(config: &Config) -> Self {
        let mut data = Vec::with_capacity(std::mem::size_of_val(config));

        BorshSerialize::serialize(config, &mut data).expect("Serialization to Vec should not fail");

        Self::try_from(data).expect("Config encoded data should fit into Data")
    }
}

#[must_use]
pub fn compute_config_pda(curve_program_id: ProgramId) -> AccountId {
    AccountId::for_public_pda(&curve_program_id, &compute_config_pda_seed())
}

#[must_use]
pub fn compute_config_pda_seed() -> PdaSeed {
    let mut bytes = [0_u8; 32];
    bytes[..6].copy_from_slice(b"config");
    PdaSeed::new(bytes)
}

/// The pool PDA's contents: ordered token roles, owner, and bounded-AMM state.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct PoolAccount {
    pub token0_definition_id: AccountId,
    pub token1_definition_id: AccountId,
    pub owner: AccountId,
    pub pool: Pool,
}

impl TryFrom<&Data> for PoolAccount {
    type Error = std::io::Error;

    fn try_from(data: &Data) -> Result<Self, Self::Error> {
        Self::try_from_slice(data.as_ref())
    }
}

impl From<&PoolAccount> for Data {
    fn from(pool_account: &PoolAccount) -> Self {
        let mut bytes = Vec::with_capacity(std::mem::size_of_val(pool_account));
        BorshSerialize::serialize(pool_account, &mut bytes)
            .expect("serialisation to a Vec cannot fail");
        Self::try_from(bytes).expect("a pool account is far below the account data size limit")
    }
}

/// One pool per ordered pair and owner. The pair hashes in fixed order.
#[must_use]
pub fn compute_pool_pda(
    curve_program_id: ProgramId,
    token0_definition_id: AccountId,
    token1_definition_id: AccountId,
    owner: AccountId,
) -> AccountId {
    AccountId::for_public_pda(
        &curve_program_id,
        &compute_pool_pda_seed(token0_definition_id, token1_definition_id, owner),
    )
}

#[must_use]
pub fn compute_pool_pda_seed(
    token0_definition_id: AccountId,
    token1_definition_id: AccountId,
    owner: AccountId,
) -> PdaSeed {
    use risc0_zkvm::sha::{Impl, Sha256 as _};

    let mut bytes = [0; 96];
    bytes[0..32].copy_from_slice(&token0_definition_id.to_bytes());
    bytes[32..64].copy_from_slice(&token1_definition_id.to_bytes());
    bytes[64..].copy_from_slice(&owner.to_bytes());

    PdaSeed::new(
        Impl::hash_bytes(&bytes)
            .as_bytes()
            .try_into()
            .expect("Hash output must be exactly 32 bytes long"),
    )
}
