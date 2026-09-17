# 0008 — Permissionless namespaces and transferable NFT authority

Status: accepted; replaces this ADR's original public-key authority model

## Context

One deployment supports independent namespaces, fees, and treasuries. A user may
operate through a different public account on each private transaction. Persisting
that temporary address as the admin or creator would strand the role after the
account changes. Moving funds through shield/deshield does not itself delegate a
role.

## Decision

Human authority is a bearer NFT, shared across namespace administration, factory
creator operations, and direct pool ownership. Its stable identity is the token
definition ID, not its current holding account. A caller supplies an authorized
holding owned by the pinned LEZ token program, containing exactly an `NftMaster`
with `print_balance = 1` and the expected definition ID. Printed copies, fungible
tokens, empty former holders, and foreign-program data grant no authority.

Issue authority NFTs with `printable_supply = 1`. The pinned token program forbids
additional NFT minting and cannot print from a master with one remaining unit.
Transfers move that unit completely and leave the former holding at zero. An NFT
master is a direct token holding, not a conventional ATA: the latter initializes
a printed-copy holding. Keys for current holdings must be retained.

Namespace identity is the NFT definition authorizing initialization. The config
seed is that immutable identity. `Config.admin` identifies the current authority
NFT; transferring its holding changes the controller without changing namespace
addresses. Configuration can explicitly nominate a replacement authority NFT.
Renunciation clears the admin identity permanently; recovering or transferring
the former token never restores that namespace's administration.

Factory creator commitments bind the creator NFT definition, namespace, and sale
salt. Close, allocation claim, and withdrawal validate the same NFT. Settlement
pays the current authorized holder's ATAs, so transferring the NFT transfers the
unclaimed creator rights too. This supersedes ADR 0007's account-address-bound
creator witness. Direct pools likewise store the owner NFT definition and derive
pool addresses from it, keeping addresses fixed across holder changes.

Factory-owned pools remain under program custody to enforce allocation and burn
policy. Their owner records a program ID and PDA seed; authorization checks the
exact derived account and its program owner. A creator cannot bypass factory
policy by presenting the creator NFT directly to the curve. This internal PDA
mechanism is distinct from human roles; all human roles use the same NFT check.

The private authority router composes token transfer from the private source to
a fresh authorized public holding, one app action, and token transfer back to the
source. Each call supplies the expected intermediate state. The action sees only
the temporary holder, while persistent role records refer to the NFT definition.
LEZ validates the complete private transaction atomically; a failure at any step
rejects all its writes. The source key and private state must survive between uses.
Creator payouts go to temporary public ATAs; returning the authority NFT does not
also shield those payouts.

## Consequences

Authority is transferable permission, not proof that two holders are the same
person. The holder may transfer it to someone else. Anyone using the unique NFT
publicly exposes its association with that role; fresh public accounts do not make
those admin actions unlinkable. Namespace settings and sale state remain public.

The SDK/CLI own token creation, transfer, and private action construction. Ordinary
spending still requires authorization of the funding account. The RFP-001 library
remains an integration seam; this PoC implements bearer authority locally.

Wire and account layouts change. Rebuild all guests and create fresh namespaces
and sales. No public-key fallback or migration is provided. Validation must cover
runtime account rules as well as role checks, transfer to a new holder, rejection
of spent/copied/forged authority, and the private round trip. Guest execution in
dev mode does not establish production-proving or live-sequencer performance.

## Resumable creation and settlement

The pinned runtime limits a private graph to ten calls including its root. Factory
creation therefore advances four confirmed stages: mint the fixed supply, allocate
factory/creator balances, prepare the pool and reserve ATAs, then fund and activate
it. Pending pools reject swaps, close, and reserve withdrawal. The original deadline
is never extended: late activation creates a closed pool that can still settle.

Before private proceeds settlement withdraws reserves, the SDK permissionlessly
collects any accrued fees from the closed pool in a public transaction (ADR 0005).
Public reserve withdrawal collects outstanding fees atomically.

Proceeds settlement advances five confirmed stages: prepare the factory collateral
ATA, withdraw the pool reserves, burn the captured unsold amount, pay the exact `R`
allocation, and pay the captured collateral. Payout stages initialize the current
holder's ATA when necessary. Donations cannot increase the recorded burn or payout.
Allocation claiming is independent and transfers exactly the committed `C` once.

Each stage rechecks the same creator NFT, is atomic, and advances persisted state
only with its successful child calls. Completed stages cannot pay twice. The entire
workflow spans transactions; an interruption leaves a resumable intermediate state.
Transferring the NFT between stages transfers remaining rights. Losing or burning
the sole NFT also loses those rights. Every private stage returns the NFT to its
private source and uses at most nine executions, including the router.

`FactorySession` and CLI `create-sale`/`withdraw` confirm each transaction before
building the next from chain state. Retrying the same launch resumes its remaining
stages; completed workflows return without another transaction. Confirmed private
account updates are decrypted and saved before reuse. After a process crash or
confirmation timeout, sync the wallet before retrying with the same launch salt.
Retain the transient public holder keys: creator payouts remain public.

The pinned public runtime inherits PDA authorization down the call tree; the privacy
circuit also remembers it across siblings. The SDK selects an explicit `Private`
instruction wrapper for privacy execution, which factory forwards to curve calls.
Ordinary instructions use public metadata rules. Neither mode grants authority:
the runtime verifies every supplied authorization and account snapshot. Nested
wrappers are rejected. Rust instruction enums remain the authoritative wire format;
SPEL declarations describe the inner operations and their account layouts.

## Execution evidence

Actual pinned RISC0 guests and native public transactions cover fee-bearing swaps,
four-stage creation, allocation claim, five-stage settlement, fresh NFT holders and
payout ATAs, zero allocations, depletion, late activation, and settlement replay
rejection. Private authority tests also cover configuration, direct/factory close,
and unrelated-NFT rejection. Private tests use development proving and synthetic
membership witnesses, not production proofs or a live sequencer.
