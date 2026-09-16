# 0007 — Factory closure and creator settlement

Authority amendment: [ADR 0008](0008-permissionless-namespaces.md) replaces the creator account-address witness with a transferable NFT definition identity. Allocation policy below still applies.

Status: accepted

## Context

ADR 0006 made pool expiry a logical state, but it did not preserve the RFP-015
launch lifecycle when the neutral pool replaced the former sale state. A factory
launch needs the tradeable token0 reserve `D` to close on depletion, while a
creator allocation and the DEX-seed allocation `R` remain factory policy.

## Decision

`PoolLifecycle::{Open, Closed, Withdrawn}` records the pool lifecycle. Factory creation and settlement stages are described in ADR 0008.
The effective status evaluates `close_timestamp` against the canonical LEZ clock;
timestamp expiry is therefore closed even before a state write. Factory pools
always set `close_on_depletion` to token0. Direct pools may opt out or select
either ordered side.

Factory creation always escrows the creator allocation. There is no immediate
allocation policy. `ClaimCreatorAllocation` is creator-witness authorized,
relayable, and available after effective closure exactly once. Its recipient ATA
is derived from the current creator NFT holding.

`CloseFactoryPool` is likewise creator-witness authorized and relayable. It
authorizes the factory PDA for the neutral curve close call. `WithdrawFactoryProceeds`
is independent of allocation claiming: after effective closure it withdraws both
pool reserves into factory ATAs, burns the withdrawn token0 amount, transfers the
reserved `R` token0 amount, and transfers all withdrawn token1 collateral to the
current creator NFT holder. Settlement is resumable across transactions; each stage and its chained calls are atomic (ADR 0008).

All creation and lifecycle account layouts carry the canonical clock. A supplied
end timestamp must be strictly later than trusted current time.

## Consequences

This supersedes ADR 0006's implication that a factory merely maps creation policy:
the factory also owns post-close launch settlement. The neutral pool remains free
of creator allocation, `R`, and token-burning policy. Automatic DEX graduation is
future work; this PoC returns `R` and collateral through factory settlement.
