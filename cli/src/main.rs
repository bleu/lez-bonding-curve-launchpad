//! The `launchpad` binary. Argument parsing belongs here; wallet handling,
//! account derivation, quoting, and transaction construction belong in
//! `launchpad-client`.

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, builder::Styles};
use launchpad_client::{
    BuyRequest, CreateSaleRequest, SellRequest, build_buy_invocation,
    build_claim_creator_allocation_invocation, build_close_factory_pool_invocation,
    build_create_sale_invocation, build_sell_invocation,
    build_withdraw_factory_proceeds_invocation, load_curve_config, load_factory_pool,
    load_factory_state, load_program, parse_account_id, quote_buy, quote_buy_with_collateral,
    quote_sell, submit_public_invocation,
};
use serde::Serialize;
use wallet::WalletCore;

#[derive(Debug, Clone, Copy)]
enum ErrorCategory {
    SlippageFloor,
    SaleReserveOvershoot,
    CollateralReserveOvershoot,
    General,
}

impl ErrorCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::SlippageFloor => "slippage_floor",
            Self::SaleReserveOvershoot => "sale_reserve_overshoot",
            Self::CollateralReserveOvershoot => "collateral_reserve_overshoot",
            Self::General => "general",
        }
    }
}

#[derive(Debug)]
struct ClassifiedError {
    category: ErrorCategory,
}

impl std::fmt::Display for ClassifiedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.category.fmt(f)
    }
}

impl std::error::Error for ClassifiedError {}

impl std::fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str((*self).as_str())
    }
}

#[derive(Debug, Parser)]
#[command(name = "launchpad", version, about = "Bonding curve launchpad on the Logos Execution Zone", styles = Styles::styled())]
struct Cli {
    /// Emit stable machine-readable output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a factory-owned launch and its neutral curve pool.
    #[command(alias = "launch")]
    CreateSale(CreateSaleArgs),
    Close(FactoryLifecycleArgs),
    Withdraw(FactoryLifecycleArgs),
    Claim(FactoryLifecycleArgs),
    Buy(BuyArgs),
    Sell(SellArgs),
    Price(PriceArgs),
    Status(LaunchReadArgs),
    SaleInfo(LaunchReadArgs),
    Configure(ConfigArgs),
}

#[derive(Debug, Args)]
struct CreateSaleArgs {
    #[command(flatten)]
    launch: LaunchArgs,
    #[arg(long)]
    name: String,
    #[arg(long)]
    uri: String,
    #[arg(long = "sale-reserve", alias = "d")]
    sale_reserve: u128,
    #[arg(long = "dex-seed-reserve", alias = "r")]
    dex_seed_reserve: u128,
    #[arg(long = "creator-allocation")]
    creator_allocation: u128,
    #[arg(long = "virtual-token-reserve", alias = "vt")]
    virtual_token_reserve: u128,
    #[arg(long = "virtual-collateral-reserve", alias = "vc")]
    virtual_collateral_reserve: u128,
    #[arg(long = "end-timestamp")]
    end_timestamp: Option<u64>,
    #[arg(long = "collateral-definition")]
    collateral_definition: String,
    /// Public account that authorizes the factory launch.
    #[arg(long)]
    creator: String,
    /// Compiled factory guest binary deployed for this walkthrough.
    #[arg(long = "factory-program-path")]
    factory_program_path: PathBuf,
    /// Compiled curve guest binary paired with the factory launch.
    #[arg(long = "curve-program-path")]
    curve_program_path: PathBuf,
}

#[derive(Debug, Clone, Args)]
struct LaunchArgs {
    /// Explicit 32-byte hexadecimal launch namespace.
    #[arg(long, value_parser = parse_launch_salt)]
    launch_salt: [u8; 32],
}

#[derive(Debug, Args)]
struct FactoryTradeArgs {
    #[command(flatten)]
    launch: LaunchArgs,
    #[arg(long = "collateral-definition")]
    collateral_definition: String,
    /// Public account that authorizes the trade.
    #[arg(long)]
    participant: String,
    #[arg(long = "factory-program-path")]
    factory_program_path: PathBuf,
    #[arg(long = "curve-program-path")]
    curve_program_path: PathBuf,
}

#[derive(Debug, Args)]
struct BuyArgs {
    #[command(flatten)]
    trade: FactoryTradeArgs,
    #[arg(long)]
    tokens: u128,
    #[arg(long = "max-collateral")]
    max_collateral: u128,
}

