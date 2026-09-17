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
    let (mut posts, mut calls) = match instruction {
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
                clock,
            ] = pre_states
                .try_into()
                .expect("SwapExactInput requires exactly eight accounts");
            settle_exact_input(
                pool,
                config,
                participant,
                participant_token_in_ata,
                pool_token_in_ata,
                pool_token_out_ata,
                participant_token_out_ata,
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
                clock,
            ] = pre_states
                .try_into()
                .expect("SwapExactOutput requires exactly eight accounts");
            settle_exact_output(
                pool,
                config,
                participant,
                participant_token_in_ata,
                pool_token_in_ata,
                pool_token_out_ata,
                participant_token_out_ata,
                clock,
                amount_out,
                max_amount_in,
                token_in,
                self_program_id,
            )
        }
        Instruction::CollectFees => collect_fees(pre_states, self_program_id),
        Instruction::WithdrawReserves => {
            // The runtime requires unique account IDs. Omit the trailing treasury
            // ATA when it is already the owner's token1 recipient.
            let mut pre_states = pre_states;
            if pre_states.len() == 8 {
                pre_states.push(pre_states[3].clone());
            }

            let [
                pool,
                owner,
                owner_token0_ata,
                owner_token1_ata,
                pool_token0_ata,
                pool_token1_ata,
                clock,
                config,
                treasury_ata,
            ] = pre_states
                .try_into()
                .expect("WithdrawReserves requires exactly nine accounts");
            settle_withdrawal(
                pool,
                owner,
                owner_token0_ata,
                owner_token1_ata,
                pool_token0_ata,
                pool_token1_ata,
                clock,
                config,
                treasury_ata,
                self_program_id,
            )
        }
    };
    posts.truncate(original.len());
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
    config: AccountWithMetadata,
    treasury_ata: AccountWithMetadata,
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
        AccountPostState::new(config.account.clone()),
        AccountPostState::new(treasury_ata.account.clone()),
    ];
    let pool_state =
        PoolAccount::try_from(&pool.account.data).expect("Pool account holds valid data");
    verify_vaults(
        &pool_state,
        &pool_token0_ata,
        &pool_token1_ata,
        pool_state.token0_definition_id,
    );
    let treasury_is_owner = treasury_ata.account_id == owner_token1_ata.account_id;
    let (fee_posts, fee_calls) = collect_fees(
        vec![pool.clone(), config, pool_token1_ata.clone(), treasury_ata],
        curve_program_id,
    );
    let collected = pool_state.pool.fees_accrued;
    let pool = AccountWithMetadata {
        account: fee_posts[0].account().clone(),
        ..pool
    };
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
    let mut calls = fee_calls;
    for call in &mut calls {
        call.pre_states[0].account = signer.account.clone();
    }
    let pool_token1_ata = if collected == 0 {
        pool_token1_ata
    } else {
        debited(pool_token1_ata, collected)
    };
    let owner_token1_ata = if treasury_is_owner && collected != 0 {
        credited(owner_token1_ata, collected)
    } else {
        owner_token1_ata
    };
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
    verify_vaults(
        &pool_state,
        &pool_token_in_ata,
        &pool_token_out_ata,
        token_in,
    );
    let mut calls = vec![ata_transfer(
        participant,
        participant_token_in_ata,
        pool_token_in_ata,
        settlement.amount_in,
    )];
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
    verify_vaults(
        &pool_state,
        &pool_token_in_ata,
        &pool_token_out_ata,
        token_in,
    );
    let mut calls = vec![ata_transfer(
        participant,
        participant_token_in_ata,
        pool_token_in_ata,
        settlement.amount_in,
    )];
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

fn credited(mut holding: AccountWithMetadata, amount: u128) -> AccountWithMetadata {
    let mut token =
        token_core::TokenHolding::try_from(&holding.account.data).expect("valid token holding");
    let token_core::TokenHolding::Fungible { balance, .. } = &mut token else {
        panic!("pool tokens must be fungible")
    };
    *balance = balance.checked_add(amount).expect("token balance overflow");
    holding.account.data = lee_core::account::Data::from(&token);
    holding
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

/// Checks physical custody against independent reserve and fee liabilities.
fn verify_vaults(
    state: &PoolAccount,
    input: &AccountWithMetadata,
    output: &AccountWithMetadata,
    token_in: lee_core::account::AccountId,
) {
    let (token0, token1) = if token_in == state.token0_definition_id {
        (input, output)
    } else {
        (output, input)
    };
    assert!(
        balance(token0) >= state.pool.real_reserve0,
        "Token0 vault is underfunded"
    );
    assert!(
        balance(token1)
            >= state
                .pool
                .real_reserve1
                .checked_add(state.pool.fees_accrued)
                .expect("collateral liabilities overflow"),
        "Collateral vault cannot cover reserves and accrued fees"
    );
}
fn balance(account: &AccountWithMetadata) -> u128 {
    assert_eq!(
        account.account.program_owner,
        crate::authority::TOKEN_PROGRAM_ID,
        "Invalid vault token program"
    );
    match token_core::TokenHolding::try_from(&account.account.data).expect("valid fungible vault") {
        token_core::TokenHolding::Fungible { balance, .. } => balance,
        _ => panic!("pool tokens must be fungible"),
    }
}
fn collect_fees(
    pre: Vec<AccountWithMetadata>,
    program: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let [pool, config, vault, treasury_ata]: [AccountWithMetadata; 4] =
        pre.try_into().expect("CollectFees requires four accounts");
    let mut state = crate::pool_swap::validated_pool(&pool, program);
    let config_data = crate::pool_swap::validated_config(&config, state.namespace, program);
    let treasury = AccountWithMetadata {
        account_id: config_data.treasury,
        account: Account::default(),
        is_authorized: false,
    };
    for (ata, owner) in [(&vault, &pool), (&treasury_ata, &treasury)] {
        associated_token_account_core::verify_ata_and_get_seed(
            ata,
            owner,
            state.token1_definition_id,
            ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
        );
    }
    assert!(
        balance(&vault)
            >= state
                .pool
                .real_reserve1
                .checked_add(state.pool.fees_accrued)
                .expect("collateral liabilities overflow"),
        "Collateral vault cannot cover reserves and accrued fees"
    );
    let amount = state.pool.collect_fees().expect("collected fees overflow");
    let mut post = pool.account.clone();
    post.data = (&state).into();
    let signer = AccountWithMetadata {
        account: post.clone(),
        is_authorized: true,
        ..pool
    };
    let posts = vec![
        AccountPostState::new(post),
        AccountPostState::new(config.account),
        AccountPostState::new(vault.account.clone()),
        AccountPostState::new(treasury_ata.account.clone()),
    ];
    let calls = if amount == 0 {
        vec![]
    } else {
        vec![
            ata_transfer(signer, vault, treasury_ata, amount).with_pda_seeds(vec![
                compute_pool_pda_seed(
                    state.namespace,
                    state.token0_definition_id,
                    state.token1_definition_id,
                    state.owner,
                ),
            ]),
        ]
    };
    (posts, calls)
}
