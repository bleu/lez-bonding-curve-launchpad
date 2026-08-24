//! The SDK. Hides wallet handling, message and witness construction, program ids, and
//! account derivation behind pool and factory lifecycle operations.
//!
//! The CLI parses arguments and calls this crate, and nothing else. The private deshield
//! to swap to re-shield flow also belongs here, because RFP-015 is explicit that the
//! program cannot enforce the re-shield and the SDK must.
//!
//! Grown by GTM-517 and GTM-521. What is here now is the account and program loading that
//! `src/bin/run_deploy_probe.rs` already needed, moved out of the root package so this
//! crate has a working consumer from the day it was created.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use associated_token_account_core::{compute_ata_seed, get_associated_token_account_id};
use common::{HashType, transaction::LeeTransaction};
use curve_core::{Config, Instruction as CurveInstruction, compute_config_pda, compute_pool_pda};
use factory_core::{
    ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID, FactoryState, Instruction as FactoryInstruction,
    compute_definition_pda, compute_escrow_pda, compute_factory_pda, compute_metadata_pda,
    compute_mint_pda,
};
use lee::{
    AccountId, PublicTransaction,
    program::Program,
    public_transaction::{Message, WitnessSet},
};
use lee_core::program::ProgramId;
use sequencer_service_rpc::RpcClient as _;
use serde::Serialize;
use wallet::WalletCore;

pub use factory_core::UnlockPolicy;

/// Accepts `Public/<base58>`, `Private/<base58>`, or a bare base58 id.
///
/// The wallet CLI prints the prefixed form while `AccountId` itself parses
/// only the base58 half, so callers can paste either.
pub fn parse_account_id(raw: &str) -> Result<AccountId> {
    let bare = raw.rsplit('/').next().unwrap_or(raw);
    bare.parse()
        .map_err(|_| anyhow!("not a valid 32-byte base58 account id: {raw}"))
}

pub fn load_program(path: &Path) -> Result<Program> {
    let bytecode =
        std::fs::read(path).with_context(|| format!("reading guest binary {}", path.display()))?;
    Program::new(bytecode.into())
        .map_err(|e| anyhow!("{} is not a valid guest program: {e:?}", path.display()))
}

/// Inputs for the factory's one-time launch and pool creation operation.
#[derive(Debug, Clone)]
pub struct CreateSaleRequest {
    pub launch_salt: [u8; 32],
    pub name: String,
    pub uri: String,
    pub sale_reserve: u128,
    pub dex_seed_reserve: u128,
    pub creator_allocation: u128,
    pub virtual_token_reserve: u128,
    pub virtual_collateral_reserve: u128,
    pub unlock_policy: UnlockPolicy,
    pub collateral_definition: AccountId,
}

/// Exact-output purchase inputs. Collateral is always the input definition for a factory launch.
#[derive(Debug, Clone, Copy)]
pub struct BuyRequest {
    pub launch_salt: [u8; 32],
    pub collateral_definition: AccountId,
    pub amount_out: u128,
    pub max_amount_in: u128,
}

/// Exact-input sale inputs. Launch tokens are always the input definition for a factory launch.
#[derive(Debug, Clone, Copy)]
pub struct SellRequest {
    pub launch_salt: [u8; 32],
    pub collateral_definition: AccountId,
    pub amount_in: u128,
    pub min_amount_out: u128,
}

/// Quotes an exact-output purchase against a snapshot of the current pool and fee config.
/// The quote is informational only; callers enforce its cap through `BuyRequest` on-chain.
pub fn quote_buy(
    pool_account: &curve_core::PoolAccount,
    config: &Config,
    amount_out: u128,
    now: u64,
) -> Result<pool::SwapOutcome> {
    let mut pool = pool_account.pool.clone();
    pool.swap_exact_output(
        pool::TokenSide::Token1,
        amount_out,
        u128::MAX,
        config.pool_fee_bps,
        config.protocol_fee_bps,
        now,
    )
    .map_err(|error| anyhow!("cannot quote purchase: {error:?}"))
}

/// Quotes an exact-input sale against a snapshot of the current pool and fee config.
/// The quote is informational only; callers enforce its floor through `SellRequest` on-chain.
pub fn quote_sell(
    pool_account: &curve_core::PoolAccount,
    config: &Config,
    amount_in: u128,
    now: u64,
) -> Result<pool::SwapOutcome> {
    let mut pool = pool_account.pool.clone();
    pool.swap_exact_input(
        pool::TokenSide::Token0,
        amount_in,
        0,
        config.pool_fee_bps,
        config.protocol_fee_bps,
        now,
    )
    .map_err(|error| anyhow!("cannot quote sale: {error:?}"))
}