#[derive(Debug, Args)]
struct FactoryLifecycleArgs {
    #[command(flatten)]
    launch: LaunchArgs,
    #[arg(long = "collateral-definition")]
    collateral_definition: String,
    #[arg(long)]
    creator: String,
    #[arg(long = "factory-program-path")]
    factory_program_path: PathBuf,
    #[arg(long = "curve-program-path")]
    curve_program_path: PathBuf,
}

#[derive(Debug, Args)]
struct LaunchReadArgs {
    #[command(flatten)]
    launch: LaunchArgs,
    #[arg(long = "collateral-definition")]
    collateral_definition: String,
    #[arg(long = "factory-program-path")]
    factory_program_path: PathBuf,
    #[arg(long = "curve-program-path")]
    curve_program_path: PathBuf,
}

#[derive(Debug, Args)]
struct SellArgs {
    #[command(flatten)]
    trade: FactoryTradeArgs,
    #[arg(long)]
    tokens: u128,
    #[arg(long = "min-collateral")]
    min_collateral: u128,
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("price-input").required(true).args(["tokens", "collateral"]))) ]
struct PriceArgs {
    #[command(flatten)]
    launch: LaunchReadArgs,
    #[arg(long)]
    tokens: Option<u128>,
    /// Collateral to spend for an exact-input purchase quote.
    #[arg(long)]
    collateral: Option<u128>,
}

#[derive(Debug, Args)]
struct ConfigArgs {
    #[arg(long)]
    admin: String,
    #[arg(long, default_value_t = 0)]
    pool_fee_bps: u16,
    #[arg(long, default_value_t = 0)]
    protocol_fee_bps: u16,
    #[arg(long)]
    treasury: String,
    #[arg(long = "curve-program-path")]
    curve_program_path: PathBuf,
}

fn parse_launch_salt(raw: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(raw).map_err(|_| "launch salt must be hexadecimal".to_owned())?;
    bytes
        .try_into()
        .map_err(|_| "launch salt must contain exactly 32 bytes (64 hex characters)".to_owned())
}

#[derive(Serialize)]
struct SubmittedLaunch {
    status: &'static str,
    transaction_hash: String,
    launch_salt: String,
}

#[derive(Serialize)]
struct SaleSnapshot {
    launch_salt: String,
    factory_program: String,
    pool: String,
    token_definition: String,
    collateral_definition: String,
    status: &'static str,
    creator_allocation_claimed: bool,
    real_token_reserve: u128,
    real_collateral_reserve: u128,
    virtual_token_reserve: u128,
    virtual_collateral_reserve: u128,
    close_timestamp: Option<u64>,
}

#[derive(Serialize)]
struct PriceQuote {
    kind: &'static str,
    amount_in: u128,
    amount_out: u128,
    pool_fee: u128,
    protocol_fee: u128,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    if let Err(error) = run(json, cli).await {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "status": "error",
                    "error": {
                        "category": classify_error(&error).as_str(),
                        "message": error.to_string(),
                    },
                })
            );
        } else {
            eprintln!("Error: {error:#}");
        }
        std::process::exit(1);
    }
}

async fn run(json: bool, cli: Cli) -> Result<()> {
    match cli.command {
        Command::CreateSale(args) => create_sale(json, args).await,
        Command::Close(args) => close_factory_pool(json, args).await,
        Command::Withdraw(args) => withdraw_factory_proceeds(json, args).await,
        Command::Claim(args) => claim_creator_allocation(json, args).await,
        Command::Buy(args) => buy(json, args).await,
        Command::Sell(args) => sell(json, args).await,
        Command::Price(args) => price(json, args).await,
        Command::Status(args) | Command::SaleInfo(args) => sale_snapshot(json, args).await,
        Command::Configure(args) => configure(json, args).await,
    }
}

fn classify_error(error: &anyhow::Error) -> ErrorCategory {
    if let Some(classified) = error.downcast_ref::<ClassifiedError>() {
        return classified.category;
    }

    let message = error.to_string().to_lowercase();
    if message.contains("slippage") || message.contains("max_amount_in") {
        ErrorCategory::SlippageFloor
    } else if message.contains("sale reserve") || message.contains("real_reserve0") {
        ErrorCategory::SaleReserveOvershoot
    } else if message.contains("collateral reserve") || message.contains("real_reserve1") {
        ErrorCategory::CollateralReserveOvershoot
    } else {
        ErrorCategory::General
    }
}

fn classified(category: ErrorCategory) -> anyhow::Error {
    ClassifiedError { category }.into()
}

