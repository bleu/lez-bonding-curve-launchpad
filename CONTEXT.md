# Context

The ubiquitous language for this repo, and the map of which crate owns what. Use these terms in code, tests, issues, and commits. Design decisions live in `docs/adr/`.

## Glossary

The curve program is a neutral bounded AMM. A token launch is a factory policy layered on top of it.

- Pool — one ordered token pair with real reserves, virtual reserves, an owner, and an optional close timestamp.
- Token0 / token1 — ordered token roles selected by the pool creator. The factory always supplies the newly minted launch token as token0 and the paired asset as token1.
- Real reserve — tokens actually held by the pool and available as swap output. A swap fails when its requested output is not backed by the matching real reserve.
- Virtual reserve — pricing-only state. `virtual_reserve0` and `virtual_reserve1` define the constant product and move with swaps.
- `k` — the constant product fixed from both virtual reserves at pool creation.
- Exact-input swap — spends `amountIn` and requires at least `minAmountOut`.
- Exact-output swap — receives `amountOut` and spends at most fee-inclusive `maxAmountIn`.
- `tokenIn` — the input token definition ID. Token0 input yields token1; token1 input yields token0.
- Protocol fee — charged from whichever token is input and routed to the treasury holding for that same token.
- Owner — the account allowed to close and withdraw. A factory-created pool stores a factory-owned PDA; a direct pool stores its caller-selected owner.
- Close timestamp — optional trusted LEZ chain time at which swaps stop. Expiry is logical closure and needs no separate close transaction.
- Manual close — the owner ending swaps before withdrawal.
- Withdrawal — owner-only transfer of both complete remaining real reserves after manual close or expiry. It permanently retires the pool.
- Treasury — the configured owner of protocol-fee holdings.
- Config — the singleton PDA holding the fee rate, treasury owner, and admin key. Created and replaced whole by `update_config`; read live by every swap. See `docs/adr/0003`.
- Genesis admin — the admin key compiled into `curve-core`, gating only the config's first write.
- ATA — an associated token account derived from an owner and token definition.
- Factory — the launch adapter. It mints a fixed supply, owns launch vocabulary and allocation policy, retains any DEX-seed allocation, and deposits only tradeable amounts into the pool.
- DEX-seed allocation — launch tokens retained by the factory for later DEX seeding. It is not pool state.
- Curve — the neutral bounded-AMM program and the RFP deliverable.

Launch-facing SDKs may say purchase, redemption, token-for-sale, collateral, and DEX-seed allocation. Those terms must not cross into the pool program, state, PDA, or pricing APIs.

## Crate map

- `crates/curve-math` — direction-neutral pricing arithmetic as pure integer functions. Empty dependency list.
- `crates/pool` — ordered pool state machine. Applies exact-input/output swaps, expiry, close, and full withdrawal. Depends only on `curve-math` and `borsh`.
- `crates/curve-core` — where `lee_core` enters. Neutral instruction enum, Borsh account state, ordered pool PDA derivation, authorization, and adapter handlers.
- `crates/launchpad-client` — the SDK boundary. Pool lifecycle operations are neutral; factory adapters may expose launch terminology.
- `cli/` — the `launchpad` binary. Parses arguments and calls `launchpad-client`.
- `methods/guest/src/bin/*.rs` — one risc0 guest per file, each a dispatch shim over its matching core crate.
- `verify/` — reviewer-facing verification.

The factory is not present in this repository yet. When added, it owns launch allocation and DEX-seed accounting; the pool must not grow those fields.
