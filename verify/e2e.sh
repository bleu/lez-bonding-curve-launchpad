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

: "${GENESIS_ADMIN_ACCOUNT:?set GENESIS_ADMIN_ACCOUNT to the configured curve genesis-admin wallet account}"

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

expect_launchpad_error() {
  local checkpoint=$1
  local expected_category=$2
  shift 2

  local response
  if response=$(cargo run --quiet -p launchpad-cli -- --json "$@" 2>&1); then
    fail "$checkpoint unexpectedly succeeded"
  fi
  assert_json "$checkpoint" "$response"
  jq -e --arg category "$expected_category" \
    '.status == "error" and .error.category == $category' \
    >/dev/null <<<"$response" \
    || fail "$checkpoint did not return error category $expected_category"
  printf '%s\n' "$response"
}

new_public_account() {
  local account
  account=$(LOGOS_SCAFFOLD_QUIET=1 lgs wallet -- account new public) \
    || fail "could not create a fixture public account"
  account=$(sed -n 's/^Public\///p' <<<"$account" | tail -n 1)
  [[ -n "$account" ]] || fail "wallet did not return a public account"
  printf '%s\n' "$account"
}

find_guest_binary() {
  local program=$1
  local guest_root candidate

  # `lgs build` supports both the root target directory and the excluded
  # methods-workspace target directory. Prefer the former when a custom build
  # puts artifacts there, then fall back to this repository's default layout.
  for guest_root in target/riscv-guest methods/target/riscv-guest; do
    candidate=$(find "$guest_root" -type f -name "$program.bin" -print -quit 2>/dev/null || true)
    if [[ -n "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

topup() {
  LOGOS_SCAFFOLD_QUIET=1 lgs wallet topup "Public/$1" \
    || fail "could not fund fixture account $1"
}

lgs build
curve_deploy=$(lgs deploy curve --json) || fail "curve deployment failed"
assert_submitted_deploy "curve deployment" "$curve_deploy"
factory_deploy=$(lgs deploy factory --json) || fail "factory deployment failed"
assert_submitted_deploy "factory deployment" "$factory_deploy"

curve_program_path=$(find_guest_binary curve || true)
factory_program_path=$(find_guest_binary factory || true)
[[ -n "$curve_program_path" && -n "$factory_program_path" ]] \
  || fail "lgs build did not produce the curve and factory guest binaries under target/riscv-guest or methods/target/riscv-guest"

creator=${GENESIS_ADMIN_ACCOUNT#*/}
treasury=$creator
buyer_one=$(new_public_account)
buyer_two=$(new_public_account)
buyer_three=$(new_public_account)
for buyer in "$buyer_one" "$buyer_two" "$buyer_three"; do
  topup "$buyer"
done

collateral_definition=$(new_public_account)
collateral_supply=$(new_public_account)
LOGOS_SCAFFOLD_QUIET=1 lgs wallet -- token new \
  --definition-account-id "Public/$collateral_definition" \
  --supply-account-id "Public/$collateral_supply" \
  --name "E2E collateral" \
  --total-supply 100000 \
  || fail "could not create the collateral token"
for buyer in "$buyer_one" "$buyer_two" "$buyer_three"; do
  LOGOS_SCAFFOLD_QUIET=1 lgs wallet -- token mint \
    --definition "Public/$collateral_definition" \
    --holder "Public/$buyer" \
    --amount 10000 \
    || fail "could not fund buyer $buyer with collateral"
done

config=$(run_launchpad_json "curve configuration" configure \
  --admin "Public/$creator" \
  --treasury "Public/$treasury" \
  --curve-program-path "$curve_program_path")
jq -e '.status == "submitted" and (.transaction_hash | type == "string" and length > 0)' \
  >/dev/null <<<"$config" \
  || fail "curve configuration JSON did not report submission"

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
  --collateral-definition "Public/$collateral_definition" \
  --creator "Public/$creator" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path")

jq -e --arg launch_salt "$launch_salt" '.launch_salt == $launch_salt' >/dev/null <<<"$launch" \
  || fail "factory launch JSON must report the fixture launch salt"
printf 'factory launch: %s\n' "$(jq -c . <<<"$launch")"

expect_launchpad_error "slippage floor" slippage_floor buy \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --participant "Public/$buyer_one" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path" \
  --tokens 1 \
  --max-collateral 0

expect_launchpad_error "sale reserve overshoot" sale_reserve_overshoot buy \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --participant "Public/$buyer_one" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path" \
  --tokens 1001 \
  --max-collateral 100000

expect_launchpad_error "collateral reserve overshoot" collateral_reserve_overshoot sell \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --participant "Public/$buyer_one" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path" \
  --tokens 1000 \
  --min-collateral 0

collateral_quote=$(run_launchpad_json "collateral buy quote" price \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path" \
  --collateral 25)
min_tokens=$(jq -er '.amount_out' <<<"$collateral_quote")
collateral_buy=$(run_launchpad_json "collateral buy" buy-with-collateral \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --participant "Public/$buyer_one" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path" \
  --collateral 25 \
  --min-tokens "$min_tokens")
jq -e '.status == "submitted"' >/dev/null <<<"$collateral_buy" \
  || fail "collateral buy JSON did not report submission"

for buyer in "$buyer_two" "$buyer_three"; do
  quote=$(run_launchpad_json "buy quote" price \
    --launch-salt "$launch_salt" \
    --collateral-definition "Public/$collateral_definition" \
    --factory-program-path "$factory_program_path" \
    --curve-program-path "$curve_program_path" \
    --tokens 250)
  max_collateral=$(jq -er '.amount_in' <<<"$quote")
  buy=$(run_launchpad_json "buy" buy \
    --launch-salt "$launch_salt" \
    --collateral-definition "Public/$collateral_definition" \
    --participant "Public/$buyer" \
    --factory-program-path "$factory_program_path" \
    --curve-program-path "$curve_program_path" \
    --tokens 250 \
    --max-collateral "$max_collateral")
  jq -e '.status == "submitted"' >/dev/null <<<"$buy" \
    || fail "buy JSON did not report submission"
done

sell=$(run_launchpad_json "sell" sell \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --participant "Public/$buyer_three" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path" \
  --tokens 50 \
  --min-collateral 0)
jq -e '.status == "submitted"' >/dev/null <<<"$sell" \
  || fail "sell JSON did not report submission"

before_terminal=$(run_launchpad_json "terminal status" status \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path")
remaining=$(jq -er '.real_token_reserve' <<<"$before_terminal")
[[ "$remaining" != 0 ]] || fail "sale closed before the terminal buy"
jq -e '
  (.sale_quantity | type == "number" and . > 0)
  and (.tokens_sold | type == "number" and . >= 0)
  and (.tokens_sold <= .sale_quantity)
' >/dev/null <<<"$before_terminal" \
  || fail "status must report bounded sale progress"
terminal_quote=$(run_launchpad_json "terminal buy quote" price \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path" \
  --tokens "$remaining")
terminal_cap=$(jq -er '.amount_in' <<<"$terminal_quote")
terminal=$(run_launchpad_json "terminal buy" buy \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --participant "Public/$buyer_one" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path" \
  --tokens "$remaining" \
  --max-collateral "$terminal_cap")
jq -e '.status == "submitted"' >/dev/null <<<"$terminal" \
  || fail "terminal buy JSON did not report submission"

after_terminal=$(run_launchpad_json "auto-close status" status \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path")
jq -e '.status == "closed"' >/dev/null <<<"$after_terminal" \
  || fail "terminal buy did not auto-close the sale"
jq -e '.tokens_sold == .sale_quantity' >/dev/null <<<"$after_terminal" \
  || fail "closed factory sale must report complete supply progress"

unlock=$(run_launchpad_json "creator unlock" unlock \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --creator "Public/$creator" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path")
jq -e '.status == "submitted"' >/dev/null <<<"$unlock" \
  || fail "creator unlock JSON did not report submission"

withdraw=$(run_launchpad_json "creator withdrawal" withdraw \
  --launch-salt "$launch_salt" \
  --collateral-definition "Public/$collateral_definition" \
  --creator "Public/$creator" \
  --factory-program-path "$factory_program_path" \
  --curve-program-path "$curve_program_path")
jq -e '.status == "submitted"' >/dev/null <<<"$withdraw" \
  || fail "creator withdrawal JSON did not report submission"

printf 'walkthrough complete: launch, buys, sell, auto-close, unlock, withdrawal\n'
