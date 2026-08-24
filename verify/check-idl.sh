#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

for program in curve factory private_buy; do
  generated=$(mktemp)
  trap 'rm -f "$generated"' EXIT
  lgs spel -- generate-idl "idl-src/${program}.rs" > "$generated"
  cmp --silent "$generated" "idl/${program}.json" || {
    echo "IDL for ${program} is stale; run ./verify/generate-idl.sh" >&2
    exit 1
  }
  rm -f "$generated"
  trap - EXIT
done
