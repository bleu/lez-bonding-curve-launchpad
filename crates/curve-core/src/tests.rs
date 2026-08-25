//! AMM-style handler tests: hand-built `AccountWithMetadata` fixtures, direct handler
//! calls, `#[should_panic]` on the gates. Mirrors `lez/programs/amm/src/tests.rs`.

use associated_token_account_core::{
    Instruction as AtaInstruction, compute_ata_seed, get_associated_token_account_id,
};
use lee_core::{
    account::{Account, AccountId, AccountWithMetadata, Data},
    program::{Claim, ProgramId},
};

use pool::Pool;
use token_core::{TokenDefinition, TokenHolding};

use crate::dispatch::process_instruction;
use crate::{
    ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID, Config, GENESIS_ADMIN, Instruction, PoolAccount,
    compute_config_pda, compute_config_pda_seed, compute_pool_pda,
    pool_create::create_pool,
    pool_lifecycle::{close_pool, withdraw_reserves},
    pool_swap::{swap_exact_input, swap_exact_output},
    update_config::update_config,
};

const CURVE_PROGRAM_ID: ProgramId = [7; 8];
const ATA_PROGRAM_ID: ProgramId = ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID;
const TOKEN_PROGRAM_ID: ProgramId = [9; 8];

fn uninitialized_config() -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: compute_config_pda(CURVE_PROGRAM_ID),
    }
}

fn signer(account_id: AccountId) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: true,
        account_id,
    }
}

fn genesis_admin_signer() -> AccountWithMetadata {
    signer(GENESIS_ADMIN)
}

fn initialized_config(admin: AccountId) -> AccountWithMetadata {
    let account = Account {
        program_owner: CURVE_PROGRAM_ID,
        data: Data::from(&Config {
            admin,
            protocol_fee_bps: 100,
            treasury: new_treasury(),
        }),
        ..Account::default()
    };
    AccountWithMetadata {
        account,
        is_authorized: false,
        account_id: compute_config_pda(CURVE_PROGRAM_ID),
    }
}

fn new_admin() -> AccountId {
    AccountId::new([1; 32])
}

fn new_treasury() -> AccountId {
    AccountId::new([2; 32])
}

fn intruder_signer() -> AccountWithMetadata {
    signer(AccountId::new([9; 32]))
}

fn token_definition(account_id: AccountId) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: TOKEN_PROGRAM_ID,
            data: Data::from(&TokenDefinition::Fungible {
                name: "Token".into(),
                total_supply: 10_000,
                metadata_id: None,
            }),
            ..Account::default()
        },
        is_authorized: false,
        account_id,
    }
}

fn ata(owner: AccountId, definition: AccountId, account: Account) -> AccountWithMetadata {
    let seed = compute_ata_seed(owner, definition);
    AccountWithMetadata {
        account,
        is_authorized: false,
        account_id: get_associated_token_account_id(&ATA_PROGRAM_ID, &seed),
    }
}

fn owner_source_ata(owner: AccountId, token_definition_id: AccountId) -> AccountWithMetadata {
    ata(
        owner,
        token_definition_id,
        Account {
            program_owner: TOKEN_PROGRAM_ID,
            data: Data::from(&TokenHolding::Fungible {
                definition_id: token_definition_id,
                balance: 1_000,
            }),
            ..Account::default()
        },
    )
}

fn holding_ata(
    owner: AccountId,
    token_definition_id: AccountId,
    balance: u128,
) -> AccountWithMetadata {
    ata(
        owner,
        token_definition_id,
        Account {
            program_owner: TOKEN_PROGRAM_ID,
            data: Data::from(&TokenHolding::Fungible {
                definition_id: token_definition_id,
                balance,
            }),
            ..Account::default()
        },
    )
}

fn trusted_clock(timestamp: u64) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            data: Data::try_from(
                clock_core::ClockAccountData {
                    block_id: 7,
                    timestamp,
                }
                .to_bytes(),
            )
            .expect("clock data fits"),
            ..Account::default()
        },
        is_authorized: false,
        account_id: clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
    }
}

