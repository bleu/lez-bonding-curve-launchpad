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
//! The handlers arrive with GTM-509 onward, which also adds the guest shim and
//! deletes `deploy_probe`.

use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::account::Data;
use lee_core::program::ProgramId;
use sale::Sale;
use serde::{Deserialize, Serialize};

/// Curve program instruction. The account lists are settled by the issue that
/// implements each handler; GTM-516 adds the `UpdateConfig` variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Instruction {
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

#[cfg(test)]
mod tests {
    use super::*;
    use sale::Sale;

    #[test]
    fn sale_state_round_trips_through_account_data() {
        let sale = Sale::create([1; 32], [2; 32], 800, 200, 1000, 100).expect("valid sale");
        let data = sale_to_data(&sale);
        assert_eq!(sale_from_data(&data).expect("data parses"), sale);
    }

    #[test]
    fn every_instruction_survives_the_guest_wire_format() {
        // `read_lee_inputs::<Instruction>()` in the guest deserialises with risc0's serde.
        let instructions = [
            Instruction::CreateSale {
                sale_reserve: 800,
                dex_seed_reserve: 200,
                virtual_token_reserve: 1000,
                virtual_collateral_reserve: 100,
                curve_program_id: [7; 8],
            },
            Instruction::Buy {
                collateral_in: 25,
                min_tokens_out: 190,
            },
            Instruction::Sell {
                tokens_in: 250,
                min_collateral_out: 18,
            },
            Instruction::Close,
            Instruction::Withdraw,
        ];
        for instruction in instructions {
            let words = risc0_zkvm::serde::to_vec(&instruction).expect("instruction serialises");
            let back: Instruction =
                risc0_zkvm::serde::from_slice(&words).expect("wire words parse");
            assert_eq!(back, instruction);
        }
    }
}
