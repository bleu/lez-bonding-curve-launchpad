//! Guest-facing instruction dispatch. Account ordering is part of the public wire interface.

use lee_core::{
    account::{Account, AccountWithMetadata},
    program::{AccountPostState, ChainedCall, ProgramId},
};

use associated_token_account_core::Instruction as AtaInstruction;

use crate::{
    ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID, Instruction, PoolAccount, compute_pool_pda_seed,
    pool_create::create_pool,
    pool_lifecycle::{close_pool, withdraw_reserves},
    pool_swap::{swap_exact_input, swap_exact_output},
    update_config::update_config,
};

#[must_use]
pub fn process_instruction(
    pre_states: Vec<AccountWithMetadata>,
    instruction: Instruction,
    self_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let (private, instruction) = match instruction {
        Instruction::Private { instruction } => (true, *instruction),
        instruction => (false, instruction),
    };
    let original = pre_states.clone();
    let (posts, mut calls) = match instruction {
        Instruction::Private { .. } => panic!("nested execution mode wrapper"),
        Instruction::UpdateConfig {
            namespace,
            admin,
            protocol_fee_bps,
            treasury,
        } => {
            let [config, authority] = pre_states
                .try_into()
                .expect("UpdateConfig requires exactly two accounts");
            (
                update_config(
                    namespace,
                    config,
                    authority,
                    admin,
                    protocol_fee_bps,
                    treasury,
                    self_program_id,
                ),
                vec![],
            )
        }
        Instruction::RenounceAdmin { namespace } => {
            let [config, authority] = pre_states
                .try_into()
                .expect("RenounceAdmin requires exactly two accounts");
            (
                crate::update_config::renounce_admin(namespace, config, authority, self_program_id),
                vec![],
            )
        }
        Instruction::CreatePool {
            defer_funding,
            namespace,
            token0_amount,
            token1_amount,
            virtual_reserve0,
            virtual_reserve1,
            close_timestamp,
            close_on_depletion,
            owner,
            owner_program,
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
                clock,
                config,
            ] = pre_states
                .try_into()
                .expect("CreatePool requires exactly ten accounts");
            crate::pool_swap::validated_config(&config, namespace, self_program_id);
            let (mut posts, calls) = create_pool(
                namespace,
                pool,
                owner_authority,
                token0_definition,
                token1_definition,
                owner_token0_ata,
                owner_token1_ata,
                pool_token0_ata,
                pool_token1_ata,
                clock.clone(),
                token0_amount,
                token1_amount,
                virtual_reserve0,
                virtual_reserve1,
                close_timestamp,
                close_on_depletion.map(Into::into),
                owner,
                curve_program_id,
                owner_program,
                defer_funding,
            );
            posts.push(AccountPostState::new(clock.account));
            posts.push(AccountPostState::new(config.account));
            (posts, calls)
        }
        Instruction::ActivatePool => crate::pool_create::activate_pool(pre_states, self_program_id),
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
            let [pool, owner, clock] = pre_states
                .try_into()
                .expect("ClosePool requires exactly three accounts");
            (close_pool(pool, owner, clock, self_program_id), vec![])
        }
        Instruction::SwapExactOutput {
            amount_out,
            max_amount_in,
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
                .expect("SwapExactOutput requires exactly nine accounts");
            settle_exact_output(
                pool,
                config,
                participant,
                participant_token_in_ata,
                pool_token_in_ata,
                pool_token_out_ata,
                participant_token_out_ata,
                treasury_token_in_ata,
                clock,
                amount_out,
                max_amount_in,
                token_in,
                self_program_id,
            )
        }
        Instruction::WithdrawReserves => {
            let [
                pool,
                owner,
                owner_token0_ata,
                owner_token1_ata,
                pool_token0_ata,
                pool_token1_ata,
                clock,
            ] = pre_states
                .try_into()
                .expect("WithdrawReserves requires exactly seven accounts");
            settle_withdrawal(
                pool,
                owner,
                owner_token0_ata,
                owner_token1_ata,
                pool_token0_ata,
                pool_token1_ata,
                clock,
                self_program_id,
            )
        }
    };
    if !private {
        use_public_authorizations(&original, &mut calls, self_program_id);
    }
    (posts, calls)
}