struct CreatePoolAccounts {
    pool: AccountWithMetadata,
    owner: AccountWithMetadata,
    token0_definition: AccountWithMetadata,
    token1_definition: AccountWithMetadata,
    owner_token0_ata: AccountWithMetadata,
    owner_token1_ata: AccountWithMetadata,
    pool_token0_ata: AccountWithMetadata,
    pool_token1_ata: AccountWithMetadata,
}

fn valid_create_pool_accounts() -> CreatePoolAccounts {
    let owner = signer(AccountId::new([3; 32]));
    let token0_definition_id = AccountId::new([4; 32]);
    let token1_definition_id = AccountId::new([5; 32]);
    let pool_id = compute_pool_pda(
        CURVE_PROGRAM_ID,
        token0_definition_id,
        token1_definition_id,
        owner.account_id,
    );
    CreatePoolAccounts {
        pool: AccountWithMetadata {
            account_id: pool_id,
            ..uninitialized_config()
        },
        owner_token0_ata: owner_source_ata(owner.account_id, token0_definition_id),
        owner_token1_ata: owner_source_ata(owner.account_id, token1_definition_id),
        owner,
        token0_definition: token_definition(token0_definition_id),
        token1_definition: token_definition(token1_definition_id),
        pool_token0_ata: ata(pool_id, token0_definition_id, Account::default()),
        pool_token1_ata: ata(pool_id, token1_definition_id, Account::default()),
    }
}

fn create_pool_with_accounts(
    accounts: CreatePoolAccounts,
    token0_amount: u128,
    token1_amount: u128,
    virtual_reserve0: u128,
    virtual_reserve1: u128,
) {
    let owner_id = accounts.owner.account_id;
    let _result = create_pool(
        accounts.pool,
        accounts.owner,
        accounts.token0_definition,
        accounts.token1_definition,
        accounts.owner_token0_ata,
        accounts.owner_token1_ata,
        accounts.pool_token0_ata,
        accounts.pool_token1_ata,
        trusted_clock(1),
        token0_amount,
        token1_amount,
        virtual_reserve0,
        virtual_reserve1,
        Some(42),
        None,
        owner_id,
        CURVE_PROGRAM_ID,
    );
}

#[should_panic(expected = "Owner authorization is missing")]
#[test]
fn create_pool_rejects_an_owner_without_authorization() {
    let mut accounts = valid_create_pool_accounts();
    accounts.owner.is_authorized = false;
    create_pool_with_accounts(accounts, 800, 25, 1_000, 100);
}

#[test]
fn trusted_ata_program_id_matches_the_pinned_lez_guest() {
    assert_eq!(ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID, programs::ata().id());
}

#[should_panic(expected = "ATA account ID does not match expected derivation")]
#[test]
fn create_pool_rejects_a_source_ata_not_owned_by_the_owner() {
    let intruder_id = AccountId::new([6; 32]);
    let mut accounts = valid_create_pool_accounts();
    accounts.owner_token0_ata = owner_source_ata(intruder_id, AccountId::new([4; 32]));
    create_pool_with_accounts(accounts, 800, 25, 1_000, 100);
}

#[should_panic(expected = "Pool account ID does not match PDA")]
#[test]
fn create_pool_rejects_a_forged_pool_account() {
    let mut accounts = valid_create_pool_accounts();
    accounts.pool.account_id = AccountId::new([6; 32]);
    create_pool_with_accounts(accounts, 800, 25, 1_000, 100);
}

#[should_panic(expected = "ATA account ID does not match expected derivation")]
#[test]
fn create_pool_rejects_a_forged_pool_token0_ata() {
    let mut accounts = valid_create_pool_accounts();
    accounts.pool_token0_ata.account_id = AccountId::new([6; 32]);
    create_pool_with_accounts(accounts, 800, 25, 1_000, 100);
}

#[should_panic(expected = "ATA account ID does not match expected derivation")]
#[test]
fn create_pool_rejects_a_forged_pool_token1_ata() {
    let mut accounts = valid_create_pool_accounts();
    accounts.pool_token1_ata.account_id = AccountId::new([6; 32]);
    create_pool_with_accounts(accounts, 800, 25, 1_000, 100);
}

#[should_panic(expected = "Pool is already initialized")]
#[test]
fn create_pool_rejects_reinitializing_an_existing_pool() {
    let mut accounts = valid_create_pool_accounts();
    accounts.pool.account.program_owner = CURVE_PROGRAM_ID;
    create_pool_with_accounts(accounts, 800, 25, 1_000, 100);
}

