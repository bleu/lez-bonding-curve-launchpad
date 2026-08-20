//! Host-side helpers shared by the runners in `src/bin/`.

pub mod runner_support {
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
}
