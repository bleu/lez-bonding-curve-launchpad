# 0003 — Admin config: one instruction, a genesis constant, and rotation as the RFP-001 seam

Status: accepted

## Context

RFP-015 says the fee rate and the treasury address are set by the program's admin authority and apply uniformly across all pools, with pool creation free. RFP-001 provides that admin authority as a library and is a listed dependency of the RFP. We do not build RFP-001. GTM-516 asked for the config PDA and an `update_config` instruction, plus a visible seam where RFP-001 plugs in later.

The design questions were: how the config account comes to exist and who the first admin is, whether the admin can change, how the fee and the treasury are represented, and whether swaps read the fee live or copy it into the pool at creation.

## Decision

One instruction, `update_config`, which creates the config PDA on the first call and replaces it whole on every call after. There is no separate `init_config`. On the empty-account branch the gate is `GENESIS_ADMIN`, a pubkey compiled into `curve-core` as a constant. The constant is part of the risc0 image ID, so a reviewer can see it cannot be front-run: changing it produces a different program with different PDAs. After the first call the gate is the `admin` stored in the config. A first-caller-wins init was rejected because a bot could take the fee stream at deploy time.

The stored admin is rotatable, and that rotation is the RFP-001 seam. Plugging the library in later means rotating the admin to the key it controls. No code change, no redeploy. A fixed admin would force a rebuild, which changes the image ID and every PDA. Rotation is single-step; a two-step propose-and-accept transfer was rejected as three times the instruction surface for a hazard the RFP does not name.

Every call carries all three fields (`admin`, `fee_bps`, `treasury`) and the handler writes the whole struct. `Option`-per-field partial updates were rejected because the init branch needs all three values anyway, and full replace makes init and update the same write.

`fee_bps` is a `u16` in basis points, capped at `MAX_FEE_BPS` (10,000). The cap is the denominator, which keeps `amount - fee` from underflowing. A tighter cap was rejected because the RFP gives no number to justify one; that is the operator's call. Zero is legal. The fee rounds down.

`treasury` is an owner pubkey, not a token holding account, because the curve runs over any token pair. As superseded by ADR 0005, each swap derives or verifies the treasury holding for whichever token is input. LEZ's token `transfer` initializes an empty recipient (`zeroized_clone_from`), so no pre-creation step exists. The all-zeros default key is rejected for both `admin` and `treasury`.

Swaps read `fee_bps` live from the config. Snapshotting the fee into the pool at creation was rejected because pools created either side of an update would charge different fees forever. The config account joins each swap transaction, and its slippage parameter protects against a fee change in flight. A swap fails if the config does not exist; a silent zero-fee default would hide a broken deploy.

## Consequences

Deployment gains one required step: whoever holds the genesis admin key calls `update_config` once before any trade. The genesis constant must be replaced with the operator's key before the deploy build.

A wrong rotation is unrecoverable. The README says so where operators will read it.

The `pool` crate receives `fee_bps` as a plain argument and never sees the config account, which keeps solvency tests free of account fixtures. `Config` lives in `curve-core`, a local type, so the upstream `TryFrom<&Data>` pattern applies.

This issue introduced the `Instruction` enum with the single `UpdateConfig` variant. ADR 0005 records the later neutral pool variants. CLI and client wrappers remain separate adapter work.
