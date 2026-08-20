#!/usr/bin/env bash
# The LEZ revision is duplicated across two workspaces with two lockfiles, and cargo has
# no way to factor it out. Drift fails silently at runtime: the host and the guest
# disagree on how state serialises and on what PDA seeds hash to. `lgs doctor` does not
# catch it, because it compares the configured pin against scaffold's default rather than
# against our manifests.
#
# Written for bash 3.2, which is what macOS ships.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

manifests="Cargo.toml methods/guest/Cargo.toml"

fail() {
  echo "check-pins: $1" >&2
  exit 1
}

# One rev per dependency line, from every manifest that names the LEZ repository.
revs="$(
  grep -h 'logos-execution-zone' $manifests |
    grep -oE 'rev = "[0-9a-f]{40}"' |
    grep -oE '[0-9a-f]{40}' |
    sort -u
)"

# `scaffold.toml` holds pins for spel, basecamp and lgpm too, so read only the lez block.
pin="$(awk '/^\[repos\.lez\]/{f=1;next} /^\[/{f=0} f' scaffold.toml |
  grep -oE '[0-9a-f]{40}' | head -1)"

[ -n "$pin" ] || fail "no pin found under [repos.lez] in scaffold.toml"
[ -n "$revs" ] || fail "no LEZ rev found in $manifests"

count="$(printf '%s\n' "$revs" | wc -l | tr -d ' ')"
[ "$count" = 1 ] || fail "manifests disagree on the LEZ rev: $(echo "$revs" | tr '\n' ' ')"
[ "$revs" = "$pin" ] || fail "scaffold.toml pins $pin but the manifests use $revs"

echo "check-pins: LEZ pinned at $pin across $manifests and scaffold.toml"