#[should_panic(expected = "Pool parameters are invalid")]
#[test]
fn create_pool_enforces_pool_parameter_validation() {
    create_pool_with_accounts(valid_create_pool_accounts(), 800, 25, 0, 100);
}

#[should_panic(expected = "Close timestamp must be in the future")]
#[test]
fn create_pool_rejects_an_end_timestamp_at_the_trusted_time_boundary() {
    let accounts = valid_create_pool_accounts();
    let owner_id = accounts.owner.account_id;
    let _ = create_pool(
        accounts.pool,
        accounts.owner,
        accounts.token0_definition,
        accounts.token1_definition,
        accounts.owner_token0_ata,
        accounts.owner_token1_ata,
        accounts.pool_token0_ata,
        accounts.pool_token1_ata,
        trusted_clock(42),
        800,
        25,
        1_000,
        100,
        Some(42),
        None,
        owner_id,
        CURVE_PROGRAM_ID,
    );
}

#[should_panic(expected = "Clock account is not the trusted LEZ clock")]
#[test]
fn create_pool_rejects_a_substituted_clock_even_without_an_end_timestamp() {
    let accounts = valid_create_pool_accounts();
    let owner_id = accounts.owner.account_id;
    let mut fake_clock = trusted_clock(1);
    fake_clock.account_id = AccountId::new([99; 32]);
    let _ = create_pool(
        accounts.pool,
        accounts.owner,
        accounts.token0_definition,
        accounts.token1_definition,
        accounts.owner_token0_ata,
        accounts.owner_token1_ata,
        accounts.pool_token0_ata,
        accounts.pool_token1_ata,
        fake_clock,
        800,
        25,
        1_000,
        100,
        None,
        None,
        owner_id,
        CURVE_PROGRAM_ID,
    );
}

#[test]
fn owner_can_open_a_pool_and_atomically_fund_both_ata_reserves() {
    let accounts = valid_create_pool_accounts();
    let owner_id = accounts.owner.account_id;
    let token0_id = accounts.token0_definition.account_id;
    let token1_id = accounts.token1_definition.account_id;
    let pool_token0_ata = accounts.pool_token0_ata.clone();
    let pool_token1_ata = accounts.pool_token1_ata.clone();
    let (post_states, chained_calls) = create_pool(
        accounts.pool,
        accounts.owner,
        accounts.token0_definition,
        accounts.token1_definition,
        accounts.owner_token0_ata,
        accounts.owner_token1_ata,
        accounts.pool_token0_ata,
        accounts.pool_token1_ata,
        trusted_clock(1),
        800,
        25,
        1_000,
        100,
        Some(42),
        None,
        owner_id,
        CURVE_PROGRAM_ID,
    );

    let pool_account = PoolAccount::try_from(&post_states[0].account().data)
        .expect("pool post-state holds a valid pool");
    assert_eq!(pool_account.pool.k, 100_000);
    assert_eq!(
        (
            pool_account.pool.real_reserve0,
            pool_account.pool.real_reserve1
        ),
        (800, 25)
    );
    assert_eq!(pool_account.owner, owner_id);
    assert_eq!(
        post_states[0].required_claim(),
        Some(Claim::Pda(crate::compute_pool_pda_seed(
            token0_id, token1_id, owner_id,
        )))
    );

    assert_eq!(chained_calls.len(), 4);
    for call in &chained_calls[..2] {
        assert_eq!(call.program_id, ATA_PROGRAM_ID);
        let instruction: AtaInstruction =
            risc0_zkvm::serde::from_slice(&call.instruction_data).expect("ATA instruction parses");
        assert!(matches!(
            instruction,
            AtaInstruction::Create {
                ata_program_id: ATA_PROGRAM_ID
            }
        ));
    }
    for (transfer, amount, destination, definition_id) in [
        (
            &chained_calls[2],
            800,
            pool_token0_ata.account_id,
            token0_id,
        ),
        (&chained_calls[3], 25, pool_token1_ata.account_id, token1_id),
    ] {
        assert_eq!(transfer.program_id, ATA_PROGRAM_ID);
        let instruction: AtaInstruction = risc0_zkvm::serde::from_slice(&transfer.instruction_data)
            .expect("ATA instruction parses");
        assert!(
            matches!(instruction, AtaInstruction::Transfer { ata_program_id: ATA_PROGRAM_ID, amount: actual } if actual == amount)
        );
        assert_eq!(transfer.pre_states[2].account_id, destination);
        assert_eq!(
            TokenHolding::try_from(&transfer.pre_states[2].account.data)
                .expect("funding recipient is initialized"),
            TokenHolding::Fungible {
                definition_id,
                balance: 0
            }
        );
    }
}

