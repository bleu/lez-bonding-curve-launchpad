# 0001 — Pin LEZ v0.2.0, spel v0.6.0, circuits 0.5.3

Status: accepted

## Context

`lgs setup` builds spel unconditionally, `lgs doctor` checks that spel and LEZ agree, and `spel-cli/Cargo.toml` vendors LEZ by tag. So the LEZ pin and the spel pin are not independent choices.

spel v0.6.0 vendors LEZ v0.2.0. spel v0.5.0 vendors LEZ v0.1.2. No spel release vendors LEZ v0.2.4, which was the earlier plan. That leaves exactly two aligned pairs: v0.1.2 with spel v0.5.0, or v0.2.0 with spel v0.6.0.

Circuits move with the LEZ pin. LEZ v0.2.0 pins circuits rev `2846ee7a`, whose commit message is "auto-update Nix hashes for v0.5.3". Scaffold's default of 0.4.1 pairs with v0.1.2. A mismatch here does not fail loudly; it produces incompatible verifier keys.

## Decision

Pin LEZ to `a58fbce2ff48c58b7bb5001b1a27e64b9596ee3a` (v0.2.0), spel to `0cb7e0980535af619482cf1c823f4d394b3ebd61` (v0.6.0), and circuits to `0.5.3`.

v0.2.0 over v0.1.2 because the `nssa` to `lee` rename happened there, so the in-tree AMM conventions the PRD points at still apply to what we write.

Pin `lgs` itself to v0.3.0 via `cargo install --git https://github.com/logos-co/scaffold --tag v0.3.0 --locked --bins`. `eth-lez-atomic-swaps` pins an exact commit and warns that other builds fail in confusing ways, but their pin is from July and predates several fixes. v0.3.0 is one CI-only commit behind master.

Nix is not required. Scaffold needs it only for `lgs basecamp`, and the mini-app is out of scope.

## Consequences

`lgs doctor` reports three "differs from scaffold default" warnings, and warns that spel does not vendor LEZ v0.1.2. That check compares against scaffold's default pin rather than the configured one, so the warning is a limitation in doctor, not a real mismatch. Three warnings are the expected clean state here.

The revision string is duplicated across two workspaces with two lockfiles, and cargo cannot factor it out. Drift fails silently, so `verify/check-pins.sh` asserts the manifests and `scaffold.toml` agree. See `docs/adr/0002`.

`[localnet] risc0_dev_mode = true` stays set for the week, because proving time on a laptop is unmeasured and an unusable demo is worse than a disclosed one. This carries an obligation: if dev mode is used for the recording or the walkthrough, say so plainly rather than letting a viewer assume real proofs. GTM-519 and GTM-520 own that disclosure.

Scaffold's own templates do not compile on a v0.2.0 pin. Both `templates/default` and `templates/lez-framework` declare `nssa_core` with no `package = "lee_core"`, and at v0.2.0 the package is named `lee_core`. Depend on `lee_core` directly, which also matches the in-tree AMM.
