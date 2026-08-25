# 0004 — Bound initial virtual reserves below 2^64 so pool arithmetic stays in u128

Status: amended by ADR 0005

ADR 0005 supersedes the earlier fee decision. This ADR remains authoritative for
the creation bound and checked trade arithmetic.

## Context

For either swap direction, exact-input pricing is

`amount_out = reserve_out - k / (reserve_in + amount_in)`

and exact-output pricing is its inverse

`amount_in = k / (reserve_out - amount_out) - reserve_in`.

Constant-product implementations often need 256-bit intermediates because multiplying two unrestricted reserves can exceed u128.

## Decision

### Part 1 — creation bound

`Pool::create` rejects either initial virtual reserve at or above 2^64 (`VIRTUAL_RESERVE_BOUND` in `crates/pool`) and rejects zero virtual reserves. The initial multiplication is `k = virtual_reserve0 * virtual_reserve1`. Two factors below 2^64 produce a value below 2^128, and `k` never changes.

All later quotes use this stored creation-time `k`; they never recompute a live
virtual-reserve product. Virtual reserves may move past 2^64 after swaps, so the
implementation does not make multiplying them a transition precondition. Trade
amounts remain untrusted `u128` inputs, and every reserve and quote operation is
checked before mutation.

### Part 2 — pool-favouring rounding

Every pricing path rounds `k / x` up. Therefore exact-input output is rounded down and exact-output input is rounded up. The stored `k` never changes; live reserves can contain integer rounding drift but that drift is not repriced as a new invariant. The state machine rejects zero requested input or output so callers cannot intentionally submit a nominal zero trade.

The combined pool and protocol fee is separated from effective input before reserve pricing. The pool portion is then retained in the input reserve and the protocol portion leaves for treasury. For exact output, gross fee-inclusive input is the smallest integer whose post-fee amount covers the conservatively rounded effective quote.

### Part 3 — real-reserve solvency

Virtual reserves determine price, but real reserves determine what can leave the pool. Both directions check the selected real output reserve before mutation. Insufficient backing fails the swap; token0 depletion does not close the pool.

`crates/curve-math` and `crates/pool` deny `clippy::arithmetic_side_effects`, so arithmetic in the correctness boundary is expressed through checked operations.

## Consequences

Reviewers audit one initial bound rather than a wide-integer library. Direction-neutral functions cover all four paths: exact input and exact output for either ordered input token. State constructed without `Pool::create` is outside the creation-bound argument, but `curve-math` remains total and returns `MathError` for invalid arithmetic states.
