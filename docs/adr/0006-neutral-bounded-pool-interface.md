# 0006 — Neutral bounded-pool interface and factory boundary

Status: accepted

## Context

The initial model embedded token-launch policy in the curve: sale/buy/sell vocabulary, a token-for-sale role, automatic close on token depletion, and a DEX-seed reserve in curve state. That made direct AMM use asymmetric and forced the factory's allocation policy into the lower-level program.

## Decision

The curve is a neutral bounded constant-product pool over an ordered `(token0, token1)` pair. Its public operations are `CreatePool`, `SwapExactInput`, `SwapExactOutput`, `ClosePool`, and `WithdrawReserves`. `token_in` is a token definition ID and selects direction. There are no liquidity-add or liquidity-remove operations.

Both tokens may have real reserves at creation. Exact-input swaps work in both directions; exact-output is deliberately limited to collateral-input buys. There is no pool fee. The protocol fee is always collateral: `ceil(collateral_in × rate)` is removed before buy pricing, while `ceil(raw_collateral_out × rate)` is removed from a sell's raw output. The treasury holding is therefore always for token1/collateral. `max_amount_in` includes the buy fee. Pricing rounds conservatively and a swap fails rather than exceeding its real output reserve. Token0 depletion does not close the pool.

Pools store a public owner and optional close timestamp. `None` permits owner-controlled manual closure only. `Some(timestamp)` rejects swaps at or after trusted LEZ chain time. Expiry is logical closure: owner-authorized full withdrawal may proceed without a separate close transaction. Withdrawal empties both real reserves and permanently retires the pool.

Pool creation is atomic at the program boundary: it verifies owner authorization and both owner source ATAs, creates both pool-PDA-owned reserve ATAs through the pinned LEZ ATA guest, and transfers each nonzero initial reserve. The pool PDA hashes token0, token1, then owner without sorting. Including owner prevents one direct caller from occupying the only PDA for a pair.

The factory maps launch policy onto this interface. It always places the newly minted token at token0, stores a factory-owned PDA as pool owner, retains any DEX-seed allocation and creator identity outside pool state, and deposits only tradeable amounts. Direct callers choose both ordered roles and owner themselves.

This is an intentional breaking PoC wire/state change. Compatibility aliases are not retained.

## Consequences

`crates/pool` replaces the sale state machine, `PoolAccount` replaces `SaleAccount`, and pricing functions and tests use exact-input/exact-output vocabulary. `CONTEXT.md` defines the AMM/factory language boundary.

`crates/factory-core` implements this mapping through the curve interface. It must not add launch allocation or creator-commitment fields back to pool state.
