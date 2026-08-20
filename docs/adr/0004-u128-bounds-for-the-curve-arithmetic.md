# 0004 — Bound the virtual reserves below 2^64 so the curve arithmetic stays in u128

Status: accepted

## Context

The pricing formulas in RFP-015 are `tokens_out = Vt - k / (Vc + C_in)`, its inverse `C_in = k / (Vt - Q) - Vc`, and the sell mirror `C_out = Vc - k / (Vt + tokens_in)`. Constant-product implementations usually reach for 256-bit intermediates, because the product of two full-width reserves does not fit the word.

## Decision

The argument has three parts, citable as "ADR 0004 part N".

### Part 1 — the bound

`Sale::create` rejects any `Vt` or `Vc` at or above 2^64 (`VIRTUAL_RESERVE_BOUND` in `crates/sale`). The only multiplication in the system is `k = Vt * Vc`, it runs once at creation, and two factors below 2^64 give a product below 2^128. `k` never changes afterwards, so no later operation recreates the risk: the formulas above only add, subtract, and divide, and a u128 division never exceeds its numerator. `Vc` and `Vt` can drift past 2^64 mid-sale as trades land, and that is safe for the same reason — nothing multiplies two reserves again.

256-bit intermediates would buy generality no real sale uses. 2^64 base units of virtual reserve is about 1.8 x 10^19 — with nine decimals, an eighteen-billion-token virtual reserve. A launch that needs more can split decimals, not the arithmetic.

### Part 2 — the pool-favour lemma

Every function in `crates/curve-math` rounds the `k / x` term up, which floors what leaves the sale and ceils what enters it. Each buy and sell therefore moves the state to a point with `Vt * Vc >= k`: the rounding drift always lands on the sale's side. That inequality is what makes every `checked_sub` in the formulas safe for any state the transitions produce, and it is what GTM-514 asserts after every operation. The accumulated surplus is spendable by an honest quote — a zero-amount trade can extract it — so the buy and sell transitions reject zero-amount trades (recorded on GTM-510 and GTM-511).

### Part 3 — the sell-side bound

The sell path computes `Vt + tokens_in`. In honest flows the sum stays below 2^65: sold-back can never exceed bought because the factory mints the whole supply, and both terms start below 2^64. That is why honest flows never see the checked-add error arm — it is not an unreachability claim. F2 lets a creator hand the curve any token pair, so tokens acquired outside the sale can come back, and trade amounts are unbounded input; the checked calls stay real guards that error instead of wrapping. `crates/curve-math` and `crates/sale` deny `clippy::arithmetic_side_effects`, so every operation is a named checked call.

## Consequences

Every operation in the pricing path is plain u128 mul/div/add/sub. Reviewers check one inequality at creation instead of auditing a wide-integer library, and the guest binary carries no 256-bit code.

The bound lives in `Sale::create`, so state constructed any other way (hand-built in a test, or a corrupted account) is outside the argument. `curve-math` stays total anyway: checked calls return `MathError` rather than trusting the bound.
