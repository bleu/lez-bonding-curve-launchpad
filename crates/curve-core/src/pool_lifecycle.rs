//! Owner-authorized lifecycle transitions for a bounded pool.

use lee_core::{
    account::AccountWithMetadata,
    program::{AccountPostState, ProgramId},
};

use crate::{PoolAccount, compute_pool_pda};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WithdrawnReserves {
    pub token0_amount: u128,
    pub token1_amount: u128,
}

#[must_use]
pub fn close_pool(
    pool: AccountWithMetadata,
    owner: AccountWithMetadata,
    clock: AccountWithMetadata,
    curve_program_id: ProgramId,
) -> Vec<AccountPostState> {
    assert!(owner.is_authorized, "Owner authorization is missing");
    let mut pool_account =
        PoolAccount::try_from(&pool.account.data).expect("Pool account holds invalid data");
    assert_eq!(
        owner.account_id, pool_account.owner,
        "Authority is not the pool owner"
    );
    assert_eq!(
        pool.account_id,
        compute_pool_pda(
            curve_program_id,
            pool_account.token0_definition_id,
            pool_account.token1_definition_id,
            pool_account.owner,
        ),
        "Pool account ID does not match PDA"
    );

    let now = trusted_time(clock);
    pool_account
        .pool
        .close_pool(now)
        .expect("Pool is already closed");
    let mut post = pool.account;
    post.data = (&pool_account).into();
    vec![AccountPostState::new(post)]
}

fn trusted_time(clock: AccountWithMetadata) -> u64 {
    assert_eq!(
        clock.account_id,
        clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
        "Clock account is not the trusted LEZ clock"
    );
    clock_core::ClockAccountData::from_bytes(clock.account.data.as_ref()).timestamp
}

/// Retires the pool and returns the exact amounts the token adapter must transfer.
/// Expiry is read only from the canonical one-block LEZ clock account.
#[must_use]
pub fn withdraw_reserves(
    pool: AccountWithMetadata,
    owner: AccountWithMetadata,
    clock: AccountWithMetadata,
    curve_program_id: ProgramId,
) -> (Vec<AccountPostState>, WithdrawnReserves) {
    assert!(owner.is_authorized, "Owner authorization is missing");
    let mut pool_account =
        PoolAccount::try_from(&pool.account.data).expect("Pool account holds invalid data");
    assert_eq!(
        owner.account_id, pool_account.owner,
        "Authority is not the pool owner"
    );
    assert_eq!(
        pool.account_id,
        compute_pool_pda(
            curve_program_id,
            pool_account.token0_definition_id,
            pool_account.token1_definition_id,
            pool_account.owner,
        ),
        "Pool account ID does not match PDA"
    );

    let now = trusted_time(clock);
    let (token0_amount, token1_amount) = pool_account
        .pool
        .withdraw_reserves(now)
        .expect("Pool must be closed or expired and not already retired");
    let mut post = pool.account;
    post.data = (&pool_account).into();
    (
        vec![AccountPostState::new(post)],
        WithdrawnReserves {
            token0_amount,
            token1_amount,
        },
    )
}
