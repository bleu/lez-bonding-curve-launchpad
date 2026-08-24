# 0005 — Dual input fees and a monotonic reserve product

Status: accepted

## Context

The GTM-514 solvency review sharpened two obligations that the earlier fee and
arithmetic ADRs did not satisfy. A single protocol fee cannot represent a fee
retained by the pool, and quoting every trade against initial `k` lets a later trade
consume accumulated rounding surplus. The required invariant is stronger: the
actual virtual-reserve product never decreases, while stored `k` remains immutable
as its creation-time lower bound.

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

Pricing uses the current `virtual_reserve0 * virtual_reserve1`, not stored `k`.
Every multiplication is checked. If the current or proposed product does not fit
`u128`, the transition rejects without mutation. Accepted trades therefore satisfy:

- the new virtual-reserve product is at least the old product;
- a nonzero retained pool fee makes that inequality strict;
- the new virtual-reserve product remains at least `k`; and
- stored `k` never changes.

Outputs still round down, exact-output inputs round up, and the combined fee rounds
up. Zero inputs and zero requested outputs reject before quoting.

## Consequences

This amends ADR 0003: its single fee, collateral-only treasury holding, and fee
round-down rules no longer apply. It also amends ADR 0004 parts 1 and 2: reserve
products are recomputed during trades, and accumulated rounding surplus is no longer
spendable by a later transition. The creation bounds and checked arithmetic still
provide the initial safety argument.

The config and `UpdateConfig` wire formats now carry two fee rates. This change lands
before deployment, so no on-chain migration is required for the proof of concept.
GTM-514's chain-free property suite is the executable obligation for these rules.
