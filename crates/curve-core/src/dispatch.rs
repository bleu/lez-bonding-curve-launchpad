//! Guest-facing instruction dispatch. Account ordering is part of the public wire interface.

use lee_core::{
    account::AccountWithMetadata,
    program::{AccountPostState, ChainedCall, ProgramId},
};

use crate::{
    Instruction, pool_create::create_pool, pool_lifecycle::close_pool, update_config::update_config,
};

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
        Instruction::CreatePool {
            token0_amount,
            token1_amount,
            virtual_reserve0,
            virtual_reserve1,
            close_timestamp,
            owner,
            curve_program_id,
        } => {
            assert_eq!(
                curve_program_id, self_program_id,
                "Curve program ID does not match executing program"
            );
            let [
                pool,
                owner_authority,
                token0_definition,
                token1_definition,
                owner_token0_ata,
                owner_token1_ata,
                pool_token0_ata,
                pool_token1_ata,
            ] = pre_states
                .try_into()
                .expect("CreatePool requires exactly eight accounts");
            create_pool(
                pool,
                owner_authority,
                token0_definition,
                token1_definition,
                owner_token0_ata,
                owner_token1_ata,
                pool_token0_ata,
                pool_token1_ata,
                token0_amount,
                token1_amount,
                virtual_reserve0,
                virtual_reserve1,
                close_timestamp,
                owner,
                curve_program_id,
            )
        }
        Instruction::ClosePool => {
            let [pool, owner] = pre_states
                .try_into()
                .expect("ClosePool requires exactly two accounts");
            (close_pool(pool, owner, self_program_id), vec![])
        }
        Instruction::SwapExactInput { .. }
        | Instruction::SwapExactOutput { .. }
        | Instruction::WithdrawReserves => panic!("instruction handler is not implemented yet"),
    }
}
