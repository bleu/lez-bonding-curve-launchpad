# 0005 — Dual input fees with an immutable pricing invariant

Status: accepted

## Context

The GTM-514 solvency review introduced a retained pool fee alongside the protocol
fee. The RFP has higher authority than that review on curve semantics: pricing must
use the creation-time constant product `k`, not a product recomputed from rounded
live reserves.

Charging and rounding pool and protocol fees independently would overcharge small
trades. Fees also need one direction-independent rule: both buys and sells charge in
the input asset.

## Decision

`Config` stores `pool_fee_bps` and `protocol_fee_bps`. Their sum must not exceed
10,000. Both are read live by each trade.

For a gross input and combined rate, the state machine:

1. Computes the combined fee once, rounded up.
2. Assigns the protocol share as
   `floor(combined_fee * protocol_fee_bps / total_fee_bps)`.
3. Assigns the remainder to the pool fee.
4. Prices with the effective input after the combined fee.
5. Adds the pool fee to the matching virtual and real pool reserve; the protocol
   fee leaves for the treasury.

The proportional floor gives a deterministic split, preserves the exact combined
fee, and gives any indivisible remainder to the pool rather than charging another
unit. The rule is direction-neutral: token0 or token1 can be input, and handlers
send the protocol fee to the treasury holding for that input token.

Pricing always uses stored creation-time `k`. Live reserves are updated with checked
addition/subtraction, but their product is not recalculated for later quotes and is
not a required `u128` intermediate. Accepted trades retain immutable `k`.

Outputs still round down, exact-output inputs round up, and the combined fee rounds
up. Zero inputs and zero requested outputs reject before quoting.

## Consequences

This amends ADR 0003: its single fee, collateral-only treasury holding, and fee
round-down rules no longer apply. It preserves ADR 0004's creation bounds and
checked arithmetic, and restores its creation-time `k` as the pricing source.

The config and `UpdateConfig` wire formats now carry two fee rates. This change lands
before deployment, so no on-chain migration is required for the proof of concept.
GTM-514's chain-free property suite is the executable obligation for these rules.