#[test]
fn zero_initial_reserve_creates_its_ata_without_emitting_a_zero_transfer() {
    let accounts = valid_create_pool_accounts();
    let owner_id = accounts.owner.account_id;
    let (_, chained_calls) = create_pool(
        accounts.pool,
        accounts.owner,
        accounts.token0_definition,
        accounts.token1_definition,
        accounts.owner_token0_ata,
        accounts.owner_token1_ata,
        accounts.pool_token0_ata,
        accounts.pool_token1_ata,
        trusted_clock(1),
        800,
        0,
        1_000,
        100,
        None,
        None,
        owner_id,
        CURVE_PROGRAM_ID,
    );

    assert_eq!(
        chained_calls.len(),
        3,
        "two ATA creates and one funding transfer"
    );
    let instruction: AtaInstruction =
        risc0_zkvm::serde::from_slice(&chained_calls[2].instruction_data)
            .expect("ATA transfer parses");
    assert!(matches!(
        instruction,
        AtaInstruction::Transfer {
            ata_program_id: ATA_PROGRAM_ID,
            amount: 800,
        }
    ));
}

#[should_panic(expected = "Authority is not the config admin")]
#[test]
fn init_rejects_an_authority_other_than_the_genesis_admin() {
    let _post_states = update_config(
        uninitialized_config(),
        intruder_signer(),
        new_admin(),
        100,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );
}

#[should_panic(expected = "Authority authorization is missing")]
#[test]
fn update_config_rejects_an_unauthorized_authority() {
    let unsigned_genesis_admin = AccountWithMetadata {
        is_authorized: false,
        ..genesis_admin_signer()
    };
    let _post_states = update_config(
        uninitialized_config(),
        unsigned_genesis_admin,
        new_admin(),
        100,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );
}

// The RFP-001 seam: plugging the admin library in later is this rotation, applied to
// whatever key that library controls. No code change, no redeploy.
#[test]
fn stored_admin_rotates_the_admin_to_a_new_key() {
    let rotated_to = AccountId::new([3; 32]);
    let post_states = update_config(
        initialized_config(new_admin()),
        signer(new_admin()),
        rotated_to,
        40,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );

    let [config_post]: [_; 1] = post_states.try_into().expect("exactly one post state");
    let config =
        Config::try_from(&config_post.account().data).expect("post state holds a valid Config");
    assert_eq!(config.admin, rotated_to);
    assert_eq!(config.protocol_fee_bps, 40);
    assert_eq!(
        config_post.required_claim(),
        None,
        "updating an existing config must not re-claim the PDA"
    );
}

#[should_panic(expected = "Authority is not the config admin")]
#[test]
fn update_rejects_the_rotated_out_admin() {
    // The config's stored admin is `new_admin`; the genesis admin was rotated out at
    // init and holds no power anymore.
    let _post_states = update_config(
        initialized_config(new_admin()),
        genesis_admin_signer(),
        new_admin(),
        100,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );
}

#[should_panic(expected = "Protocol fee exceeds 10,000 basis points")]
#[test]
fn update_config_rejects_protocol_fee_above_the_denominator() {
    let _post_states = update_config(
        uninitialized_config(),
        genesis_admin_signer(),
        new_admin(),
        10_001,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );
}

// Zero is legal (pool creation remains free and a fee-free pool is valid), and
// 10,000 is the denominator itself.
#[test]
fn protocol_fee_boundaries_are_legal() {
    for protocol_fee_bps in [0, 10_000] {
        let post_states = update_config(
            uninitialized_config(),
            genesis_admin_signer(),
            new_admin(),
            protocol_fee_bps,
            new_treasury(),
            CURVE_PROGRAM_ID,
        );
        let [config_post]: [_; 1] = post_states.try_into().expect("exactly one post state");
        let config =
            Config::try_from(&config_post.account().data).expect("post state holds a valid Config");
        assert_eq!(config.protocol_fee_bps, protocol_fee_bps);
    }
}

