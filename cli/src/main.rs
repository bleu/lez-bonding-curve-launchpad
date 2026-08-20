//! The `launchpad` binary. Parses arguments and calls `launchpad-client`, and nothing
//! else. The `launchpad-client` dependency is declared before it is used so the boundary
//! is enforced by the manifest from the start rather than argued about later.
//!
//! Subcommands land in GTM-517, against the lifecycle operations the client exposes.

use anyhow::{Result, bail};
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "launchpad",
    version,
    about = "Bonding curve launchpad on the Logos Execution Zone"
)]
struct Cli {}

fn main() -> Result<()> {
    let Cli {} = Cli::parse();
    bail!("no subcommands yet — the curve program lands first (GTM-509)")
}
