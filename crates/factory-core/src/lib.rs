//! Launch policy layered over the neutral curve pool.
//!
//! The factory creates one fixed-supply launch token, records the public split, and
//! atomically tail-calls the curve's neutral `CreatePool` instruction.  The factory
//! owns the token-definition PDA and intentionally exposes no mint or metadata-update
//! instruction: this is the authority-revocation boundary for the pinned token API.

use borsh::{BorshDeserialize, BorshSerialize};
use curve_core::{DepletionSide, Instruction as CurveInstruction, PoolAccount, compute_pool_pda};
use lee_core::{
    account::{Account, AccountId, AccountWithMetadata, Data},
    program::{AccountPostState, ChainedCall, Claim, PdaSeed, ProgramId},
};
use pool::PoolLifecycle;
use serde::{Deserialize, Serialize};
use token_core::{MetadataStandard, NewTokenDefinition, NewTokenMetadata, TokenHolding};

pub const ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID: ProgramId =
    curve_core::ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID;

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct FactoryState {
    pub launch_salt: [u8; 32],
    pub token_definition_id: AccountId,
    pub collateral_definition_id: AccountId,
    pub sale_reserve: u128,
    pub dex_seed_reserve: u128,
    pub creator_allocation: u128,
    pub total_supply: u128,
    pub virtual_token_reserve: u128,
    pub virtual_collateral_reserve: u128,
    pub curve_program_id: ProgramId,
    pub creator_commitment: [u8; 32],
    pub creator_escrow_id: AccountId,
    pub pool_id: AccountId,
    pub creator_allocation_claimed: bool,
}

impl TryFrom<&Data> for FactoryState {
    type Error = std::io::Error;
    fn try_from(data: &Data) -> Result<Self, Self::Error> {
        Self::try_from_slice(data.as_ref())
    }
}

impl From<&FactoryState> for Data {
    fn from(state: &FactoryState) -> Self {
        let mut bytes = Vec::with_capacity(std::mem::size_of_val(state));
        BorshSerialize::serialize(state, &mut bytes).expect("factory state serialises");
        Self::try_from(bytes).expect("factory state fits account data")
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "the compact wire enum keeps guest serialization direct"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Instruction {
    CreateFactoryPool {
        launch_salt: [u8; 32],
        name: String,
        uri: String,
        sale_reserve: u128,
        dex_seed_reserve: u128,
        creator_allocation: u128,
        virtual_token_reserve: u128,
        virtual_collateral_reserve: u128,
        end_timestamp: Option<u64>,
        curve_program_id: ProgramId,
    },
    CloseFactoryPool,
    ClaimCreatorAllocation,
    WithdrawFactoryProceeds,
}

/// The only factory-facing validation error. Account and token adapter violations use
/// explicit panics, matching the neighbouring curve-core handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateError {
    SaleReserveZero,
    SupplyOverflow,
    EmptyName,
    EmptyUri,
}

impl std::fmt::Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::SaleReserveZero => "a factory launch needs a non-zero sale reserve",
            Self::SupplyOverflow => "launch allocations exceed u128 total supply",
            Self::EmptyName => "token name must not be empty",
            Self::EmptyUri => "token metadata URI must not be empty",
        };
        f.write_str(text)
    }
}
impl std::error::Error for CreateError {}

pub fn total_supply(
    sale_reserve: u128,
    dex_seed_reserve: u128,
    creator_allocation: u128,
) -> Result<u128, CreateError> {
    if sale_reserve == 0 {
        return Err(CreateError::SaleReserveZero);
    }
    sale_reserve
        .checked_add(dex_seed_reserve)
        .and_then(|value| value.checked_add(creator_allocation))
        .ok_or(CreateError::SupplyOverflow)
}

fn seed(tag: &[u8], launch_salt: [u8; 32]) -> PdaSeed {
    use risc0_zkvm::sha::{Impl, Sha256 as _};
    let mut bytes = [0_u8; 64];
    bytes[..tag.len()].copy_from_slice(tag);
    bytes[32..].copy_from_slice(&launch_salt);
    PdaSeed::new(
        Impl::hash_bytes(&bytes)
            .as_bytes()
            .try_into()
            .expect("sha256 is 32 bytes"),
    )
}
pub fn compute_factory_seed(launch_salt: [u8; 32]) -> PdaSeed {
    seed(b"factory", launch_salt)
}
pub fn compute_definition_seed(launch_salt: [u8; 32]) -> PdaSeed {
    seed(b"definition", launch_salt)
}
pub fn compute_mint_seed(launch_salt: [u8; 32]) -> PdaSeed {
    seed(b"mint", launch_salt)
}
pub fn compute_metadata_seed(launch_salt: [u8; 32]) -> PdaSeed {
    seed(b"metadata", launch_salt)
}
pub fn compute_escrow_seed(launch_salt: [u8; 32]) -> PdaSeed {
    seed(b"escrow", launch_salt)
}
pub fn compute_factory_pda(factory_program_id: ProgramId, launch_salt: [u8; 32]) -> AccountId {
    AccountId::for_public_pda(&factory_program_id, &compute_factory_seed(launch_salt))
}
pub fn compute_definition_pda(factory_program_id: ProgramId, launch_salt: [u8; 32]) -> AccountId {
    AccountId::for_public_pda(&factory_program_id, &compute_definition_seed(launch_salt))
}
pub fn compute_mint_pda(factory_program_id: ProgramId, launch_salt: [u8; 32]) -> AccountId {
    AccountId::for_public_pda(&factory_program_id, &compute_mint_seed(launch_salt))
}
pub fn compute_metadata_pda(factory_program_id: ProgramId, launch_salt: [u8; 32]) -> AccountId {
    AccountId::for_public_pda(&factory_program_id, &compute_metadata_seed(launch_salt))
}
pub fn compute_escrow_pda(factory_program_id: ProgramId, launch_salt: [u8; 32]) -> AccountId {
    AccountId::for_public_pda(&factory_program_id, &compute_escrow_seed(launch_salt))
}

