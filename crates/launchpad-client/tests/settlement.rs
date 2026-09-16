//! Actual guest execution, including LEZ's call limit and chained-state validation.
use lee::{
    privacy_preserving_transaction::circuit::{ProgramWithDependencies, execute_and_prove},
    program::Program,
};
use lee_core::{
    InputAccountIdentity,
    account::{Account, AccountId, AccountWithMetadata, Data},
};
use std::{collections::HashMap, path::PathBuf};
use token_core::TokenHolding;

fn guest(name: &str) -> Program {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../methods/target/riscv-guest/launchpad_methods/launchpad_programs/riscv32im-risc0-zkvm-elf/release");
    launchpad_client::load_program(&root.join(format!("{name}.bin"))).unwrap()
}
fn id(n: u8) -> AccountId {
    if n == 3 || n == 20 || n >= 99 {
        AccountId::from(&lee::PublicKey::new_from_private_key(
            &lee::PrivateKey::try_new([n; 32]).unwrap(),
        ))
    } else {
        AccountId::new([n; 32])
    }
}
fn ata(owner: AccountId, definition: AccountId) -> AccountId {
    associated_token_account_core::get_associated_token_account_id(
        &programs::ata().id(),
        &associated_token_account_core::compute_ata_seed(owner, definition),
    )
}
fn holding(definition: AccountId, balance: u128) -> Account {
    Account {
        program_owner: programs::token().id(),
        data: Data::from(&TokenHolding::Fungible {
            definition_id: definition,
            balance,
        }),
        ..Account::default()
    }
}
struct Chain {
    public_mode: bool,
    curve: Program,
    factory: Program,
    accounts: HashMap<AccountId, Account>,
}
impl Chain {
    fn new() -> Self {
        let mut accounts = HashMap::new();
        accounts.insert(
            clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
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
        );
        Self {
            public_mode: false,
            curve: guest("curve"),
            factory: guest("factory"),
            accounts,
        }
    }
    fn run<I: launchpad_client::PrivateInstruction>(
        &mut self,
        call: launchpad_client::PublicInvocation<I>,
    ) {
        if self.public_mode {
            self.run_public(call);
            return;
        }
        let program = if call.program_id == self.curve.id() {
            self.curve.clone()
        } else {
            self.factory.clone()
        };
        let pre: Vec<_> = call
            .account_ids
            .iter()
            .map(|id| {
                AccountWithMetadata::new(
                    self.accounts.get(id).cloned().unwrap_or_default(),
                    call.signer_accounts.contains(id),
                    *id,
                )
            })
            .collect();
        let identities = pre.iter().map(|_| InputAccountIdentity::Public).collect();
        let graph = ProgramWithDependencies::new(
            program,
            HashMap::from([
                (self.curve.id(), self.curve.clone()),
                (self.factory.id(), self.factory.clone()),
                (programs::token().id(), programs::token()),
                (programs::ata().id(), programs::ata()),
            ]),
        );
        let (out, _) = execute_and_prove(
            pre,
            Program::serialize_instruction(call.instruction.for_private_execution()).unwrap(),
            identities,
            &graph,
        )
        .expect("guest graph must execute");
        for (pre, post) in out
            .public_pre_states
            .into_iter()
            .zip(out.public_post_states)
        {
            self.accounts.insert(pre.account_id, post);
        }
    }
    fn run_public<I: serde::Serialize>(&mut self, call: launchpad_client::PublicInvocation<I>) {
        let mut state = lee::V03State::new()
            .with_public_accounts(self.accounts.clone())
            .with_programs([
                self.curve.clone(),
                self.factory.clone(),
                programs::token(),
                programs::ata(),
            ]);
        let keys: Vec<_> = call
            .signer_accounts
            .iter()
            .map(|signer| {
                let n = (1..=255)
                    .find(|n| id(*n) == *signer)
                    .expect("fixture signing key");
                lee::PrivateKey::try_new([n; 32]).unwrap()
            })
            .collect();
        let nonces = call
            .signer_accounts
            .iter()
            .map(|id| state.get_account_by_id(*id).nonce)
            .collect();
        let ids = call.account_ids.clone();
        let message = lee::public_transaction::Message::try_new(
            call.program_id,
            call.account_ids,
            nonces,
            call.instruction,
        )
        .unwrap();
        let witnesses = lee::public_transaction::WitnessSet::for_message(
            &message,
            &keys.iter().collect::<Vec<_>>(),
        );
        state
            .transition_from_public_transaction(
                &lee::PublicTransaction::new(message, witnesses),
                1,
                1,
            )
            .expect("public execution must validate sibling authorizations");
        for id in ids {
            self.accounts.insert(id, state.get_account_by_id(id));
        }
    }
    fn balance(&self, account: AccountId) -> u128 {
        if self
            .accounts
            .get(&account)
            .is_none_or(|a| *a == Account::default())
        {
            return 0;
        }
        match TokenHolding::try_from(&self.accounts[&account].data).unwrap() {
            TokenHolding::Fungible { balance, .. } => balance,
            _ => panic!("fungible holding"),
        }
    }
}
#[test]
#[ignore = "build methods and set RISC0_DEV_MODE=1"]
fn fee_transfers_use_current_balances_in_both_directions() {
    for public_mode in [false, true] {
        for sell in [false, true] {
            let mut chain = Chain::new();
            chain.public_mode = public_mode;
            let (namespace, owner, trader, token0, token1, treasury) =
                (id(1), id(2), id(3), id(4), id(5), id(6));
            let pool_id =
                curve_core::compute_pool_pda(namespace, chain.curve.id(), token0, token1, owner);
            let pool = curve_core::PoolAccount {
                funded: true,
                namespace,
                owner,
                owner_program: None,
                token0_definition_id: token0,
                token1_definition_id: token1,
                pool: pool::Pool::create(800, 100, 1000, 100, None, None).unwrap(),
            };
            chain.accounts.insert(
                pool_id,
                Account {
                    program_owner: chain.curve.id(),
                    data: Data::from(&pool),
                    ..Account::default()
                },
            );
            let config_id = curve_core::compute_config_pda(namespace, chain.curve.id());
            chain.accounts.insert(
                config_id,
                Account {
                    program_owner: chain.curve.id(),
                    data: Data::from(&curve_core::Config {
                        admin: namespace,
                        treasury,
                        protocol_fee_bps: if sell { 500 } else { 1000 },
                    }),
                    ..Account::default()
                },
            );
            chain.accounts.insert(
                trader,
                Account {
                    program_owner: [77; 8],
                    ..Account::default()
                },
            );
            for (owner, definition, balance) in [
                (pool_id, token0, 800),
                (pool_id, token1, 100),
                (trader, token0, 1000),
                (trader, token1, 1000),
                (treasury, token1, 0),
            ] {
                chain
                    .accounts
                    .insert(ata(owner, definition), holding(definition, balance));
            }
            let (input, output) = if sell {
                (token0, token1)
            } else {
                (token1, token0)
            };
            chain.run(launchpad_client::PublicInvocation {
                program_id: chain.curve.id(),
                account_ids: vec![
                    pool_id,
                    config_id,
                    trader,
                    ata(trader, input),
                    ata(pool_id, input),
                    ata(pool_id, output),
                    ata(trader, output),
                    ata(treasury, token1),
                    clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
                ],
                signer_accounts: vec![trader],
                instruction: if sell {
                    curve_core::Instruction::SwapExactInput {
                        token_in: token0,
                        amount_in: 250,
                        min_amount_out: 19,
                    }
                } else {
                    curve_core::Instruction::SwapExactOutput {
                        token_in: token1,
                        amount_out: 200,
                        max_amount_in: 28,
                    }
                },
            });
            if sell {
                assert_eq!(chain.balance(ata(trader, token0)), 750);
                assert_eq!(chain.balance(ata(trader, token1)), 1019);
                assert_eq!(chain.balance(ata(treasury, token1)), 1);
                assert_eq!(chain.balance(ata(pool_id, token1)), 80);
            } else {
                assert_eq!(chain.balance(ata(trader, token0)), 1200);
                assert_eq!(chain.balance(ata(trader, token1)), 972);
                assert_eq!(chain.balance(ata(treasury, token1)), 3);
                assert_eq!(chain.balance(ata(pool_id, token1)), 125);
            }
        }
    }
}