async fn configure(json: bool, args: ConfigArgs) -> Result<()> {
    let curve_program = load_program(&args.curve_program_path)?;
    let admin = parse_account_id(&args.admin)?;
    let treasury = parse_account_id(&args.treasury)?;
    let invocation = launchpad_client::build_update_config_invocation(
        curve_program.id(),
        admin,
        args.pool_fee_bps,
        args.protocol_fee_bps,
        treasury,
    );
    let wallet = WalletCore::from_env().context("opening the project wallet")?;
    let transaction_hash = submit_public_invocation(&wallet, &curve_program, invocation).await?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "status": "submitted",
                "transaction_hash": hex::encode(transaction_hash),
            })
        );
    } else {
        println!(
            "submitted curve configuration: tx_hash={}",
            hex::encode(transaction_hash)
        );
    }
    Ok(())
}

async fn withdraw_factory_proceeds(json: bool, args: FactoryLifecycleArgs) -> Result<()> {
    let factory_program = load_program(&args.factory_program_path)?;
    let curve_program = load_program(&args.curve_program_path)?;
    let creator = parse_account_id(&args.creator)?;
    let collateral_definition = parse_account_id(&args.collateral_definition)?;
    let invocation = build_withdraw_factory_proceeds_invocation(
        factory_program.id(),
        curve_program.id(),
        creator,
        args.launch.launch_salt,
        collateral_definition,
    );
    let wallet = WalletCore::from_env().context("opening the project wallet")?;
    let transaction_hash = submit_public_invocation(&wallet, &factory_program, invocation).await?;
    print_submission(
        json,
        "factory withdrawal",
        transaction_hash,
        args.launch.launch_salt,
    )
}

async fn price(json: bool, args: PriceArgs) -> Result<()> {
    let factory_program = load_program(&args.launch.factory_program_path)?;
    let curve_program = load_program(&args.launch.curve_program_path)?;
    let collateral_definition = parse_account_id(&args.launch.collateral_definition)?;
    let wallet = WalletCore::from_env().context("opening the project wallet")?;
    let pool = load_factory_pool(
        &wallet,
        factory_program.id(),
        curve_program.id(),
        args.launch.launch.launch_salt,
        collateral_definition,
    )
    .await?;
    let config = load_curve_config(&wallet, curve_program.id()).await?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("reading the current time")?
        .as_secs();
    let (kind, quote) = match (args.tokens, args.collateral) {
        (Some(tokens), None) => ("buy", quote_buy(&pool, &config, tokens, now)?),
        (None, Some(collateral)) => (
            "buy",
            quote_buy_with_collateral(&pool, &config, collateral, now)?,
        ),
        _ => anyhow::bail!("provide exactly one of --tokens or --collateral"),
    };
    let output = PriceQuote {
        kind,
        amount_in: quote.amount_in,
        amount_out: quote.amount_out,
        pool_fee: quote.pool_fee,
        protocol_fee: quote.protocol_fee,
    };
    if json {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!(
            "{kind} quote: input={} output={} pool_fee={} protocol_fee={}",
            output.amount_in, output.amount_out, output.pool_fee, output.protocol_fee
        );
    }
    Ok(())
}

async fn sale_snapshot(json: bool, args: LaunchReadArgs) -> Result<()> {
    let factory_program = load_program(&args.factory_program_path)?;
    let curve_program = load_program(&args.curve_program_path)?;
    let collateral_definition = parse_account_id(&args.collateral_definition)?;
    let wallet = WalletCore::from_env().context("opening the project wallet")?;
    let factory =
        load_factory_state(&wallet, factory_program.id(), args.launch.launch_salt).await?;
    let pool = load_factory_pool(
        &wallet,
        factory_program.id(),
        curve_program.id(),
        args.launch.launch_salt,
        collateral_definition,
    )
    .await?;
    let status = match pool.pool.lifecycle {
        pool::PoolLifecycle::Open => "open",
        pool::PoolLifecycle::Closed => "closed",
        pool::PoolLifecycle::Withdrawn => "withdrawn",
    };
    let snapshot = SaleSnapshot {
        launch_salt: hex::encode(args.launch.launch_salt),
        factory_program: factory_program
            .id()
            .iter()
            .map(|word| format!("{word:08x}"))
            .collect(),
        pool: factory.pool_id.to_string(),
        token_definition: factory.token_definition_id.to_string(),
        collateral_definition: factory.collateral_definition_id.to_string(),
        status,
        creator_allocation_claimed: factory.creator_allocation_claimed,
        real_token_reserve: pool.pool.real_reserve0,
        real_collateral_reserve: pool.pool.real_reserve1,
        virtual_token_reserve: pool.pool.virtual_reserve0,
        virtual_collateral_reserve: pool.pool.virtual_reserve1,
        close_timestamp: pool.pool.close_timestamp,
    };
    if json {
        println!("{}", serde_json::to_string(&snapshot)?);
    } else {
        println!(
            "sale {status}: pool={} token_reserve={} collateral_reserve={}",
            snapshot.pool, snapshot.real_token_reserve, snapshot.real_collateral_reserve
        );
    }
    Ok(())
}

