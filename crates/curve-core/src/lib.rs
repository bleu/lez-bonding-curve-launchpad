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
//! One consequence of `Sale` living in the `sale` crate: `impl TryFrom<&Data> for Sale`
//! is blocked by the orphan rule, both types being foreign. Read the state through a free
//! function calling borsh's `try_from_slice` instead of the upstream `TryFrom` impl.
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
    CreateSale {
        sale_reserve: u128,
        dex_seed_reserve: u128,
        virtual_token_reserve: u128,
        virtual_collateral_reserve: u128,
        /// The program cannot see its own id; PDA derivation needs it passed in.
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

/// Reads a [`Sale`] out of the sale PDA's data.
pub fn sale_from_data(data: &Data) -> std::io::Result<Sale> {
    Sale::try_from_slice(data.as_ref())
}

/// Serialises a [`Sale`] into account data.
#[must_use]
pub fn sale_to_data(sale: &Sale) -> Data {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(sale));
    BorshSerialize::serialize(sale, &mut bytes).expect("serialisation to a Vec cannot fail");
    Data::try_from(bytes).expect("a sale is far below the account data size limit")
}
