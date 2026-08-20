# lez-bonding-curve-launchpad

Proof of concept for [Logos RFP-015](https://github.com/logos-co/rfp/blob/master/RFPs/RFP-015-bonding-curve-launchpad.md): a bonding curve token launchpad on the Logos Execution Zone. Our proposal is [logos-co/rfp#118](https://github.com/logos-co/rfp/issues/118).

Read `CONTEXT.md` for the vocabulary and the crate map, and `docs/adr/` for why things are the way they are. The reviewer-facing writeup, the requirement mapping and the honest list of what is not covered arrive with GTM-519.

## Layout

Two programs. The curve runs a sale over any token pair handed to it, and is the RFP deliverable. The factory mints a token with a fixed supply, publishes the split, and tail-calls into the curve to open the sale.

Handlers live in the host workspace under `crates/`, and each risc0 guest in `methods/guest/src/bin/` is a dispatch shim over its core crate. `docs/adr/0002` explains why that differs from the in-tree programs.

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
