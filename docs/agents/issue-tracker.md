# Issue tracker: Linear

Issues, PRDs, and plans for this repo live in Linear, in the **GTM** team.
Use the Linear MCP tools. There is no CLI for this tracker.

Default project: **LEZ Bonding Curve Launchpad PoC**
(`https://linear.app/bleu-builders/project/lez-bonding-curve-launchpad-poc-74268c0c5427`).
New issues attach to it unless the user says otherwise.

## Conventions

- Create an issue: `save_issue` with `team: "GTM"`, `project: "LEZ Bonding Curve Launchpad PoC"`, `title`, and a Markdown `description`. Omit `id` when creating.
- Update an issue: `save_issue` with `id` set to the identifier (e.g. `GTM-510`). Prefer `patch` over resending the whole `description`.
- Read an issue: `get_issue`. Comments come from `list_comments`.
- List issues: `list_issues`, filtered by `team`, `project`, `label`, or `state`.
- Comment: `save_comment`.
- Labels: pass the full set to `save_issue.labels`. It replaces rather than appends, so read the current labels first if you are adding one.
- Close: `save_issue` with `state: "Done"`, or `"Canceled"` for work that will not happen.

Write Markdown into `description` directly. Do not escape newlines.

## When a skill says "publish to the issue tracker"

Create a Linear issue in GTM, attached to the default project.

## When a skill says "fetch the relevant ticket"

`get_issue` with the identifier, then `list_comments` for the discussion.
