#!/usr/bin/env bash
# Create the native Claude Code and Cursor discovery paths from the canonical
# Codex/Agent Skills files. The links are local generated state, not source.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

fail() {
  echo "sync-agent-skills: $1" >&2
  exit 1
}

for source in .agents/skills/*/SKILL.md; do
  [ -f "$source" ] || fail "no canonical skills found under .agents/skills"

  name="$(basename "$(dirname "$source")")"
  claude=".claude/skills/$name/SKILL.md"
  cursor=".cursor/rules/$name.mdc"

  mkdir -p "$(dirname "$claude")" "$(dirname "$cursor")"
  rm -f "$claude" "$cursor"
  ln -s "../../../.agents/skills/$name/SKILL.md" "$claude"
  ln -s "../../.agents/skills/$name/SKILL.md" "$cursor"
done

echo "sync-agent-skills: linked Claude Code and Cursor to .agents/skills"
