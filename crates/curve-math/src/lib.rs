//! Pricing arithmetic for the bonding curve, as pure functions over integers.
//!
//! No accounts, no state, no chain. The job of this crate is to hide the rounding
//! discipline and the overflow bounds behind named functions, so that the rounding
//! direction at every call site is visible at the call rather than inferred from
//! context.
//!
//! Filled in by GTM-508, which also writes the u128 bound argument as an ADR.
