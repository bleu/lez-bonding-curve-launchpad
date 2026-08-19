# Triage Labels

The skills speak in five canonical triage roles. All five exist in this Linear
workspace under exactly those names, so the mapping is the identity.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

## Reading

An issue sitting in the native `Triage` workflow status counts as `needs-triage`,
even without the label. Inbound issues from integrations land there.

## Writing

Apply the label. Do not move issues into the `Triage` status.

`save_issue.labels` replaces the whole label set, so read the current labels before
adding one or you will drop the others.

## Labels that are not ours

The workspace has several near-synonyms from other workflows. Never use these for
triage state, and never treat them as equivalent to `ready-for-agent`:

`afk-ready`, `agent-ready`, `agent-assist`, `delegable`, `delegatable`,
`exec/agent-ready`, `exec/agent-assist`, `exec/human`, `exec/investigate-only`
