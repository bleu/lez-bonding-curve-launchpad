//! Guest-facing instruction dispatch. Account ordering is part of the public wire interface.

use lee_core::{
    account::{Account, AccountWithMetadata},
    program::{AccountPostState, ChainedCall, ProgramId},
};

use associated_token_account_core::Instruction as AtaInstruction;

use crate::{
    ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID, Instruction, PoolAccount, compute_pool_pda_seed,
    pool_create::create_pool, pool_lifecycle::close_pool, pool_swap::swap_exact_input,
    update_config::update_config,
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
        Instruction::SwapExactInput {
            amount_in,
            min_amount_out,
            token_in,
        } => {
            let [
                pool,
                config,
                participant,
                participant_token_in_ata,
                pool_token_in_ata,
                pool_token_out_ata,
                participant_token_out_ata,
                treasury_token_in_ata,
                clock,
            ] = pre_states
                .try_into()
                .expect("SwapExactInput requires exactly nine accounts");
            settle_exact_input(
                pool,
                config,
                participant,
                participant_token_in_ata,
                pool_token_in_ata,
                pool_token_out_ata,
                participant_token_out_ata,
                treasury_token_in_ata,
                clock,
                amount_in,
                min_amount_out,
                token_in,
                self_program_id,
            )
        }
        Instruction::ClosePool => {
            let [pool, owner] = pre_states
                .try_into()
                .expect("ClosePool requires exactly two accounts");
            (close_pool(pool, owner, self_program_id), vec![])
        }
        Instruction::SwapExactOutput { .. } | Instruction::WithdrawReserves => {
            panic!("instruction handler is not implemented yet")
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the public swap account list is explicit"
)]
fn settle_exact_input(
    pool: AccountWithMetadata,
    config: AccountWithMetadata,
    participant: AccountWithMetadata,
    participant_token_in_ata: AccountWithMetadata,
    pool_token_in_ata: AccountWithMetadata,
    pool_token_out_ata: AccountWithMetadata,
    participant_token_out_ata: AccountWithMetadata,
    treasury_token_in_ata: AccountWithMetadata,
    clock: AccountWithMetadata,
    amount_in: u128,
    min_amount_out: u128,
    token_in: lee_core::account::AccountId,
    curve_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    assert!(
        participant.is_authorized,
        "Participant authorization is missing"
    );
    let pool_state =
        PoolAccount::try_from(&pool.account.data).expect("Pool account holds valid data");
    let (posts, settlement) = swap_exact_input(
        pool.clone(),
        config,
        Some(clock),
        amount_in,
        min_amount_out,
        token_in,
        curve_program_id,
    );

    let pool_authority = AccountWithMetadata {
        account: pool.account.clone(),
        is_authorized: false,
        account_id: pool.account_id,
    };
    associated_token_account_core::verify_ata_and_get_seed(
        &participant_token_in_ata,
        &participant,
        settlement.token_in,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &participant_token_out_ata,
        &participant,
        settlement.token_out,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &pool_token_in_ata,
        &pool_authority,
        settlement.token_in,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &pool_token_out_ata,
        &pool_authority,
        settlement.token_out,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    let treasury = AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: settlement.treasury,
    };
    associated_token_account_core::verify_ata_and_get_seed(
        &treasury_token_in_ata,
        &treasury,
        settlement.token_in,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );

    let retained_input = settlement
        .effective_amount_in
        .checked_add(settlement.pool_fee)
        .expect("validated swap amounts fit u128");
    let mut calls = vec![ata_transfer(
        participant.clone(),
        participant_token_in_ata.clone(),
        pool_token_in_ata,
        retained_input,
    )];
    if settlement.protocol_fee != 0 {
        calls.push(ata_transfer(
            participant,
            participant_token_in_ata,
            treasury_token_in_ata,
            settlement.protocol_fee,
        ));
    }
    let mut pool_signer = pool_authority;
    pool_signer.is_authorized = true;
    calls.push(
        ata_transfer(
            pool_signer,
            pool_token_out_ata,
            participant_token_out_ata,
            settlement.amount_out,
        )
        .with_pda_seeds(vec![compute_pool_pda_seed(
            pool_state.token0_definition_id,
            pool_state.token1_definition_id,
            pool_state.owner,
        )]),
    );
    (posts, calls)
}

fn ata_transfer(
    owner: AccountWithMetadata,
    source: AccountWithMetadata,
    destination: AccountWithMetadata,
    amount: u128,
) -> ChainedCall {
    ChainedCall::new(
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
        vec![owner, source, destination],
        &AtaInstruction::Transfer {
            ata_program_id: ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
            amount,
        },
    )
}
