#!/usr/bin/env bash
#
# Reviewer-facing CLI walkthrough for the factory and curve programs.

set -euo pipefail

root_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

fail() {
  printf 'e2e: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable: $1"
}

require_command jq
require_command lgs

localnet_status=$(lgs localnet status --json) || fail "could not inspect localnet ownership"
ownership=$(jq -er '.ownership' <<<"$localnet_status") \
  || fail "localnet status did not report ownership"

if [[ "$ownership" == "foreign" ]]; then
  fail "foreign listener detected; refusing to reset or stop a localnet not managed by this project"
fi

managed_localnet=false
cleanup() {
  if ! "$managed_localnet"; then
    return
  fi

  local final_status final_ownership
  final_status=$(lgs localnet status --json) || return
  final_ownership=$(jq -er '.ownership' <<<"$final_status") || return
  if [[ "$final_ownership" == "managed" ]]; then
    lgs localnet stop || printf 'e2e: failed to stop managed localnet\n' >&2
  fi
}
trap cleanup EXIT

lgs localnet reset --yes --reset-wallet
managed_localnet=true

assert_json() {
  local checkpoint=$1
  local document=$2
  jq -e . >/dev/null <<<"$document" \
    || fail "$checkpoint did not return valid JSON"
}

assert_submitted_deploy() {
  local checkpoint=$1
  local document=$2

  assert_json "$checkpoint" "$document"
  jq -e '
    (.deploys | type == "array" and length == 1)
    and (.deploys[0].status == "submitted")
    and (.deploys[0].program_id | type == "string" and length > 0)
  ' >/dev/null <<<"$document" \
    || fail "$checkpoint must report one submitted program with its program ID"
}

run_launchpad_json() {
  local checkpoint=$1
  shift

  local response
  response=$(cargo run --quiet -p launchpad-cli -- --json "$@") \
    || fail "$checkpoint command failed"
  assert_json "$checkpoint" "$response"
  printf '%s\n' "$response"
}

lgs build
curve_deploy=$(lgs deploy curve --json) || fail "curve deployment failed"
assert_submitted_deploy "curve deployment" "$curve_deploy"
factory_deploy=$(lgs deploy factory --json) || fail "factory deployment failed"
assert_submitted_deploy "factory deployment" "$factory_deploy"

launch_salt=0000000000000000000000000000000000000000000000000000000000000001
launch=$(run_launchpad_json "factory launch" \
  create-sale \
  --launch-salt "$launch_salt" \
  --name "E2E token" \
  --uri "https://example.invalid/e2e-token.json" \
  --sale-reserve 1000 \
  --dex-seed-reserve 100 \
  --creator-allocation 50 \
  --virtual-token-reserve 2000 \
  --virtual-collateral-reserve 100 \
  --unlock-policy on-close \
  --collateral-definition e2e-collateral)

jq -e --arg launch_salt "$launch_salt" '.launch_salt == $launch_salt' >/dev/null <<<"$launch" \
  || fail "factory launch JSON must report the fixture launch salt"
printf 'factory launch: %s\n' "$(jq -c . <<<"$launch")"

fail "live transaction walkthrough requires a funded collateral definition and initialized curve config; see README.md#admin-authority"