/// Quotes an exact-input purchase spending collateral against a pool snapshot.
pub fn quote_buy_with_collateral(
    pool_account: &curve_core::PoolAccount,
    config: &Config,
    collateral_in: u128,
    now: u64,
) -> Result<pool::SwapOutcome> {
    let mut pool = pool_account.pool.clone();
    pool.swap_exact_input(
        pool::TokenSide::Token1,
        collateral_in,
        0,
        config.pool_fee_bps,
        config.protocol_fee_bps,
        now,
    )
    .map_err(|error| anyhow!("cannot quote collateral purchase: {error:?}"))
}

/// A public program call with all account addresses and required signatures resolved.
#[derive(Debug, Clone)]
pub struct PublicInvocation<I> {
    pub program_id: ProgramId,
    pub account_ids: Vec<AccountId>,
    pub signer_accounts: Vec<AccountId>,
    pub instruction: I,
}

/// Builds the complete factory call for a launch. All PDAs and ATAs are derived locally;
/// only the creator account is caller-provided.
pub fn build_create_sale_invocation(
    factory_program_id: ProgramId,
    curve_program_id: ProgramId,
    creator: AccountId,
    request: CreateSaleRequest,
) -> Result<PublicInvocation<FactoryInstruction>> {
    let (factory, token_definition, pool) = factory_pool_addresses(
        factory_program_id,
        curve_program_id,
        request.launch_salt,
        request.collateral_definition,
    );
    let account_ids = vec![
        factory,
        token_definition,
        compute_mint_pda(factory_program_id, request.launch_salt),
        compute_metadata_pda(factory_program_id, request.launch_salt),
        compute_escrow_pda(factory_program_id, request.launch_salt),
        creator,
        associated_token_account(creator, token_definition),
        request.collateral_definition,
        associated_token_account(factory, token_definition),
        associated_token_account(factory, request.collateral_definition),
        pool,
        associated_token_account(pool, token_definition),
        associated_token_account(pool, request.collateral_definition),
    ];
    Ok(PublicInvocation {
        program_id: factory_program_id,
        account_ids,
        signer_accounts: vec![creator],
        instruction: FactoryInstruction::CreateFactoryPool {
            launch_salt: request.launch_salt,
            name: request.name,
            uri: request.uri,
            sale_reserve: request.sale_reserve,
            dex_seed_reserve: request.dex_seed_reserve,
            creator_allocation: request.creator_allocation,
            virtual_token_reserve: request.virtual_token_reserve,
            virtual_collateral_reserve: request.virtual_collateral_reserve,
            unlock_policy: request.unlock_policy,
            curve_program_id,
        },
    })
}

/// Builds the factory-mediated logical close. The factory authorizes its pool-owner PDA,
/// while its state verifies the creator's signed commitment.
#[must_use]
pub fn build_close_factory_pool_invocation(
    factory_program_id: ProgramId,
    curve_program_id: ProgramId,
    creator: AccountId,
    launch_salt: [u8; 32],
    collateral_definition: AccountId,
) -> PublicInvocation<FactoryInstruction> {
    let (factory, _, pool) = factory_pool_addresses(
        factory_program_id,
        curve_program_id,
        launch_salt,
        collateral_definition,
    );
    PublicInvocation {
        program_id: factory_program_id,
        account_ids: vec![factory, pool, creator],
        signer_accounts: vec![creator],
        instruction: FactoryInstruction::CloseFactoryPool,
    }
}

/// Builds the creator-authorized factory withdrawal that retires a closed (or expired) pool
/// and forwards its remaining reserves under the factory's allocation policy.
#[must_use]
pub fn build_withdraw_factory_pool_invocation(
    factory_program_id: ProgramId,
    curve_program_id: ProgramId,
    creator: AccountId,
    launch_salt: [u8; 32],
    collateral_definition: AccountId,
) -> PublicInvocation<FactoryInstruction> {
    let (factory, token_definition, pool) = factory_pool_addresses(
        factory_program_id,
        curve_program_id,
        launch_salt,
        collateral_definition,
    );
    PublicInvocation {
        program_id: factory_program_id,
        account_ids: vec![
            factory,
            pool,
            creator,
            token_definition,
            collateral_definition,
            associated_token_account(pool, token_definition),
            associated_token_account(pool, collateral_definition),
            associated_token_account(factory, token_definition),
            associated_token_account(factory, collateral_definition),
            associated_token_account(creator, token_definition),
            associated_token_account(creator, collateral_definition),
            clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
        ],
        signer_accounts: vec![creator],
        instruction: FactoryInstruction::WithdrawFactoryPool,
    }
}

