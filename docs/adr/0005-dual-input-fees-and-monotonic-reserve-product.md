# 0005 — Collateral-only protocol fees with an immutable pricing invariant

Status: accepted; settlement revised for RFP PR 204

## Context

RFP-015 gives the protocol fee a precise asset rule: it is always collateral,
not a retained AMM fee and not the launch token. The earlier dual-input fee model
conflicted with that higher-authority requirement.

Pricing still uses the creation-time constant product `k`; rounded live reserves
are accounting state, not a new pricing invariant.

## Decision

`Config` stores only `protocol_fee_bps` and the treasury owner. The rate is live
for every swap and must be at most 10,000 basis points.

| Trade | Fee | Curve input | Trader receives | Accrued fee |
| --- | --- | --- | --- | --- |
| Buy (token1/collateral in) | `ceil(collateral_in × rate)` | `collateral_in - fee` | quoted launch tokens | collateral fee |
| Sell (token0/launch token in) | `ceil(raw_collateral_out × rate)` | launch tokens | `raw_collateral_out - fee` | collateral fee |

There is no retained pool fee. A buy's `max_amount_in` is the gross collateral
debit. Exact-output swaps are buys only; exact-input supports both directions.
Fees remain in the token1 vault and increment `fees_accrued`; swaps make only
an input transfer and a net-output transfer. The treasury ATA is absent from the
swap account list. This replaces immediate settlement following
[RFP PR 204](https://github.com/logos-co/rfp/pull/204).

`CollectFees` requires no authority and transfers exactly `fees_accrued` to the
namespace treasury resolved at collection time. It resets that counter and increments
`fees_collected`, without changing pricing or reserves. Zero collection emits no
transfer. Donations do not become fees. The adapter checks
`vault1 >= real_reserve1 + fees_accrued`; sells remove raw output from reserves
but transfer only net output, leaving the fee in custody.

Reserve withdrawal collects atomically before transferring principal. The second
vault debit uses the post-collection balance and final pool snapshot. The SDK
initializes a missing treasury ATA before collection or withdrawal; treasury
rotation between preparation and execution rejects a stale destination safely.
Collection is available on open, expired, closed and withdrawn pools.
LEZ requires unique account IDs: withdrawal omits the trailing treasury ATA when
it is already a recipient, and carries the fee credit into that recipient's next
transfer snapshot. SDK factory builders handle this omission automatically.

Private factory withdrawal pre-collects publicly from the closed pool before its
reserve-withdrawal stage, keeping the private call graph within the pinned budget.
The pool cannot accrue new fees after closing; reserve withdrawal still collects
atomically if fees remain. Batch collection is a sequence of independently atomic,
idempotent transactions, not an all-or-nothing batch.

## Consequences

Pool account and instruction layouts change, requiring fresh deployments. Tests
cover both trade directions, ceiling rounding, treasury rotation, repeated collection,
reserve withdrawal with and without outstanding fees, isolated namespaces, donations,
and randomized fee/reserve conservation.
