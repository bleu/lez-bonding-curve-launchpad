# lez-bonding-curve-launchpad

A one-week proof of concept for [Logos RFP-015](https://github.com/logos-co/rfp/blob/master/RFPs/RFP-015-bonding-curve-launchpad.md): a two-way, constant-product bonding-curve launchpad on the Logos Execution Zone. It is not RFP-015 milestone M1, a testnet deployment, or a production launchpad. Our proposal is [logos-co/rfp#118](https://github.com/logos-co/rfp/issues/118).

The deliverable is a deployable curve program, a factory that mints a fixed-supply launch token and applies launch policy, an SDK boundary, a CLI, and a reviewer walkthrough. The curve remains a neutral bounded AMM; the factory owns token-launch supply and settlement policy. Read [CONTEXT.md](CONTEXT.md) for the vocabulary and crate map, and [docs/adr](docs/adr) for the design decisions.

## Run the reviewer walkthrough

Prerequisites are Rust (the pinned toolchain in `rust-toolchain.toml`), `jq`, and scaffold `lgs` v0.3.0:

```bash
cargo install --git https://github.com/logos-co/scaffold --tag v0.3.0 --locked --bins
```

Before a real walkthrough, replace [`GENESIS_ADMIN`](crates/curve-core/src/lib.rs) with a public wallet account you control and rebuild: the key is compiled into the curve image. Create or identify that account with `lgs wallet -- account new public`, then run the canonical walkthrough from a clean checkout:

```bash
GENESIS_ADMIN_ACCOUNT=Public/<configured-genesis-admin-account> ./verify/e2e.sh
```

The script resets only this project's managed localnet and wallet, builds and deploys both guests, configures the curve, creates a launch, exercises rejected and successful buys plus a sell, exhausts the sale reserve, then checks auto-close, creator unlock, and withdrawal. It stops the localnet it started. It refuses to touch a foreign listener.

> **Important — development proving mode.** [`scaffold.toml`](scaffold.toml) sets `risc0_dev_mode = true`. This walkthrough demonstrates deployed-program integration and state transitions on a sequencer; it does **not** measure, demonstrate, or make a production-security claim about real ZK proving.

For smaller checks or troubleshooting:

```bash
cargo test --workspace
./verify/tests/e2e.sh    # validates walkthrough control flow with mocked commands
./verify/check-pins.sh   # checks the duplicated LEZ/scaffold pins
lgs build                # builds host workspace and RISC0 guest binaries
```

Nix is not required. Scaffold needs it only for `lgs basecamp`, which is outside this PoC.

## Solvency and supply boundary

### Arithmetic and reserves

At creation, [`Pool::create`](crates/pool/src/lib.rs) rejects zero virtual reserves and either virtual reserve at or above `2^64`. Thus the immutable creation-time `k = V0 × V1` is strictly below `2^128` and fits `u128`. That initial bound is not an assumption about future state: every current and proposed virtual-reserve product, plus all reserve, fee, and quote arithmetic, is checked. A trade that would overflow or leave its real output reserve rejects before state mutation. The complete argument is in [ADR 0004](docs/adr/0004-u128-bounds-for-the-curve-arithmetic.md).

Quotes use the **current** virtual-reserve product, while stored `k` remains the immutable lower bound. Exact-input output uses ceiling division internally so the payout rounds down; exact-output pricing rounds required input up. The combined input fee rounds up, is split deterministically, and the retained pool portion stays in the matching real and virtual reserve; the protocol portion is sent to the configured treasury ATA. These directions favour the pool, and each successful trade preserves `current_product >= k`. See [`curve-math`](crates/curve-math/src/lib.rs), [`pool`](crates/pool/src/lib.rs), and [ADR 0005](docs/adr/0005-dual-input-fees-and-monotonic-reserve-product.md).

The executable property suite in [`crates/pool/tests/proptest_invariants.rs`](crates/pool/tests/proptest_invariants.rs) generates 512 randomized sequences of up to 128 exact-input/exact-output swaps, close attempts, and withdrawals across both token directions, boundary amounts, and valid/invalid fee combinations. It asserts successful swaps conserve the modeled real reserves, never pay beyond the selected real output reserve, keep the virtual product non-decreasing and at least `k`, and retain immutable `k`; rejected actions leave state unchanged. It is a pure state-machine test—not a proof of LEZ account/ATA wiring, concurrent sequencer execution, private-flow behavior, or a mathematical proof over all inputs. Adapter tests in [`curve-core`](crates/curve-core/src/tests.rs) cover the account, authorization, and custody boundary.

### What “sold back never exceeds bought” means here

The neutral curve guarantees only that no swap pays more than its real output reserve. It cannot make a supply claim about a token definition supplied by an unrelated direct pool creator: that creator could deposit more token0 later through a different construction path.

The factory provides the launch-level boundary. [`create_factory_pool`](crates/factory-core/src/lib.rs) computes one checked fixed supply at genesis—sale reserve, DEX-seed reserve, and creator allocation—and has no subsequent mint or metadata-update instruction. It deposits only the tradeable sale reserve into the pool, retains the DEX seed allocation outside pool state, and auto-closes the factory pool when token0 is depleted. Consequently, within a factory launch, tokens returned to the curve are bounded by tokens that came from its fixed issuance; the curve's real-reserve check independently bounds collateral redemption. This claim does not apply to arbitrary direct pools or to an off-chain/other-program token mint.

## Requirement mapping

Status is evidence-aware: **implemented and demonstrated** means the canonical localnet walkthrough exercises deployed programs; **implemented but not demonstrated** means code and focused tests exist but that exact behavior is not a walkthrough checkpoint; **seam/future integration** identifies a deliberate extension point; **not covered** is outside this PoC.

| RFP-015 requirement | Implementation location | Verification evidence | Status |
| --- | --- | --- | --- |
| F1: deterministic two-way curve, integer pricing, reserve backing, inverse quote | `crates/curve-math`, `crates/pool`, `crates/launchpad-client` | math/unit/property tests; `verify/e2e.sh` buys and sells a deployed launch | **implemented and demonstrated** |
| F2: creator-defined `D`, optional `R`, virtual reserves, distinct allocations | `crates/factory-core`, factory `CreateFactoryPool` | factory tests; walkthrough creates `D=1000`, `R=100` and inspects terminal reserve | **implemented and demonstrated** |
| F3: public and deshield→trade→re-shield participation | public CLI/client in `launchpad-client`; privacy boundary in [ADR 0005](docs/adr/0005-private-trade-boundary-and-verification.md) | walkthrough exercises public trades only | **not covered** for the private path |
| F4: automatic close when sale reserve exhausts | factory token0 depletion policy; `Pool::close_if_depleted` | walkthrough buys the remaining reserve and asserts `closed` | **implemented and demonstrated** |
| F5: post-close collateral and `R` settlement | `WithdrawFactoryProceeds` in `factory-core` | walkthrough unlocks and withdraws after close | **implemented and demonstrated** |
| F6: buy/sell slippage protection | exact-output/input pool operations and CLI caps/floors | unit tests; walkthrough checks rejected buy slippage | **implemented but not demonstrated** for sell-floor rejection |
| F7: ATA custody | `curve-core` create/swap/lifecycle adapters | adapter tests and deployed walkthrough invoke ATA transfers | **implemented and demonstrated** |
| U01: SDK lifecycle for public and private users | `launchpad-client` | public invocation/quote tests and walkthrough | **implemented but not demonstrated** for broad discovery/position UX; private lifecycle is **not covered** |
| U02, U04–U08: mini-app, confirmation, privacy UX, analytics | — | — | **not covered** |
| U03: essential creator/participant CLI | `cli/src/main.rs` | CLI parsing tests and walkthrough (`configure`, `create-sale`, `price`, `buy`, `sell`, `status`, `unlock`, `withdraw`) | **implemented and demonstrated** |
| U09: SPEL-generated IDL | — | — | **not covered**; this raw LEZ template deliberately has no framework IDL ([ADR 0002](docs/adr/0002-repo-layout-and-guest-shim.md)) |
| U10: actionable rejected-buy errors | CLI JSON error categories and pool errors | walkthrough checks slippage and reserve-overshoot error categories | **implemented and demonstrated** |
| R1–R2: concurrent-safe invariant/accounting and atomic failed buy | checked state transitions; curve account adapters | property suite checks rejected transition atomicity; adapter tests; walkthrough rejection checkpoints | **implemented but not demonstrated** for adversarial concurrent submissions |
| R3: atomic auto-close, no later buy | factory closure policy | walkthrough exhausts reserve and asserts closure | **implemented and demonstrated** |
| P1–P2: one-transaction buy/close | chained-call adapters | deployed walkthrough submits individual lifecycle operations | **implemented and demonstrated** as integration behavior, not performance measurement |
| P3: documented CU costs and testnet version | — | — | **not covered** |
| S1, S6, S7: testnet/mainnet deployments and milestone plan | — | — | **not covered**; this is a one-week PoC |
| S2: sequencer E2E in CI | `verify/e2e.sh`, `verify/tests/e2e.sh` | mocked control-flow test exists; current CI runs unit tests and pin checks, not a live sequencer | **implemented but not demonstrated** in CI |
| S3: test per hard requirement | tests across crates and `verify/` | mapping above identifies unimplemented/private/UI/performance gaps | **not covered** as a complete RFP claim |
| S4: README deployment and end-to-end use | this README and `verify/e2e.sh` | canonical command above | **implemented and demonstrated** for CLI/localnet; no mini-app |
| S5 and Privacy requirements: full private-flow guarantees and privacy document | [ADR 0005](docs/adr/0005-private-trade-boundary-and-verification.md) | documents the boundary and acceptance criteria only | **not covered** |

### Platform-dependency evidence

RFP-015 still names [LP-0013 token authorities](https://github.com/logos-co/lambda-prize/blob/master/prizes/LP-0013.md) as an open hard blocker. Code structure is not evidence that the runtime authority primitive is compatible. The canonical walkthrough is designed to provide practical evidence: it deploys these programs and exercises custody transfers on a LEZ sequencer. Until that walkthrough has succeeded against the intended sequencer/pin, LP-0013 compatibility remains an open blocker.

General cross-program calls are exercised by the factory/curve flow. The RFP-001 admin-authority library is not built here: [`Config`](crates/curve-core/src/lib.rs) is the seam, and rotating its stored admin to an RFP-001-controlled key needs no redeploy ([ADR 0003](docs/adr/0003-admin-config-and-the-rfp-001-seam.md)). RFP-004 is likewise not integrated; automatic DEX graduation is future work, and the factory currently settles DEX-seed tokens under its post-close policy ([ADR 0007](docs/adr/0007-factory-closure-and-creator-settlement.md)).

## Scope limits

- This PoC has no mini-app, generated IDL, testnet/mainnet deployment, CU measurements, or automatic DEX graduation.
- Private trades are a documented SDK responsibility and acceptance target, not a demonstrated feature. The privacy goal is anonymous participation—not confidential trade amounts: reserves, pricing, fees, and public state transitions remain visible.
- The production RFP's collateral-only protocol-fee rule is not the policy implemented by this neutral AMM: it supports dual input-token fees. This is an intentional PoC interface decision documented in ADR 0005 and must be reconciled before claiming full RFP conformance.
- Development proving mode means this repository must not be used as evidence of production proof performance or production security.

## Privacy boundary

The first privacy goal is **anonymous participation**, not confidential market
activity. A participant may fund a trade from a private account, but the sale,
its reserves, price movement, fees, and token-account changes are public. Those
public changes can reveal or constrain a trade's effective size. The program
does not and must not claim otherwise.

The SDK owns the private trade lifecycle: it assembles the guest binaries needed
to prove the main call and every chained call, submits the private transaction,
syncs the private account, and re-shields received assets where applicable. The
program cannot enforce that last step. The implementation and acceptance checks
for that lifecycle are recorded in [ADR 0005](docs/adr/0005-private-trade-boundary-and-verification.md).

## Layout

Two program layers. The curve is a neutral bounded AMM over an ordered token pair and is the RFP deliverable. The factory is the launch adapter: it mints a fixed supply, owns launch allocation policy, retains any DEX-seed allocation, and creates a pool with only the amounts intended for trading. Direct pool creation remains supported.

The pool wire interface is intentionally small and breaking for this PoC: `CreatePool`, `SwapExactInput`, `SwapExactOutput`, `ClosePool`, and `WithdrawReserves`. Swaps work in either direction; `tokenIn` selects the input definition. Optional expiry uses trusted LEZ chain time, and expiry itself is sufficient to permit an owner-authorized full withdrawal.

## Supply boundary

A generic pool limits every payout to its real output reserve, but it cannot prevent
an independently supplied token0 from being swapped in for token1. The launch-level
fixed-supply claim instead depends on the factory's one-time mint and allocation
policy; it is not enforced by the pool.

Handlers live in the host workspace under `crates/`, and each risc0 guest in `methods/guest/src/bin/` is a dispatch shim over its core crate. `docs/adr/0002` explains why that differs from the in-tree programs.

## Admin authority

The pool and protocol fee rates and the treasury owner live in a singleton config PDA, read live by every swap. One instruction manages it: `update_config` creates the config on the first call and replaces it whole after, gated on the admin key. The first call must be signed by the genesis admin, a constant compiled into `curve-core`. Replace it with your key before the deploy build; it is part of the risc0 image ID, so changing it produces a different program. Deployment then has one required step: call `update_config` once before any swap, because swaps fail while the config does not exist.

RFP-015 sources the admin authority from the RFP-001 library, which this proof of concept does not build. The seam it plugs into is the admin field itself: rotate the admin to the key that library controls and it holds the gate from then on, with no code change and no redeploy. Rotation is single-step, so a rotation to a wrong key is unrecoverable — check the key before you sign.

## Ownership and privacy boundary

Pool ownership is deliberately public. `create_pool` stores its authorized owner in pool state and scopes the pool PDA by the ordered token pair and owner. Direct creators may own pools themselves; the factory path supplies a factory-owned PDA so close and withdrawal must pass through factory policy. Creation verifies the owner's source ATAs, creates both pool-owned reserve ATAs, and atomically transfers both initial real reserves.

Creator identity and privacy are launch policy, not AMM state. The future factory may commit to a private creator authority while exposing only its own owner PDA to the neutral pool.

`lgs doctor` reports three "differs from scaffold default" warnings and one about spel not vendoring LEZ v0.1.2. Those are expected: doctor compares against scaffold's default pin rather than the configured one. See `docs/adr/0001`.

## Licence

Dual licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. This is the licensing the proposal promises.
