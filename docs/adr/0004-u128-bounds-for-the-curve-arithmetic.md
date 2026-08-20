# 0004 — Bound the virtual reserves below 2^64 so the curve arithmetic stays in u128

Status: accepted

## Context

The pricing formulas in RFP-015 are `tokens_out = Vt - k / (Vc + C_in)`, its inverse `C_in = k / (Vt - Q) - Vc`, and the sell mirror `C_out = Vc - k / (Vt + tokens_in)`. Constant-product implementations usually reach for 256-bit intermediates, because the product of two full-width reserves does not fit the word.

## Decision

`Sale::create` rejects any `Vt` or `Vc` at or above 2^64 (`VIRTUAL_RESERVE_BOUND` in `crates/sale`). That bound is the whole overflow argument. The only multiplication in the system is `k = Vt * Vc`, it runs once at creation, and two factors below 2^64 give a product below 2^128. `k` never changes afterwards, so no later operation recreates the risk: the formulas above only add, subtract, and divide, and a u128 division never exceeds its numerator. `Vc` and `Vt` can drift past 2^64 mid-sale as trades land, and that is safe for the same reason — nothing multiplies two reserves again. The additions can still overflow u128 in theory, so `crates/curve-math` and `crates/sale` deny `clippy::arithmetic_side_effects` and every operation is a named checked call that errors instead of wrapping.

256-bit intermediates would buy generality no real sale uses. 2^64 base units of virtual reserve is about 1.8 x 10^19 — with nine decimals, an eighteen-billion-token virtual reserve. A launch that needs more can split decimals, not the arithmetic.

## Consequences

Every operation in the pricing path is plain u128 mul/div/add/sub. Reviewers check one inequality at creation instead of auditing a wide-integer library, and the guest binary carries no 256-bit code.

The bound lives in `Sale::create`, so state constructed any other way (hand-built in a test, or a corrupted account) is outside the argument. `curve-math` stays total anyway: checked calls return `MathError` rather than trusting the bound.