// The default key is indistinguishable from unset, and the token program would
// happily auto-initialize a holding for it. Reject it for both key fields.
#[should_panic(expected = "Admin must not be the default key")]
#[test]
fn update_config_rejects_a_default_admin() {
    let _post_states = update_config(
        uninitialized_config(),
        genesis_admin_signer(),
        AccountId::default(),
        100,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );
}

#[should_panic(expected = "Treasury must not be the default key")]
#[test]
fn update_config_rejects_a_default_treasury() {
    let _post_states = update_config(
        uninitialized_config(),
        genesis_admin_signer(),
        new_admin(),
        100,
        AccountId::default(),
        CURVE_PROGRAM_ID,
    );
}

#[should_panic(expected = "Config account ID does not match PDA")]
#[test]
fn update_config_rejects_a_forged_config_account() {
    let forged_config = AccountWithMetadata {
        account_id: AccountId::new([66; 32]),
        ..uninitialized_config()
    };
    let _post_states = update_config(
        forged_config,
        genesis_admin_signer(),
        new_admin(),
        100,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );
}

#[test]
fn first_update_config_initializes_the_config() {
    let post_states = update_config(
        uninitialized_config(),
        genesis_admin_signer(),
        new_admin(),
        100,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );

    let [config_post]: [_; 1] = post_states.try_into().expect("exactly one post state");
    let config =
        Config::try_from(&config_post.account().data).expect("post state holds a valid Config");
    assert_eq!(config.admin, new_admin());
    assert_eq!(config.protocol_fee_bps, 100);
    assert_eq!(config.treasury, new_treasury());
    assert_eq!(
        config_post.required_claim(),
        Some(Claim::Pda(compute_config_pda_seed())),
        "creation must claim the config PDA"
    );
}

#[test]
fn pool_state_round_trips_with_ordered_tokens_owner_and_optional_expiry() {
    let pool_account = PoolAccount {
        token0_definition_id: AccountId::new([1; 32]),
        token1_definition_id: AccountId::new([2; 32]),
        owner: AccountId::new([3; 32]),
        pool: Pool::create(800, 25, 1000, 100, Some(42), None).expect("valid pool"),
    };
    let data = Data::from(&pool_account);
    assert_eq!(
        PoolAccount::try_from(&data).expect("data parses"),
        pool_account
    );
}

