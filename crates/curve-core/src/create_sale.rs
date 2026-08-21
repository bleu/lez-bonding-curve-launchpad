//! `create_sale`: opens public curve state and atomically establishes ATA custody.

use associated_token_account_core::Instruction as AtaInstruction;
use lee_core::{
    account::{Account, AccountWithMetadata, Data},
    program::{AccountPostState, ChainedCall, Claim, ProgramId},
};
use token_core::{TokenDefinition, TokenHolding};

use crate::{SaleAccount, compute_creator_commitment, compute_sale_pda, compute_sale_pda_seed};

#[expect(
    clippy::too_many_arguments,
    reason = "the public instruction has seven accounts and six scalar parameters"
)]
#[must_use]
pub fn create_sale(
    sale_account: AccountWithMetadata,
    creator: AccountWithMetadata,
    token_definition: AccountWithMetadata,
    collateral_definition: AccountWithMetadata,
    creator_token_ata: AccountWithMetadata,
    sale_token_ata: AccountWithMetadata,
    sale_collateral_ata: AccountWithMetadata,
    sale_reserve: u128,
    dex_seed_reserve: u128,
    virtual_token_reserve: u128,
    virtual_collateral_reserve: u128,
    curve_program_id: ProgramId,
    ata_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    assert!(creator.is_authorized, "Creator authorization is missing");
    assert_eq!(
        sale_account.account_id,
        compute_sale_pda(
            curve_program_id,
            token_definition.account_id,
            collateral_definition.account_id,
        ),
        "Sale account ID does not match PDA"
    );
    assert_eq!(
        sale_account.account,
        Account::default(),
        "Sale account is already initialized"
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &creator_token_ata,
        &creator,
        token_definition.account_id,
        ata_program_id,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &sale_token_ata,
        &sale_account,
        token_definition.account_id,
        ata_program_id,
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &sale_collateral_ata,
        &sale_account,
        collateral_definition.account_id,
        ata_program_id,
    );

    let sale = sale::Sale::create(
        sale_reserve,
        dex_seed_reserve,
        virtual_token_reserve,
        virtual_collateral_reserve,
    )
    .expect("invalid sale parameters");
    let deposit = sale_reserve
        .checked_add(dex_seed_reserve)
        .expect("sale reserve plus DEX seed reserve overflows");

    let mut sale_post = sale_account.account;
    sale_post.data = Data::from(&SaleAccount {
        token_definition_id: token_definition.account_id,
        collateral_definition_id: collateral_definition.account_id,
        creator_commitment: compute_creator_commitment(creator.account_id, sale_account.account_id),
        sale,
    });
    let sale_seed = compute_sale_pda_seed(
        token_definition.account_id,
        collateral_definition.account_id,
    );

    let sale_owner = AccountWithMetadata {
        account: Account {
            program_owner: curve_program_id,
            ..sale_post.clone()
        },
        is_authorized: false,
        account_id: sale_account.account_id,
    };
    let create_token_ata = ChainedCall::new(
        ata_program_id,
        vec![
            sale_owner.clone(),
            token_definition.clone(),
            sale_token_ata.clone(),
        ],
        &AtaInstruction::Create { ata_program_id },
    );
    let create_collateral_ata = ChainedCall::new(
        ata_program_id,
        vec![
            sale_owner,
            collateral_definition.clone(),
            sale_collateral_ata.clone(),
        ],
        &AtaInstruction::Create { ata_program_id },
    );

    let definition = TokenDefinition::try_from(&token_definition.account.data)
        .expect("token definition account must hold a valid definition");
    let initialized_sale_token_ata = AccountWithMetadata {
        account: Account {
            program_owner: token_definition.account.program_owner,
            data: Data::from(&TokenHolding::zeroized_from_definition(
                token_definition.account_id,
                &definition,
            )),
            ..Account::default()
        },
        ..sale_token_ata.clone()
    };
    let fund_token_ata = ChainedCall::new(
        ata_program_id,
        vec![
            creator.clone(),
            creator_token_ata.clone(),
            initialized_sale_token_ata,
        ],
        &AtaInstruction::Transfer {
            ata_program_id,
            amount: deposit,
        },
    );

    let post_states = vec![
        AccountPostState::new_claimed_if_default(sale_post, Claim::Pda(sale_seed)),
        AccountPostState::new(creator.account),
        AccountPostState::new(token_definition.account),
        AccountPostState::new(collateral_definition.account),
        AccountPostState::new(creator_token_ata.account),
        AccountPostState::new(sale_token_ata.account),
        AccountPostState::new(sale_collateral_ata.account),
    ];

    (
        post_states,
        vec![create_token_ata, create_collateral_ata, fund_token_ata],
    )
}
