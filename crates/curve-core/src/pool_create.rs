//! `create_pool`: opens neutral pool state and atomically establishes ATA custody.

use associated_token_account_core::Instruction as AtaInstruction;
use lee_core::{
    account::{Account, AccountId, AccountWithMetadata, Data},
    program::{AccountPostState, ChainedCall, Claim, ProgramId},
};
use pool::{Pool, TokenSide};
use token_core::{TokenDefinition, TokenHolding};

use crate::{
    ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID, PoolAccount, compute_pool_pda, compute_pool_pda_seed,
};

#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "the public instruction has eight accounts and explicit pool parameters"
)]
pub fn create_pool(
    namespace: AccountId,
    pool_account: AccountWithMetadata,
    owner: AccountWithMetadata,
    token0_definition: AccountWithMetadata,
    token1_definition: AccountWithMetadata,
    owner_token0_ata: AccountWithMetadata,
    owner_token1_ata: AccountWithMetadata,
    pool_token0_ata: AccountWithMetadata,
    pool_token1_ata: AccountWithMetadata,
    clock: AccountWithMetadata,
    token0_amount: u128,
    token1_amount: u128,
    virtual_reserve0: u128,
    virtual_reserve1: u128,
    close_timestamp: Option<u64>,
    close_on_depletion: Option<TokenSide>,
    expected_owner: AccountId,
    curve_program_id: ProgramId,
    owner_program: Option<(ProgramId, [u8; 32])>,
    defer_funding: bool,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let ata_program_id = ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID;
    assert!(owner.is_authorized, "Owner authorization is missing");
    assert_eq!(
        crate::authority::pool_owner(&owner, owner_program),
        expected_owner,
        "Authorized account is not the selected pool owner"
    );
    assert_ne!(
        token0_definition.account_id, token1_definition.account_id,
        "Pool tokens must differ"
    );
    assert_ne!(
        expected_owner,
        AccountId::default(),
        "Pool owner must not be the default key"
    );
    assert_eq!(
        pool_account.account_id,
        compute_pool_pda(
            namespace,
            curve_program_id,
            token0_definition.account_id,
            token1_definition.account_id,
            expected_owner,
        ),
        "Pool account ID does not match PDA"
    );
    assert_eq!(
        pool_account.account,
        Account::default(),
        "Pool is already initialized"
    );

    associated_token_account_core::verify_ata_and_get_seed(
        &owner_token0_ata,
        &owner,
        token0_definition.account_id,
        ata_program_id,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &owner_token1_ata,
        &owner,
        token1_definition.account_id,
        ata_program_id,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &pool_token0_ata,
        &pool_account,
        token0_definition.account_id,
        ata_program_id,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &pool_token1_ata,
        &pool_account,
        token1_definition.account_id,
        ata_program_id,
    );

    assert_eq!(
        clock.account_id,
        clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
        "Clock account is not the trusted LEZ clock"
    );
    let now = clock_core::ClockAccountData::from_bytes(clock.account.data.as_ref()).timestamp;
    if let Some(close) = close_timestamp {
        assert!(
            defer_funding || close > now,
            "Close timestamp must be in the future"
        );
    }
    let pool = Pool::create(
        token0_amount,
        token1_amount,
        virtual_reserve0,
        virtual_reserve1,
        close_timestamp,
        close_on_depletion,
    )
    .expect("Pool parameters are invalid");
    let mut pool_post = pool_account.account;
    pool_post.data = Data::from(&PoolAccount {
        funded: !defer_funding,
        namespace,
        token0_definition_id: token0_definition.account_id,
        token1_definition_id: token1_definition.account_id,
        owner: expected_owner,
        owner_program,
        pool,
    });

    let pool_owner = AccountWithMetadata {
        account: Account {
            program_owner: curve_program_id,
            ..pool_post.clone()
        },
        is_authorized: false,
        account_id: pool_account.account_id,
    };
    let mut chained_calls = vec![
        ChainedCall::new(
            ata_program_id,
            vec![
                pool_owner.clone(),
                token0_definition.clone(),
                pool_token0_ata.clone(),
            ],
            &AtaInstruction::Create { ata_program_id },
        ),
        ChainedCall::new(
            ata_program_id,
            vec![
                pool_owner,
                token1_definition.clone(),
                pool_token1_ata.clone(),
            ],
            &AtaInstruction::Create { ata_program_id },
        ),
    ];

    if !defer_funding && token0_amount != 0 {
        chained_calls.push(funding_call(
            &owner,
            &owner_token0_ata,
            &pool_token0_ata,
            &token0_definition,
            token0_amount,
            ata_program_id,
        ));
    }
    if !defer_funding && token1_amount != 0 {
        chained_calls.push(funding_call(
            &owner,
            &owner_token1_ata,
            &pool_token1_ata,
            &token1_definition,
            token1_amount,
            ata_program_id,
        ));
    }

    let post_states = vec![
        AccountPostState::new_claimed_if_default(
            pool_post,
            Claim::Pda(compute_pool_pda_seed(
                namespace,
                token0_definition.account_id,
                token1_definition.account_id,
                expected_owner,
            )),
        ),
        AccountPostState::new(owner.account),
        AccountPostState::new(token0_definition.account),
        AccountPostState::new(token1_definition.account),
        AccountPostState::new(owner_token0_ata.account),
        AccountPostState::new(owner_token1_ata.account),
        AccountPostState::new(pool_token0_ata.account),
        AccountPostState::new(pool_token1_ata.account),
    ];

    (post_states, chained_calls)
}

