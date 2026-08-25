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

`launchpad-client` owns the private-buy lifecycle. It:

1. creates a fresh single-use public account A;
2. builds one privacy-preserving transaction for `private_buy` with the curve,
   native-transfer, token, and ATA guest binaries as dependencies;
3. chains, in order: native gas deshield to A, collateral ATA creation,
   launch-token ATA creation, collateral deshield, exact-output buy, and
   re-shield of the bought launch tokens to private destination B; and
4. returns the transaction hash plus A and B for callers to sync or inspect.

Callers must not locate or supply well-known dependency binaries by hand. The
SDK exposes lifecycle operations, not proof-builder plumbing.

## Verification

`cargo check --manifest-path methods/guest/Cargo.toml --bin private_buy`
compiles the guest against the pinned LEZ programs. SDK and CLI tests enforce a
nonzero gas reserve, a nonzero collateral cap/output, private source and
destination inputs, and the router-binary argument. This PoC does not claim
localnet or live submission evidence.

## Consequences

The SDK has an explicit responsibility beyond transaction submission, and its
private-flow API carries a complete program-dependency manifest. The router
enforces the re-shield call inside the same proof composition; callers cannot
turn this API into separate funding and purchase transactions.

Public observability remains a design constraint. Confidential trade amounts
would require a different sale-state and settlement design, not merely a
private-account flag or a variable-privacy transfer.
