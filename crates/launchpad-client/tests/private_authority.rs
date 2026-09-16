//! Run after building methods, with RISC0_DEV_MODE=1. Executes actual pinned guest ELFs.
use lee::{
    privacy_preserving_transaction::circuit::{ProgramWithDependencies, execute_and_prove},
    program::Program,
};
use lee_core::{
    EncryptionScheme, EphemeralPublicKey, InputAccountIdentity, SharedSecretKey,
    account::{Account, AccountId, AccountWithMetadata, Data, Nonce},
};
use std::{collections::HashMap, path::PathBuf};
use token_core::TokenHolding;

fn guest(name: &str) -> Program {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../methods/target/riscv-guest/launchpad_methods/launchpad_programs/riscv32im-risc0-zkvm-elf/release");
    launchpad_client::load_program(&root.join(format!("{name}.bin")))
        .expect("build methods before running guest tests")
}

#[test]
#[ignore = "executes RISC0 guests; build methods and set RISC0_DEV_MODE=1"]
fn private_namespace_action_returns_authority_and_rejects_foreign_nft() {
    let router = guest("private_authority");
    let curve = guest("curve");
    let nsk = [4; 32];
    let npk = lee_core::NullifierPublicKey::from(&nsk);
    let source_id = AccountId::for_regular_private_account(&npk, 0);
    let namespace = AccountId::new([81; 32]);
    let source = AccountWithMetadata::new(
        Account {
            program_owner: programs::token().id(),
            data: Data::from(&TokenHolding::NftMaster {
                definition_id: namespace,
                print_balance: 1,
            }),
            nonce: Nonce::private_account_nonce_init(&source_id),
            ..Account::default()
        },
        true,
        source_id,
    );

    let ssk = SharedSecretKey([5; 32]);
    let identities = vec![
        InputAccountIdentity::PrivateAuthorizedUpdate {
            epk: EphemeralPublicKey(Vec::new()),
            view_tag: Default::default(),
            ssk,
            nsk,
            membership_proof: (0, vec![]),
            identifier: 0,
        },
        InputAccountIdentity::Public,
        InputAccountIdentity::Public,
    ];
    let config_id = curve_core::compute_config_pda(namespace, curve.id());
    let pre = vec![
        source.clone(),
        AccountWithMetadata::new(Account::default(), false, config_id),
        AccountWithMetadata::new(Account::default(), true, AccountId::new([82; 32])),
    ];
    let action = curve_core::Instruction::UpdateConfig {
        namespace,
        admin: namespace,
        protocol_fee_bps: 25,
        treasury: AccountId::new([83; 32]),
    };
    let instruction = private_flow_core::PrivateAuthorityInstruction {
        program_id: curve.id(),
        authority_index: 1,
        instruction_data: Program::serialize_instruction(action).unwrap(),
    };
    let graph = ProgramWithDependencies::new(
        router,
        HashMap::from([
            (curve.id(), curve),
            (programs::token().id(), programs::token()),
        ]),
    );
    let (output, _) = execute_and_prove(
        pre.clone(),
        Program::serialize_instruction(instruction.clone()).unwrap(),
        identities.clone(),
        &graph,
    )
    .expect("atomic private authority action");
    let (_, returned) = EncryptionScheme::decrypt(
        &output.encrypted_private_post_states[0].ciphertext,
        &ssk,
        &output.new_commitments[0],
        0,
    )
    .unwrap();
    assert_eq!(
        TokenHolding::try_from(&returned.data).unwrap(),
        TokenHolding::NftMaster {
            definition_id: namespace,
            print_balance: 1
        }
    );
    assert_eq!(output.new_nullifiers.len(), 1);
    assert_eq!(output.new_commitments.len(), 1);
    assert!(output.public_post_states.iter().any(|post| {
        curve_core::Config::try_from(&post.data).is_ok_and(|config| config.protocol_fee_bps == 25)
    }));
    let mut bad_pre = pre;
    bad_pre[0].account.data = Data::from(&TokenHolding::NftMaster {
        definition_id: AccountId::new([84; 32]),
        print_balance: 1,
    });
    assert!(
        execute_and_prove(
            bad_pre,
            Program::serialize_instruction(instruction).unwrap(),
            identities,
            &graph
        )
        .is_err()
    );
}

