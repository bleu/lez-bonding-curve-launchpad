//! AMM-style handler tests: hand-built `AccountWithMetadata` fixtures, direct handler
//! calls, `#[should_panic]` on the gates. Mirrors `lez/programs/amm/src/tests.rs`.

use lee_core::{
    account::{Account, AccountId, AccountWithMetadata, Data},
    program::{Claim, ProgramId},
};

use pool::Pool;

use crate::{
    Config, GENESIS_ADMIN, Instruction, PoolAccount, compute_config_pda, compute_config_pda_seed,
    compute_pool_pda,
    pool_create::create_pool,
    pool_lifecycle::{close_pool, withdraw_reserves},
    pool_swap::{swap_exact_input, swap_exact_output},
    update_config::update_config,
};

const CURVE_PROGRAM_ID: ProgramId = [7; 8];

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
            fee_bps: 250,
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

#[should_panic(expected = "Authority is not the config admin")]
#[test]
fn init_rejects_an_authority_other_than_the_genesis_admin() {
    let _post_states = update_config(
        uninitialized_config(),
        intruder_signer(),
        new_admin(),
        250,
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
        250,
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
        100,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );

    let [config_post]: [_; 1] = post_states.try_into().expect("exactly one post state");
    let config =
        Config::try_from(&config_post.account().data).expect("post state holds a valid Config");
    assert_eq!(config.admin, rotated_to);
    assert_eq!(config.fee_bps, 100);
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
        250,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );
}

#[should_panic(expected = "Fee rate exceeds 10,000 basis points")]
#[test]
fn update_config_rejects_a_fee_above_the_denominator() {
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
fn fee_boundaries_are_legal() {
    for fee_bps in [0, 10_000] {
        let post_states = update_config(
            uninitialized_config(),
            genesis_admin_signer(),
            new_admin(),
            fee_bps,
            new_treasury(),
            CURVE_PROGRAM_ID,
        );
        let [config_post]: [_; 1] = post_states.try_into().expect("exactly one post state");
        let config =
            Config::try_from(&config_post.account().data).expect("post state holds a valid Config");
        assert_eq!(config.fee_bps, fee_bps);
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
        250,
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
        250,
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
        250,
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
        250,
        new_treasury(),
        CURVE_PROGRAM_ID,
    );

    let [config_post]: [_; 1] = post_states.try_into().expect("exactly one post state");
    let config =
        Config::try_from(&config_post.account().data).expect("post state holds a valid Config");
    assert_eq!(config.admin, new_admin());
    assert_eq!(config.fee_bps, 250);
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
        pool: Pool::create(800, 25, 1000, 100, Some(42)).expect("valid pool"),
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
        pool: Pool::create(800, 25, 1000, 100, None).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1),
    };

    let [post]: [_; 1] = close_pool(pool, signer(owner), CURVE_PROGRAM_ID)
        .try_into()
        .expect("one post state");
    let closed = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert!(!closed.pool.open);
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
        pool: Pool::create(800, 25, 1000, 100, None).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1),
    };
    let _ = close_pool(pool, intruder_signer(), CURVE_PROGRAM_ID);
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
        pool: Pool::create(800, 25, 1000, 100, Some(42)).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1),
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

    let (posts, withdrawn) = withdraw_reserves(pool, signer(owner), Some(clock), CURVE_PROGRAM_ID);
    assert_eq!(
        (withdrawn.token0_amount, withdrawn.token1_amount),
        (800, 25)
    );
    let [post]: [_; 1] = posts.try_into().expect("one post state");
    let retired = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert!(retired.pool.retired);
}

#[test]
fn exact_input_handler_selects_direction_and_pairs_the_fee_with_token_in() {
    let owner = AccountId::new([3; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool_account = PoolAccount {
        token0_definition_id: token0,
        token1_definition_id: token1,
        owner,
        pool: Pool::create(800, 100, 1000, 100, None).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1),
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
    assert_eq!(settlement.fee, 6);
    assert_eq!(settlement.treasury, new_treasury());
    let [post]: [_; 1] = posts.try_into().expect("one post state");
    let updated = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert_eq!(
        (updated.pool.real_reserve0, updated.pool.real_reserve1),
        (1044, 81)
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
        pool: Pool::create(800, 100, 1000, 100, None).expect("valid pool"),
    };
    let pool = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&pool_account),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1),
    };
    let config = AccountWithMetadata {
        account: Account {
            program_owner: CURVE_PROGRAM_ID,
            data: Data::from(&Config {
                admin: new_admin(),
                fee_bps: 1_000,
                treasury: new_treasury(),
            }),
            ..Account::default()
        },
        is_authorized: false,
        account_id: compute_config_pda(CURVE_PROGRAM_ID),
    };

    let (_, settlement) = swap_exact_output(pool, config, None, 200, 27, token1, CURVE_PROGRAM_ID);
    assert_eq!(settlement.token_in, token1);
    assert_eq!(settlement.token_out, token0);
    assert_eq!(
        (settlement.amount_in, settlement.amount_out, settlement.fee),
        (27, 200, 2)
    );
}

#[test]
fn direct_creation_funds_both_ordered_reserves_and_stores_the_selected_owner() {
    let owner = AccountId::new([3; 32]);
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    let pool = AccountWithMetadata {
        account: Account::default(),
        is_authorized: false,
        account_id: compute_pool_pda(CURVE_PROGRAM_ID, token0, token1),
    };

    let (posts, funding) = create_pool(
        pool,
        token0,
        token1,
        800,
        25,
        1000,
        100,
        Some(42),
        owner,
        CURVE_PROGRAM_ID,
    );
    assert_eq!((funding.token0_amount, funding.token1_amount), (800, 25));
    let [post]: [_; 1] = posts.try_into().expect("one post state");
    assert_eq!(
        post.required_claim(),
        Some(Claim::Pda(crate::compute_pool_pda_seed(token0, token1)))
    );
    let created = PoolAccount::try_from(&post.account().data).expect("valid pool state");
    assert_eq!(created.owner, owner);
    assert_eq!(created.pool.close_timestamp, Some(42));
}

#[test]
fn every_instruction_survives_the_guest_wire_format() {
    // `read_lee_inputs::<Instruction>()` in the guest deserialises with risc0's serde.
    let instructions = [
        Instruction::UpdateConfig {
            admin: new_admin(),
            fee_bps: 250,
            treasury: new_treasury(),
        },
        Instruction::CreatePool {
            token0_amount: 800,
            token1_amount: 25,
            virtual_reserve0: 1000,
            virtual_reserve1: 100,
            close_timestamp: Some(42),
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
    let token0 = AccountId::new([1; 32]);
    let token1 = AccountId::new([2; 32]);
    assert_ne!(
        compute_pool_pda(CURVE_PROGRAM_ID, token0, token1),
        compute_pool_pda(CURVE_PROGRAM_ID, token1, token0)
    );
}
