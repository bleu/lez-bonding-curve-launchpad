# Domain Docs

How the engineering skills should consume this repo's domain documentation when
exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root.
- **`docs/adr/`** — read ADRs that touch the area you are about to work in.

If any of these files do not exist, **proceed silently**. Do not flag their absence
and do not suggest creating them upfront. The producer skill (`/grill-with-docs`)
creates them lazily when terms or decisions actually get resolved.

## File structure

This is a single-context repo.

```
/
├── CONTEXT.md
├── docs/adr/
│   ├── 0001-pin-lez-v0.2.0-and-spel-v0.6.0.md
│   └── 0002-repo-layout-and-guest-shim.md
├── crates/
│   ├── curve-math/
│   ├── sale/
│   ├── curve-core/
│   └── launchpad-client/
├── cli/
├── methods/guest/src/bin/
└── verify/
```

`CONTEXT.md` carries the crate map as well as the glossary. Read it for which crate owns what.

There is no `CONTEXT-MAP.md`. The curve program, the factory program, and the CLI
share one vocabulary, so splitting them would duplicate the glossary rather than
separate anything.

## Use the glossary's vocabulary

When your output names a domain concept (an issue title, a refactor proposal, a
hypothesis, a test name), use the term as defined in `CONTEXT.md`. Do not drift to
synonyms the glossary explicitly avoids.

RFP-015 supplies most of the ubiquitous language here: sale, sale reserve, DEX seed
reserve, virtual token reserve, virtual collateral reserve, real collateral reserve,
spot price, supply target, auto-close, graduation. Prefer those terms over
AMM-generic ones. A sale is not a pool; the sale reserve is not liquidity.

If the concept you need is not in the glossary yet, that is a signal — either you are
inventing language the project does not use (reconsider) or there is a real gap (note
it for `/grill-with-docs`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than
silently overriding:

> _Contradicts ADR-0002 (handlers live in the host workspace) — but worth reopening because…_
