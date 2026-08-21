# 0004 — Bound initial virtual reserves below 2^64 so pool arithmetic stays in u128

Status: accepted (updated by ADR 0005)

## Context

For either swap direction, exact-input pricing is

`amount_out = reserve_out - k / (reserve_in + amount_in)`

and exact-output pricing is its inverse

`amount_in = k / (reserve_out - amount_out) - reserve_in`.

Constant-product implementations often need 256-bit intermediates because multiplying two unrestricted reserves can exceed u128.

## Decision

### Part 1 — creation bound

`Pool::create` rejects either initial virtual reserve at or above 2^64 (`VIRTUAL_RESERVE_BOUND` in `crates/pool`) and rejects zero virtual reserves. The only reserve multiplication is `k = virtual_reserve0 * virtual_reserve1`, performed once at creation. Two factors below 2^64 produce a value below 2^128. `k` never changes; later quotes use checked add, subtract, divide, and remainder operations.

Virtual reserves may move past 2^64 after swaps. That is safe because they are never multiplied together again. Trade amounts remain untrusted u128 inputs, so checked arithmetic still returns errors rather than wrapping.

### Part 2 — pool-favouring rounding

Every pricing path rounds `k / x` up. Therefore exact-input output is rounded down and exact-output input is rounded up. Each successful swap leaves `virtual_reserve0 * virtual_reserve1 >= k`; rounding drift lands on the pool's side. The state machine rejects zero requested input or output so callers cannot intentionally extract accumulated rounding surplus with a nominal zero trade.

Protocol fees are separated from the net input before reserve pricing. For exact output, gross fee-inclusive input is the smallest integer whose post-fee amount covers the conservatively rounded net quote.

### Part 3 — real-reserve solvency

Virtual reserves determine price, but real reserves determine what can leave the pool. Both directions check the selected real output reserve before mutation. Insufficient backing fails the swap; token0 depletion does not close the pool.

`crates/curve-math` and `crates/pool` deny `clippy::arithmetic_side_effects`, so arithmetic in the correctness boundary is expressed through checked operations.

## Consequences

Reviewers audit one initial bound rather than a wide-integer library. Direction-neutral functions cover all four paths: exact input and exact output for either ordered input token. State constructed without `Pool::create` is outside the creation-bound argument, but `curve-math` remains total and returns `MathError` for invalid arithmetic states.
