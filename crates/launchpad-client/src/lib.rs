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
use lee::{AccountId, program::Program};

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