#[test]
fn stored_pool_owner_can_close_the_pool() {
    let owner = AccountId::new([3; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool_account = PoolAccount {
        token0_definition_id: token0,
        token1_definition_id: token1,
        owner,
        pool: Pool::create(800, 25, 1000, 100, None, None).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, owner),
    };

    let [post]: [_; 1] = close_pool(pool, signer(owner), trusted_clock(1), CURVE_PROGRAM_ID)
        .try_into()
        .expect("one post state");
    let closed = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert_eq!(closed.pool.lifecycle, pool::PoolLifecycle::Closed);
}

#[should_panic(expected = "Authority is not the pool owner")]
#[test]
fn an_unrelated_signer_cannot_close_the_pool() {
    let owner = AccountId::new([3; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool_account = PoolAccount {
        token0_definition_id: token0,
        token1_definition_id: token1,
        owner,
        pool: Pool::create(800, 25, 1000, 100, None, None).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, owner),
    };
    let _ = close_pool(pool, intruder_signer(), trusted_clock(1), CURVE_PROGRAM_ID);
}

#[test]
fn expired_pool_owner_can_withdraw_both_reserves_without_closing_first() {
    let owner = AccountId::new([3; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool_account = PoolAccount {
        token0_definition_id: token0,
        token1_definition_id: token1,
        owner,
        pool: Pool::create(800, 25, 1000, 100, Some(42), None).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, owner),
    };
    let clock = AccountWithMetadata {
        account: Account {
            data: Data::try_from(
                clock_core::ClockAccountData {
                    block_id: 7,
                    timestamp: 42,
                }
                .to_bytes(),
            )
            .expect("clock data fits"),
            ..Account::default()
        },
        is_authorized: false,
        account_id: clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
    };

    let (posts, withdrawn) = withdraw_reserves(pool, signer(owner), clock, CURVE_PROGRAM_ID);
    assert_eq!(
        (withdrawn.token0_amount, withdrawn.token1_amount),
        (800, 25)
    );
    let [post]: [_; 1] = posts.try_into().expect("one post state");
    let retired = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert_eq!(retired.pool.lifecycle, pool::PoolLifecycle::Withdrawn);
}

#[test]
fn withdraw_dispatch_transfers_both_real_reserves_and_retires_the_pool() {
    let owner = AccountId::new([3; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool_id = compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, owner);
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&PoolAccount {
                token0_definition_id: token0,
                token1_definition_id: token1,
                owner,
                pool: Pool::create(800, 100, 1000, 100, Some(42), None).expect("valid pool"),
            }),
            ..Account::default()
        },
        is_authorized: false,
        account_id: pool_id,
    };
    let pool_token0 = holding_ata(pool_id, token0, 800);
    let pool_token1 = holding_ata(pool_id, token1, 100);
    let owner_token0 = holding_ata(owner, token0, 0);
    let owner_token1 = holding_ata(owner, token1, 0);

    let (posts, calls) = process_instruction(
        vec![
            pool,
            signer(owner),
            owner_token0.clone(),
            owner_token1.clone(),
            pool_token0.clone(),
            pool_token1.clone(),
            trusted_clock(42),
        ],
        Instruction::WithdrawReserves,
        CURVE_PROGRAM_ID,
    );

    let [post]: [_; 1] = posts
        .try_into()
        .expect("only the pool state changes directly");
    let retired = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert_eq!(retired.pool.lifecycle, pool::PoolLifecycle::Withdrawn);
    assert_eq!(
        (retired.pool.real_reserve0, retired.pool.real_reserve1),
        (0, 0)
    );
    assert_eq!(calls.len(), 2, "one transfer for each reserve");
    let transfers: Vec<(AccountId, AccountId, AccountId, u128)> = calls
        .iter()
        .map(|call| {
            let instruction: AtaInstruction = risc0_zkvm::serde::from_slice(&call.instruction_data)
                .expect("ATA instruction parses");
            let AtaInstruction::Transfer { amount, .. } = instruction else {
                panic!("withdrawal contains only transfers");
            };
            (
                call.pre_states[0].account_id,
                call.pre_states[1].account_id,
                call.pre_states[2].account_id,
                amount,
            )
        })
        .collect();
    assert_eq!(
        transfers,
        vec![
            (
                pool_id,
                pool_token0.account_id,
                owner_token0.account_id,
                800
            ),
            (
                pool_id,
                pool_token1.account_id,
                owner_token1.account_id,
                100
            ),
        ]
    );
}

#[test]
fn exact_input_sell_charges_the_protocol_fee_in_collateral() {
    let owner = AccountId::new([3; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool_account = PoolAccount {
        token0_definition_id: token0,
        token1_definition_id: token1,
        owner,
        pool: Pool::create(800, 100, 1000, 100, None, None).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, owner),
    };

    let (posts, settlement) = swap_exact_input(
        pool,
        initialized_config(new_admin()),
        None,
        250,
        19,
        token0,
        CURVE_PROGRAM_ID,
    );
    assert_eq!(settlement.token_in, token0);
    assert_eq!(settlement.token_out, token1);
    assert_eq!(settlement.effective_amount_in, 250);
    assert_eq!(settlement.raw_amount_out, 20);
    assert_eq!(settlement.protocol_fee, 1);
    assert!(settlement.protocol_fee_on_output);
    assert_eq!(settlement.treasury, new_treasury());
    let [post]: [_; 1] = posts.try_into().expect("one post state");
    let updated = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert_eq!(
        (updated.pool.real_reserve0, updated.pool.real_reserve1),
        (1050, 80)
    );
}

#[test]
fn token0_to_token1_exact_input_settles_all_three_transfers() {
    let owner = AccountId::new([3; 32]);
    let participant = AccountId::new([4; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool_id = compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, owner);
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&PoolAccount {
                token0_definition_id: token0,
                token1_definition_id: token1,
                owner,
                pool: Pool::create(800, 100, 1000, 100, Some(42), None).expect("valid pool"),
            }),
            ..Account::default()
        },
        is_authorized: false,
        account_id: pool_id,
    };
    let participant_token0 = holding_ata(participant, token0, 1_000);
    let participant_token1 = holding_ata(participant, token1, 0);
    let pool_token0 = holding_ata(pool_id, token0, 800);
    let pool_token1 = holding_ata(pool_id, token1, 100);
    let treasury_token1 = ata(new_treasury(), token1, Account::default());

    let (posts, calls) = process_instruction(
        vec![
            pool,
            initialized_config(new_admin()),
            signer(participant),
            participant_token0.clone(),
            pool_token0.clone(),
            pool_token1.clone(),
            participant_token1.clone(),
            treasury_token1.clone(),
            trusted_clock(41),
        ],
        Instruction::SwapExactInput {
            amount_in: 250,
            min_amount_out: 19,
            token_in: token0,
        },
        CURVE_PROGRAM_ID,
    );

    let [post]: [_; 1] = posts
        .try_into()
        .expect("only the pool state changes directly");
    let updated = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert_eq!(
        (updated.pool.real_reserve0, updated.pool.real_reserve1),
        (1050, 80)
    );
    assert_eq!(calls.len(), 3, "input, protocol fee, and output transfers");
    for call in &calls {
        assert_eq!(call.program_id, ATA_PROGRAM_ID);
    }
    assert!(
        calls[2].pre_states[0].is_authorized,
        "the pool must authorize the output transfer"
    );
    assert_eq!(
        calls[2].pda_seeds,
        vec![crate::compute_pool_pda_seed(token0, token1, owner)],
        "the pool must prove its PDA authority for the output transfer"
    );
    let transfers: Vec<(AccountId, AccountId, AccountId, u128)> = calls
        .iter()
        .map(|call| {
            let instruction: AtaInstruction = risc0_zkvm::serde::from_slice(&call.instruction_data)
                .expect("ATA instruction parses");
            let AtaInstruction::Transfer { amount, .. } = instruction else {
                panic!("swap settlement contains only transfers");
            };
            (
                call.pre_states[0].account_id,
                call.pre_states[1].account_id,
                call.pre_states[2].account_id,
                amount,
            )
        })
        .collect();
    assert_eq!(
        transfers,
        vec![
            (
                participant,
                participant_token0.account_id,
                pool_token0.account_id,
                250
            ),
            (
                pool_id,
                pool_token1.account_id,
                participant_token1.account_id,
                19
            ),
            (
                pool_id,
                pool_token1.account_id,
                treasury_token1.account_id,
                1
            ),
        ]
    );
}

