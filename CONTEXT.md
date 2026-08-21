# Context

The ubiquitous language for this repo, and the map of which crate owns what. RFP-015 supplies most of the vocabulary. Use these terms in code, tests, issues and commits. Do not drift to synonyms.

Design decisions live in `docs/adr/`, not here.

## Glossary

A sale is not a pool. The sale reserve is not liquidity. Those two AMM-generic words are the ones most likely to creep in, and they name the wrong things.

- Sale — one token launch on a bonding curve. Has its own state account, its own reserves, and an open or closed flag.
- Creator — the account that opens a sale, supplies the tokens, and withdraws after close.
- Participant — any account that buys from or sells back to the curve.
- Sale reserve — the token quantity the curve can still dispense. Written `D`. One of the two accounting buckets.
- DEX seed reserve — the token quantity held back for seeding a DEX after close. Written `R`. The other bucket. Withdrawn by the creator while graduation is unavailable.
- Supply target — the sale reserve at creation, which is the quantity that has to sell for the sale to complete.
- Real collateral reserve — the collateral actually held by the sale, as opposed to the virtual amount used for pricing.
- Virtual token reserve — the pricing-only token quantity. Written `Vt`. Exceeds the sale reserve, which is what makes the curve completable.
- Virtual collateral reserve — the pricing-only collateral quantity. Written `Vc`. Sets the starting price with `Vt`.
- `k` — the constant product, fixed at creation from `Vt` and `Vc`, and never changed afterwards.
- Spot price — the marginal price implied by the current `Vt` and `Vc`.
- Auto-close — the sale closing inside the same transaction as the buy that exhausts the sale reserve. Not a separate instruction.
- Manual close — the creator ending a sale that will not reach its supply target.
- Unlock policy — whether the creator's own allocation is redeemable immediately or only after close.
- Graduation — moving a completed sale's tokens and collateral into a DEX. Out of scope; the live path is manual withdrawal.
- Treasury — the owner protocol fees are transferred to, in the same transaction as the trade. The receiving account is the treasury's ATA for the sale's collateral.
- Admin authority — the key allowed to update the fee rate and the treasury address.
- Config — the singleton PDA holding the fee rate, the treasury owner, and the admin key. Created and replaced whole by `update_config`, read live by every trade. See `docs/adr/0003`.
- Genesis admin — the admin key compiled into `curve-core`, gating only the config's first write. Part of the risc0 image ID.
- ATA — an associated token account, derived from an owner and a token definition. Reserves are ATAs owned by the sale PDA; participant balances are ATAs owned by the participant.
- Factory — the program that mints a token with a fixed supply, publishes the split, and tail-calls into the curve to open the sale. The reason sold-back can never exceed bought.
- Curve — the program that runs a sale over any token pair handed to it. The RFP deliverable.

## Crate map

Boundaries are enforced by cargo rather than by convention, so a reviewer can check them by opening a manifest.

- `crates/curve-math` — the pricing arithmetic as pure functions over integers. Empty dependency list. Denies `clippy::arithmetic_side_effects`, so every operation is a named checked call.
- `crates/sale` — the sale state machine. Applies a buy, a sell, or a close and returns what should move. Depends on `curve-math` and `borsh` only, so the solvency property test needs no account fixtures. Denies `arithmetic_side_effects` too.
- `crates/curve-core` — where `lee_core` enters. Instruction enum, Borsh state, PDA derivation, and the handlers. The handlers are shallow: deserialize, call `sale`, translate the outcome into post-states and chained calls. AMM-style tests live here.
- `crates/launchpad-client` — the SDK. Wallet handling, message and witness construction, program ids, account derivation. The private deshield to buy to re-shield flow belongs here, because the program cannot enforce the re-shield.
- `cli/` — the `launchpad` binary. Parses arguments and calls `launchpad-client`, and nothing else.
- `methods/guest/src/bin/*.rs` — one risc0 guest per file. Each is a dispatch shim over the matching core crate. Excluded from the root workspace so the guest keeps its own `[profile.release]`, which is part of program identity.
- `verify/` — what a reviewer runs.

`curve-core` owns the ATA derivation checks and chained custody calls for sale reserves. Nothing yet owns the factory; it arrives with GTM-515 as modules or crates depending on what that boundary turns out to need.
