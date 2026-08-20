//! The curve program: instruction enum, Borsh state, PDA derivation, and the handlers.
//!
//! Modelled on `lez/programs/amm/core/src/lib.rs` plus `lez/programs/amm/src/`, with one
//! deliberate difference. Upstream puts the handlers in the guest binary crate; here they
//! live in the host workspace and `methods/guest/src/bin/curve.rs` is a dispatch shim, so
//! the AMM-style tests run under `cargo test --workspace`. See `docs/adr/0002`.
//!
//! The handlers stay shallow. They deserialize accounts, call `sale`, and translate the
//! returned outcome into account post-states and chained calls. No decisions here.
//!
//! One consequence of `Sale` living in the `sale` crate: `impl TryFrom<&Data> for Sale`
//! is blocked by the orphan rule, both types being foreign. Read the state through a free
//! function calling borsh's `try_from_slice` instead of the upstream `TryFrom` impl.
//!
//! Filled in by GTM-508 onward. GTM-509 adds the guest shim and deletes `deploy_probe`.
