//! Guest-facing instruction dispatch. Account ordering is part of the public wire interface.

use lee_core::{
    account::AccountWithMetadata,
    program::{AccountPostState, ChainedCall, ProgramId},
};

use crate::{Instruction, create_sale::create_sale, update_config::update_config};

#[must_use]
pub fn process_instruction(
    pre_states: Vec<AccountWithMetadata>,
    instruction: Instruction,
    self_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    match instruction {
        Instruction::UpdateConfig {
            admin,
            pool_fee_bps,
            protocol_fee_bps,
            treasury,
        } => {
            let [config, authority] = pre_states
                .try_into()
                .expect("UpdateConfig requires exactly two accounts");
            (
                update_config(
                    config,
                    authority,
                    admin,
                    pool_fee_bps,
                    protocol_fee_bps,
                    treasury,
                    self_program_id,
                ),
                vec![],
            )
        }
        Instruction::CreateSale {
            sale_reserve,
            dex_seed_reserve,
            virtual_token_reserve,
            virtual_collateral_reserve,
            curve_program_id,
        } => {
            assert_eq!(
                curve_program_id, self_program_id,
                "Curve program ID does not match executing program"
            );
            let [
                sale,
                creator,
                token_definition,
                collateral_definition,
                creator_token_ata,
                sale_token_ata,
                sale_collateral_ata,
            ] = pre_states
                .try_into()
                .expect("CreateSale requires exactly seven accounts");
            create_sale(
                sale,
                creator,
                token_definition,
                collateral_definition,
                creator_token_ata,
                sale_token_ata,
                sale_collateral_ata,
                sale_reserve,
                dex_seed_reserve,
                virtual_token_reserve,
                virtual_collateral_reserve,
                curve_program_id,
            )
        }
        Instruction::Buy { .. }
        | Instruction::Sell { .. }
        | Instruction::Close
        | Instruction::Withdraw => panic!("instruction handler is not implemented yet"),
    }
}
