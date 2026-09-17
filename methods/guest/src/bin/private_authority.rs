//! Atomic deshield, authorized app action, and re-shield of an authority NFT.
use lee_core::program::{ProgramOutput, read_lee_inputs};
use private_flow_core::{PrivateAuthorityInstruction, authority_action};

fn main() {
    let (input, words) = read_lee_inputs::<PrivateAuthorityInstruction>();
    let (posts, calls) = authority_action(input.pre_states.clone(), input.instruction);
    ProgramOutput::new(
        input.self_program_id,
        input.caller_program_id,
        words,
        input.pre_states,
        posts,
    )
    .with_chained_calls(calls)
    .write();
}