#[expect(
    clippy::too_many_arguments,
    reason = "withdrawal has fixed public accounts"
)]
fn settle_withdrawal(
    pool: AccountWithMetadata,
    owner: AccountWithMetadata,
    owner_token0_ata: AccountWithMetadata,
    owner_token1_ata: AccountWithMetadata,
    pool_token0_ata: AccountWithMetadata,
    pool_token1_ata: AccountWithMetadata,
    clock: AccountWithMetadata,
    curve_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let mut complete_posts = vec![
        AccountPostState::new(pool.account.clone()),
        AccountPostState::new(owner.account.clone()),
        AccountPostState::new(owner_token0_ata.account.clone()),
        AccountPostState::new(owner_token1_ata.account.clone()),
        AccountPostState::new(pool_token0_ata.account.clone()),
        AccountPostState::new(pool_token1_ata.account.clone()),
        AccountPostState::new(clock.account.clone()),
    ];
    let pool_state =
        PoolAccount::try_from(&pool.account.data).expect("Pool account holds valid data");
    let (posts, reserves) = withdraw_reserves(pool.clone(), owner.clone(), clock, curve_program_id);
    let authority = AccountWithMetadata {
        account: posts[0].account().clone(),
        is_authorized: false,
        account_id: pool.account_id,
    };
    associated_token_account_core::verify_ata_and_get_seed(
        &owner_token0_ata,
        &owner,
        pool_state.token0_definition_id,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &owner_token1_ata,
        &owner,
        pool_state.token1_definition_id,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &pool_token0_ata,
        &authority,
        pool_state.token0_definition_id,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &pool_token1_ata,
        &authority,
        pool_state.token1_definition_id,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    let mut signer = authority;
    signer.is_authorized = true;
    let seeds = vec![compute_pool_pda_seed(
        pool_state.namespace,
        pool_state.token0_definition_id,
        pool_state.token1_definition_id,
        pool_state.owner,
    )];
    let mut calls = Vec::new();
    if reserves.token0_amount != 0 {
        calls.push(
            ata_transfer(
                signer.clone(),
                pool_token0_ata,
                owner_token0_ata,
                reserves.token0_amount,
            )
            .with_pda_seeds(seeds.clone()),
        );
    }
    if reserves.token1_amount != 0 {
        calls.push(
            ata_transfer(
                signer,
                pool_token1_ata,
                owner_token1_ata,
                reserves.token1_amount,
            )
            .with_pda_seeds(seeds),
        );
    }
    complete_posts[0] = posts[0].clone();
    (complete_posts, calls)
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
    let mut complete_posts = vec![
        AccountPostState::new(pool.account.clone()),
        AccountPostState::new(config.account.clone()),
        AccountPostState::new(participant.account.clone()),
        AccountPostState::new(participant_token_in_ata.account.clone()),
        AccountPostState::new(pool_token_in_ata.account.clone()),
        AccountPostState::new(pool_token_out_ata.account.clone()),
        AccountPostState::new(participant_token_out_ata.account.clone()),
        AccountPostState::new(treasury_token_in_ata.account.clone()),
        AccountPostState::new(clock.account.clone()),
    ];
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
        account: posts[0].account().clone(),
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
        if settlement.protocol_fee_on_output {
            settlement.token_out
        } else {
            settlement.token_in
        },
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );

    let mut calls = vec![ata_transfer(
        participant.clone(),
        participant_token_in_ata.clone(),
        pool_token_in_ata,
        settlement.effective_amount_in,
    )];
    if settlement.protocol_fee != 0 && !settlement.protocol_fee_on_output {
        calls.push(ata_transfer(
            participant,
            debited(participant_token_in_ata, settlement.effective_amount_in),
            treasury_token_in_ata.clone(),
            settlement.protocol_fee,
        ));
    }
    let mut pool_signer = pool_authority;
    pool_signer.is_authorized = true;
    calls.push(
        ata_transfer(
            pool_signer.clone(),
            pool_token_out_ata.clone(),
            participant_token_out_ata,
            settlement.amount_out,
        )
        .with_pda_seeds(vec![compute_pool_pda_seed(
            pool_state.namespace,
            pool_state.token0_definition_id,
            pool_state.token1_definition_id,
            pool_state.owner,
        )]),
    );
    if settlement.protocol_fee != 0 && settlement.protocol_fee_on_output {
        calls.push(
            ata_transfer(
                pool_signer,
                debited(pool_token_out_ata, settlement.amount_out),
                treasury_token_in_ata,
                settlement.protocol_fee,
            )
            .with_pda_seeds(vec![compute_pool_pda_seed(
                pool_state.namespace,
                pool_state.token0_definition_id,
                pool_state.token1_definition_id,
                pool_state.owner,
            )]),
        );
    }
    complete_posts[0] = posts[0].clone();
    (complete_posts, calls)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the public swap account list is explicit"
)]
fn settle_exact_output(
    pool: AccountWithMetadata,
    config: AccountWithMetadata,
    participant: AccountWithMetadata,
    participant_token_in_ata: AccountWithMetadata,
    pool_token_in_ata: AccountWithMetadata,
    pool_token_out_ata: AccountWithMetadata,
    participant_token_out_ata: AccountWithMetadata,
    treasury_token_in_ata: AccountWithMetadata,
    clock: AccountWithMetadata,
    amount_out: u128,
    max_amount_in: u128,
    token_in: lee_core::account::AccountId,
    curve_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let mut complete_posts = vec![
        AccountPostState::new(pool.account.clone()),
        AccountPostState::new(config.account.clone()),
        AccountPostState::new(participant.account.clone()),
        AccountPostState::new(participant_token_in_ata.account.clone()),
        AccountPostState::new(pool_token_in_ata.account.clone()),
        AccountPostState::new(pool_token_out_ata.account.clone()),
        AccountPostState::new(participant_token_out_ata.account.clone()),
        AccountPostState::new(treasury_token_in_ata.account.clone()),
        AccountPostState::new(clock.account.clone()),
    ];
    assert!(
        participant.is_authorized,
        "Participant authorization is missing"
    );
    let pool_state =
        PoolAccount::try_from(&pool.account.data).expect("Pool account holds valid data");
    let (posts, settlement) = swap_exact_output(
        pool.clone(),
        config,
        Some(clock),
        amount_out,
        max_amount_in,
        token_in,
        curve_program_id,
    );
    let pool_authority = AccountWithMetadata {
        account: posts[0].account().clone(),
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
        if settlement.protocol_fee_on_output {
            settlement.token_out
        } else {
            settlement.token_in
        },
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    let mut calls = vec![ata_transfer(
        participant.clone(),
        participant_token_in_ata.clone(),
        pool_token_in_ata,
        settlement.effective_amount_in,
    )];
    if settlement.protocol_fee != 0 && !settlement.protocol_fee_on_output {
        calls.push(ata_transfer(
            participant,
            debited(participant_token_in_ata, settlement.effective_amount_in),
            treasury_token_in_ata.clone(),
            settlement.protocol_fee,
        ));
    }
    let mut pool_signer = pool_authority;
    pool_signer.is_authorized = true;
    calls.push(
        ata_transfer(
            pool_signer.clone(),
            pool_token_out_ata.clone(),
            participant_token_out_ata,
            settlement.amount_out,
        )
        .with_pda_seeds(vec![compute_pool_pda_seed(
            pool_state.namespace,
            pool_state.token0_definition_id,
            pool_state.token1_definition_id,
            pool_state.owner,
        )]),
    );
    if settlement.protocol_fee != 0 && settlement.protocol_fee_on_output {
        calls.push(
            ata_transfer(
                pool_signer,
                debited(pool_token_out_ata, settlement.amount_out),
                treasury_token_in_ata,
                settlement.protocol_fee,
            )
            .with_pda_seeds(vec![compute_pool_pda_seed(
                pool_state.namespace,
                pool_state.token0_definition_id,
                pool_state.token1_definition_id,
                pool_state.owner,
            )]),
        );
    }
    complete_posts[0] = posts[0].clone();
    (complete_posts, calls)
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

fn debited(mut holding: AccountWithMetadata, amount: u128) -> AccountWithMetadata {
    let mut token =
        token_core::TokenHolding::try_from(&holding.account.data).expect("valid token holding");
    let token_core::TokenHolding::Fungible { balance, .. } = &mut token else {
        panic!("pool tokens must be fungible")
    };
    *balance = balance
        .checked_sub(amount)
        .expect("sufficient token balance");
    holding.account.data = lee_core::account::Data::from(&token);
    holding.is_authorized = true;
    holding
}

/// Public LEZ calls inherit authorization from their caller, not preceding siblings.
/// State snapshots stay sequential; only the authorization metadata differs from the
/// pinned privacy circuit's transaction-wide authorization set.
pub fn use_public_authorizations(
    pre: &[AccountWithMetadata],
    calls: &mut [ChainedCall],
    program: ProgramId,
) {
    for call in calls {
        for account in &mut call.pre_states {
            account.is_authorized = pre
                .iter()
                .any(|p| p.account_id == account.account_id && p.is_authorized)
                || call.pda_seeds.iter().any(|seed| {
                    lee_core::account::AccountId::for_public_pda(&program, seed)
                        == account.account_id
                });
        }
    }
}