async fn buy(json: bool, args: BuyArgs) -> Result<()> {
    let factory_program = load_program(&args.trade.factory_program_path)?;
    let curve_program = load_program(&args.trade.curve_program_path)?;
    let participant = parse_account_id(&args.trade.participant)?;
    let collateral_definition = parse_account_id(&args.trade.collateral_definition)?;
    let wallet = WalletCore::from_env().context("opening the project wallet")?;
    let config = load_curve_config(&wallet, curve_program.id()).await?;
    let pool = load_factory_pool(
        &wallet,
        factory_program.id(),
        curve_program.id(),
        args.trade.launch.launch_salt,
        collateral_definition,
    )
    .await?;
    if args.tokens > pool.pool.real_reserve0 {
        return Err(classified(ErrorCategory::SaleReserveOvershoot));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("reading the current time")?
        .as_secs();
    let quote = quote_buy(&pool, &config, args.tokens, now)?;
    if quote.amount_in > args.max_collateral {
        return Err(classified(ErrorCategory::SlippageFloor));
    }
    let invocation = build_buy_invocation(
        factory_program.id(),
        curve_program.id(),
        participant,
        config.treasury,
        BuyRequest {
            launch_salt: args.trade.launch.launch_salt,
            collateral_definition,
            amount_out: args.tokens,
            max_amount_in: args.max_collateral,
        },
    );
    let transaction_hash = submit_public_invocation(&wallet, &curve_program, invocation).await?;
    print_submission(
        json,
        "purchase",
        transaction_hash,
        args.trade.launch.launch_salt,
    )
}

async fn sell(json: bool, args: SellArgs) -> Result<()> {
    let factory_program = load_program(&args.trade.factory_program_path)?;
    let curve_program = load_program(&args.trade.curve_program_path)?;
    let participant = parse_account_id(&args.trade.participant)?;
    let collateral_definition = parse_account_id(&args.trade.collateral_definition)?;
    let wallet = WalletCore::from_env().context("opening the project wallet")?;
    let config = load_curve_config(&wallet, curve_program.id()).await?;
    let pool = load_factory_pool(
        &wallet,
        factory_program.id(),
        curve_program.id(),
        args.trade.launch.launch_salt,
        collateral_definition,
    )
    .await?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("reading the current time")?
        .as_secs();
    let quote = quote_sell(&pool, &config, args.tokens, now)?;
    if quote.amount_out > pool.pool.real_reserve1 {
        return Err(classified(ErrorCategory::CollateralReserveOvershoot));
    }
    if quote.amount_out < args.min_collateral {
        return Err(classified(ErrorCategory::SlippageFloor));
    }
    let invocation = build_sell_invocation(
        factory_program.id(),
        curve_program.id(),
        participant,
        config.treasury,
        SellRequest {
            launch_salt: args.trade.launch.launch_salt,
            collateral_definition,
            amount_in: args.tokens,
            min_amount_out: args.min_collateral,
        },
    );
    let transaction_hash = submit_public_invocation(&wallet, &curve_program, invocation).await?;
    print_submission(
        json,
        "sale",
        transaction_hash,
        args.trade.launch.launch_salt,
    )
}

fn print_submission(
    json: bool,
    action: &str,
    transaction_hash: common::HashType,
    launch_salt: [u8; 32],
) -> Result<()> {
    let output = SubmittedLaunch {
        status: "submitted",
        transaction_hash: hex::encode(transaction_hash),
        launch_salt: hex::encode(launch_salt),
    };
    if json {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!(
            "submitted {action}: tx_hash={} launch_salt={}",
            output.transaction_hash, output.launch_salt
        );
    }
    Ok(())
}

async fn close_factory_pool(json: bool, args: FactoryLifecycleArgs) -> Result<()> {
    let factory_program = load_program(&args.factory_program_path)?;
    let curve_program = load_program(&args.curve_program_path)?;
    let creator = parse_account_id(&args.creator)?;
    let collateral_definition = parse_account_id(&args.collateral_definition)?;
    let invocation = build_close_factory_pool_invocation(
        factory_program.id(),
        curve_program.id(),
        creator,
        args.launch.launch_salt,
        collateral_definition,
    );
    let wallet = WalletCore::from_env().context("opening the project wallet")?;
    let transaction_hash = submit_public_invocation(&wallet, &factory_program, invocation).await?;
    let output = SubmittedLaunch {
        status: "submitted",
        transaction_hash: hex::encode(transaction_hash),
        launch_salt: hex::encode(args.launch.launch_salt),
    };
    if json {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!(
            "submitted factory close: tx_hash={} launch_salt={}",
            output.transaction_hash, output.launch_salt
        );
    }
    Ok(())
}

async fn claim_creator_allocation(json: bool, args: FactoryLifecycleArgs) -> Result<()> {
    let factory_program = load_program(&args.factory_program_path)?;
    let curve_program = load_program(&args.curve_program_path)?;
    let creator = parse_account_id(&args.creator)?;
    let collateral_definition = parse_account_id(&args.collateral_definition)?;
    let invocation = build_claim_creator_allocation_invocation(
        factory_program.id(),
        curve_program.id(),
        creator,
        args.launch.launch_salt,
        collateral_definition,
    );
    let wallet = WalletCore::from_env().context("opening the project wallet")?;
    let transaction_hash = submit_public_invocation(&wallet, &factory_program, invocation).await?;
    let output = SubmittedLaunch {
        status: "submitted",
        transaction_hash: hex::encode(transaction_hash),
        launch_salt: hex::encode(args.launch.launch_salt),
    };
    if json {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!(
            "submitted creator allocation claim: tx_hash={} launch_salt={}",
            output.transaction_hash, output.launch_salt
        );
    }
    Ok(())
}

async fn create_sale(json: bool, args: CreateSaleArgs) -> Result<()> {
    let factory_program = load_program(&args.factory_program_path)?;
    let curve_program = load_program(&args.curve_program_path)?;
    let creator = parse_account_id(&args.creator)?;
    let collateral_definition = parse_account_id(&args.collateral_definition)?;
    let invocation = build_create_sale_invocation(
        factory_program.id(),
        curve_program.id(),
        creator,
        CreateSaleRequest {
            launch_salt: args.launch.launch_salt,
            name: args.name,
            uri: args.uri,
            sale_reserve: args.sale_reserve,
            dex_seed_reserve: args.dex_seed_reserve,
            creator_allocation: args.creator_allocation,
            virtual_token_reserve: args.virtual_token_reserve,
            virtual_collateral_reserve: args.virtual_collateral_reserve,
            end_timestamp: args.end_timestamp,
            collateral_definition,
        },
    )?;
    let wallet = WalletCore::from_env().context("opening the project wallet")?;
    let transaction_hash = submit_public_invocation(&wallet, &factory_program, invocation).await?;
    let output = SubmittedLaunch {
        status: "submitted",
        transaction_hash: hex::encode(transaction_hash),
        launch_salt: hex::encode(args.launch.launch_salt),
    };
    if json {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!(
            "submitted factory launch: tx_hash={} launch_salt={}",
            output.transaction_hash, output.launch_salt
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn launchpad_exposes_creator_and_participant_commands() {
        let help = Cli::command().render_help().to_string();
        for command in [
            "create-sale",
            "close",
            "withdraw",
            "claim",
            "buy",
            "sell",
            "price",
            "status",
            "sale-info",
        ] {
            assert!(help.contains(command), "missing command: {command}");
        }
    }

    #[test]
    fn buy_requires_exact_output_and_collateral_cap() {
        let cli = Cli::try_parse_from([
            "launchpad",
            "buy",
            "--launch-salt",
            &"11".repeat(32),
            "--tokens",
            "25",
            "--max-collateral",
            "100",
            "--collateral-definition",
            "collateral",
            "--participant",
            "Public/participant",
            "--factory-program-path",
            "factory.bin",
            "--curve-program-path",
            "curve.bin",
        ])
        .expect("buy command should parse");
        assert!(matches!(
            cli.command,
            Command::Buy(BuyArgs {
                tokens: 25,
                max_collateral: 100,
                ..
            })
        ));
    }

    #[test]
    fn sell_accepts_the_factory_and_curve_context() {
        let cli = Cli::try_parse_from([
            "launchpad",
            "sell",
            "--launch-salt",
            &"11".repeat(32),
            "--tokens",
            "25",
            "--min-collateral",
            "10",
            "--collateral-definition",
            "collateral",
            "--participant",
            "Public/participant",
            "--factory-program-path",
            "factory.bin",
            "--curve-program-path",
            "curve.bin",
        ])
        .expect("sell command should parse");
        assert!(matches!(cli.command, Command::Sell(_)));
    }

    #[test]
    fn withdraw_accepts_the_factory_lifecycle_context() {
        let cli = Cli::try_parse_from([
            "launchpad",
            "withdraw",
            "--launch-salt",
            &"11".repeat(32),
            "--collateral-definition",
            "collateral",
            "--creator",
            "Public/creator",
            "--factory-program-path",
            "factory.bin",
            "--curve-program-path",
            "curve.bin",
        ])
        .expect("withdraw command should parse");
        assert!(matches!(cli.command, Command::Withdraw(_)));
    }

    #[test]
    fn create_sale_accepts_the_program_binaries_and_creator_account() {
        let cli = Cli::try_parse_from([
            "launchpad",
            "create-sale",
            "--launch-salt",
            &"11".repeat(32),
            "--name",
            "E2E token",
            "--uri",
            "https://example.invalid/e2e-token.json",
            "--sale-reserve",
            "800",
            "--dex-seed-reserve",
            "100",
            "--creator-allocation",
            "50",
            "--virtual-token-reserve",
            "2000",
            "--virtual-collateral-reserve",
            "100",
            "--collateral-definition",
            "collateral",
            "--creator",
            "Public/creator",
            "--factory-program-path",
            "factory.bin",
            "--curve-program-path",
            "curve.bin",
        ])
        .expect("create sale command should parse");
        assert!(matches!(cli.command, Command::CreateSale(_)));
    }

    #[test]
    fn close_accepts_the_factory_lifecycle_context() {
        let cli = Cli::try_parse_from([
            "launchpad",
            "close",
            "--launch-salt",
            &"11".repeat(32),
            "--collateral-definition",
            "collateral",
            "--creator",
            "Public/creator",
            "--factory-program-path",
            "factory.bin",
            "--curve-program-path",
            "curve.bin",
        ])
        .expect("close command should parse");
        assert!(matches!(cli.command, Command::Close(_)));
    }

    #[test]
    fn status_accepts_the_program_context_needed_to_read_live_pool_state() {
        let cli = Cli::try_parse_from([
            "launchpad",
            "status",
            "--launch-salt",
            &"11".repeat(32),
            "--collateral-definition",
            "collateral",
            "--factory-program-path",
            "factory.bin",
            "--curve-program-path",
            "curve.bin",
        ])
        .expect("status command should parse");
        assert!(matches!(cli.command, Command::Status(_)));
    }

    #[test]
    fn price_accepts_an_exact_output_buy_quote_with_live_program_context() {
        let cli = Cli::try_parse_from([
            "launchpad",
            "price",
            "--launch-salt",
            &"11".repeat(32),
            "--tokens",
            "25",
            "--collateral-definition",
            "collateral",
            "--factory-program-path",
            "factory.bin",
            "--curve-program-path",
            "curve.bin",
        ])
        .expect("price command should parse");
        assert!(matches!(cli.command, Command::Price(_)));
    }

    #[test]
    fn configure_accepts_the_curve_admin_and_treasury_context() {
        let cli = Cli::try_parse_from([
            "launchpad",
            "--json",
            "configure",
            "--admin",
            "Public/admin",
            "--treasury",
            "Public/treasury",
            "--curve-program-path",
            "curve.bin",
        ])
        .expect("configure command should parse");
        assert!(matches!(cli.command, Command::Configure(_)));
    }

    #[test]
    fn json_error_categories_are_stable() {
        for (message, expected) in [
            ("slippage cap exceeded", "slippage_floor"),
            ("sale reserve exhausted", "sale_reserve_overshoot"),
            (
                "collateral reserve exhausted",
                "collateral_reserve_overshoot",
            ),
        ] {
            assert_eq!(classify_error(&anyhow::anyhow!(message)).as_str(), expected);
        }
    }

    #[test]
    fn launch_salt_must_be_exactly_32_bytes() {
        let error = Cli::try_parse_from(["launchpad", "status", "--launch-salt", "00"])
            .expect_err("short launch salts must be rejected");
        assert!(error.to_string().contains("exactly 32 bytes"));
    }
}
