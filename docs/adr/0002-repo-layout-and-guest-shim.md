# 0002 — Repo layout: handlers in the host workspace, guests as shims

Status: accepted

## Context

`lgs deploy` globs `methods/guest/src/bin/*.rs` at the project root and searches `target/riscv-guest` and `methods/target` for the artefacts. The guest crate is excluded from the root workspace so it keeps its own `[profile.release]`, which is part of program identity: the risc0 image ID is computed over the release-stripped binary, and every PDA derives from it.

The in-tree programs are laid out differently. `lez/programs/amm/core/` holds the instruction enum, the Borsh state and the PDA derivation, while `lez/programs/amm/src/` holds the handlers, `main.rs`, and 3563 lines of adapter tests with `lee = { features = ["test-utils"] }` as a dev-dependency. Every other in-tree program follows the same split.

Copying that literally would put the handlers and their tests inside `methods/guest`. Because the root workspace excludes that directory, `cargo test --workspace` would never reach them, and neither would anything in the `lgs` pipeline.

## Decision

Keep the `core` plus adapter split, but move both into the root workspace and reduce the guest to a dispatch shim.

```
crates/curve-math/       pricing arithmetic, empty dependency list
crates/pool/             bounded pool state machine, curve-math and borsh only
crates/curve-core/       instruction enum, Borsh state, PDA derivation, handlers, tests
crates/launchpad-client/ the SDK
cli/                     the `launchpad` binary
methods/                 risc0 methods crate, excluded from the workspace
methods/guest/src/bin/   one guest per file, each a shim over its core crate
verify/                  what a reviewer runs
docs/adr/  CONTEXT.md
```

`curve-math` and `pool` are separate crates rather than modules because that boundary must be real. An empty `[dependencies]` is checkable; a module is a promise that one careless `use lee_core` breaks. Both crates deny `clippy::arithmetic_side_effects`, so bare arithmetic is impossible where the correctness claims live and every exception has to carry a written reason.

Stay on scaffold's `default` template rather than `lez-framework`. The framework macro generates its own instruction enum, which collides with the hand-written enum shared by `curve-core`, the guest and the client. To retain that single source of wire semantics while satisfying U09, `idl-src/` carries SPEL declarative interface sources with `instruction = "…::Instruction"`; the project-pinned `spel generate-idl` produces checked-in `idl/*.json`. `verify/check-idl.sh` rejects drift. The declarations describe account order and arguments; handler validation remains in the hand-written guest.

`lee_core` is declared in `[workspace.dependencies]` without the `host` feature. Workspace inheritance resolves against the depending crate's own workspace, so `curve-core` writing `lee_core.workspace = true` would hand `host` to the guest as well, pulling in `ml-kem` and `getrandom` for `riscv32im-risc0-zkvm-elf`. LEZ declares it the same way and lets consumers opt in. On the host side the feature arrives anyway, because `lee` enables it.

`methods/guest/Cargo.toml` declares `curve-core` as a path dependency before anything uses it. Cargo compiles declared dependencies whether they are referenced or not, so `curve-core`, `pool`, `curve-math` and `lee_core` are all built for the guest target on every `lgs build`. A host-feature leak becomes a build failure here rather than a surprise later.

## Consequences

As superseded by ADR 0006, `Pool` lives in `crates/pool` and holds only what pricing and solvency need. `curve-core` wraps it in `PoolAccount` next to ordered token IDs and owner, then serialises it into the pool PDA. `PoolAccount` is local to `curve-core`, so the account data conversions follow the in-tree `TryFrom<&Data>` and `From<&PoolAccount>` style.

GTM-509 replaced `deploy_probe` with the real curve guest and collapsed the root package to a virtual manifest. `runner_support` remains in `crates/launchpad-client`. The `lez-template` skill identifies a default-template project partly by the former root `src/lib.rs`, so identification now rests on `scaffold.toml` and `methods/guest/src/bin/`. Nothing in `lgs` requires a root package: `build` runs `cargo build --workspace`, and `deploy` globs the guest directory.

Diverging from the in-tree layout is defensible on precedent. `eth-lez-atomic-swaps` puts its program at `programs/lez-htlc/methods/` and pins a scaffold commit specifically to set `deploy = false` and skip scaffold's deploy step. With two programs and one week, that is not worth it.

`rustfmt.toml` is copied verbatim from LEZ so diffs read the same way, but seven of its eleven options are nightly-only and the pinned stable toolchain ignores them with a warning on every run. Their clippy configuration is not copied: `clippy::restriction` denied wholesale with 48 individual allow lines is configuration we would be carrying without having reasoned about it, and `warnings = "deny"` turns every `todo!()` into a build failure mid-sprint.