impl Chain {
    fn private_source(&mut self, nft: AccountId) -> AccountId {
        let nsk = [9; 32];
        let id = if self.public_mode {
            id(99)
        } else {
            AccountId::for_regular_private_account(&lee_core::NullifierPublicKey::from(&nsk), 0)
        };
        self.accounts.insert(
            id,
            Account {
                program_owner: programs::token().id(),
                data: Data::from(&TokenHolding::NftMaster {
                    definition_id: nft,
                    print_balance: 1,
                }),
                nonce: lee_core::account::Nonce::private_account_nonce_init(&id),
                ..Account::default()
            },
        );
        id
    }
    fn try_private<I: launchpad_client::PrivateInstruction>(
        &mut self,
        source: AccountId,
        call: launchpad_client::PublicInvocation<I>,
    ) -> Result<(), lee::error::LeeError> {
        let holder = call.signer_accounts[0];
        let instruction =
            launchpad_client::build_private_authority_instruction(&call, holder).unwrap();
        let mut pre = vec![AccountWithMetadata::new(
            self.accounts[&source].clone(),
            true,
            source,
        )];
        pre.extend(call.account_ids.iter().map(|id| {
            AccountWithMetadata::new(
                self.accounts.get(id).cloned().unwrap_or_default(),
                *id == holder,
                *id,
            )
        }));
        let ssk = lee_core::SharedSecretKey([10; 32]);
        let mut identities = vec![InputAccountIdentity::PrivateAuthorizedUpdate {
            epk: lee_core::EphemeralPublicKey(vec![]),
            view_tag: Default::default(),
            ssk,
            nsk: [9; 32],
            membership_proof: (0, vec![]),
            identifier: 0,
        }];
        identities.extend(
            call.account_ids
                .iter()
                .map(|_| InputAccountIdentity::Public),
        );
        let graph = ProgramWithDependencies::new(
            guest("private_authority"),
            HashMap::from([
                (self.curve.id(), self.curve.clone()),
                (self.factory.id(), self.factory.clone()),
                (programs::token().id(), programs::token()),
                (programs::ata().id(), programs::ata()),
            ]),
        );
        let (out, _) = execute_and_prove(
            pre,
            Program::serialize_instruction(instruction).unwrap(),
            identities,
            &graph,
        )?;
        let (_, returned) = lee_core::EncryptionScheme::decrypt(
            &out.encrypted_private_post_states[0].ciphertext,
            &ssk,
            &out.new_commitments[0],
            0,
        )
        .unwrap();
        assert_eq!(
            returned.data, self.accounts[&source].data,
            "authority returns after each stage"
        );
        self.accounts.insert(source, returned);
        for (pre, post) in out
            .public_pre_states
            .into_iter()
            .zip(out.public_post_states)
        {
            self.accounts.insert(pre.account_id, post);
        }
        assert!(matches!(
            TokenHolding::try_from(&self.accounts[&holder].data).unwrap(),
            TokenHolding::NftMaster {
                print_balance: 0,
                ..
            }
        ));
        Ok(())
    }
    fn run_private<I: launchpad_client::PrivateInstruction>(
        &mut self,
        source: AccountId,
        call: launchpad_client::PublicInvocation<I>,
    ) {
        if self.public_mode {
            let holder = call.signer_accounts[0];
            self.run_public(launchpad_client::build_transfer_authority_invocation(
                source, holder,
            ));
            self.run_public(call);
            self.run_public(launchpad_client::build_transfer_authority_invocation(
                holder, source,
            ));
        } else {
            self.try_private(source, call)
                .expect("private stage fits LEZ limit and validates all chained states");
        }
    }
}
#[test]
#[ignore = "build methods and set RISC0_DEV_MODE=1"]
fn private_sale_resumes_creation_and_settlement_with_fresh_payout_holders() {
    for public_mode in [false, true] {
        for (allocation, seed, bought, deadline, collateral_due, remaining_supply) in [
            (100, 100, 200, None, 12, 400),
            (0, 0, 800, None, 67, 800),
            (100, 100, 0, Some(2), 0, 200),
        ] {
            let mut chain = Chain::new();
            chain.public_mode = public_mode;
            let namespace = id(1);
            let nft = id(2);
            let collateral = id(5);
            let salt = [7; 32];
            let source = chain.private_source(nft);
            chain.accounts.insert(
                collateral,
                Account {
                    program_owner: programs::token().id(),
                    data: Data::from(&token_core::TokenDefinition::Fungible {
                        name: "Collateral".into(),
                        total_supply: 10000,
                        metadata_id: None,
                    }),
                    ..Account::default()
                },
            );
            chain.accounts.insert(
                curve_core::compute_config_pda(namespace, chain.curve.id()),
                Account {
                    program_owner: chain.curve.id(),
                    data: Data::from(&curve_core::Config {
                        admin: namespace,
                        treasury: id(6),
                        protocol_fee_bps: 0,
                    }),
                    ..Account::default()
                },
            );
            let request = launchpad_client::CreateSaleRequest {
                launch_salt: salt,
                name: "Sale".into(),
                uri: "ipfs://sale".into(),
                sale_reserve: 800,
                dex_seed_reserve: seed,
                creator_allocation: allocation,
                virtual_token_reserve: 2000,
                virtual_collateral_reserve: 100,
                end_timestamp: deadline,
                collateral_definition: collateral,
            };
            let factory_id = factory_core::compute_factory_pda(namespace, chain.factory.id(), salt);
            let token = factory_core::compute_definition_pda(namespace, chain.factory.id(), salt);
            let pool_id = curve_core::compute_pool_pda(
                namespace,
                chain.curve.id(),
                token,
                collateral,
                factory_id,
            );
            for step in 0..4 {
                let existing = chain
                    .accounts
                    .get(&factory_id)
                    .map(|a| factory_core::FactoryState::try_from(&a.data).unwrap());
                let call = launchpad_client::next_creation_invocation(
                    namespace,
                    chain.factory.id(),
                    chain.curve.id(),
                    id(100 + step),
                    request.clone(),
                    existing.as_ref(),
                )
                .unwrap()
                .expect("next creation stage");
                chain.run_private(source, call);
                if step == 1 && deadline.is_some() {
                    chain
                        .accounts
                        .get_mut(&clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID)
                        .unwrap()
                        .data = Data::try_from(
                        clock_core::ClockAccountData {
                            block_id: 2,
                            timestamp: 3,
                        }
                        .to_bytes(),
                    )
                    .unwrap();
                }
                if step == 2 {
                    assert!(
                        !curve_core::PoolAccount::try_from(&chain.accounts[&pool_id].data)
                            .unwrap()
                            .funded
                    );
                }
            }
            let state =
                factory_core::FactoryState::try_from(&chain.accounts[&factory_id].data).unwrap();
            assert_eq!(state.creation_stage, factory_core::CreationStage::Active);
            assert!(
                launchpad_client::next_creation_invocation(
                    namespace,
                    chain.factory.id(),
                    chain.curve.id(),
                    id(104),
                    request.clone(),
                    Some(&state)
                )
                .unwrap()
                .is_none()
            );
            let mut changed = request.clone();
            changed.sale_reserve = 700;
            assert!(
                launchpad_client::next_creation_invocation(
                    namespace,
                    chain.factory.id(),
                    chain.curve.id(),
                    id(104),
                    changed,
                    Some(&state)
                )
                .is_err()
            );
            assert!(
                curve_core::PoolAccount::try_from(&chain.accounts[&pool_id].data)
                    .unwrap()
                    .funded
            );
            assert_eq!(chain.balance(ata(pool_id, token)), 800);
            assert_eq!(chain.balance(ata(factory_id, token)), seed);
            assert_eq!(
                chain.balance(factory_core::compute_escrow_pda(
                    namespace,
                    chain.factory.id(),
                    salt
                )),
                allocation
            );
            assert_eq!(
                chain.balance(factory_core::compute_mint_pda(
                    namespace,
                    chain.factory.id(),
                    salt
                )),
                0
            );
            let trader = id(20);
            chain.accounts.insert(
                trader,
                Account {
                    program_owner: [77; 8],
                    ..Account::default()
                },
            );
            chain.accounts.insert(ata(trader, token), holding(token, 0));
            chain
                .accounts
                .insert(ata(trader, collateral), holding(collateral, 1000));
            chain
                .accounts
                .insert(ata(id(6), collateral), holding(collateral, 0));
            if bought != 0 {
                chain.run(launchpad_client::build_buy_invocation(
                    namespace,
                    chain.factory.id(),
                    chain.curve.id(),
                    trader,
                    id(6),
                    launchpad_client::BuyRequest {
                        launch_salt: salt,
                        collateral_definition: collateral,
                        amount_out: bought,
                        max_amount_in: 100,
                    },
                ));
            }
            assert_eq!(chain.balance(ata(pool_id, collateral)), collateral_due);
            if bought < 800 && deadline.is_none() {
                chain.run_private(
                    source,
                    launchpad_client::build_close_factory_pool_invocation(
                        namespace,
                        chain.factory.id(),
                        chain.curve.id(),
                        id(104),
                        salt,
                        collateral,
                    ),
                );
            }
            chain.run_private(
                source,
                launchpad_client::build_claim_creator_allocation_invocation(
                    namespace,
                    chain.factory.id(),
                    chain.curve.id(),
                    id(105),
                    salt,
                    collateral,
                ),
            );
            assert_eq!(chain.balance(ata(id(105), token)), allocation);
            for step in 0..5 {
                chain.run_private(
                    source,
                    launchpad_client::build_withdraw_factory_proceeds_invocation(
                        namespace,
                        chain.factory.id(),
                        chain.curve.id(),
                        id(106 + step),
                        salt,
                        collateral,
                    ),
                );
            }
            assert_eq!(
                factory_core::FactoryState::try_from(&chain.accounts[&factory_id].data)
                    .unwrap()
                    .settlement_stage,
                factory_core::SettlementStage::Complete
            );
            assert_eq!(chain.balance(ata(id(109), token)), seed);
            assert_eq!(chain.balance(ata(id(110), collateral)), collateral_due);
            assert_eq!(chain.balance(ata(factory_id, token)), 0);
            assert_eq!(chain.balance(ata(factory_id, collateral)), 0);
            let token_core::TokenDefinition::Fungible { total_supply, .. } =
                token_core::TokenDefinition::try_from(&chain.accounts[&token].data).unwrap()
            else {
                panic!("fungible definition")
            };
            assert_eq!(
                total_supply, remaining_supply,
                "burn only the unsold allocation"
            );
            if !public_mode {
                let saved = chain.accounts.clone();
                assert!(
                    chain
                        .try_private(
                            source,
                            launchpad_client::build_withdraw_factory_proceeds_invocation(
                                namespace,
                                chain.factory.id(),
                                chain.curve.id(),
                                id(111),
                                salt,
                                collateral
                            )
                        )
                        .is_err()
                );
                assert_eq!(
                    chain.accounts, saved,
                    "replay leaves all balances unchanged"
                );
            }
        }
    }
}