#[test]
#[ignore = "executes RISC0 guests; build methods and set RISC0_DEV_MODE=1"]
fn private_nft_closes_direct_and_factory_pools() {
    for factory_owned in [false, true] {
        let curve = guest("curve");
        let factory_program = guest("factory");
        let namespace = AccountId::new([81; 32]);
        let nft = AccountId::new([85; 32]);
        let token0 = AccountId::new([86; 32]);
        let token1 = AccountId::new([87; 32]);
        let salt = [6; 32];
        let factory_id = factory_core::compute_factory_pda(namespace, factory_program.id(), salt);
        let owner = if factory_owned { factory_id } else { nft };
        let pool_id = curve_core::compute_pool_pda(namespace, curve.id(), token0, token1, owner);
        let pool_state = curve_core::PoolAccount {
            funded: true,
            namespace,
            token0_definition_id: token0,
            token1_definition_id: token1,
            owner,
            owner_program: factory_owned.then(|| {
                (
                    factory_program.id(),
                    *factory_core::compute_factory_seed(namespace, salt).as_bytes(),
                )
            }),
            pool: pool::Pool::create(800, 0, 2000, 100, None, Some(pool::TokenSide::Token0))
                .unwrap(),
        };
        let pool_account = AccountWithMetadata::new(
            Account {
                program_owner: curve.id(),
                data: Data::from(&pool_state),
                ..Account::default()
            },
            false,
            pool_id,
        );
        let clock = AccountWithMetadata::new(
            Account {
                program_owner: [88; 8],
                data: Data::try_from(
                    clock_core::ClockAccountData {
                        block_id: 1,
                        timestamp: 1,
                    }
                    .to_bytes(),
                )
                .unwrap(),
                ..Account::default()
            },
            false,
            clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
        );
        let transient =
            AccountWithMetadata::new(Account::default(), true, AccountId::new([82; 32]));
        let (action, accounts, authority_index, program_id) = if factory_owned {
            let state = factory_core::FactoryState {
                settlement_stage: factory_core::SettlementStage::Unstarted,
                creation_stage: factory_core::CreationStage::Active,
                end_timestamp: None,
                namespace,
                launch_salt: salt,
                token_definition_id: token0,
                collateral_definition_id: token1,
                sale_reserve: 800,
                dex_seed_reserve: 100,
                creator_allocation: 100,
                total_supply: 1000,
                virtual_token_reserve: 2000,
                virtual_collateral_reserve: 100,
                curve_program_id: curve.id(),
                creator_commitment: factory_core::compute_creator_commitment(namespace, nft, salt),
                creator_escrow_id: factory_core::compute_escrow_pda(
                    namespace,
                    factory_program.id(),
                    salt,
                ),
                pool_id,
                creator_allocation_claimed: false,
            };
            let factory = AccountWithMetadata::new(
                Account {
                    program_owner: factory_program.id(),
                    data: Data::from(&state),
                    ..Account::default()
                },
                false,
                factory_id,
            );
            (
                Program::serialize_instruction(factory_core::Instruction::CloseFactoryPool)
                    .unwrap(),
                vec![factory, pool_account, transient, clock],
                2,
                factory_program.id(),
            )
        } else {
            (
                Program::serialize_instruction(curve_core::Instruction::ClosePool).unwrap(),
                vec![pool_account, transient, clock],
                1,
                curve.id(),
            )
        };
        let nsk = [4; 32];
        let source_id =
            AccountId::for_regular_private_account(&lee_core::NullifierPublicKey::from(&nsk), 0);
        let source = AccountWithMetadata::new(
            Account {
                program_owner: programs::token().id(),
                data: Data::from(&TokenHolding::NftMaster {
                    definition_id: nft,
                    print_balance: 1,
                }),
                nonce: Nonce::private_account_nonce_init(&source_id),
                ..Account::default()
            },
            true,
            source_id,
        );
        let ssk = SharedSecretKey([5; 32]);
        let mut identities = vec![InputAccountIdentity::PrivateAuthorizedUpdate {
            epk: EphemeralPublicKey(Vec::new()),
            view_tag: Default::default(),
            ssk,
            nsk,
            membership_proof: (0, vec![]),
            identifier: 0,
        }];
        identities.extend(accounts.iter().map(|_| InputAccountIdentity::Public));
        let mut pre = vec![source];
        pre.extend(accounts);
        let graph = ProgramWithDependencies::new(
            guest("private_authority"),
            HashMap::from([
                (curve.id(), curve),
                (factory_program.id(), factory_program),
                (programs::token().id(), programs::token()),
            ]),
        );
        let instruction = private_flow_core::PrivateAuthorityInstruction {
            program_id,
            instruction_data: action,
            authority_index,
        };
        let (output, _) = execute_and_prove(
            pre,
            Program::serialize_instruction(instruction).unwrap(),
            identities,
            &graph,
        )
        .expect("private close");
        let (_, returned) = EncryptionScheme::decrypt(
            &output.encrypted_private_post_states[0].ciphertext,
            &ssk,
            &output.new_commitments[0],
            0,
        )
        .unwrap();
        assert_eq!(
            TokenHolding::try_from(&returned.data).unwrap(),
            TokenHolding::NftMaster {
                definition_id: nft,
                print_balance: 1
            }
        );
        assert!(output.public_post_states.iter().any(|post| {
            curve_core::PoolAccount::try_from(&post.data)
                .is_ok_and(|state| state.pool.effective_lifecycle(1) == pool::PoolLifecycle::Closed)
        }));
    }
}
