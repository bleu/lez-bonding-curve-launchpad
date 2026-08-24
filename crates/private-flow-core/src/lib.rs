//! Wire instruction for the private-buy router guest.

use lee_core::{account::AccountId, program::ProgramId};
use serde::{Deserialize, Serialize};

/// Executes the RFP-015 private purchase composition in one privacy-preserving transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateBuyInstruction {
    pub curve_program_id: ProgramId,
    pub token_program_id: ProgramId,
    pub ata_program_id: ProgramId,
    pub native_transfer_program_id: ProgramId,
    pub amount_out: u128,
    pub max_collateral_in: u128,
    pub gas_reserve: u128,
    pub collateral_definition: AccountId,
}
