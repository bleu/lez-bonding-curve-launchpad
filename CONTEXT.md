# Context

The ubiquitous language for this repo, and the map of which crate owns what. Use these terms in code, tests, issues, and commits. Design decisions live in `docs/adr/`.

## Glossary

The curve program is a neutral bounded AMM. A token launch is a factory policy layered on top of it.

- Pool — one ordered token pair with real reserves, virtual reserves, an owner, and an optional close timestamp.
- Token0 / token1 — ordered token roles selected by the pool creator. The factory always supplies the newly minted launch token as token0 and the paired asset as token1.
- Real reserve — tokens actually held by the pool's ATAs and available as swap output. A swap fails when its requested output is not backed by the matching real reserve.
- Virtual reserve — pricing-only state. `virtual_reserve0` and `virtual_reserve1` define the constant-product price and move with swaps.
- `k` — the immutable product of both virtual reserves at pool creation. It is a lower bound; quotes use the current reserve product so rounding surplus cannot be consumed later.
- Exact-input swap — spends gross `amountIn` and requires at least `minAmountOut`.
- Exact-output swap — receives `amountOut` and spends at most fee-inclusive `maxAmountIn`.
- `tokenIn` — the input token definition ID. Token0 input yields token1; token1 input yields token0.
- Pool fee — the input-token fee portion retained in the matching real and virtual pool reserves.
- Protocol fee — the input-token fee portion routed to the treasury ATA for that same token.
- Owner — the public account allowed to close and withdraw. A factory-created pool stores a factory-owned PDA; a direct pool stores its caller-selected owner.
- Close timestamp — optional trusted LEZ chain time at which swaps stop. Expiry is logical closure and needs no separate close transaction.
- Manual close — the owner ending swaps before withdrawal.
- Withdrawal — owner-only transfer of both complete remaining real reserves after manual close or expiry. It permanently retires the pool.
- Treasury — the configured owner of protocol-fee ATAs.
- Admin authority — the key allowed to update both fee rates and the treasury address.
- Config — the singleton PDA holding pool and protocol fee rates, treasury owner, and admin key. Created and replaced whole by `update_config`; read live by every swap. See `docs/adr/0003` and `docs/adr/0005`.
- Genesis admin — the admin key compiled into `curve-core`, gating only the config's first write. It is part of the risc0 image ID.
- ATA — an associated token account derived from an owner and token definition. Pool reserves are ATAs owned by the pool PDA.
- Factory — the launch adapter. It mints a fixed supply, owns launch vocabulary and allocation policy, retains any DEX-seed allocation, and deposits only tradeable amounts into the pool.
- DEX-seed allocation — launch tokens retained by the factory for later DEX seeding. It is not pool state.
- Curve — the neutral bounded-AMM program and the RFP deliverable.

Launch-facing SDKs may say purchase, redemption, token-for-sale, collateral, creator, and DEX-seed allocation. Those terms must not cross into the pool program, state, PDA, or pricing APIs.

## Crate map

- `crates/curve-math` — direction-neutral pricing arithmetic as pure checked integer functions. Empty dependency list.
- `crates/pool` — ordered pool state machine. Applies dual-fee exact-input/output swaps, expiry, close, and full withdrawal. Its randomized solvency suite needs no account fixtures.
- `crates/curve-core` — where `lee_core` enters. Owns the neutral wire enum, Borsh account state, owner-scoped pool PDA, ATA validation and custody calls, authorization, and adapter handlers.
- `crates/launchpad-client` — the SDK boundary. Pool lifecycle operations are neutral; factory adapters may expose launch terminology.
- `cli/` — the `launchpad` binary. Parses arguments and calls `launchpad-client`.
- `methods/guest/src/bin/*.rs` — one risc0 guest per file, each a dispatch shim over its matching core crate. Excluded from the root workspace so the guest keeps its own release profile and program identity.
- `verify/` — reviewer-facing verification.

The factory is not present in this repository yet. When added, it owns creator privacy, launch allocation, unlock policy, and DEX-seed accounting; the pool must not grow those fields.
