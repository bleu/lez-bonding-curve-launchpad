# lez-bonding-curve-launchpad

A one-week proof of concept for [Logos RFP-015](https://github.com/logos-co/rfp/blob/master/RFPs/RFP-015-bonding-curve-launchpad.md): a two-way, constant-product bonding-curve launchpad on the Logos Execution Zone. It is not RFP-015 milestone M1, a testnet deployment, or a production launchpad. Our proposal is [logos-co/rfp#118](https://github.com/logos-co/rfp/issues/118).

The deliverable is a deployable curve program, a factory that mints a fixed-supply launch token and applies launch policy, an SDK boundary, a CLI, and a reviewer walkthrough. The curve remains a neutral bounded AMM; the factory owns token-launch supply and settlement policy. Read [CONTEXT.md](CONTEXT.md) for the vocabulary and crate map, and [docs/adr](docs/adr) for the design decisions.

## Agent skills

The canonical agent skills are tracked only in `.agents/skills`. To enable native
discovery in Claude Code and Cursor, run:

```bash
./verify/sync-agent-skills.sh
```

Run the same command again after `lgs init` if it regenerates either native
discovery directory.

## Run the reviewer walkthrough

Prerequisites are Rust (the pinned toolchain in `rust-toolchain.toml`), `jq`, and scaffold `lgs` v0.3.0:

```bash
cargo install --git https://github.com/logos-co/scaffold --tag v0.3.0 --locked --bins
```

Before a walkthrough, create or identify a public wallet account with `lgs wallet -- account new public`. That account authorizes namespace creation; no key is compiled into the program. Run:

```bash
NAMESPACE_ADMIN_ACCOUNT=Public/<namespace-creator-account> ./verify/e2e.sh
```

The script resets only this project's managed localnet and wallet, builds and deploys both guests, configures the curve, creates a launch, exercises rejected and successful buys plus a sell, exhausts the sale reserve, then checks auto-close, creator unlock, and withdrawal. It stops the localnet it started and refuses to touch a foreign listener. It is a manual integration harness, not evidence used by this PoC review.

> **Important — development proving mode.** [`scaffold.toml`](scaffold.toml) sets `risc0_dev_mode = true`. This walkthrough demonstrates deployed-program integration and state transitions on a sequencer; it does **not** measure, demonstrate, or make a production-security claim about real ZK proving.

For smaller checks or troubleshooting:

```bash
cargo test --workspace
./verify/tests/e2e.sh    # validates walkthrough control flow with mocked commands
./verify/check-pins.sh   # checks the duplicated LEZ/scaffold pins
./verify/check-idl.sh    # proves checked-in SPEL IDLs match their source declarations
lgs build                # builds host workspace and RISC0 guest binaries
```

With this excluded `methods/` workspace, scaffold currently writes guest artifacts beneath
`methods/target/riscv-guest/…/release/`; a custom build may instead use
`target/riscv-guest/…/release/`. The walkthrough accepts either layout and resolves both
program paths itself.

Nix is not required. Scaffold needs it only for `lgs basecamp`, which is outside this PoC.

## Solvency and supply boundary

### Arithmetic and reserves

At creation, [`Pool::create`](crates/pool/src/lib.rs) rejects zero virtual reserves and either virtual reserve at or above `2^64`. Thus the immutable creation-time `k = V0 × V1` is strictly below `2^128` and fits `u128`. Trades use that stored `k` for every quote, while reserve additions/subtractions and all quote arithmetic remain checked. A trade that would overflow or leave its real output reserve rejects before state mutation. The complete argument is in [ADR 0004](docs/adr/0004-u128-bounds-for-the-curve-arithmetic.md).

Quotes use the immutable creation-time `k`, as RFP-015 specifies. Exact-input output uses ceiling division internally so the payout rounds down; exact-output pricing rounds required input up. The only fee is the protocol fee, rounded up and always settled in collateral: it is deducted from collateral input on buys and from raw collateral output on sells. The pool receives no retained fee. See [`curve-math`](crates/curve-math/src/lib.rs), [`pool`](crates/pool/src/lib.rs), and [ADR 0005](docs/adr/0005-dual-input-fees-and-monotonic-reserve-product.md).

The executable property suite in [`crates/pool/tests/proptest_invariants.rs`](crates/pool/tests/proptest_invariants.rs) generates 512 randomized sequences of up to 128 exact-input/exact-output swaps, close attempts, and withdrawals across both token directions, boundary amounts, and valid/invalid fee combinations. It asserts successful swaps conserve the modeled real reserves, never pay beyond the selected real output reserve, and retain the immutable pricing `k`; rejected actions leave state unchanged. It is a pure state-machine test—not a proof of LEZ account/ATA wiring, concurrent sequencer execution, private-flow behavior, or a mathematical proof over all inputs. Adapter tests in [`curve-core`](crates/curve-core/src/tests.rs) cover the account, authorization, and custody boundary.

### What “sold back never exceeds bought” means here

The neutral curve guarantees only that no swap pays more than its real output reserve. It cannot make a supply claim about a token definition supplied by an unrelated direct pool creator: that creator could deposit more token0 later through a different construction path.

The factory provides the launch-level boundary. [`create_factory_pool`](crates/factory-core/src/lib.rs) computes one checked fixed supply at genesis—sale reserve, DEX-seed reserve, and creator allocation—and has no subsequent mint or metadata-update instruction. It deposits only the tradeable sale reserve into the pool, retains the DEX seed allocation outside pool state, and auto-closes the factory pool when token0 is depleted. Consequently, within a factory launch, tokens returned to the curve are bounded by tokens that came from its fixed issuance; the curve's real-reserve check independently bounds collateral redemption. This claim does not apply to arbitrary direct pools or to an off-chain/other-program token mint.

## Requirement mapping

Status is evidence-aware and deliberately excludes mini-app and live-deployment
evidence for this PoC review. **implemented and test-covered** means focused unit,
property, adapter, or mocked-harness tests exercise the behavior; **implemented but
not test-covered** means code exists without a focused check; **seam/future
integration** identifies a deliberate extension point; **not covered** is outside
this PoC or explicitly deferred.

| RFP-015 requirement | Implementation location | Verification evidence | Status |
| --- | --- | --- | --- |
| F1: deterministic two-way curve, integer pricing, reserve backing, inverse quote | `crates/curve-math`, `crates/pool`, `crates/curve-core`, `crates/launchpad-client` | math, buy/sell collateral-fee unit tests, dispatcher settlement tests, quote tests, and 512-case state-machine property tests | **implemented and test-covered** |
| F2: immutable namespace, creator-defined `D`, optional `R`, virtual reserves, distinct allocations | `crates/factory-core`, factory `CreateFactoryPool` | factory tests cover fixed supply and reject `Vt <= D` | **implemented and test-covered** |
| F3: public and deshield→trade→re-shield participation | `private-flow-core`, `methods/guest/src/bin/private_buy.rs`, `launchpad-client`, CLI | guest compile check; client/CLI validation tests; router chains native funding, collateral deshield, buy, and re-shield in one private transaction | **implemented and statically test-covered**; no live sequencer evidence claimed |
| F4: automatic close when sale reserve exhausts | factory token0 depletion policy; `Pool::close_if_depleted` | pool/factory lifecycle tests | **implemented and test-covered** |
| F5: post-close collateral and `R` settlement | `WithdrawFactoryProceeds` in `factory-core` | factory chained-call tests | **implemented and test-covered** |
| F6: buy/sell slippage protection | exact-output/input pool operations and CLI caps/floors | unit, client, and CLI parsing tests | **implemented and test-covered** |
| F7: ATA custody | `curve-core` create/swap/lifecycle adapters | adapter tests verify ATA derivation and settlement accounts | **implemented and test-covered** |
| F8–F10: live collateral fees, permissionless namespaces, isolated accounts | `curve-core`, `factory-core` | namespace/admin/fee-update and mismatched-account tests | **implemented and test-covered** at the host adapter boundary |
| U11–U12: namespace selection, creation, admin transfer/renunciation, fee/treasury display | `launchpad-client`, CLI | client/CLI and core tests | **implemented** in SDK/CLI; mini-app, listings, history and analytics remain outside this PoC |
| U01: SDK lifecycle for public and private users | `launchpad-client` | public invocation/quote tests; private request validation and router composition compile check | **implemented and test-covered** at the SDK/guest construction boundary |
| U02, U04–U08: mini-app, confirmation, privacy UX, analytics | — | — | **not covered** |
| U03: essential creator/participant CLI | `cli/src/main.rs` | CLI parsing tests for `configure`, `create-sale`, `price`, `buy`, `buy-with-collateral`, `sell`, `status`, `unlock`, and `withdraw`; status reports configured sale quantity, tokens sold, and reserves | **implemented and test-covered** |
| U09: SPEL-generated IDL | `idl-src/`, `idl/`, `verify/check-idl.sh` | project-pinned `spel generate-idl` reproduces all three checked-in JSON interfaces | **implemented and test-covered** |
| U10: actionable rejected-buy errors | CLI JSON error categories and pool errors | CLI and pool error tests | **implemented and test-covered** |
| R1–R2: concurrent-safe invariant/accounting and atomic failed buy | checked state transitions; curve account adapters | property suite checks rejected transition atomicity; adapter tests | **implemented and test-covered** at the state-machine boundary, not under adversarial concurrent submissions |
| R4–R6: exact fee accounting, execution-time updates, namespace isolation | `pool`, `curve-core` | fee conservation, two-namespace isolation, and quote-to-execution slippage tests | **implemented and test-covered** at the host adapter boundary |
| R3: atomic auto-close, no later buy | factory closure policy | pool/factory lifecycle tests | **implemented and test-covered** |
| P1–P2: one-transaction buy/close | chained-call adapters | adapter/factory tests inspect chained calls | **implemented and test-covered** as construction behavior, not performance measurement |
| P3: documented CU costs and testnet version | — | — | **not covered** |
| S1, S6, S7: testnet/mainnet deployments and milestone plan | — | — | **not covered**; this is a one-week PoC |
| S2: sequencer E2E in CI | `verify/e2e.sh`, `verify/tests/e2e.sh` | mocked control-flow test exists; current CI runs unit tests and pin checks, not a live sequencer | **implemented but not demonstrated** in CI |
| S3: test per hard requirement | tests across crates and `verify/` | mapping above identifies unimplemented/private/UI/performance gaps | **not covered** as a complete RFP claim |
| S4: README deployment and end-to-end use | this README and `verify/e2e.sh` | canonical command and mocked harness control-flow test | **implemented and test-covered**; live execution excluded from this review |
| S5 and Privacy requirements: atomic private-flow construction and privacy document | [ADR 0005](docs/adr/0005-private-trade-boundary-and-verification.md), private router | guest compile check plus documented public-observability boundary | **implemented and statically test-covered**; live privacy validation is outside this PoC review |

### Platform-dependency evidence

RFP-015 still names [LP-0013 token authorities](https://github.com/logos-co/lambda-prize/blob/master/prizes/LP-0013.md) as an open hard blocker. Code structure is not evidence that the runtime authority primitive is compatible. The canonical walkthrough is designed to provide practical evidence: it deploys these programs and exercises custody transfers on a LEZ sequencer. Until that walkthrough has succeeded against the intended sequencer/pin, LP-0013 compatibility remains an open blocker.

General cross-program calls are exercised by the factory/curve flow. The RFP-001 admin-authority library is not built here: [`Config`](crates/curve-core/src/lib.rs) is the seam, and rotating its stored admin to an RFP-001-controlled key needs no redeploy ([ADR 0003](docs/adr/0003-admin-config-and-the-rfp-001-seam.md)). RFP-004 is likewise not integrated; automatic DEX graduation is future work, and the factory currently settles DEX-seed tokens under its post-close policy ([ADR 0007](docs/adr/0007-factory-closure-and-creator-settlement.md)).

## Scope limits

- This PoC has no mini-app, testnet/mainnet deployment, CU measurements, or automatic DEX graduation.
- Private buys are constructed as one privacy-preserving transaction, but this review deliberately does not treat a local or live submission as evidence. The privacy goal is anonymous participation—not confidential trade amounts: reserves, pricing, fees, and public state transitions remain visible.
- Development proving mode means this repository must not be used as evidence of production proof performance or production security.

## Privacy boundary

The first privacy goal is **anonymous participation**, not confidential market
activity. A participant may fund a trade from a private account, but the sale,
its reserves, price movement, fees, and token-account changes are public. Those
public changes can reveal or constrain a trade's effective size. The program
does not and must not claim otherwise.

The SDK owns the private-buy lifecycle. It creates a fresh public account, proves
one router transaction with the curve, native-transfer, token, and ATA guest
dependencies, then chains native funding, collateral deshielding, the buy, and
re-shielding the purchased tokens to the caller's private destination. The router
fixes that order; callers cannot submit a partial version through this API. The
implementation and limits are recorded in [ADR 0005](docs/adr/0005-private-trade-boundary-and-verification.md).

## SPEL IDL

The hand-written guests retain their shared Rust instruction enums. Their SPEL
interface declarations live in [`idl-src`](idl-src), and generated JSON is checked
into [`idl`](idl). Regenerate it with `./verify/generate-idl.sh`; CI/review can
verify it has not drifted with `./verify/check-idl.sh`.

## Layout

Two program layers. The curve is a neutral bounded AMM over an ordered token pair and is the RFP deliverable. The factory is the launch adapter: it mints a fixed supply, owns launch allocation policy, retains any DEX-seed allocation, and creates a pool with only the amounts intended for trading. Direct pool creation remains supported.

The pool wire interface is intentionally small and breaking for this PoC: `CreatePool`, `SwapExactInput`, `SwapExactOutput`, `ClosePool`, and `WithdrawReserves`. Swaps work in either direction; `tokenIn` selects the input definition. Optional expiry uses trusted LEZ chain time, and expiry itself is sufficient to permit an owner-authorized full withdrawal.

## Supply boundary

A generic pool limits every payout to its real output reserve, but it cannot prevent
an independently supplied token0 from being swapped in for token1. The launch-level
fixed-supply claim instead depends on the factory's one-time mint and allocation
policy; it is not enforced by the pool.

Handlers live in the host workspace under `crates/`, and each risc0 guest in `methods/guest/src/bin/` is a dispatch shim over its core crate. `docs/adr/0002` explains why that differs from the in-tree programs.

## Namespace administration

This PoC targets the namespace requirements proposed in [Logos PR #204](https://github.com/logos-co/rfp/pull/204). One deployment hosts independent configurations. The namespace identity is the public key that signs its first `configure` call. An operator can create further identities and assign the same admin to them. The identity stays fixed after admin transfer or renunciation; it is not a deployment-wide authority.

Every command requires `--namespace Public/<identity>`. The SDK takes the namespace explicitly, including the private-buy builder. Sale salts are local to a namespace. Namespace selection scopes factory, token, escrow, pool, and reserve addresses. The curve resolves swap fees and treasury from the pool's stored namespace and rejects mismatched configurations or reserve accounts.

```bash
# Create a namespace. Its identity must authorize this first call.
launchpad --namespace Public/<identity> configure \
  --curve-program-path <curve.bin> --admin Public/<identity> \
  --protocol-fee-bps 100 --treasury Public/<treasury>

# Change settings or transfer administration using the current admin.
launchpad --namespace Public/<identity> configure \
  --curve-program-path <curve.bin> --admin Public/<current-admin> \
  --new-admin Public/<next-admin> --protocol-fee-bps 50 --treasury Public/<treasury>

launchpad --namespace Public/<identity> namespace-info --curve-program-path <curve.bin>
launchpad --namespace Public/<identity> renounce-admin \
  --curve-program-path <curve.bin> --admin Public/<current-admin>
```

`configure` replaces the fee, treasury, and admin together; omit `--new-admin` to retain the signing admin. Transfer is single-step. Renunciation is permanent: it disables updates and transfers while retaining the last fee and treasury for trading. The RFP-001 library remains a future integration; this PoC implements its admin behavior locally.

Zero fees are accepted. The 10,000-basis-point denominator is an arithmetic boundary, not a commercial tier or policy cap; a buy whose fee consumes its input is rejected. Swaps read the current namespace configuration at execution, and enforce the trader's net-output floor or gross-input cap. `price --sell --tokens <amount>` shows raw collateral, fee, and net proceeds; price/status output includes the namespace, fee rate, and treasury.

This changes the wire interfaces and account derivation. Rebuild and redeploy the PoC and create fresh namespaces and sales; it does not migrate accounts from older program images. [ADR 0008](docs/adr/0008-permissionless-namespaces.md) supersedes the singleton/genesis-admin decisions in ADR 0003.

## Ownership and privacy boundary

Pool ownership is deliberately public. `create_pool` stores its authorized owner in pool state and scopes the pool PDA by namespace, ordered token pair, and owner. Direct creators may own pools themselves; the factory path supplies a factory-owned PDA so close and withdrawal must pass through factory policy. Creation verifies the owner's source ATAs, creates both pool-owned reserve ATAs, and atomically transfers both initial real reserves.

Creator identity and privacy are launch policy, not AMM state. The future factory may commit to a private creator authority while exposing only its own owner PDA to the neutral pool.

`lgs doctor` reports three "differs from scaffold default" warnings and one about spel not vendoring LEZ v0.1.2. Those are expected: doctor compares against scaffold's default pin rather than the configured one. See `docs/adr/0001`.

## Licence

Dual licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. This is the licensing the proposal promises.
