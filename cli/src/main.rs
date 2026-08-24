//! The `launchpad` binary. Argument parsing belongs here; wallet handling,
//! account derivation, quoting, and transaction construction belong in
//! `launchpad-client`.

use anyhow::Result;
use clap::{Args, Parser, Subcommand, builder::Styles};

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
    Close(LaunchArgs),
    Withdraw(LaunchArgs),
    Claim(LaunchArgs),
    Buy(TradeArgs),
    Sell(SellArgs),
    Price(PriceArgs),
    Status(LaunchArgs),
    SaleInfo(LaunchArgs),
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
}

#[derive(Debug, Clone, Args)]
struct LaunchArgs {
    /// Explicit 32-byte hexadecimal launch namespace.
    #[arg(long, value_parser = parse_launch_salt)]
    launch_salt: [u8; 32],
}

#[derive(Debug, Args)]
struct TradeArgs {
    #[command(flatten)]
    launch: LaunchArgs,
    #[arg(long)]
    tokens: u128,
    #[arg(long = "max-collateral")]
    max_collateral: u128,
}

#[derive(Debug, Args)]
struct SellArgs {
    #[command(flatten)]
    launch: LaunchArgs,
    #[arg(long)]
    tokens: u128,
    #[arg(long = "min-collateral")]
    min_collateral: u128,
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("price-input").required(true).args(["tokens", "collateral"]))) ]
struct PriceArgs {
    #[command(flatten)]
    launch: LaunchArgs,
    #[arg(long)]
    tokens: Option<u128>,
    #[arg(long)]
    collateral: Option<u128>,
}

fn parse_launch_salt(raw: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(raw).map_err(|_| "launch salt must be hexadecimal".to_owned())?;
    bytes
        .try_into()
        .map_err(|_| "launch salt must contain exactly 32 bytes (64 hex characters)".to_owned())
}

fn main() -> Result<()> {
    let _cli = Cli::parse();
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
        ])
        .expect("buy command should parse");
        assert!(matches!(
            cli.command,
            Command::Buy(TradeArgs {
                tokens: 25,
                max_collateral: 100,
                ..
            })
        ));
    }

    #[test]
    fn launch_salt_must_be_exactly_32_bytes() {
        let error = Cli::try_parse_from(["launchpad", "status", "--launch-salt", "00"])
            .expect_err("short launch salts must be rejected");
        assert!(error.to_string().contains("exactly 32 bytes"));
    }
}