/// Builds the creator-authorized release of an `OnClose` allocation from factory escrow.
#[must_use]
pub fn build_unlock_creator_allocation_invocation(
    factory_program_id: ProgramId,
    curve_program_id: ProgramId,
    creator: AccountId,
    launch_salt: [u8; 32],
    collateral_definition: AccountId,
) -> PublicInvocation<FactoryInstruction> {
    let (factory, token_definition, pool) = factory_pool_addresses(
        factory_program_id,
        curve_program_id,
        launch_salt,
        collateral_definition,
    );
    PublicInvocation {
        program_id: factory_program_id,
        account_ids: vec![
            factory,
            pool,
            compute_escrow_pda(factory_program_id, launch_salt),
            creator,
            token_definition,
            associated_token_account(creator, token_definition),
        ],
        signer_accounts: vec![creator],
        instruction: FactoryInstruction::UnlockCreatorAllocation,
    }
}

/// Builds a factory-launch purchase as a neutral curve exact-output swap.
#[must_use]
pub fn build_buy_invocation(
    factory_program_id: ProgramId,
    curve_program_id: ProgramId,
    participant: AccountId,
    treasury: AccountId,
    request: BuyRequest,
) -> PublicInvocation<CurveInstruction> {
    let (_, token_definition, pool) = factory_pool_addresses(
        factory_program_id,
        curve_program_id,
        request.launch_salt,
        request.collateral_definition,
    );
    PublicInvocation {
        program_id: curve_program_id,
        account_ids: vec![
            pool,
            compute_config_pda(curve_program_id),
            participant,
            associated_token_account(participant, request.collateral_definition),
            associated_token_account(pool, request.collateral_definition),
            associated_token_account(pool, token_definition),
            associated_token_account(participant, token_definition),
            associated_token_account(treasury, request.collateral_definition),
            clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
        ],
        signer_accounts: vec![participant],
        instruction: CurveInstruction::SwapExactOutput {
            amount_out: request.amount_out,
            max_amount_in: request.max_amount_in,
            token_in: request.collateral_definition,
        },
    }
}

/// Builds a factory-launch sale as a neutral curve exact-input swap.
#[must_use]
pub fn build_sell_invocation(
    factory_program_id: ProgramId,
    curve_program_id: ProgramId,
    participant: AccountId,
    treasury: AccountId,
    request: SellRequest,
) -> PublicInvocation<CurveInstruction> {
    let (_, token_definition, pool) = factory_pool_addresses(
        factory_program_id,
        curve_program_id,
        request.launch_salt,
        request.collateral_definition,
    );
    PublicInvocation {
        program_id: curve_program_id,
        account_ids: vec![
            pool,
            compute_config_pda(curve_program_id),
            participant,
            associated_token_account(participant, token_definition),
            associated_token_account(pool, token_definition),
            associated_token_account(pool, request.collateral_definition),
            associated_token_account(participant, request.collateral_definition),
            associated_token_account(treasury, token_definition),
            clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID,
        ],
        signer_accounts: vec![participant],
        instruction: CurveInstruction::SwapExactInput {
            amount_in: request.amount_in,
            min_amount_out: request.min_amount_out,
            token_in: token_definition,
        },
    }
}

/// Reads the live curve configuration so callers direct protocol fees to its configured treasury.
pub async fn load_curve_config(wallet: &WalletCore, curve_program_id: ProgramId) -> Result<Config> {
    let config_id = compute_config_pda(curve_program_id);
    let account = wallet
        .get_account_public(config_id)
        .await
        .context("reading the live curve configuration")?;
    Config::try_from(&account.data).context("decoding the live curve configuration")
}

/// Reads the immutable launch policy and current factory lifecycle flags for a launch salt.
pub async fn load_factory_state(
    wallet: &WalletCore,
    factory_program_id: ProgramId,
    launch_salt: [u8; 32],
) -> Result<FactoryState> {
    let factory_id = compute_factory_pda(factory_program_id, launch_salt);
    let account = wallet
        .get_account_public(factory_id)
        .await
        .context("reading the factory launch state")?;
    FactoryState::try_from(&account.data).context("decoding the factory launch state")
}