#[test]
fn exact_output_handler_caps_fee_inclusive_input_in_the_reverse_direction() {
    let owner = AccountId::new([3; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool_account = PoolAccount {
        token0_definition_id: token0,
        token1_definition_id: token1,
        owner,
        pool: Pool::create(800, 100, 1000, 100, None, None).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, owner),
    };
    let config = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&Config {
                admin: new_admin(),
                protocol_fee_bps: 1_000,
                treasury: new_treasury(),
            }),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_config_pda(CURVE_PROGRAM_ID),
    };

    let (_, settlement) = swap_exact_output(pool, config, None, 200, 28, token1, CURVE_PROGRAM_ID);
    assert_eq!(settlement.token_in, token1);
    assert_eq!(settlement.token_out, token0);
    assert_eq!(
        (
            settlement.amount_in,
            settlement.amount_out,
            settlement.protocol_fee,
            settlement.raw_amount_out,
        ),
        (28, 200, 3, 200)
    );
}

#[test]
fn exact_output_dispatch_settles_fee_inclusive_input_and_requested_output_atomically() {
    let owner = AccountId::new([3; 32]);
    let participant = AccountId::new([4; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool_id = compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, owner);
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&PoolAccount {
                token0_definition_id: token0,
                token1_definition_id: token1,
                owner,
                pool: Pool::create(800, 100, 1000, 100, Some(42), None).expect("valid pool"),
            }),
            ..Account::default()
        },
        is_authorized: false,
        account_id: pool_id,
    };
    let participant_token0 = holding_ata(participant, token0, 0);
    let participant_token1 = holding_ata(participant, token1, 1_000);
    let pool_token0 = holding_ata(pool_id, token0, 800);
    let pool_token1 = holding_ata(pool_id, token1, 100);
    let treasury_token1 = ata(new_treasury(), token1, Account::default());
    let config = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&Config {
                admin: new_admin(),
                protocol_fee_bps: 1_000,
                treasury: new_treasury(),
            }),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_config_pda(CURVE_PROGRAM_ID),
    };

    let (posts, calls) = process_instruction(
        vec![
            pool,
            config,
            signer(participant),
            participant_token1.clone(),
            pool_token1.clone(),
            pool_token0.clone(),
            participant_token0.clone(),
            treasury_token1,
            trusted_clock(41),
        ],
        Instruction::SwapExactOutput {
            amount_out: 200,
            max_amount_in: 28,
            token_in: token1,
        },
        CURVE_PROGRAM_ID,
    );

    let [post]: [_; 1] = posts
        .try_into()
        .expect("only the pool state changes directly");
    let updated = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert_eq!(
        (updated.pool.real_reserve0, updated.pool.real_reserve1),
        (600, 125)
    );
    assert_eq!(
        calls.len(),
        3,
        "effective collateral, protocol fee, and requested output transfers"
    );
    let amounts: Vec<u128> = calls
        .iter()
        .map(|call| {
            let instruction: AtaInstruction = risc0_zkvm::serde::from_slice(&call.instruction_data)
                .expect("ATA instruction parses");
            let AtaInstruction::Transfer { amount, .. } = instruction else {
                panic!("swap settlement contains only transfers");
            };
            amount
        })
        .collect();
    assert_eq!(amounts, vec![25, 3, 200]);
    assert_eq!(calls[0].pre_states[0].account_id, participant);
    assert_eq!(
        calls[0].pre_states[1].account_id,
        participant_token1.account_id
    );
    assert_eq!(calls[0].pre_states[2].account_id, pool_token1.account_id);
    assert_eq!(calls[1].pre_states[0].account_id, participant);
    assert_eq!(
        calls[1].pre_states[1].account_id,
        participant_token1.account_id
    );
    assert_eq!(calls[2].pre_states[0].account_id, pool_id);
    assert_eq!(calls[2].pre_states[1].account_id, pool_token0.account_id);
    assert_eq!(
        calls[2].pre_states[2].account_id,
        participant_token0.account_id
    );
}

