#!/usr/bin/env bash

set -euo pipefail

root_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
e2e_script="$root_dir/verify/e2e.sh"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/bin"
log_file="$tmp_dir/commands.log"

cat >"$tmp_dir/bin/lgs" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$E2E_COMMAND_LOG"

if [[ "$*" == "localnet status --json" ]]; then
  printf '%s\n' '{"ownership":"foreign","listener_present":true}'
  exit 0
fi

printf 'unexpected lgs invocation: %s\n' "$*" >&2
exit 64
EOF
chmod +x "$tmp_dir/bin/lgs"

if E2E_COMMAND_LOG="$log_file" PATH="$tmp_dir/bin:$PATH" "$e2e_script" >"$tmp_dir/stdout" 2>"$tmp_dir/stderr"; then
  fail "a foreign localnet listener must make the walkthrough fail"
fi

rg -q 'foreign listener' "$tmp_dir/stderr" || fail "failure must identify the foreign listener"
[[ $(wc -l <"$log_file") -eq 1 ]] || fail "the script must not reset or stop a foreign listener"

printf 'ok: foreign listener is rejected before reset\n'

cat >"$tmp_dir/bin/lgs" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$E2E_COMMAND_LOG"

case "$*" in
  "localnet status --json")
    if [[ -f "$E2E_RESET_COMPLETE" ]]; then
      printf '%s\n' '{"ownership":"managed","listener_present":true}'
    else
      printf '%s\n' '{"ownership":"stopped","listener_present":false}'
    fi
    ;;
  "localnet reset --yes --reset-wallet")
    touch "$E2E_RESET_COMPLETE"
    ;;
  "build" | "deploy curve --json" | "deploy factory --json")
    if [[ "$*" == "deploy curve --json" && -n "${E2E_CURVE_DEPLOY_FAILURE:-}" ]]; then
      printf '%s\n' '{"deploys":[{"status":"failed","error":"submission rejected"}]}'
    else
      printf '%s\n' '{"deploys":[{"status":"submitted","program_id":"test"}]}'
    fi
    ;;
  "localnet stop") ;;
  *)
    printf 'unexpected lgs invocation: %s\n' "$*" >&2
    exit 64
    ;;
esac
EOF
chmod +x "$tmp_dir/bin/lgs"
cat >"$tmp_dir/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$E2E_COMMAND_LOG"
printf '%s\n' '{"launch_salt":"0000000000000000000000000000000000000000000000000000000000000001"}'
EOF
chmod +x "$tmp_dir/bin/cargo"
: >"$log_file"

if E2E_COMMAND_LOG="$log_file" E2E_RESET_COMPLETE="$tmp_dir/reset" PATH="$tmp_dir/bin:$PATH" \
  "$e2e_script" >"$tmp_dir/stdout" 2>"$tmp_dir/stderr"; then
  fail "the incomplete walkthrough must not report success"
fi

rg -qx 'localnet reset --yes --reset-wallet' "$log_file" \
  || fail "the walkthrough must reset its project-local wallet with the localnet"
rg -qx 'localnet stop' "$log_file" \
  || fail "the walkthrough must stop the managed localnet on exit"
rg -q -- '--json create-sale' "$log_file" \
  || fail "the walkthrough must create the fixture through launchpad JSON output"

printf 'ok: managed localnet is reset and stopped\n'

: >"$log_file"
rm -f "$tmp_dir/reset"

if E2E_COMMAND_LOG="$log_file" E2E_RESET_COMPLETE="$tmp_dir/reset" E2E_CURVE_DEPLOY_FAILURE=1 \
  PATH="$tmp_dir/bin:$PATH" "$e2e_script" >"$tmp_dir/stdout" 2>"$tmp_dir/stderr"; then
  fail "a failed curve deployment must make the walkthrough fail"
fi

rg -q 'curve deployment' "$tmp_dir/stderr" || fail "deployment failure must identify the curve"
if rg -q '^run ' "$log_file"; then
  fail "the walkthrough must not invoke launchpad after a failed deployment"
fi

printf 'ok: failed deployment stops the walkthrough before launchpad\n'
