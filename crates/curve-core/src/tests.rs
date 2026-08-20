//! AMM-style handler tests: hand-built `AccountWithMetadata` fixtures, direct handler
//! calls, `#[should_panic]` on the gates. Mirrors `lez/programs/amm/src/tests.rs`.

use lee_core::{
    account::{Account, AccountId, AccountWithMetadata, Data},
    program::{Claim, ProgramId},
};

use sale::Sale;

use crate::{
    Config, GENESIS_ADMIN, Instruction, SaleAccount, compute_config_pda, compute_config_pda_seed,
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

// Zero is RFP text ("sale creation free" plus a free curve is a legal config), and
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
fn sale_state_round_trips_through_account_data() {
    let sale_account = SaleAccount {
        token_definition_id: AccountId::new([1; 32]),
        collateral_definition_id: AccountId::new([2; 32]),
        sale: Sale::create(800, 200, 1000, 100).expect("valid sale"),
    };
    let data = Data::from(&sale_account);
    assert_eq!(
        SaleAccount::try_from(&data).expect("data parses"),
        sale_account
    );
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
        Instruction::CreateSale {
            sale_reserve: 800,
            dex_seed_reserve: 200,
            virtual_token_reserve: 1000,
            virtual_collateral_reserve: 100,
            curve_program_id: [7; 8],
        },
        Instruction::Buy {
            collateral_in: 25,
            min_tokens_out: 190,
        },
        Instruction::Sell {
            tokens_in: 250,
            min_collateral_out: 18,
        },
        Instruction::Close,
        Instruction::Withdraw,
    ];
    for instruction in instructions {
        let words = risc0_zkvm::serde::to_vec(&instruction).expect("instruction serialises");
        let back: Instruction = risc0_zkvm::serde::from_slice(&words).expect("wire words parse");
        assert_eq!(back, instruction);
    }
}
