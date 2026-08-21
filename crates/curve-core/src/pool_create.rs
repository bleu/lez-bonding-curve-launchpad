//! Pool creation and the funding amounts consumed by the token adapter.

use lee_core::{
    account::{Account, AccountId, AccountWithMetadata, Data},
    program::{AccountPostState, Claim, ProgramId},
};
use pool::Pool;

use crate::{PoolAccount, compute_pool_pda, compute_pool_pda_seed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolFunding {
    pub token0_amount: u128,
    pub token1_amount: u128,
}

#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "the public CreatePool fields are explicit"
)]
pub fn create_pool(
    pool: AccountWithMetadata,
    token0_definition_id: AccountId,
    token1_definition_id: AccountId,
    token0_amount: u128,
    token1_amount: u128,
    virtual_reserve0: u128,
    virtual_reserve1: u128,
    close_timestamp: Option<u64>,
    owner: AccountId,
    curve_program_id: ProgramId,
) -> (Vec<AccountPostState>, PoolFunding) {
    assert_eq!(
        pool.account,
        Account::default(),
        "Pool is already initialized"
    );
    assert_ne!(
        token0_definition_id, token1_definition_id,
        "Pool tokens must differ"
    );
    assert_ne!(
        owner,
        AccountId::default(),
        "Pool owner must not be the default key"
    );
    assert_eq!(
        pool.account_id,
        compute_pool_pda(curve_program_id, token0_definition_id, token1_definition_id),
        "Pool account ID does not match PDA"
    );

    let pool_state = Pool::create(
        token0_amount,
        token1_amount,
        virtual_reserve0,
        virtual_reserve1,
        close_timestamp,
    )
    .expect("Pool parameters are invalid");
    let account = PoolAccount {
        token0_definition_id,
        token1_definition_id,
        owner,
        pool: pool_state,
    };
    let mut post = pool.account;
    post.data = Data::from(&account);
    (
        vec![AccountPostState::new_claimed_if_default(
            post,
            Claim::Pda(compute_pool_pda_seed(
                token0_definition_id,
                token1_definition_id,
            )),
        )],
        PoolFunding {
            token0_amount,
            token1_amount,
        },
    )
}
