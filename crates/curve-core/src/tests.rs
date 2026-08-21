//! AMM-style handler tests: hand-built `AccountWithMetadata` fixtures, direct handler
//! calls, `#[should_panic]` on the gates. Mirrors `lez/programs/amm/src/tests.rs`.

use associated_token_account_core::{
    Instruction as AtaInstruction, compute_ata_seed, get_associated_token_account_id,
};
use lee_core::{
    account::{Account, AccountId, AccountWithMetadata, Data},
    program::{Claim, ProgramId},
};

use sale::Sale;
use token_core::{TokenDefinition, TokenHolding};

use crate::{
    ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID, Config, GENESIS_ADMIN, Instruction, SaleAccount,
    compute_config_pda, compute_config_pda_seed, compute_creator_commitment, compute_sale_pda,
    create_sale::create_sale, update_config::update_config,
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

fn creator_source_ata(creator: AccountId, token_definition_id: AccountId) -> AccountWithMetadata {
    ata(
        creator,
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

struct CreateSaleAccounts {
    sale: AccountWithMetadata,
    creator: AccountWithMetadata,
    token_definition: AccountWithMetadata,
    collateral_definition: AccountWithMetadata,
    creator_token_ata: AccountWithMetadata,
    sale_token_ata: AccountWithMetadata,
    sale_collateral_ata: AccountWithMetadata,
}

fn valid_create_sale_accounts() -> CreateSaleAccounts {
    let creator = signer(AccountId::new([3; 32]));
    let token_definition_id = AccountId::new([4; 32]);
    let collateral_definition_id = AccountId::new([5; 32]);
    let sale_id = compute_sale_pda(
        CURVE_PROGRAM_ID,
        token_definition_id,
        collateral_definition_id,
    );
    CreateSaleAccounts {
        sale: AccountWithMetadata {
            account_id: sale_id,
            ..uninitialized_config()
        },
        creator_token_ata: creator_source_ata(creator.account_id, token_definition_id),
        creator,
        token_definition: token_definition(token_definition_id),
        collateral_definition: token_definition(collateral_definition_id),
        sale_token_ata: ata(sale_id, token_definition_id, Account::default()),
        sale_collateral_ata: ata(sale_id, collateral_definition_id, Account::default()),
    }
}

fn create_sale_with_accounts(
    accounts: CreateSaleAccounts,
    sale_reserve: u128,
    dex_seed_reserve: u128,
    virtual_token_reserve: u128,
    virtual_collateral_reserve: u128,
) {
    let _result = create_sale(
        accounts.sale,
        accounts.creator,
        accounts.token_definition,
        accounts.collateral_definition,
        accounts.creator_token_ata,
        accounts.sale_token_ata,
        accounts.sale_collateral_ata,
        sale_reserve,
        dex_seed_reserve,
        virtual_token_reserve,
        virtual_collateral_reserve,
        CURVE_PROGRAM_ID,
    );
}

fn create_valid_sale_with(creator: AccountWithMetadata, creator_token_ata: AccountWithMetadata) {
    let mut accounts = valid_create_sale_accounts();
    accounts.creator = creator;
    accounts.creator_token_ata = creator_token_ata;
    create_sale_with_accounts(accounts, 800, 200, 1_000, 100);
}

#[should_panic(expected = "Creator authorization is missing")]
#[test]
fn create_sale_rejects_a_creator_without_authorization() {
    let creator_id = AccountId::new([3; 32]);
    create_valid_sale_with(
        AccountWithMetadata {
            is_authorized: false,
            ..signer(creator_id)
        },
        creator_source_ata(creator_id, AccountId::new([4; 32])),
    );
}

#[test]
fn trusted_ata_program_id_matches_the_pinned_lez_guest() {
    assert_eq!(ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID, programs::ata().id());
}

#[should_panic(expected = "ATA account ID does not match expected derivation")]
#[test]
fn create_sale_rejects_a_source_ata_not_owned_by_the_creator() {
    let creator_id = AccountId::new([3; 32]);
    let intruder_id = AccountId::new([6; 32]);
    create_valid_sale_with(
        signer(creator_id),
        creator_source_ata(intruder_id, AccountId::new([4; 32])),
    );
}

#[should_panic(expected = "Sale account ID does not match PDA")]
#[test]
fn create_sale_rejects_a_forged_sale_account() {
    let mut accounts = valid_create_sale_accounts();
    accounts.sale.account_id = AccountId::new([6; 32]);
    create_sale_with_accounts(accounts, 800, 200, 1_000, 100);
}

#[should_panic(expected = "ATA account ID does not match expected derivation")]
#[test]
fn create_sale_rejects_a_forged_sale_token_ata() {
    let mut accounts = valid_create_sale_accounts();
    accounts.sale_token_ata.account_id = AccountId::new([6; 32]);
    create_sale_with_accounts(accounts, 800, 200, 1_000, 100);
}

#[should_panic(expected = "ATA account ID does not match expected derivation")]
#[test]
fn create_sale_rejects_a_forged_sale_collateral_ata() {
    let mut accounts = valid_create_sale_accounts();
    accounts.sale_collateral_ata.account_id = AccountId::new([6; 32]);
    create_sale_with_accounts(accounts, 800, 200, 1_000, 100);
}

#[should_panic(expected = "Sale account is already initialized")]
#[test]
fn create_sale_rejects_reinitializing_an_existing_sale() {
    let mut accounts = valid_create_sale_accounts();
    accounts.sale.account.program_owner = CURVE_PROGRAM_ID;
    create_sale_with_accounts(accounts, 800, 200, 1_000, 100);
}

#[should_panic(expected = "sale reserve plus DEX seed reserve overflows")]
#[test]
fn create_sale_rejects_a_deposit_that_cannot_fit_in_u128() {
    create_sale_with_accounts(valid_create_sale_accounts(), 800, u128::MAX, 1_000, 100);
}

#[should_panic(expected = "invalid sale parameters")]
#[test]
fn create_sale_enforces_the_sale_parameter_validation() {
    create_sale_with_accounts(valid_create_sale_accounts(), 0, 200, 1_000, 100);
}

#[test]
fn creator_can_open_a_sale_and_atomically_fund_its_ata_reserves() {
    let creator = signer(AccountId::new([3; 32]));
    let token_definition_id = AccountId::new([4; 32]);
    let collateral_definition_id = AccountId::new([5; 32]);
    let sale_id = compute_sale_pda(
        CURVE_PROGRAM_ID,
        token_definition_id,
        collateral_definition_id,
    );
    let sale = AccountWithMetadata {
        account_id: sale_id,
        ..uninitialized_config()
    };
    let sale_token_ata = ata(sale_id, token_definition_id, Account::default());
    let sale_collateral_ata = ata(sale_id, collateral_definition_id, Account::default());

    let (post_states, chained_calls) = create_sale(
        sale,
        creator.clone(),
        token_definition(token_definition_id),
        token_definition(collateral_definition_id),
        creator_source_ata(creator.account_id, token_definition_id),
        sale_token_ata.clone(),
        sale_collateral_ata,
        800,
        200,
        1_000,
        100,
        CURVE_PROGRAM_ID,
    );

    let sale_account = SaleAccount::try_from(&post_states[0].account().data)
        .expect("sale post-state holds a valid sale");
    assert_eq!(sale_account.sale.k, 100_000);
    assert_eq!(sale_account.sale.sale_reserve, 800);
    assert_eq!(sale_account.sale.dex_seed_reserve, 200);
    assert_eq!(
        sale_account.creator_commitment,
        compute_creator_commitment(creator.account_id, sale_id)
    );
    assert_eq!(
        post_states[0].required_claim(),
        Some(Claim::Pda(crate::compute_sale_pda_seed(
            token_definition_id,
            collateral_definition_id,
        )))
    );

    assert_eq!(chained_calls.len(), 3);
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
    let transfer = &chained_calls[2];
    assert_eq!(transfer.program_id, ATA_PROGRAM_ID);
    let instruction: AtaInstruction =
        risc0_zkvm::serde::from_slice(&transfer.instruction_data).expect("ATA instruction parses");
    assert!(matches!(
        instruction,
        AtaInstruction::Transfer {
            ata_program_id: ATA_PROGRAM_ID,
            amount: 1_000,
        }
    ));
    assert_eq!(transfer.pre_states[2].account_id, sale_token_ata.account_id);
    assert_eq!(
        TokenHolding::try_from(&transfer.pre_states[2].account.data)
            .expect("funding recipient is initialized"),
        TokenHolding::Fungible {
            definition_id: token_definition_id,
            balance: 0,
        }
    );
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
        creator_commitment: [3; 32],
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

#[test]
fn the_sale_pda_hashes_the_pair_in_fixed_order() {
    // Token and collateral are different roles: swapping them is a different sale.
    let token = AccountId::new([1; 32]);
    let collateral = AccountId::new([2; 32]);
    assert_ne!(
        compute_sale_pda(CURVE_PROGRAM_ID, token, collateral),
        compute_sale_pda(CURVE_PROGRAM_ID, collateral, token)
    );
}
