# 0005 — Collateral-only protocol fees with an immutable pricing invariant

Status: accepted

## Context

RFP-015 gives the protocol fee a precise asset rule: it is always collateral,
not a retained AMM fee and not the launch token. The earlier dual-input fee model
conflicted with that higher-authority requirement.

Pricing still uses the creation-time constant product `k`; rounded live reserves
are accounting state, not a new pricing invariant.

## Decision

`Config` stores only `protocol_fee_bps` and the treasury owner. The rate is live
for every swap and must be at most 10,000 basis points.

| Trade | Fee | Curve input | Trader receives | Treasury receives |
| --- | --- | --- | --- | --- |
| Buy (token1/collateral in) | `ceil(collateral_in × rate)` | `collateral_in - fee` | quoted launch tokens | collateral fee |
| Sell (token0/launch token in) | `ceil(raw_collateral_out × rate)` | launch tokens | `raw_collateral_out - fee` | collateral fee |

There is no retained pool fee. A buy's `max_amount_in` is the gross collateral
debit. Exact-output swaps are buys only; exact-input supports both directions.
Each fee transfer is a chained ATA call in the same curve transaction as the
pool reserve movement and trader payout.

## Consequences

The treasury ATA switches with settlement direction only in account position, not
in asset: it is always the collateral ATA. Sell reserve accounting removes the
raw collateral output, including the part delivered to treasury. Tests cover both
directions, ceiling rounding, transaction wiring, and state-machine conservation.
