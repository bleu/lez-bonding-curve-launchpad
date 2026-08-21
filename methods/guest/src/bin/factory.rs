//! Factory guest dispatch shim. Domain handlers live in `factory-core` for host tests.
use factory_core::{Instruction, process_instruction};
use lee_core::program::{ProgramInput, ProgramOutput, read_lee_inputs};

fn main() {
    let (input, instruction_words) = read_lee_inputs::<Instruction>();
    let ProgramInput {
        self_program_id,
        caller_program_id,
        pre_states,
        instruction,
    } = input;
    let pre_states_clone = pre_states.clone();
    let (post_states, chained_calls) =
        process_instruction(pre_states, instruction, self_program_id);
    ProgramOutput::new(
        self_program_id,
        caller_program_id,
        instruction_words,
        pre_states_clone,
        post_states,
    )
    .with_chained_calls(chained_calls)
    .write();
}
