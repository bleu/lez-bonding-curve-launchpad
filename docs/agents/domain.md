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
│   ├── pool/
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

The curve is a neutral bounded AMM over an ordered token pair. Use pool, token0,
token1, real reserve, virtual reserve, exact-input swap, and exact-output swap in
the AMM layer. Sale, purchase, redemption, token-for-sale, collateral, and DEX-seed
allocation belong only to the factory or launch-facing adapters.

If the concept you need is not in the glossary yet, that is a signal — either you are
inventing language the project does not use (reconsider) or there is a real gap (note
it for `/grill-with-docs`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than
silently overriding:

> _Contradicts ADR-0002 (handlers live in the host workspace) — but worth reopening because…_
