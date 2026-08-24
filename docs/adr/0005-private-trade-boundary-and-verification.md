# 0005 — Private trades protect participants, not the public curve

Status: accepted

## Context

LEZ private transactions can prove that a private account authorized a program
call without revealing that account or its balance to outside observers. The
bonding curve nevertheless has public sale state: its sale reserve, real and
virtual reserves, fee transfers, token-account changes and the resulting spot
price are observable. A buy or sell can therefore reveal, or tightly constrain,
its effective size through the public state transition.

A private transaction that produces a `ChainedCall` must be proved with the ELF
for the main program and every chained target. A public call can appear to work
without that dependency set, so treating public success as evidence for private
support produces a late and confusing failure. Once the private transaction is
submitted, the wallet must sync its private account before it can reliably
display or spend the new state.

The curve program cannot enforce that a participant re-shields assets received
from a trade. That action has to be part of the SDK lifecycle.

## Decision

The user-facing privacy claim for this PoC is **anonymous participation**:

- A participant may authorize a trade with a private account without exposing
  the account identity or its private balance in the public transaction data.
- The sale state, token definitions, config, treasury, reserve movements, fees,
  and price movement remain public.
- The PoC makes no claim to hide trade amount, execution time, or price impact.
  Those values may be inferred from the public curve transition even when the
  underlying transfer wrapper supports variable privacy.

`launchpad-client` owns the private trade lifecycle. When private buy and sell
operations land, it must:

1. load the deployed curve ELF and every ELF for a program reached through a
   chained call;
2. build the private proof with that complete dependency set;
3. submit the transaction and sync the private account after confirmation; and
4. re-shield received assets when the operation requires it.

Callers must not locate or supply well-known dependency binaries by hand. The
SDK exposes lifecycle operations, not proof-builder plumbing.

## Verification

The current repository has no buy or sell handler, so these are acceptance
criteria for the issues that add those operations rather than executable tests
today:

1. A public localnet buy and sell establish the expected sale, participant,
   treasury and reserve post-states.
2. The equivalent private buy and sell succeed with the SDK-provided dependency
   set, followed by `sync-private`; the private balance and received assets are
   usable afterwards.
3. Omitting a chained-program ELF fails before submission with an actionable
   dependency error. This guards against a public-only happy path.
4. An observer-facing assertion records the public sale transition and confirms
   that it contains no private participant account ID or private balance.
5. The same assertion records which public fields permit amount inference, so a
   future UI or README cannot overstate confidentiality.

The first four run against a local LEZ sequencer; the last is a documented
privacy review beside the test output. The end-to-end script supplements, but
does not replace, the pure `curve-math` tests, the `sale` state-machine property
test, and the `curve-core` account/authorization adapter tests.

## Consequences

The SDK has an explicit responsibility beyond transaction submission, and its
private-flow API must carry a complete program-dependency manifest. This is a
larger client surface than public trades, but it keeps LEZ proof mechanics out
of the CLI and makes private failures reproducible.

Public observability remains a design constraint. Confidential trade amounts
would require a different sale-state and settlement design, not merely a
private-account flag or a variable-privacy transfer.
