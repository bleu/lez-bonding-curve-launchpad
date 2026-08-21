//! `update_config`: creates the config PDA on the first call, replaces it whole after.

use lee_core::{
    account::{Account, AccountId, AccountWithMetadata, Data},
    program::{AccountPostState, Claim, ProgramId},
};

use crate::{Config, GENESIS_ADMIN, MAX_FEE_BPS, compute_config_pda, compute_config_pda_seed};

#[must_use]
pub fn update_config(
    config: AccountWithMetadata,
    authority: AccountWithMetadata,
    admin: AccountId,
    pool_fee_bps: u16,
    protocol_fee_bps: u16,
    treasury: AccountId,
    curve_program_id: ProgramId,
) -> Vec<AccountPostState> {
    assert_eq!(
        config.account_id,
        compute_config_pda(curve_program_id),
        "Config account ID does not match PDA"
    );
    assert!(
        authority.is_authorized,
        "Authority authorization is missing"
    );

    // On the first call the account is empty and the gate is the compiled-in genesis
    // admin; afterwards it is whatever admin the config stores.
    let expected_admin = if config.account == Account::default() {
        GENESIS_ADMIN
    } else {
        Config::try_from(&config.account.data)
            .expect("Config account holds invalid data")
            .admin
    };
    assert_eq!(
        authority.account_id, expected_admin,
        "Authority is not the config admin"
    );

    let total_fee_bps = pool_fee_bps
        .checked_add(protocol_fee_bps)
        .expect("Combined fee rate exceeds u16");
    assert!(
        total_fee_bps <= MAX_FEE_BPS,
        "Combined fee rate exceeds 10,000 basis points"
    );
    assert_ne!(
        admin,
        AccountId::default(),
        "Admin must not be the default key"
    );
    assert_ne!(
        treasury,
        AccountId::default(),
        "Treasury must not be the default key"
    );

    let mut config_post = config.account;
    config_post.data = Data::from(&Config {
        admin,
        pool_fee_bps,
        protocol_fee_bps,
        treasury,
    });

    vec![AccountPostState::new_claimed_if_default(
        config_post,
        Claim::Pda(compute_config_pda_seed()),
    )]
}
