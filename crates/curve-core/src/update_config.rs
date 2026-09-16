//! `update_config`: creates the config PDA on the first call, replaces it whole after.

use lee_core::{
    account::{Account, AccountId, AccountWithMetadata, Data},
    program::{AccountPostState, Claim, ProgramId},
};

use crate::{Config, MAX_FEE_BPS, compute_config_pda, compute_config_pda_seed};

#[must_use]
pub fn update_config(
    namespace: AccountId,
    config: AccountWithMetadata,
    authority: AccountWithMetadata,
    admin: AccountId,
    protocol_fee_bps: u16,
    treasury: AccountId,
    curve_program_id: ProgramId,
) -> Vec<AccountPostState> {
    assert_eq!(
        config.account_id,
        compute_config_pda(namespace, curve_program_id),
        "Config account ID does not match PDA"
    );
    assert!(
        authority.is_authorized,
        "Authority authorization is missing"
    );

    assert_ne!(
        namespace,
        AccountId::default(),
        "Namespace must not be the default key"
    );
    // On the first call the account is empty and the gate is the namespace identity; afterwards it is whatever admin the config stores.
    let expected_admin = if config.account == Account::default() {
        namespace
    } else {
        assert_eq!(
            config.account.program_owner, curve_program_id,
            "Config owner does not match program"
        );
        Config::try_from(&config.account.data)
            .expect("Config account holds invalid data")
            .admin
    };
    assert_ne!(
        expected_admin,
        AccountId::default(),
        "Namespace administration is renounced"
    );
    assert_eq!(
        authority.account_id, expected_admin,
        "Authority is not the config admin"
    );

    assert!(
        protocol_fee_bps <= MAX_FEE_BPS,
        "Protocol fee exceeds 10,000 basis points"
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
        protocol_fee_bps,
        treasury,
    });

    vec![AccountPostState::new_claimed_if_default(
        config_post,
        Claim::Pda(compute_config_pda_seed(namespace)),
    )]
}

/// Renunciation preserves the namespace and fee settings, but removes all admin power.
pub fn renounce_admin(
    namespace: AccountId,
    config: AccountWithMetadata,
    authority: AccountWithMetadata,
    curve_program_id: ProgramId,
) -> Vec<AccountPostState> {
    assert_eq!(
        config.account_id,
        compute_config_pda(namespace, curve_program_id),
        "Config account ID does not match PDA"
    );
    assert_eq!(
        config.account.program_owner, curve_program_id,
        "Config owner does not match program"
    );
    let mut data =
        Config::try_from(&config.account.data).expect("Config account holds invalid data");
    assert_ne!(
        data.admin,
        AccountId::default(),
        "Namespace administration is renounced"
    );
    assert!(
        authority.is_authorized,
        "Authority authorization is missing"
    );
    assert_eq!(
        authority.account_id, data.admin,
        "Authority is not the config admin"
    );
    data.admin = AccountId::default();
    let mut post = config.account;
    post.data = Data::from(&data);
    vec![
        AccountPostState::new(post),
        AccountPostState::new(authority.account),
    ]
}
