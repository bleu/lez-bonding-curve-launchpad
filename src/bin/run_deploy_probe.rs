//! Day 1 gate runner (GTM-506): submit a transaction to our own program and
//! wait for the sequencer to include it.
//!
//! Waiting matters. LEZ's own example discards the send response, so it exits 0
//! whether the transaction lands or is silently dropped — which is how a broken
//! guest build looks like success.
//!
//! Usage: run_deploy_probe <account_id> [payload]
//! Set PROGRAM_PATH to override guest binary discovery.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use common::transaction::LeeTransaction;
use launchpad_client::{load_program, parse_account_id};
use lee::{
    PublicTransaction,
    public_transaction::{Message, WitnessSet},
};
use sequencer_service_rpc::RpcClient as _;
use wallet::WalletCore;

const GUEST_BIN_CANDIDATES: &[&str] = &[
    "methods/target/riscv-guest/launchpad_methods/launchpad_programs/riscv32im-risc0-zkvm-elf/release/deploy_probe.bin",
    "target/riscv-guest/launchpad_methods/launchpad_programs/riscv32im-risc0-zkvm-elf/release/deploy_probe.bin",
];

fn resolve_guest_bin() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("PROGRAM_PATH") {
        return Ok(PathBuf::from(explicit));
    }
    for candidate in GUEST_BIN_CANDIDATES {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Ok(path);
        }
    }
    bail!(
        "no guest binary found; run `lgs build`, or set PROGRAM_PATH. Looked in: {}",
        GUEST_BIN_CANDIDATES.join(", ")
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let account_arg = args
        .next()
        .context("usage: run_deploy_probe <account_id> [payload]")?;
    let account_id = parse_account_id(&account_arg)?;
    let payload = args.next().unwrap_or_else(|| "day-1".to_string());

    // Not async on LEZ v0.2.0.
    let wallet_core = WalletCore::from_env()
        .context("opening wallet — export LEE_WALLET_HOME_DIR (and NSSA_WALLET_HOME_DIR)")?;

    let guest_bin = resolve_guest_bin()?;
    let program = load_program(&guest_bin)?;
    println!("program:    {:?}", program.id());
    println!("guest bin:  {}", guest_bin.display());
    println!("account:    {account_id}");
    println!("payload:    {payload:?}");

    // The guest claims the account with `Claim::Authorized`, so the signer must
    // have authorized it. Submitting unsigned gets the transaction dropped with
    // `InvalidProgramBehavior(ClaimedUnauthorizedAccount)` — visible only in the
    // sequencer log, never in the submit response.
    let signing_key = wallet_core
        .storage()
        .key_chain()
        .pub_account_signing_key(account_id)
        .context("target must be a self-owned public account from this wallet")?;
    let nonces = wallet_core
        .get_accounts_nonces(vec![account_id])
        .await
        .map_err(|e| anyhow::anyhow!("querying nonces: {e:?}"))?;
    let signing_keys = [signing_key];

    let message = Message::try_new(program.id(), vec![account_id], nonces, payload.into_bytes())
        .map_err(|e| anyhow::anyhow!("building message: {e:?}"))?;
    let witness_set = WitnessSet::for_message(&message, &signing_keys);
    let tx = PublicTransaction::new(message, witness_set);

    let hash = wallet_core
        .sequencer_client
        .send_transaction(LeeTransaction::Public(tx))
        .await
        .map_err(|e| anyhow::anyhow!("sequencer rejected the submission: {e:?}"))?;
    println!("submitted:  {hash}");

    // Submitted is not included. Block until the sequencer confirms, so a
    // dropped transaction fails loudly instead of passing as success.
    wallet_core
        .poll_native_token_transfer(hash)
        .await
        .map_err(|e| anyhow::anyhow!("never included after polling: {e:?}"))?;
    println!("included:   yes");
    println!("verify:     lgs wallet -- account get --account-id Public/{account_id}");
    Ok(())
}