/// Commits a private creator account to one launch without recording its account ID in factory
/// state. The creator account itself must be authorized whenever this commitment is used.
pub fn compute_creator_commitment(creator_id: AccountId, launch_salt: [u8; 32]) -> [u8; 32] {
    use risc0_zkvm::sha::{Impl, Sha256 as _};
    let mut bytes = [0_u8; 64];
    bytes[..32].copy_from_slice(&creator_id.to_bytes());
    bytes[32..].copy_from_slice(&launch_salt);
    Impl::hash_bytes(&bytes)
        .as_bytes()
        .try_into()
        .expect("sha256 is 32 bytes")
}

#[expect(
    clippy::too_many_arguments,
    reason = "the public launch interface owns its explicit accounts"
)]
#[must_use]
pub fn create_factory_pool(
    factory: AccountWithMetadata,
    token_definition: AccountWithMetadata,
    mint_holding: AccountWithMetadata,
    metadata: AccountWithMetadata,
    creator_escrow: AccountWithMetadata,
    creator: AccountWithMetadata,
    creator_holding: AccountWithMetadata,
    collateral_definition: AccountWithMetadata,
    factory_token_ata: AccountWithMetadata,
    factory_collateral_ata: AccountWithMetadata,
    pool: AccountWithMetadata,
    pool_token_ata: AccountWithMetadata,
    pool_collateral_ata: AccountWithMetadata,
    launch_salt: [u8; 32],
    name: String,
    uri: String,
    sale_reserve: u128,
    dex_seed_reserve: u128,
    creator_allocation: u128,
    virtual_token_reserve: u128,
    virtual_collateral_reserve: u128,
    end_timestamp: Option<u64>,
    clock: AccountWithMetadata,
    factory_program_id: ProgramId,
    curve_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    assert!(!name.is_empty(), "token name must not be empty");
    assert!(!uri.is_empty(), "token metadata URI must not be empty");
    let supply = total_supply(sale_reserve, dex_seed_reserve, creator_allocation)
        .expect("launch allocations are valid");
    assert_eq!(
        factory.account_id,
        compute_factory_pda(factory_program_id, launch_salt),
        "Factory account ID does not match PDA"
    );
    assert_eq!(
        factory.account,
        Account::default(),
        "Launch salt is already in use"
    );
    assert_eq!(
        token_definition.account_id,
        compute_definition_pda(factory_program_id, launch_salt),
        "Token definition ID does not match PDA"
    );
    assert_eq!(
        mint_holding.account_id,
        compute_mint_pda(factory_program_id, launch_salt),
        "Mint holding ID does not match PDA"
    );
    assert_eq!(
        metadata.account_id,
        compute_metadata_pda(factory_program_id, launch_salt),
        "Metadata ID does not match PDA"
    );
    assert_eq!(
        creator_escrow.account_id,
        compute_escrow_pda(factory_program_id, launch_salt),
        "Creator escrow ID does not match PDA"
    );
    assert!(creator.is_authorized, "Creator authorization is missing");
    let now = trusted_time(&clock);
    assert!(
        end_timestamp.is_none_or(|timestamp| timestamp > now),
        "End timestamp must be in the future"
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &creator_holding,
        &creator,
        token_definition.account_id,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    assert_eq!(
        pool.account_id,
        compute_pool_pda(
            curve_program_id,
            token_definition.account_id,
            collateral_definition.account_id,
            factory.account_id
        ),
        "Pool ID does not match factory-owned PDA"
    );

    let factory_authorized = AccountWithMetadata {
        is_authorized: true,
        ..factory.clone()
    };
    let definition_authorized = AccountWithMetadata {
        is_authorized: true,
        ..token_definition.clone()
    };
    let mint_authorized = AccountWithMetadata {
        is_authorized: true,
        ..mint_holding.clone()
    };
    let metadata_authorized = AccountWithMetadata {
        is_authorized: true,
        ..metadata.clone()
    };
    let mut calls = vec![
        ChainedCall::new(
            token_definition.account.program_owner,
            vec![definition_authorized, mint_authorized, metadata_authorized],
            &token_core::Instruction::NewDefinitionWithMetadata {
                new_definition: NewTokenDefinition::Fungible {
                    name,
                    total_supply: supply,
                },
                metadata: Box::new(NewTokenMetadata {
                    standard: MetadataStandard::Simple,
                    uri,
                    creators: String::new(),
                }),
            },
        )
        .with_pda_seeds(vec![
            compute_definition_seed(launch_salt),
            compute_mint_seed(launch_salt),
            compute_metadata_seed(launch_salt),
        ]),
    ];

    // First establish the factory ATA, then move D + R from the one-time mint holding.
    calls.push(ChainedCall::new(
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
        vec![
            factory.clone(),
            token_definition.clone(),
            factory_token_ata.clone(),
        ],
        &associated_token_account_core::Instruction::Create {
            ata_program_id: ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
        },
    ));
    let factory_token_allocation = sale_reserve
        .checked_add(dex_seed_reserve)
        .expect("total supply was checked");
    calls.push(
        ChainedCall::new(
            token_definition.account.program_owner,
            vec![
                AccountWithMetadata {
                    is_authorized: true,
                    ..mint_holding.clone()
                },
                factory_token_ata.clone(),
            ],
            &token_core::Instruction::Transfer {
                amount_to_transfer: factory_token_allocation,
            },
        )
        .with_pda_seeds(vec![compute_mint_seed(launch_salt)]),
    );
    if creator_allocation != 0 {
        // Every creator allocation is escrowed until the pool is effectively closed.
        let recipient = AccountWithMetadata {
            is_authorized: true,
            ..creator_escrow.clone()
        };
        let allocation_seeds = vec![
            compute_mint_seed(launch_salt),
            compute_escrow_seed(launch_salt),
        ];
        calls.push(
            ChainedCall::new(
                token_definition.account.program_owner,
                vec![
                    AccountWithMetadata {
                        is_authorized: true,
                        ..mint_holding.clone()
                    },
                    recipient,
                ],
                &token_core::Instruction::Transfer {
                    amount_to_transfer: creator_allocation,
                },
            )
            .with_pda_seeds(allocation_seeds),
        );
    }
    // The pool owns exactly the tradeable D portion. The factory retains R.
    calls.push(
        ChainedCall::new(
            curve_program_id,
            vec![
                pool.clone(),
                factory_authorized,
                token_definition.clone(),
                collateral_definition.clone(),
                factory_token_ata.clone(),
                factory_collateral_ata.clone(),
                pool_token_ata.clone(),
                pool_collateral_ata.clone(),
                clock.clone(),
            ],
            &CurveInstruction::CreatePool {
                token0_amount: sale_reserve,
                token1_amount: 0,
                virtual_reserve0: virtual_token_reserve,
                virtual_reserve1: virtual_collateral_reserve,
                close_timestamp: end_timestamp,
                close_on_depletion: Some(DepletionSide::Token0),
                owner: factory.account_id,
                curve_program_id,
            },
        )
        .with_pda_seeds(vec![compute_factory_seed(launch_salt)]),
    );

    let state = FactoryState {
        launch_salt,
        token_definition_id: token_definition.account_id,
        collateral_definition_id: collateral_definition.account_id,
        sale_reserve,
        dex_seed_reserve,
        creator_allocation,
        total_supply: supply,
        virtual_token_reserve,
        virtual_collateral_reserve,
        curve_program_id,
        creator_commitment: compute_creator_commitment(creator.account_id, launch_salt),
        creator_escrow_id: creator_escrow.account_id,
        pool_id: pool.account_id,
        creator_allocation_claimed: false,
    };
    let mut post = factory.account;
    post.data = Data::from(&state);
    (
        vec![
            AccountPostState::new_claimed_if_default(
                post,
                Claim::Pda(compute_factory_seed(launch_salt)),
            ),
            AccountPostState::new(token_definition.account),
            AccountPostState::new(mint_holding.account),
            AccountPostState::new(metadata.account),
            AccountPostState::new(creator_escrow.account),
            AccountPostState::new(creator.account),
            AccountPostState::new(creator_holding.account),
            AccountPostState::new(collateral_definition.account),
            AccountPostState::new(factory_token_ata.account),
            AccountPostState::new(factory_collateral_ata.account),
            AccountPostState::new(pool.account),
            AccountPostState::new(pool_token_ata.account),
            AccountPostState::new(pool_collateral_ata.account),
        ],
        calls,
    )
}

/// Marks a delayed allocation as released once the factory-owned pool is closed. Token
/// transfer plumbing remains in the token adapter; this state transition makes repeats impossible.
#[must_use]
#[expect(clippy::too_many_arguments, reason = "fixed public claim accounts")]
pub fn claim_creator_allocation(
    factory: AccountWithMetadata,
    pool: AccountWithMetadata,
    escrow: AccountWithMetadata,
    creator: AccountWithMetadata,
    token_definition: AccountWithMetadata,
    creator_holding: AccountWithMetadata,
    clock: AccountWithMetadata,
    factory_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let mut state =
        FactoryState::try_from(&factory.account.data).expect("Factory account holds invalid data");
    assert_eq!(
        factory.account_id,
        compute_factory_pda(factory_program_id, state.launch_salt),
        "Factory account ID does not match PDA"
    );
    assert!(creator.is_authorized, "Creator authorization is missing");
    assert_eq!(
        compute_creator_commitment(creator.account_id, state.launch_salt),
        state.creator_commitment,
        "Creator commitment does not match launch"
    );
    assert_eq!(
        pool.account_id, state.pool_id,
        "Pool does not belong to factory launch"
    );
    assert_eq!(
        escrow.account_id, state.creator_escrow_id,
        "Creator escrow does not belong to factory launch"
    );
    assert_eq!(
        token_definition.account_id, state.token_definition_id,
        "Token definition does not belong to factory launch"
    );
    associated_token_account_core::verify_ata_and_get_seed(
        &creator_holding,
        &creator,
        state.token_definition_id,
        ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
    );
    assert!(
        !state.creator_allocation_claimed,
        "Creator allocation is already claimed"
    );
    let pool_state =
        PoolAccount::try_from(&pool.account.data).expect("Pool account holds invalid data");
    assert!(
        pool_state.pool.effective_lifecycle(trusted_time(&clock)) != PoolLifecycle::Open,
        "Pool must be closed before creator allocation claim"
    );
    state.creator_allocation_claimed = true;
    let mut post = factory.account;
    post.data = Data::from(&state);
    let calls = if state.creator_allocation == 0 {
        vec![]
    } else {
        let holding = TokenHolding::try_from(&escrow.account.data)
            .expect("Creator escrow must hold launch tokens");
        let amount = match holding {
            TokenHolding::Fungible { balance, .. } => balance,
            _ => panic!("Creator escrow must hold fungible launch tokens"),
        };
        vec![
            ChainedCall::new(
                ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
                vec![
                    creator.clone(),
                    token_definition.clone(),
                    creator_holding.clone(),
                ],
                &associated_token_account_core::Instruction::Create {
                    ata_program_id: ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
                },
            ),
            ChainedCall::new(
                escrow.account.program_owner,
                vec![
                    AccountWithMetadata {
                        is_authorized: true,
                        ..escrow.clone()
                    },
                    creator_holding.clone(),
                ],
                &token_core::Instruction::Transfer {
                    amount_to_transfer: amount,
                },
            )
            .with_pda_seeds(vec![compute_escrow_seed(state.launch_salt)]),
        ]
    };
    (
        vec![
            AccountPostState::new(post),
            AccountPostState::new(pool.account),
            AccountPostState::new(escrow.account.clone()),
            AccountPostState::new(creator.account),
            AccountPostState::new(token_definition.account),
            AccountPostState::new(creator_holding.account.clone()),
        ],
        calls,
    )
}

/// Relays a creator-authorized manual close to the neutral curve while authorizing the
/// factory-owned pool owner PDA with the factory's seed.
#[must_use]
pub fn close_factory_pool(
    factory: AccountWithMetadata,
    pool: AccountWithMetadata,
    creator: AccountWithMetadata,
    clock: AccountWithMetadata,
    factory_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let state =
        FactoryState::try_from(&factory.account.data).expect("Factory account holds invalid data");
    assert_eq!(
        factory.account_id,
        compute_factory_pda(factory_program_id, state.launch_salt),
        "Factory account ID does not match PDA"
    );
    assert!(creator.is_authorized, "Creator authorization is missing");
    assert_eq!(
        compute_creator_commitment(creator.account_id, state.launch_salt),
        state.creator_commitment,
        "Creator commitment does not match launch"
    );
    assert_eq!(
        pool.account_id, state.pool_id,
        "Pool does not belong to factory launch"
    );
    let factory_authorized = AccountWithMetadata {
        is_authorized: true,
        ..factory.clone()
    };
    (
        vec![
            AccountPostState::new(factory.account),
            AccountPostState::new(pool.account.clone()),
            AccountPostState::new(creator.account),
        ],
        vec![
            ChainedCall::new(
                state.curve_program_id,
                vec![pool, factory_authorized, clock],
                &CurveInstruction::ClosePool,
            )
            .with_pda_seeds(vec![compute_factory_seed(state.launch_salt)]),
        ],
    )
}

/// Withdraws the closed pool, burns unsold token0, then returns R and all collateral to creator.
#[must_use]
#[expect(clippy::too_many_arguments, reason = "fixed public lifecycle accounts")]
pub fn withdraw_factory_proceeds(
    factory: AccountWithMetadata,
    pool: AccountWithMetadata,
    creator: AccountWithMetadata,
    token_definition: AccountWithMetadata,
    collateral_definition: AccountWithMetadata,
    factory_token_ata: AccountWithMetadata,
    factory_collateral_ata: AccountWithMetadata,
    pool_token_ata: AccountWithMetadata,
    pool_collateral_ata: AccountWithMetadata,
    creator_token_ata: AccountWithMetadata,
    creator_collateral_ata: AccountWithMetadata,
    clock: AccountWithMetadata,
    factory_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    let state =
        FactoryState::try_from(&factory.account.data).expect("Factory account holds invalid data");
    assert_eq!(
        factory.account_id,
        compute_factory_pda(factory_program_id, state.launch_salt),
        "Factory account ID does not match PDA"
    );
    assert!(creator.is_authorized, "Creator authorization is missing");
    assert_eq!(
        compute_creator_commitment(creator.account_id, state.launch_salt),
        state.creator_commitment,
        "Creator commitment does not match launch"
    );
    assert_eq!(
        pool.account_id, state.pool_id,
        "Pool does not belong to factory launch"
    );
    let pool_state =
        PoolAccount::try_from(&pool.account.data).expect("Pool account holds invalid data");
    assert!(
        pool_state.pool.effective_lifecycle(trusted_time(&clock)) != PoolLifecycle::Open,
        "Pool must be closed before proceeds withdrawal"
    );
    let factory_owner = AccountWithMetadata {
        is_authorized: true,
        ..factory.clone()
    };
    for (ata, owner, definition) in [
        (
            &factory_token_ata,
            &factory_owner,
            state.token_definition_id,
        ),
        (
            &factory_collateral_ata,
            &factory_owner,
            state.collateral_definition_id,
        ),
        (&creator_token_ata, &creator, state.token_definition_id),
        (
            &creator_collateral_ata,
            &creator,
            state.collateral_definition_id,
        ),
    ] {
        associated_token_account_core::verify_ata_and_get_seed(
            ata,
            owner,
            definition,
            ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
        );
    }
    let mut calls = vec![
        ChainedCall::new(
            state.curve_program_id,
            vec![
                pool.clone(),
                factory_owner.clone(),
                factory_token_ata.clone(),
                factory_collateral_ata.clone(),
                pool_token_ata.clone(),
                pool_collateral_ata.clone(),
                clock,
            ],
            &CurveInstruction::WithdrawReserves,
        )
        .with_pda_seeds(vec![compute_factory_seed(state.launch_salt)]),
    ];
    let factory_seeds = vec![compute_factory_seed(state.launch_salt)];
    // The curve call precedes these calls; its returned reserves make the chained sequence atomic.
    calls.push(
        ChainedCall::new(
            ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
            vec![
                factory_owner.clone(),
                factory_token_ata.clone(),
                token_definition.clone(),
            ],
            &associated_token_account_core::Instruction::Burn {
                ata_program_id: ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
                amount: pool_state.pool.real_reserve0,
            },
        )
        .with_pda_seeds(factory_seeds.clone()),
    );
    if state.dex_seed_reserve != 0 {
        calls.push(
            ChainedCall::new(
                ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
                vec![
                    factory_owner.clone(),
                    factory_token_ata.clone(),
                    creator_token_ata.clone(),
                ],
                &associated_token_account_core::Instruction::Transfer {
                    ata_program_id: ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
                    amount: state.dex_seed_reserve,
                },
            )
            .with_pda_seeds(factory_seeds.clone()),
        );
    }
    if pool_state.pool.real_reserve1 != 0 {
        calls.push(
            ChainedCall::new(
                ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
                vec![
                    factory_owner,
                    factory_collateral_ata.clone(),
                    creator_collateral_ata.clone(),
                ],
                &associated_token_account_core::Instruction::Transfer {
                    ata_program_id: ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
                    amount: pool_state.pool.real_reserve1,
                },
            )
            .with_pda_seeds(factory_seeds),
        );
    }
    (
        vec![
            AccountPostState::new(factory.account),
            AccountPostState::new(pool.account),
            AccountPostState::new(creator.account),
            AccountPostState::new(token_definition.account),
            AccountPostState::new(collateral_definition.account),
            AccountPostState::new(factory_token_ata.account),
            AccountPostState::new(factory_collateral_ata.account),
            AccountPostState::new(pool_token_ata.account),
            AccountPostState::new(pool_collateral_ata.account),
            AccountPostState::new(creator_token_ata.account),
            AccountPostState::new(creator_collateral_ata.account),
        ],
        calls,
    )
}

fn trusted_time(clock: &AccountWithMetadata) -> u64 {
    assert_eq!(
        clock.account_id,
        clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
        "Clock account is not the trusted LEZ clock"
    );
    clock_core::ClockAccountData::from_bytes(clock.account.data.as_ref()).timestamp
}

#[must_use]
pub fn process_instruction(
    pre_states: Vec<AccountWithMetadata>,
    instruction: Instruction,
    factory_program_id: ProgramId,
) -> (Vec<AccountPostState>, Vec<ChainedCall>) {
    match instruction {
        Instruction::CreateFactoryPool {
            launch_salt,
            name,
            uri,
            sale_reserve,
            dex_seed_reserve,
            creator_allocation,
            virtual_token_reserve,
            virtual_collateral_reserve,
            end_timestamp,
            curve_program_id,
        } => {
            let [
                factory,
                definition,
                mint,
                metadata,
                escrow,
                creator,
                creator_holding,
                collateral_definition,
                factory_token_ata,
                factory_collateral_ata,
                pool,
                pool_token_ata,
                pool_collateral_ata,
                clock,
            ] = pre_states
                .try_into()
                .expect("CreateFactoryPool requires exactly fourteen accounts");
            create_factory_pool(
                factory,
                definition,
                mint,
                metadata,
                escrow,
                creator,
                creator_holding,
                collateral_definition,
                factory_token_ata,
                factory_collateral_ata,
                pool,
                pool_token_ata,
                pool_collateral_ata,
                launch_salt,
                name,
                uri,
                sale_reserve,
                dex_seed_reserve,
                creator_allocation,
                virtual_token_reserve,
                virtual_collateral_reserve,
                end_timestamp,
                clock,
                factory_program_id,
                curve_program_id,
            )
        }
        Instruction::ClaimCreatorAllocation => {
            let [
                factory,
                pool,
                escrow,
                creator,
                token_definition,
                creator_holding,
                clock,
            ] = pre_states
                .try_into()
                .expect("ClaimCreatorAllocation requires exactly seven accounts");
            claim_creator_allocation(
                factory,
                pool,
                escrow,
                creator,
                token_definition,
                creator_holding,
                clock,
                factory_program_id,
            )
        }
        Instruction::CloseFactoryPool => {
            let [factory, pool, creator, clock] = pre_states
                .try_into()
                .expect("CloseFactoryPool requires exactly four accounts");
            close_factory_pool(factory, pool, creator, clock, factory_program_id)
        }
        Instruction::WithdrawFactoryProceeds => {
            let [
                factory,
                pool,
                creator,
                token_definition,
                collateral_definition,
                factory_token_ata,
                factory_collateral_ata,
                pool_token_ata,
                pool_collateral_ata,
                creator_token_ata,
                creator_collateral_ata,
                clock,
            ] = pre_states
                .try_into()
                .expect("WithdrawFactoryProceeds requires exactly twelve accounts");
            withdraw_factory_proceeds(
                factory,
                pool,
                creator,
                token_definition,
                collateral_definition,
                factory_token_ata,
                factory_collateral_ata,
                pool_token_ata,
                pool_collateral_ata,
                creator_token_ata,
                creator_collateral_ata,
                clock,
                factory_program_id,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pool::Pool;

    const FACTORY_PROGRAM_ID: ProgramId = [7; 8];
    const CURVE_PROGRAM_ID: ProgramId = [6; 8];
    const TOKEN_PROGRAM_ID: ProgramId = [5; 8];

    fn ata_id(owner: AccountId, definition: AccountId) -> AccountId {
        associated_token_account_core::get_associated_token_account_id(
            &ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
            &associated_token_account_core::compute_ata_seed(owner, definition),
        )
    }

    fn creator(id: u8, is_authorized: bool) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account::default(),
            account_id: AccountId::new([id; 32]),
            is_authorized,
        }
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
            account_id: clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
            is_authorized: false,
        }
    }

    fn token_definition(account_id: AccountId) -> AccountWithMetadata {
        AccountWithMetadata {
            account: Account {
                program_owner: TOKEN_PROGRAM_ID,
                ..Account::default()
            },
            account_id,
            is_authorized: false,
        }
    }

    fn delayed_factory(creator: &AccountWithMetadata) -> (AccountWithMetadata, FactoryState) {
        let launch_salt = [1; 32];
        let state = FactoryState {
            launch_salt,
            token_definition_id: AccountId::new([2; 32]),
            collateral_definition_id: AccountId::new([3; 32]),
            sale_reserve: 800,
            dex_seed_reserve: 100,
            creator_allocation: 100,
            total_supply: 1000,
            virtual_token_reserve: 2000,
            virtual_collateral_reserve: 100,
            curve_program_id: CURVE_PROGRAM_ID,
            creator_commitment: compute_creator_commitment(creator.account_id, launch_salt),
            creator_escrow_id: AccountId::new([4; 32]),
            pool_id: AccountId::new([5; 32]),
            creator_allocation_claimed: false,
        };
        (
            AccountWithMetadata {
                account: Account {
                    data: Data::from(&state),
                    ..Account::default()
                },
                account_id: compute_factory_pda(FACTORY_PROGRAM_ID, launch_salt),
                is_authorized: false,
            },
            state,
        )
    }
    #[test]
    fn fixed_supply_is_the_exact_three_way_split() {
        assert_eq!(total_supply(800, 150, 50), Ok(1_000));
    }
    #[test]
    fn launch_requires_tradeable_supply() {
        assert_eq!(total_supply(0, 1, 1), Err(CreateError::SaleReserveZero));
    }
    #[test]
    fn overflow_cannot_create_an_unbacked_supply() {
        assert_eq!(
            total_supply(u128::MAX, 1, 0),
            Err(CreateError::SupplyOverflow)
        );
    }
    #[test]
    fn launch_salt_scopes_every_factory_address() {
        let id = [1; 8];
        assert_ne!(
            compute_factory_pda(id, [1; 32]),
            compute_factory_pda(id, [2; 32])
        );
        assert_ne!(
            compute_definition_pda(id, [1; 32]),
            compute_factory_pda(id, [1; 32])
        );
    }

    #[test]
    fn creator_commitment_is_scoped_to_creator_and_launch() {
        let creator_account = creator(9, true);
        assert_ne!(
            compute_creator_commitment(creator_account.account_id, [1; 32]),
            compute_creator_commitment(creator_account.account_id, [2; 32])
        );
        assert_ne!(
            compute_creator_commitment(creator_account.account_id, [1; 32]),
            compute_creator_commitment(creator(8, true).account_id, [1; 32])
        );
    }

    #[test]
    #[should_panic(expected = "Creator authorization is missing")]
    fn unlock_requires_the_creator_private_witness() {
        let expected_creator = creator(9, true);
        let (factory, state) = delayed_factory(&expected_creator);
        let _ = claim_creator_allocation(
            factory,
            AccountWithMetadata {
                account_id: state.pool_id,
                ..creator(1, false)
            },
            AccountWithMetadata {
                account_id: state.creator_escrow_id,
                ..creator(2, false)
            },
            creator(9, false),
            token_definition(state.token_definition_id),
            creator(3, false),
            trusted_clock(1),
            FACTORY_PROGRAM_ID,
        );
    }

    #[test]
    #[should_panic(expected = "Creator commitment does not match launch")]
    fn copied_commitment_cannot_unlock_to_another_creator() {
        let expected_creator = creator(9, true);
        let (factory, state) = delayed_factory(&expected_creator);
        let _ = claim_creator_allocation(
            factory,
            AccountWithMetadata {
                account_id: state.pool_id,
                ..creator(1, false)
            },
            AccountWithMetadata {
                account_id: state.creator_escrow_id,
                ..creator(2, false)
            },
            creator(8, true),
            token_definition(state.token_definition_id),
            creator(3, false),
            trusted_clock(1),
            FACTORY_PROGRAM_ID,
        );
    }

    #[test]
    #[should_panic(expected = "ATA account ID does not match expected derivation")]
    fn unlock_rejects_a_recipient_not_owned_by_the_creator() {
        let expected_creator = creator(9, true);
        let (factory, state) = delayed_factory(&expected_creator);
        let _ = claim_creator_allocation(
            factory,
            AccountWithMetadata {
                account_id: state.pool_id,
                ..creator(1, false)
            },
            AccountWithMetadata {
                account_id: state.creator_escrow_id,
                ..creator(2, false)
            },
            expected_creator,
            token_definition(state.token_definition_id),
            creator(3, false),
            trusted_clock(1),
            FACTORY_PROGRAM_ID,
        );
    }

    #[test]
    fn zero_delayed_allocation_unlocks_without_an_escrow_holding() {
        let expected_creator = creator(9, true);
        let (mut factory, mut state) = delayed_factory(&expected_creator);
        state.creator_allocation = 0;
        state.total_supply = state.sale_reserve + state.dex_seed_reserve;
        factory.account.data = Data::from(&state);

        let mut closed_pool = Pool::create(800, 0, 1_000, 100, None, None).expect("valid pool");
        assert_eq!(closed_pool.close_pool(0), Ok(()));
        let pool = AccountWithMetadata {
            account: Account {
                data: Data::from(&PoolAccount {
                    token0_definition_id: state.token_definition_id,
                    token1_definition_id: state.collateral_definition_id,
                    owner: factory.account_id,
                    pool: closed_pool,
                }),
                ..Account::default()
            },
            account_id: state.pool_id,
            is_authorized: false,
        };
        let creator_holding = AccountWithMetadata {
            account: Account::default(),
            account_id: ata_id(expected_creator.account_id, state.token_definition_id),
            is_authorized: false,
        };
        let escrow = AccountWithMetadata {
            account: Account::default(),
            account_id: state.creator_escrow_id,
            is_authorized: false,
        };

        let (post_states, calls) = claim_creator_allocation(
            factory,
            pool,
            escrow,
            expected_creator,
            token_definition(state.token_definition_id),
            creator_holding,
            trusted_clock(1),
            FACTORY_PROGRAM_ID,
        );

        assert!(calls.is_empty());
        let unlocked =
            FactoryState::try_from(&post_states[0].account().data).expect("factory state parses");
        assert!(unlocked.creator_allocation_claimed);
    }

    #[test]
    fn delayed_unlock_initializes_a_fresh_creator_ata_before_transfer() {
        let expected_creator = creator(9, true);
        let (factory, state) = delayed_factory(&expected_creator);
        let mut closed_pool = Pool::create(800, 0, 1_000, 100, None, None).expect("valid pool");
        assert_eq!(closed_pool.close_pool(0), Ok(()));
        let pool = AccountWithMetadata {
            account: Account {
                data: Data::from(&PoolAccount {
                    token0_definition_id: state.token_definition_id,
                    token1_definition_id: state.collateral_definition_id,
                    owner: factory.account_id,
                    pool: closed_pool,
                }),
                ..Account::default()
            },
            account_id: state.pool_id,
            is_authorized: false,
        };
        let escrow = AccountWithMetadata {
            account: Account {
                program_owner: TOKEN_PROGRAM_ID,
                data: Data::from(&TokenHolding::Fungible {
                    definition_id: state.token_definition_id,
                    balance: state.creator_allocation,
                }),
                ..Account::default()
            },
            account_id: state.creator_escrow_id,
            is_authorized: false,
        };
        let creator_holding = AccountWithMetadata {
            account: Account::default(),
            account_id: ata_id(expected_creator.account_id, state.token_definition_id),
            is_authorized: false,
        };

        let (_post_states, calls) = claim_creator_allocation(
            factory,
            pool,
            escrow,
            expected_creator,
            token_definition(state.token_definition_id),
            creator_holding,
            trusted_clock(1),
            FACTORY_PROGRAM_ID,
        );

        assert_eq!(
            calls.len(),
            2,
            "a fresh creator ATA must be initialized before the escrow transfer"
        );
        assert_eq!(calls[0].program_id, ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID);
    }

    #[test]
    fn create_mints_the_exact_split_and_funds_only_the_tradeable_pool_reserve() {
        let launch_salt = [3; 32];
        let factory_id = compute_factory_pda(FACTORY_PROGRAM_ID, launch_salt);
        let token_definition_id = compute_definition_pda(FACTORY_PROGRAM_ID, launch_salt);
        let collateral_definition_id = AccountId::new([4; 32]);
        let expected_creator = creator(9, true);
        let pool_id = compute_pool_pda(
            CURVE_PROGRAM_ID,
            token_definition_id,
            collateral_definition_id,
            factory_id,
        );
        let factory = AccountWithMetadata {
            account: Account::default(),
            account_id: factory_id,
            is_authorized: false,
        };
        let definition = AccountWithMetadata {
            account: Account {
                program_owner: TOKEN_PROGRAM_ID,
                ..Account::default()
            },
            account_id: token_definition_id,
            is_authorized: false,
        };
        let mint = AccountWithMetadata {
            account_id: compute_mint_pda(FACTORY_PROGRAM_ID, launch_salt),
            ..creator(1, false)
        };
        let metadata = AccountWithMetadata {
            account_id: compute_metadata_pda(FACTORY_PROGRAM_ID, launch_salt),
            ..creator(2, false)
        };
        let escrow = AccountWithMetadata {
            account_id: compute_escrow_pda(FACTORY_PROGRAM_ID, launch_salt),
            ..creator(3, false)
        };
        let creator_holding = AccountWithMetadata {
            account_id: ata_id(expected_creator.account_id, token_definition_id),
            ..creator(4, false)
        };
        let collateral_definition = AccountWithMetadata {
            account_id: collateral_definition_id,
            ..creator(5, false)
        };
        let factory_token_ata = AccountWithMetadata {
            account_id: ata_id(factory_id, token_definition_id),
            ..creator(6, false)
        };
        let factory_collateral_ata = AccountWithMetadata {
            account_id: ata_id(factory_id, collateral_definition_id),
            ..creator(7, false)
        };
        let pool = AccountWithMetadata {
            account_id: pool_id,
            ..creator(8, false)
        };
        let pool_token_ata = AccountWithMetadata {
            account_id: ata_id(pool_id, token_definition_id),
            ..creator(10, false)
        };
        let pool_collateral_ata = AccountWithMetadata {
            account_id: ata_id(pool_id, collateral_definition_id),
            ..creator(11, false)
        };

        let (post_states, calls) = create_factory_pool(
            factory.clone(),
            definition.clone(),
            mint.clone(),
            metadata.clone(),
            escrow.clone(),
            expected_creator.clone(),
            creator_holding.clone(),
            collateral_definition.clone(),
            factory_token_ata.clone(),
            factory_collateral_ata.clone(),
            pool.clone(),
            pool_token_ata.clone(),
            pool_collateral_ata.clone(),
            launch_salt,
            "Launch".into(),
            "https://example.test/launch.json".into(),
            800,
            150,
            50,
            2_000,
            100,
            None,
            trusted_clock(1),
            FACTORY_PROGRAM_ID,
            CURVE_PROGRAM_ID,
        );

        assert_eq!(post_states.len(), 13);
        let state =
            FactoryState::try_from(&post_states[0].account().data).expect("factory state parses");
        assert_eq!(state.total_supply, 1_000);
        assert_eq!(state.sale_reserve, 800);
        assert_eq!(state.dex_seed_reserve, 150);
        assert_eq!(state.creator_allocation, 50);
        assert!(!state.creator_allocation_claimed);

        let [
            definition_call,
            factory_ata_call,
            factory_transfer,
            creator_transfer,
            pool_call,
        ]: [_; 5] = calls
            .try_into()
            .expect("definition, ATA, split, and pool calls");
        let definition_instruction: token_core::Instruction =
            risc0_zkvm::serde::from_slice(&definition_call.instruction_data)
                .expect("token definition instruction parses");
        match definition_instruction {
            token_core::Instruction::NewDefinitionWithMetadata {
                new_definition: NewTokenDefinition::Fungible { name, total_supply },
                metadata,
            } => {
                assert_eq!(name, "Launch");
                assert_eq!(total_supply, 1_000);
                assert_eq!(metadata.uri, "https://example.test/launch.json");
            }
            _ => panic!("factory must create a fungible definition"),
        }
        let ata_instruction: associated_token_account_core::Instruction =
            risc0_zkvm::serde::from_slice(&factory_ata_call.instruction_data)
                .expect("factory ATA instruction parses");
        assert!(matches!(
            ata_instruction,
            associated_token_account_core::Instruction::Create { .. }
        ));
        let factory_transfer_instruction: token_core::Instruction =
            risc0_zkvm::serde::from_slice(&factory_transfer.instruction_data)
                .expect("factory allocation instruction parses");
        assert!(matches!(
            factory_transfer_instruction,
            token_core::Instruction::Transfer {
                amount_to_transfer: 950
            }
        ));
        let creator_transfer_instruction: token_core::Instruction =
            risc0_zkvm::serde::from_slice(&creator_transfer.instruction_data)
                .expect("creator allocation instruction parses");
        assert!(matches!(
            creator_transfer_instruction,
            token_core::Instruction::Transfer {
                amount_to_transfer: 50
            }
        ));
        assert_eq!(creator_transfer.pre_states[1].account_id, escrow.account_id);
        assert!(
            creator_transfer.pre_states[1].is_authorized,
            "a fresh delayed-allocation escrow must be authorized for Token::Transfer"
        );
        assert_eq!(
            creator_transfer.pda_seeds,
            vec![
                compute_mint_seed(launch_salt),
                compute_escrow_seed(launch_salt),
            ],
            "the factory must authorize its escrow PDA for the delayed transfer"
        );
        let pool_instruction: CurveInstruction =
            risc0_zkvm::serde::from_slice(&pool_call.instruction_data)
                .expect("curve create instruction parses");
        assert!(matches!(
            pool_instruction,
            CurveInstruction::CreatePool {
                token0_amount: 800,
                token1_amount: 0,
                owner,
                ..
            } if owner == factory_id
        ));
        assert_eq!(pool_call.pda_seeds, vec![compute_factory_seed(launch_salt)]);
    }

    #[test]
    fn close_relays_to_the_recorded_curve_with_factory_pda_authorization() {
        let expected_creator = creator(9, true);
        let (factory, state) = delayed_factory(&expected_creator);
        let pool = AccountWithMetadata {
            account_id: state.pool_id,
            ..creator(1, false)
        };
        let (post_states, calls) = close_factory_pool(
            factory,
            pool.clone(),
            expected_creator,
            trusted_clock(1),
            FACTORY_PROGRAM_ID,
        );

        assert_eq!(post_states.len(), 3);
        let [call]: [_; 1] = calls.try_into().expect("one curve close call");
        assert_eq!(call.program_id, CURVE_PROGRAM_ID);
        assert_eq!(call.pre_states[0], pool);
        assert!(call.pre_states[1].is_authorized);
        assert_eq!(
            call.pda_seeds,
            vec![compute_factory_seed(state.launch_salt)]
        );
        let instruction: CurveInstruction = risc0_zkvm::serde::from_slice(&call.instruction_data)
            .expect("curve instruction parses");
        assert_eq!(instruction, CurveInstruction::ClosePool);
    }

    #[test]
    #[should_panic(expected = "Creator commitment does not match launch")]
    fn close_rejects_another_creator() {
        let expected_creator = creator(9, true);
        let (factory, state) = delayed_factory(&expected_creator);
        let _ = close_factory_pool(
            factory,
            AccountWithMetadata {
                account_id: state.pool_id,
                ..creator(1, false)
            },
            creator(8, true),
            trusted_clock(1),
            FACTORY_PROGRAM_ID,
        );
    }

    #[test]
    fn proceeds_withdrawal_chains_neutral_withdrawal_burn_and_creator_transfers() {
        let expected_creator = creator(9, true);
        let (factory, state) = delayed_factory(&expected_creator);
        let mut closed_pool = Pool::create(70, 30, 1_000, 100, None, Some(pool::TokenSide::Token0))
            .expect("valid pool");
        assert_eq!(closed_pool.close_pool(1), Ok(()));
        let pool = AccountWithMetadata {
            account: Account {
                data: Data::from(&PoolAccount {
                    token0_definition_id: state.token_definition_id,
                    token1_definition_id: state.collateral_definition_id,
                    owner: factory.account_id,
                    pool: closed_pool,
                }),
                ..Account::default()
            },
            account_id: state.pool_id,
            is_authorized: false,
        };
        let factory_token = AccountWithMetadata {
            account_id: ata_id(factory.account_id, state.token_definition_id),
            ..creator(1, false)
        };
        let factory_collateral = AccountWithMetadata {
            account_id: ata_id(factory.account_id, state.collateral_definition_id),
            ..creator(2, false)
        };
        let pool_token = AccountWithMetadata {
            account_id: ata_id(state.pool_id, state.token_definition_id),
            ..creator(3, false)
        };
        let pool_collateral = AccountWithMetadata {
            account_id: ata_id(state.pool_id, state.collateral_definition_id),
            ..creator(4, false)
        };
        let creator_token = AccountWithMetadata {
            account_id: ata_id(expected_creator.account_id, state.token_definition_id),
            ..creator(5, false)
        };
        let creator_collateral = AccountWithMetadata {
            account_id: ata_id(expected_creator.account_id, state.collateral_definition_id),
            ..creator(6, false)
        };
        let (_posts, calls) = withdraw_factory_proceeds(
            factory,
            pool,
            expected_creator,
            token_definition(state.token_definition_id),
            token_definition(state.collateral_definition_id),
            factory_token,
            factory_collateral,
            pool_token,
            pool_collateral,
            creator_token,
            creator_collateral,
            trusted_clock(1),
            FACTORY_PROGRAM_ID,
        );
        assert_eq!(
            calls.len(),
            4,
            "curve withdrawal, burn, R return, and collateral return"
        );
        let burn: associated_token_account_core::Instruction =
            risc0_zkvm::serde::from_slice(&calls[1].instruction_data).expect("burn parses");
        assert!(matches!(
            burn,
            associated_token_account_core::Instruction::Burn { amount: 70, .. }
        ));
        let dex_seed: associated_token_account_core::Instruction =
            risc0_zkvm::serde::from_slice(&calls[2].instruction_data).expect("transfer parses");
        assert!(matches!(
            dex_seed,
            associated_token_account_core::Instruction::Transfer { amount: 100, .. }
        ));
        let collateral: associated_token_account_core::Instruction =
            risc0_zkvm::serde::from_slice(&calls[3].instruction_data).expect("transfer parses");
        assert!(matches!(
            collateral,
            associated_token_account_core::Instruction::Transfer { amount: 30, .. }
        ));
    }

    #[test]
    fn factory_state_round_trips_without_private_authorization_material() {
        let state = FactoryState {
            launch_salt: [1; 32],
            token_definition_id: AccountId::new([2; 32]),
            collateral_definition_id: AccountId::new([3; 32]),
            sale_reserve: 800,
            dex_seed_reserve: 100,
            creator_allocation: 100,
            total_supply: 1000,
            virtual_token_reserve: 2000,
            virtual_collateral_reserve: 100,
            curve_program_id: CURVE_PROGRAM_ID,
            creator_commitment: [4; 32],
            creator_escrow_id: AccountId::new([5; 32]),
            pool_id: AccountId::new([6; 32]),
            creator_allocation_claimed: false,
        };
        assert_eq!(
            FactoryState::try_from(&Data::from(&state)).expect("state parses"),
            state
        );
    }
}
