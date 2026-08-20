//! Day 1 gate program (GTM-506): proves this repo can compile a guest, deploy
//! it, and have the sequencer execute it against real account state.
//!
//! Deliberately trivial — it appends its instruction bytes to a single account,
//! claiming that account when it is uninitialized. Delete this once the curve
//! program lands; it is scaffolding for the toolchain, not a domain concept.

use lee_core::program::{AccountPostState, Claim, ProgramInput, ProgramOutput, read_lee_inputs};

type Instruction = Vec<u8>;

fn main() {
    let (
        ProgramInput {
            self_program_id,
            caller_program_id,
            pre_states,
            instruction: payload,
        },
        instruction_data,
    ) = read_lee_inputs::<Instruction>();

    let [pre_state] = pre_states
        .try_into()
        .unwrap_or_else(|_| panic!("expected exactly one input account"));

    let post_account = {
        let mut account = pre_state.account.clone();
        let mut bytes = account.data.into_inner();
        bytes.extend_from_slice(&payload);
        account.data = bytes
            .try_into()
            .expect("payload exceeds account data limit");
        account
    };

    let post_state = AccountPostState::new_claimed_if_default(post_account, Claim::Authorized);

    ProgramOutput::new(
        self_program_id,
        caller_program_id,
        instruction_data,
        vec![pre_state],
        vec![post_state],
    )
    .write();
}