#[test]
fn every_instruction_survives_the_guest_wire_format() {
    // `read_lee_inputs::<Instruction>()` in the guest deserialises with risc0's serde.
    let instructions = [
        Instruction::UpdateConfig {
            admin: new_admin(),
            protocol_fee_bps: 100,
            treasury: new_treasury(),
        },
        Instruction::CreatePool {
            token0_amount: 800,
            token1_amount: 25,
            virtual_reserve0: 1000,
            virtual_reserve1: 100,
            close_timestamp: Some(42),
            close_on_depletion: None,
            owner: AccountId::new([3; 32]),
            curve_program_id: [7; 8],
        },
        Instruction::SwapExactInput {
            amount_in: 25,
            min_amount_out: 190,
            token_in: AccountId::new([1; 32]),
        },
        Instruction::SwapExactOutput {
            amount_out: 250,
            max_amount_in: 18,
            token_in: AccountId::new([2; 32]),
        },
        Instruction::ClosePool,
        Instruction::WithdrawReserves,
    ];
    for instruction in instructions {
        let words = risc0_zkvm::serde::to_vec(&instruction).expect("instruction serialises");
        let back: Instruction = risc0_zkvm::serde::from_slice(&words).expect("wire words parse");
        assert_eq!(back, instruction);
    }
}

#[test]
fn the_pool_pda_hashes_the_pair_in_fixed_order() {
    let owner = AccountId::new([3; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    assert_ne!(
        compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, owner),
        compute_pool_pda(CURVE_PROGRAM_ID, token1, token0, owner)
    );
}

#[test]
fn different_owners_have_distinct_pool_pdas_for_the_same_pair() {
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    assert_ne!(
        compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, AccountId::new([3; 32]),),
        compute_pool_pda(CURVE_PROGRAM_ID, token0, token1, AccountId::new([4; 32]),)
    );
}
