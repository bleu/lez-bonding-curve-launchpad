//! Direction and config adapter for pool swaps.

use lee_core::{
    account::{AccountId, AccountWithMetadata},
    program::{AccountPostState, ProgramId},
};
use pool::TokenSide;

use crate::{Config, PoolAccount, compute_config_pda, compute_pool_pda};

/// Values the token-account adapter must settle atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapSettlement {
    pub token_in: AccountId,
    pub token_out: AccountId,
    pub amount_in: u128,
    pub amount_out: u128,
    pub fee: u128,
    pub treasury: AccountId,
}

#[must_use]
pub fn swap_exact_input(
    pool: AccountWithMetadata,
    config: AccountWithMetadata,
    clock: Option<AccountWithMetadata>,
    amount_in: u128,
    min_amount_out: u128,
    token_in: AccountId,
    curve_program_id: ProgramId,
) -> (Vec<AccountPostState>, SwapSettlement) {
    let config_data = validated_config(&config, curve_program_id);
    let mut pool_account = validated_pool(&pool, curve_program_id);
    let (side, token_out) = direction(&pool_account, token_in);
    let now = trusted_time(&pool_account, clock);
    let outcome = pool_account
        .pool
        .swap_exact_input(side, amount_in, min_amount_out, config_data.fee_bps, now)
        .expect("Exact-input swap is invalid");

    let mut post = pool.account;
    post.data = (&pool_account).into();
    (
        vec![AccountPostState::new(post)],
        SwapSettlement {
            token_in,
            token_out,
            amount_in: outcome.amount_in,
            amount_out: outcome.amount_out,
            fee: outcome.fee,
            treasury: config_data.treasury,
        },
    )
}

#[must_use]
pub fn swap_exact_output(
    pool: AccountWithMetadata,
    config: AccountWithMetadata,
    clock: Option<AccountWithMetadata>,
    amount_out: u128,
    max_amount_in: u128,
    token_in: AccountId,
    curve_program_id: ProgramId,
) -> (Vec<AccountPostState>, SwapSettlement) {
    let config_data = validated_config(&config, curve_program_id);
    let mut pool_account = validated_pool(&pool, curve_program_id);
    let (side, token_out) = direction(&pool_account, token_in);
    let now = trusted_time(&pool_account, clock);
    let outcome = pool_account
        .pool
        .swap_exact_output(side, amount_out, max_amount_in, config_data.fee_bps, now)
        .expect("Exact-output swap is invalid");

    let mut post = pool.account;
    post.data = (&pool_account).into();
    (
        vec![AccountPostState::new(post)],
        SwapSettlement {
            token_in,
            token_out,
            amount_in: outcome.amount_in,
            amount_out: outcome.amount_out,
            fee: outcome.fee,
            treasury: config_data.treasury,
        },
    )
}

fn validated_config(config: &AccountWithMetadata, curve_program_id: ProgramId) -> Config {
    assert_eq!(
        config.account_id,
        compute_config_pda(curve_program_id),
        "Config account ID does not match PDA"
    );
    Config::try_from(&config.account.data).expect("Config account holds invalid data")
}

fn validated_pool(pool: &AccountWithMetadata, curve_program_id: ProgramId) -> PoolAccount {
    let pool_account =
        PoolAccount::try_from(&pool.account.data).expect("Pool account holds invalid data");
    assert_eq!(
        pool.account_id,
        compute_pool_pda(
            curve_program_id,
            pool_account.token0_definition_id,
            pool_account.token1_definition_id,
        ),
        "Pool account ID does not match PDA"
    );
    pool_account
}

fn direction(pool: &PoolAccount, token_in: AccountId) -> (TokenSide, AccountId) {
    if token_in == pool.token0_definition_id {
        (TokenSide::Token0, pool.token1_definition_id)
    } else if token_in == pool.token1_definition_id {
        (TokenSide::Token1, pool.token0_definition_id)
    } else {
        panic!("token_in is not a token definition for the pool")
    }
}

fn trusted_time(pool: &PoolAccount, clock: Option<AccountWithMetadata>) -> u64 {
    if pool.pool.close_timestamp.is_none() {
        return 0;
    }
    let clock = clock.expect("Trusted LEZ clock is required for a pool with expiry");
    assert_eq!(
        clock.account_id,
        clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
        "Clock account is not the trusted LEZ clock"
    );
    clock_core::ClockAccountData::from_bytes(clock.account.data.as_ref()).timestamp
}