/// Reads the current neutral curve reserves for a factory launch.
pub async fn load_factory_pool(
    wallet: &WalletCore,
    factory_program_id: ProgramId,
    curve_program_id: ProgramId,
    launch_salt: [u8; 32],
    collateral_definition: AccountId,
) -> Result<curve_core::PoolAccount> {
    let (_, _, pool_id) = factory_pool_addresses(
        factory_program_id,
        curve_program_id,
        launch_salt,
        collateral_definition,
    );
    let account = wallet
        .get_account_public(pool_id)
        .await
        .context("reading the factory curve pool")?;
    curve_core::PoolAccount::try_from(&account.data).context("decoding the factory curve pool")
}

/// Signs and submits a public invocation through the configured project wallet.
///
/// The caller supplies only application-level accounts; this boundary obtains each nonce,
/// resolves the corresponding wallet key, and constructs the LEZ public transaction.
pub async fn submit_public_invocation<I: Serialize>(
    wallet: &WalletCore,
    program: &Program,
    invocation: PublicInvocation<I>,
) -> Result<HashType> {
    if program.id() != invocation.program_id {
        return Err(anyhow!(
            "program binary does not match the invocation program ID"
        ));
    }
    let signing_keys = invocation
        .signer_accounts
        .iter()
        .map(|account_id| {
            wallet
                .get_account_public_signing_key(*account_id)
                .ok_or_else(|| anyhow!("wallet has no public signing key for {account_id}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let nonces = wallet
        .get_accounts_nonces(invocation.signer_accounts)
        .await
        .context("querying signer account nonces")?;
    let message = Message::try_new(
        invocation.program_id,
        invocation.account_ids,
        nonces,
        invocation.instruction,
    )
    .context("serializing launchpad instruction")?;
    let witnesses = WitnessSet::for_message(&message, &signing_keys);
    wallet
        .sequencer_client
        .send_transaction(LeeTransaction::Public(PublicTransaction::new(
            message, witnesses,
        )))
        .await
        .context("submitting public launchpad transaction")
}

fn factory_pool_addresses(
    factory_program_id: ProgramId,
    curve_program_id: ProgramId,
    launch_salt: [u8; 32],
    collateral_definition: AccountId,
) -> (AccountId, AccountId, AccountId) {
    let factory = compute_factory_pda(factory_program_id, launch_salt);
    let token_definition = compute_definition_pda(factory_program_id, launch_salt);
    let pool = compute_pool_pda(
        curve_program_id,
        token_definition,
        collateral_definition,
        factory,
    );
    (factory, token_definition, pool)
}

fn associated_token_account(owner: AccountId, token_definition: AccountId) -> AccountId {
    get_associated_token_account_id(
        &ASSOCIATED_TOKEN_ACCOUNT_PROGRAM_ID,
        &compute_ata_seed(owner, token_definition),
    )
}

#[cfg(test)]
mod tests {
    use factory_core::{
        UnlockPolicy, compute_definition_pda, compute_escrow_pda, compute_factory_pda,
        compute_metadata_pda, compute_mint_pda,
    };
    use lee::AccountId;
    use pool::Pool;

    use super::{
        BuyRequest, CreateSaleRequest, SellRequest, build_buy_invocation,
        build_close_factory_pool_invocation, build_create_sale_invocation, build_sell_invocation,
        build_unlock_creator_allocation_invocation, build_withdraw_factory_pool_invocation,
        quote_buy,
    };

    const FACTORY_PROGRAM_ID: [u32; 8] = [7; 8];
    const CURVE_PROGRAM_ID: [u32; 8] = [6; 8];

    #[test]
    fn factory_launch_builds_the_derived_accounts_and_creator_authorization() {
        let launch_salt = [1; 32];
        let creator = AccountId::new([9; 32]);
        let collateral_definition = AccountId::new([5; 32]);
        let invocation = build_create_sale_invocation(
            FACTORY_PROGRAM_ID,
            CURVE_PROGRAM_ID,
            creator,
            CreateSaleRequest {
                launch_salt,
                name: "E2E token".into(),
                uri: "https://example.invalid/e2e-token.json".into(),
                sale_reserve: 800,
                dex_seed_reserve: 100,
                creator_allocation: 50,
                virtual_token_reserve: 2_000,
                virtual_collateral_reserve: 100,
                unlock_policy: UnlockPolicy::OnClose,
                collateral_definition,
            },
        )
        .expect("valid factory launch invocation");

        let factory = compute_factory_pda(FACTORY_PROGRAM_ID, launch_salt);
        let definition = compute_definition_pda(FACTORY_PROGRAM_ID, launch_salt);
        assert_eq!(invocation.program_id, FACTORY_PROGRAM_ID);
        assert_eq!(invocation.signer_accounts, vec![creator]);
        assert_eq!(
            invocation.account_ids[0..6],
            [
                factory,
                definition,
                compute_mint_pda(FACTORY_PROGRAM_ID, launch_salt),
                compute_metadata_pda(FACTORY_PROGRAM_ID, launch_salt),
                compute_escrow_pda(FACTORY_PROGRAM_ID, launch_salt),
                creator,
            ]
        );
        assert_eq!(invocation.account_ids.len(), 13);
        assert_eq!(invocation.account_ids[7], collateral_definition);
    }

    #[test]
    fn factory_close_uses_the_recorded_factory_owned_pool_and_creator_signature() {
        let launch_salt = [1; 32];
        let creator = AccountId::new([9; 32]);
        let collateral_definition = AccountId::new([5; 32]);
        let invocation = build_close_factory_pool_invocation(
            FACTORY_PROGRAM_ID,
            CURVE_PROGRAM_ID,
            creator,
            launch_salt,
            collateral_definition,
        );

        let factory = compute_factory_pda(FACTORY_PROGRAM_ID, launch_salt);
        let definition = compute_definition_pda(FACTORY_PROGRAM_ID, launch_salt);
        assert_eq!(invocation.program_id, FACTORY_PROGRAM_ID);
        assert_eq!(invocation.signer_accounts, vec![creator]);
        assert_eq!(
            invocation.account_ids,
            vec![
                factory,
                curve_core::compute_pool_pda(
                    CURVE_PROGRAM_ID,
                    definition,
                    collateral_definition,
                    factory,
                ),
                creator,
            ]
        );
        assert!(matches!(
            invocation.instruction,
            factory_core::Instruction::CloseFactoryPool
        ));
    }

    #[test]
    fn creator_unlock_uses_the_factory_escrow_and_derived_recipient_ata() {
        let launch_salt = [1; 32];
        let creator = AccountId::new([9; 32]);
        let collateral_definition = AccountId::new([5; 32]);
        let invocation = build_unlock_creator_allocation_invocation(
            FACTORY_PROGRAM_ID,
            CURVE_PROGRAM_ID,
            creator,
            launch_salt,
            collateral_definition,
        );

        let factory = compute_factory_pda(FACTORY_PROGRAM_ID, launch_salt);
        let definition = compute_definition_pda(FACTORY_PROGRAM_ID, launch_salt);
        assert_eq!(invocation.program_id, FACTORY_PROGRAM_ID);
        assert_eq!(invocation.signer_accounts, vec![creator]);
        assert_eq!(invocation.account_ids[0], factory);
        assert_eq!(
            invocation.account_ids[2],
            compute_escrow_pda(FACTORY_PROGRAM_ID, launch_salt)
        );
        assert_eq!(invocation.account_ids[4], definition);
        assert_eq!(
            invocation.account_ids[5],
            super::associated_token_account(creator, definition)
        );
        assert!(matches!(
            invocation.instruction,
            factory_core::Instruction::UnlockCreatorAllocation
        ));
    }

    #[test]
    fn buy_builds_an_exact_output_swap_with_collateral_as_input() {
        let launch_salt = [1; 32];
        let participant = AccountId::new([9; 32]);
        let collateral_definition = AccountId::new([5; 32]);
        let treasury = AccountId::new([4; 32]);
        let invocation = build_buy_invocation(
            FACTORY_PROGRAM_ID,
            CURVE_PROGRAM_ID,
            participant,
            treasury,
            BuyRequest {
                launch_salt,
                collateral_definition,
                amount_out: 25,
                max_amount_in: 100,
            },
        );

        let factory = compute_factory_pda(FACTORY_PROGRAM_ID, launch_salt);
        let definition = compute_definition_pda(FACTORY_PROGRAM_ID, launch_salt);
        let pool = curve_core::compute_pool_pda(
            CURVE_PROGRAM_ID,
            definition,
            collateral_definition,
            factory,
        );
        assert_eq!(invocation.program_id, CURVE_PROGRAM_ID);
        assert_eq!(invocation.signer_accounts, vec![participant]);
        assert_eq!(invocation.account_ids[0], pool);
        assert_eq!(invocation.account_ids[2], participant);
        assert_eq!(
            invocation.account_ids[3],
            super::associated_token_account(participant, collateral_definition)
        );
        assert_eq!(
            invocation.account_ids[6],
            super::associated_token_account(participant, definition)
        );
        assert_eq!(
            invocation.account_ids[7],
            super::associated_token_account(treasury, collateral_definition)
        );
        assert_eq!(
            invocation.account_ids[8],
            clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID
        );
        assert!(matches!(
            invocation.instruction,
            curve_core::Instruction::SwapExactOutput {
                amount_out: 25,
                max_amount_in: 100,
                token_in,
            } if token_in == collateral_definition
        ));
    }

    #[test]
    fn sell_builds_an_exact_input_swap_with_launch_tokens_as_input() {
        let launch_salt = [1; 32];
        let participant = AccountId::new([9; 32]);
        let collateral_definition = AccountId::new([5; 32]);
        let treasury = AccountId::new([4; 32]);
        let invocation = build_sell_invocation(
            FACTORY_PROGRAM_ID,
            CURVE_PROGRAM_ID,
            participant,
            treasury,
            SellRequest {
                launch_salt,
                collateral_definition,
                amount_in: 25,
                min_amount_out: 10,
            },
        );

        let definition = compute_definition_pda(FACTORY_PROGRAM_ID, launch_salt);
        assert_eq!(invocation.program_id, CURVE_PROGRAM_ID);
        assert_eq!(invocation.signer_accounts, vec![participant]);
        assert_eq!(
            invocation.account_ids[3],
            super::associated_token_account(participant, definition)
        );
        assert_eq!(
            invocation.account_ids[6],
            super::associated_token_account(participant, collateral_definition)
        );
        assert_eq!(
            invocation.account_ids[7],
            super::associated_token_account(treasury, definition)
        );
        assert!(matches!(
            invocation.instruction,
            curve_core::Instruction::SwapExactInput {
                amount_in: 25,
                min_amount_out: 10,
                token_in,
            } if token_in == definition
        ));
    }

    #[test]
    fn buy_quote_uses_current_reserves_and_live_fee_settings() {
        let token_definition = AccountId::new([2; 32]);
        let collateral_definition = AccountId::new([5; 32]);
        let pool = curve_core::PoolAccount {
            token0_definition_id: token_definition,
            token1_definition_id: collateral_definition,
            owner: AccountId::new([1; 32]),
            pool: Pool::create(800, 100, 1_000, 100, None).expect("valid pool"),
        };
        let config = curve_core::Config {
            admin: AccountId::new([3; 32]),
            pool_fee_bps: 0,
            protocol_fee_bps: 0,
            treasury: AccountId::new([4; 32]),
        };

        let quote = quote_buy(&pool, &config, 200, 1).expect("quote should succeed");
        assert_eq!(quote.amount_in, 25);
        assert_eq!(quote.amount_out, 200);
    }

    #[test]
    fn factory_withdrawal_routes_closed_pool_reserves_through_the_factory_policy() {
        let launch_salt = [1; 32];
        let creator = AccountId::new([9; 32]);
        let collateral_definition = AccountId::new([5; 32]);
        let invocation = build_withdraw_factory_pool_invocation(
            FACTORY_PROGRAM_ID,
            CURVE_PROGRAM_ID,
            creator,
            launch_salt,
            collateral_definition,
        );
        let factory = compute_factory_pda(FACTORY_PROGRAM_ID, launch_salt);
        let definition = compute_definition_pda(FACTORY_PROGRAM_ID, launch_salt);
        assert_eq!(invocation.program_id, FACTORY_PROGRAM_ID);
        assert_eq!(invocation.signer_accounts, vec![creator]);
        assert_eq!(invocation.account_ids.len(), 12);
        assert_eq!(invocation.account_ids[0], factory);
        assert_eq!(invocation.account_ids[3], definition);
        assert_eq!(
            invocation.account_ids[9],
            super::associated_token_account(creator, definition)
        );
        assert_eq!(
            invocation.account_ids[11],
            clock_core::CLOCK_01_PROGRAM_ACCOUNT_ID
        );
        assert!(matches!(
            invocation.instruction,
            factory_core::Instruction::WithdrawFactoryPool
        ));
    }
}
