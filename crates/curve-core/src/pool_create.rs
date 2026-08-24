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
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let ata_program_id = ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID;
    assert!(owner.is_authorized, "Owner authorization is missing");
    assert_eq!(
        owner.account_id, expected_owner,
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
        assert!(close > now, "Close timestamp must be in the future");
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
        token0_definition_id: token0_definition.account_id,
        token1_definition_id: token1_definition.account_id,
        owner: expected_owner,
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

    if token0_amount != 0 {
        chained_calls.push(funding_call(
            &owner,
            &owner_token0_ata,
            &pool_token0_ata,
            &token0_definition,
            token0_amount,
            ata_program_id,
        ));
    }
    if token1_amount != 0 {
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
    let definition = TokenDefinition::try_from(&definition_account.account.data)
        .expect("token definition account must hold a valid definition");
    let initialized_destination = AccountWithMetadata {
        account: Account {
            program_owner: definition_account.account.program_owner,
            data: Data::from(&TokenHolding::zeroized_from_definition(
                definition_account.account_id,
                &definition,
            )),
            ..Account::default()
        },
        ..destination.clone()
    };
    ChainedCall::new(
        ata_program_id,
        vec![owner.clone(), source.clone(), initialized_destination],
        &AtaInstruction::Transfer {
            ata_program_id,
            amount,
        },
    )
}
