# Context

The ubiquitous language for this repo, and the map of which crate owns what. Use these terms in code, tests, issues, and commits. Design decisions live in `docs/adr/`.

## Glossary

The curve program is a neutral bounded AMM. A token launch is a factory policy layered on top of it.

- Namespace — an independent launchpad identity with its own admin, protocol fee rate, and treasury. Its identity stays fixed when administration changes.
- Pool — belongs immutably to one namespace; one ordered token pair with real reserves, virtual reserves, an owner, and an optional close timestamp.
- Token0 / token1 — ordered token roles selected by the pool creator. The factory always supplies the newly minted launch token as token0 and the paired asset as token1.
- Real reserve — tokens actually held by the pool's ATAs and available as swap output. A swap fails when its requested output is not backed by the matching real reserve.
- Virtual reserve — pricing-only state. `virtual_reserve0` and `virtual_reserve1` define the constant-product price and move with swaps.
- `k` — the immutable product of both virtual reserves at pool creation, used for all subsequent pricing.
- Exact-input swap — spends gross `amountIn` and requires at least `minAmountOut`.
- Exact-output swap — receives `amountOut` and spends at most fee-inclusive `maxAmountIn`.
- `tokenIn` — the input token definition ID. Token0 input yields token1; token1 input yields token0.
- Protocol fee — charged in token1 (collateral for factory launches), deducted from buy input or raw sell output and accrued separately from reserves in the pool vault.
- Accrued fees — uncollected protocol fees; they never back swap output or creator proceeds.
- Fee collection — permissionless transfer of accrued fees to the current namespace treasury, available at any time and included atomically in reserve withdrawal.
- Collected fees — cumulative protocol fees already paid to treasury, reported separately from reserves and uncollected fees.
- Authority NFT — a unique transferable NFT master granting a role. Its token definition is the stable role identity; its current holding account proves control.
- Owner — the authority allowed to close and withdraw. A direct pool uses an authority NFT; a factory-created pool remains under factory program custody.
- Close timestamp — optional trusted LEZ chain time at which swaps stop. Expiry is logical closure and needs no separate close transaction.
- Manual close — the owner ending swaps before withdrawal.
- Withdrawal — owner-only transfer of both complete remaining real reserves after manual close or expiry. It permanently retires the pool.
- Treasury — the configured owner of protocol-fee ATAs.
- Admin authority — the NFT granting permission to update a namespace fee and treasury, replace its authority, or renounce it permanently. Transferring the NFT transfers control.
- Creator authority — the NFT granting close, allocation-claim, and proceeds-withdrawal rights for a factory launch. Rights follow its current holder.
- Config — the settings of one namespace, shared by its pools and read live at execution.
- ATA — an associated token account derived from an owner and token definition. Pool reserves are ATAs owned by the pool PDA.
- Factory — the launch adapter. It mints a fixed supply, owns launch vocabulary and allocation policy, retains any DEX-seed allocation, and deposits only tradeable amounts into the pool.
- DEX-seed allocation — launch tokens retained by the factory for later DEX seeding. It is not pool state.
- Curve — the neutral bounded-AMM program and the RFP deliverable.

Launch-facing SDKs may say purchase, redemption, token-for-sale, collateral, creator, and DEX-seed allocation. Those terms must not cross into the pool program, state, PDA, or pricing APIs.

## Crate map

- `crates/curve-math` — direction-neutral pricing arithmetic as pure checked integer functions. Empty dependency list.
- `crates/pool` — ordered pool state machine. Applies collateral-fee exact-input/output swaps, expiry, close, and full withdrawal. Its randomized solvency suite needs no account fixtures.
- `crates/curve-core` — where `lee_core` enters. Owns the neutral wire enum, Borsh account state, namespace/owner-scoped pool PDA, ATA validation and custody calls, authorization, and adapter handlers.
- `crates/factory-core` — launch adapter. Mints fixed supply, owns launch allocation and DEX-seed policy, stores creator commitments and post-close claiming, and tail-calls the curve's neutral pool interface.
- `crates/launchpad-client` — the SDK boundary. Pool lifecycle operations are neutral; factory adapters may expose launch terminology.
- `cli/` — the `launchpad` binary. Parses arguments and calls `launchpad-client`.
- `methods/guest/src/bin/*.rs` — one risc0 guest per file, each a dispatch shim over its matching core crate. Excluded from the root workspace so the guest keeps its own release profile and program identity.
- `verify/` — reviewer-facing verification.

The factory owns creator commitments, launch allocation, post-close creator claims, and DEX-seed accounting; the pool must not grow those fields.

Factory creation and proceeds settlement persist resumable stages (ADR 0008). Pending pools reject trading and lifecycle actions until funded. Each stage rechecks the creator NFT and commits atomically with its child calls; the whole workflow spans transactions.
