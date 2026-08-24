//! One private transaction: native funding + collateral deshield + buy + re-shield.

use associated_token_account_core::Instruction as AtaInstruction;
use curve_core::Instruction as CurveInstruction;
use lee_core::program::{AccountPostState, ChainedCall, ProgramInput, ProgramOutput, read_lee_inputs};
use private_flow_core::PrivateBuyInstruction;

fn main() {
    let (
        ProgramInput {
            self_program_id,
            caller_program_id,
            pre_states,
            instruction,
        },
        instruction_words,
    ) = read_lee_inputs::<PrivateBuyInstruction>();

    let [private_source, transient, collateral_definition, launch_definition, transient_collateral_ata, transient_token_ata, pool, config, pool_collateral_ata, pool_token_ata, treasury_collateral_ata, clock, private_destination] = pre_states
        .clone()
        .try_into()
        .expect("PrivateBuy requires exactly thirteen accounts");

    assert!(instruction.gas_reserve > 0, "PrivateBuy requires native gas");
    assert!(instruction.max_collateral_in > 0, "PrivateBuy requires collateral");
    assert_eq!(collateral_definition.account_id, instruction.collateral_definition, "Collateral definition does not match instruction");
    assert_eq!(private_source.account.program_owner, instruction.token_program_id, "Private source must be a token holding");

    let mut source_authorized = private_source.clone();
    source_authorized.is_authorized = true;
    let mut transient_authorized = transient.clone();
    transient_authorized.is_authorized = true;

    let calls = vec![
        ChainedCall::new(
            instruction.native_transfer_program_id,
            vec![source_authorized.clone(), transient_authorized.clone()],
            &authenticated_transfer_core::Instruction::Transfer { amount: instruction.gas_reserve },
        ),
        ChainedCall::new(
            instruction.ata_program_id,
            vec![transient_authorized.clone(), collateral_definition.clone(), transient_collateral_ata.clone()],
            &AtaInstruction::Create { ata_program_id: instruction.ata_program_id },
        ),
        ChainedCall::new(
            instruction.ata_program_id,
            vec![transient_authorized.clone(), launch_definition.clone(), transient_token_ata.clone()],
            &AtaInstruction::Create { ata_program_id: instruction.ata_program_id },
        ),
        ChainedCall::new(
            instruction.token_program_id,
            vec![source_authorized, transient_collateral_ata.clone()],
            &token_core::Instruction::Transfer { amount_to_transfer: instruction.max_collateral_in },
        ),
        ChainedCall::new(
            instruction.curve_program_id,
            vec![pool, config, transient_authorized.clone(), transient_collateral_ata, pool_collateral_ata, pool_token_ata, transient_token_ata.clone(), treasury_collateral_ata, clock],
            &CurveInstruction::SwapExactOutput {
                amount_out: instruction.amount_out,
                max_amount_in: instruction.max_collateral_in,
                token_in: instruction.collateral_definition,
            },
        ),
        ChainedCall::new(
            instruction.ata_program_id,
            vec![transient_authorized, transient_token_ata, private_destination],
            &AtaInstruction::Transfer { ata_program_id: instruction.ata_program_id, amount: instruction.amount_out },
        ),
    ];

    let post_states = pre_states
        .iter()
        .map(|state| AccountPostState::new(state.account.clone()))
        .collect();
    ProgramOutput::new(self_program_id, caller_program_id, instruction_words, pre_states, post_states)
        .with_chained_calls(calls)
        .write();
}
