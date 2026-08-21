//! The curve program: instruction enum, Borsh state, PDA derivation, and the handlers.
//!
//! Modelled on `lez/programs/amm/core/src/lib.rs` plus `lez/programs/amm/src/`, with one
//! deliberate difference. Upstream puts the handlers in the guest binary crate; here they
//! live in the host workspace and `methods/guest/src/bin/curve.rs` is a dispatch shim, so
//! the AMM-style tests run under `cargo test --workspace`. See `docs/adr/0002`.
//!
//! The handlers stay shallow. They deserialize accounts, call `sale`, and translate the
//! returned outcome into account post-states and chained calls. No decisions here.
//!
//! The sale handlers arrive with GTM-509 onward, which also adds the guest shim and
//! deletes `deploy_probe`.

use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::{
    account::{AccountId, Data},
    program::{PdaSeed, ProgramId},
};
use sale::Sale;
use serde::{Deserialize, Serialize};

pub mod create_sale;
pub mod dispatch;
pub mod update_config;

#[cfg(test)]
mod tests;

/// Curve program instruction. The account lists of the sale variants are settled
/// by the issue that implements each handler.
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
        fee_bps: u16,
        treasury: AccountId,
    },
    /// Opens a sale over the token pair handed to it. Handler: GTM-509.
    ///
    /// Required accounts:
    /// - Sale PDA (uninitialized)
    /// - Creator authority (authorized)
    /// - Project-token definition
    /// - Collateral-token definition
    /// - Creator's project-token ATA
    /// - Sale PDA's project-token ATA (uninitialized)
    /// - Sale PDA's collateral-token ATA (uninitialized)
    CreateSale {
        sale_reserve: u128,
        dex_seed_reserve: u128,
        virtual_token_reserve: u128,
        virtual_collateral_reserve: u128,
        /// Included in the wire instruction following the LEZ program convention;
        /// dispatch verifies it against the executing program id.
        curve_program_id: ProgramId,
    },
    /// Buys from the curve, auto-closing on the buy that exhausts the sale
    /// reserve. Handler: GTM-510.
    Buy {
        collateral_in: u128,
        min_tokens_out: u128,
    },
    /// Sells back to the curve while the sale is open. Handler: GTM-511.
    Sell {
        tokens_in: u128,
        min_collateral_out: u128,
    },
    /// Creator-only manual close. Handler: GTM-512.
    Close,
    /// Creator withdrawal of the real collateral reserve plus the unused DEX
    /// seed reserve, after close. Handler: GTM-512.
    Withdraw,
}

/// The admin key allowed to initialize the config. Compiled in, so it is part of the
/// risc0 image ID: changing it is a different program, and it cannot be front-run.
/// Replace with the operator's key before deploying. See the README, "Admin authority".
pub const GENESIS_ADMIN: AccountId = AccountId::new([0xAD; 32]);

/// The fee denominator. A `fee_bps` above this would make `amount - fee` underflow,
/// so `update_config` rejects it. Any tighter cap is the operator's call, not ours.
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
    pub fee_bps: u16,
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

/// The sale PDA's whole contents: the token pair, a privacy-preserving creator
/// commitment, and the [`Sale`] state machine that prices it.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SaleAccount {
    pub token_definition_id: AccountId,
    pub collateral_definition_id: AccountId,
    /// Commits to the creator without publishing the creator account id.
    pub creator_commitment: [u8; 32],
    pub sale: Sale,
}

impl TryFrom<&Data> for SaleAccount {
    type Error = std::io::Error;

    fn try_from(data: &Data) -> Result<Self, Self::Error> {
        Self::try_from_slice(data.as_ref())
    }
}

impl From<&SaleAccount> for Data {
    fn from(sale_account: &SaleAccount) -> Self {
        let mut bytes = Vec::with_capacity(std::mem::size_of_val(sale_account));

        BorshSerialize::serialize(sale_account, &mut bytes)
            .expect("serialisation to a Vec cannot fail");

        Self::try_from(bytes).expect("a sale account is far below the account data size limit")
    }
}

/// One sale per ordered pair per program, ever. Token and collateral are
/// different roles, so the pair hashes in fixed order with no sort; the
/// factory mints a fresh token per launch, so the pair never repeats in
/// practice.
#[must_use]
pub fn compute_sale_pda(
    curve_program_id: ProgramId,
    token_definition_id: AccountId,
    collateral_definition_id: AccountId,
) -> AccountId {
    AccountId::for_public_pda(
        &curve_program_id,
        &compute_sale_pda_seed(token_definition_id, collateral_definition_id),
    )
}

#[must_use]
pub fn compute_sale_pda_seed(
    token_definition_id: AccountId,
    collateral_definition_id: AccountId,
) -> PdaSeed {
    use risc0_zkvm::sha::{Impl, Sha256 as _};

    let mut bytes = [0; 64];
    bytes[0..32].copy_from_slice(&token_definition_id.to_bytes());
    bytes[32..].copy_from_slice(&collateral_definition_id.to_bytes());

    PdaSeed::new(
        Impl::hash_bytes(&bytes)
            .as_bytes()
            .try_into()
            .expect("Hash output must be exactly 32 bytes long"),
    )
}

/// Commits to the creator authority for this specific sale while keeping the
/// authority's account id out of public sale state.
#[must_use]
pub fn compute_creator_commitment(creator_id: AccountId, sale_id: AccountId) -> [u8; 32] {
    use risc0_zkvm::sha::{Impl, Sha256 as _};

    let mut bytes = [0; 64];
    bytes[..32].copy_from_slice(&creator_id.to_bytes());
    bytes[32..].copy_from_slice(&sale_id.to_bytes());
    Impl::hash_bytes(&bytes)
        .as_bytes()
        .try_into()
        .expect("hash output is exactly 32 bytes")
}
