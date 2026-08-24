# lez-bonding-curve-launchpad

Proof of concept for [Logos RFP-015](https://github.com/logos-co/rfp/blob/master/RFPs/RFP-015-bonding-curve-launchpad.md): a bonding curve token launchpad on the Logos Execution Zone. Our proposal is [logos-co/rfp#118](https://github.com/logos-co/rfp/issues/118).

Read `CONTEXT.md` for the vocabulary and the crate map, and `docs/adr/` for why things are the way they are. The reviewer-facing writeup, the requirement mapping and the honest list of what is not covered arrive with GTM-519.

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

## Build

```bash
cargo install --git https://github.com/logos-co/scaffold --tag v0.3.0 --locked --bins
lgs build              # host workspace, then the risc0 guest crate
cargo test --workspace
./verify/check-pins.sh # the LEZ pin is duplicated across two workspaces
```

Nix is not required. Scaffold needs it only for `lgs basecamp`, which is out of scope here.

`lgs doctor` reports three "differs from scaffold default" warnings and one about spel not vendoring LEZ v0.1.2. Those are expected: doctor compares against scaffold's default pin rather than the configured one. See `docs/adr/0001`.

## Licence

Dual licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. This is the licensing the proposal promises.