fn funding_call(
    owner: &AccountWithMetadata,
    source: &AccountWithMetadata,
    destination: &AccountWithMetadata,
    definition_account: &AccountWithMetadata,
    amount: u128,
    ata_program_id: ProgramId,
) -> ChainedCall {
    let initialized_destination = after_ata_creation(destination, definition_account);
    ChainedCall::new(
        ata_program_id,
        vec![owner.clone(), source.clone(), initialized_destination],
        &AtaInstruction::Transfer {
            ata_program_id,
            amount,
        },
    )
}

/// Completes a prepared pool in a separate atomic funding transaction.
pub fn activate_pool(
    pre: Vec<AccountWithMetadata>,
    curve_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let [
        pool,
        owner,
        definition0,
        definition1,
        source0,
        source1,
        reserve0,
        reserve1,
        clock,
        config,
    ]: [_; 10] = pre
        .clone()
        .try_into()
        .expect("ActivatePool requires ten accounts");
    let mut state = PoolAccount::try_from(&pool.account.data).expect("valid prepared pool");
    assert!(!state.funded, "Pool is already funded");
    assert_eq!(
        crate::authority::pool_owner(&owner, state.owner_program),
        state.owner,
        "Authority is not the pool owner"
    );
    assert_eq!(
        pool.account_id,
        crate::compute_pool_pda(
            state.namespace,
            curve_program_id,
            state.token0_definition_id,
            state.token1_definition_id,
            state.owner
        ),
        "Pool account ID does not match PDA"
    );
    assert_eq!(
        definition0.account_id, state.token0_definition_id,
        "Wrong token0 definition"
    );
    assert_eq!(
        definition1.account_id, state.token1_definition_id,
        "Wrong token1 definition"
    );
    crate::pool_swap::validated_config(&config, state.namespace, curve_program_id);
    assert_eq!(
        clock.account_id,
        clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
        "Trusted clock required"
    );
    let now = clock_core::ClockAccountData::from_bytes(clock.account.data.as_ref()).timestamp;
    let mut calls = vec![];
    for (source, reserve, definition, amount) in [
        (&source0, &reserve0, &definition0, state.pool.real_reserve0),
        (&source1, &reserve1, &definition1, state.pool.real_reserve1),
    ] {
        associated_token_account_core::verify_ata_and_get_seed(
            source,
            &owner,
            definition.account_id,
            ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
        );
        associated_token_account_core::verify_ata_and_get_seed(
            reserve,
            &pool,
            definition.account_id,
            ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
        );
        assert_ne!(
            reserve.account,
            Account::default(),
            "Reserve ATA must be prepared"
        );
        if amount != 0 {
            calls.push(funding_call(
                &owner,
                source,
                reserve,
                definition,
                amount,
                ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
            ));
        }
    }
    state.funded = true;
    // A resumed launch keeps its original deadline. Funding after expiry produces a
    // closed pool whose reserves can immediately be recovered through withdrawal.
    if state
        .pool
        .close_timestamp
        .is_some_and(|deadline| now >= deadline)
    {
        state.pool.lifecycle = pool::PoolLifecycle::Closed;
    }
    let mut posts: Vec<_> = pre
        .into_iter()
        .map(|p| AccountPostState::new(p.account))
        .collect();
    posts[0].account_mut().data = Data::from(&state);
    (posts, calls)
}

/// Snapshot after the ATA guest's idempotent Create, including its nested PDA authorization.
pub fn after_ata_creation(
    ata: &AccountWithMetadata,
    definition: &AccountWithMetadata,
) -> AccountWithMetadata {
    if ata.account != Account::default() {
        return ata.clone();
    }
    let token =
        TokenDefinition::try_from(&definition.account.data).expect("valid token definition");
    AccountWithMetadata {
        account: Account {
            program_owner: definition.account.program_owner,
            data: Data::from(&TokenHolding::zeroized_from_definition(
                definition.account_id,
                &token,
            )),
            ..Account::default()
        },
        is_authorized: true,
        ..ata.clone()
    }
}
