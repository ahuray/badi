#!/usr/bin/env bash

set -euo pipefail

tests_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck disable=SC1090,SC1091
source "$tests_dir/process-group.sh"
artifact_dir=$(cd -- "$tests_dir/.." && pwd)
harness_source="$tests_dir/client-harness/shell.qml"
test_root=$(mktemp -d "${TMPDIR:-/tmp}/badi-client-lifecycle.XXXXXX")
harness_dir="$test_root/harness"
runtime_dir="$test_root/runtime"
call_log="$test_root/badictl.calls"
pid_log="$test_root/badictl.pids"
shell_log="$test_root/quickshell.log"
shell_pid=""

cleanup() {
  if [[ $shell_pid =~ ^[1-9][0-9]*$ ]]; then
    badi_stop_private_process_group "$shell_pid" "$test_root" || true
    shell_pid=""
  fi
  if [[ -f $pid_log ]]; then
    while IFS= read -r pid; do
      [[ $pid =~ ^[1-9][0-9]*$ ]] || continue
      [[ -r /proc/$pid/environ ]] || continue
      if tr '\0' '\n' <"/proc/$pid/environ" 2>/dev/null |
          grep -Fx "BADI_FAKE_PID_LOG=$pid_log" >/dev/null; then
        kill -KILL "$pid" 2>/dev/null || true
      fi
    done <"$pid_log"
  fi
  rm -rf -- "$test_root"
}
trap cleanup EXIT

command -v qs >/dev/null 2>&1 || {
  echo "client lifecycle check: Quickshell is required" >&2
  exit 1
}

mkdir -p "$harness_dir" "$runtime_dir" "$test_root/config" "$test_root/cache" \
  "$test_root/data" "$test_root/state"
chmod 700 "$runtime_dir" "$test_root/config"
cp -- "$harness_source" "$harness_dir/shell.qml"
cp -- "$artifact_dir/BadiClient.qml" "$harness_dir/BadiClient.qml"

runtime_env=(
  env
  "HOME=$test_root"
  "XDG_CONFIG_HOME=$test_root/config"
  "XDG_DATA_HOME=$test_root/data"
  "XDG_CACHE_HOME=$test_root/cache"
  "XDG_STATE_HOME=$test_root/state"
  "XDG_RUNTIME_DIR=$runtime_dir"
  "PATH=$tests_dir/fake-bin:$PATH"
  "BADI_FAKE_SCENARIO=term-ignoring-mutation"
  "BADI_FAKE_CALL_LOG=$call_log"
  "BADI_FAKE_PID_LOG=$pid_log"
  "QT_QPA_PLATFORM=offscreen"
  "NO_COLOR=1"
)

setsid "${runtime_env[@]}" qs --no-color -p "$harness_dir" >"$shell_log" 2>&1 &
shell_pid=$!
badi_assert_private_process_group "$shell_pid"

ipc() {
  "${runtime_env[@]}" qs --no-color -p "$harness_dir" ipc call \
    badi-client-lifecycle "$@"
}

for _ in $(seq 1 80); do
  if ! kill -0 "$shell_pid" 2>/dev/null; then
    cat "$shell_log" >&2
    echo "client lifecycle check: Quickshell exited before IPC was ready" >&2
    exit 1
  fi
  [[ $(ipc ping 2>/dev/null || true) == ok ]] && break
  sleep 0.05
done
[[ $(ipc ping 2>/dev/null || true) == ok ]] || {
  cat "$shell_log" >&2
  echo "client lifecycle check: IPC did not become ready" >&2
  exit 1
}

wait_for_call_count() {
  local expected=$1
  local observed=0
  for _ in $(seq 1 120); do
    [[ -f $call_log ]] && observed=$(wc -l <"$call_log")
    ((observed == expected)) && return 0
    ((observed > expected)) && break
    sleep 0.05
  done
  cat "$shell_log" >&2
  echo "client lifecycle check: expected $expected calls, observed $observed" >&2
  return 1
}

wait_until_idle() {
  local state=""
  for _ in $(seq 1 120); do
    state=$(ipc state)
    [[ $(jq -r '.busy' <<<"$state") == false ]] && return 0
    sleep 0.05
  done
  cat "$shell_log" >&2
  echo "client lifecycle check: client stayed busy: $state" >&2
  return 1
}

wait_for_pid_exit() {
  local pid=$1
  for _ in $(seq 1 80); do
    ! kill -0 "$pid" 2>/dev/null && return 0
    sleep 0.05
  done
  echo "client lifecycle check: TERM-ignoring PID $pid survived teardown" >&2
  return 1
}

# Closed mutation: its stale exit must not refresh the inactive client.
ipc activate >/dev/null
wait_for_call_count 1
wait_until_idle
state=$(ipc state)
jq -e '
  .overviewSchema == "badi.overview.v2"
  and .supportScope == "verified_test_cells_only"
  and .supportGeneralization == "none"
  and .supportAuthorization == "not_granted_by_evidence"
  and .verifiedSupportCells == 3
  and .browserSupportActivation == "always"
  and .nativeSupportActivation == "explicit_manual"
  and .settingsSchema == "badi.settings.v2"
  and .settingsDocumentValid == true
  and .subjectCount == 2
  and .targetSubjectIndex == 0
' <<<"$state" >/dev/null
legacy_settings='{"schema":"badi.settings.v1","revision":0,"paused":true,"subjects":[]}'
[[ $(ipc validateSettings "$legacy_settings") == false ]]
invalid_linux_settings=$(jq -cn '{
  schema: "badi.settings.v2",
  revision: 1,
  paused: false,
  subjects: [{
    identity: {kind: "linux_app", adapter: "fcitx", app_id: "Omawrite Window"},
    permissions: {
      suggest: "block", display: "block", context_read: "block", learn: "block",
      retention: {mode: "none"}
    }
  }]
}')
[[ $(ipc validateSettings "$invalid_linux_settings") == false ]]
linux_learning_settings=$(jq -cn '{
  schema: "badi.settings.v2",
  revision: 1,
  paused: false,
  subjects: [{
    identity: {kind: "linux_app", adapter: "fcitx", app_id: "omawrite"},
    permissions: {
      suggest: "allow", display: "allow", context_read: "allow", learn: "allow",
      retention: {mode: "none"}
    }
  }]
}')
[[ $(ipc validateSettings "$linux_learning_settings") == false ]]

# A browser-policy mutation must preserve the native Fcitx rule that the same
# settings v2 document carries through the control center.
ipc blockTarget >/dev/null
wait_for_call_count 3
wait_until_idle
replacement=$(sed -n '2p' "$call_log" | cut -d' ' -f6-)
jq -e '
  .schema == "badi.settings.v2"
  and any(.subjects[];
    .identity == {kind: "linux_app", adapter: "fcitx", app_id: "omawrite"})
' <<<"$replacement" >/dev/null

ipc clearMemory >/dev/null
wait_for_call_count 4
first_pid=$(head -n 1 "$pid_log")
ipc deactivate >/dev/null
sleep 0.7
wait_for_pid_exit "$first_pid"
wait_for_call_count 4
[[ $(jq -r '.active' <<<"$(ipc state)") == false ]]

# Reopen while the second TERM-ignoring mutation is still tearing down. The
# stale mutation result is discarded and exactly one fresh overview is queued.
ipc activate >/dev/null
wait_for_call_count 5
wait_until_idle
ipc clearMemory >/dev/null
wait_for_call_count 6
second_pid=$(tail -n 1 "$pid_log")
ipc deactivate >/dev/null
ipc activate >/dev/null
wait_for_pid_exit "$second_pid"
wait_for_call_count 7
wait_until_idle
sleep 0.2
wait_for_call_count 7
state=$(ipc state)
[[ $(jq -r '.active' <<<"$state") == true ]]
[[ $(jq -r '.refreshQueued' <<<"$state") == false ]]

ipc deactivate >/dev/null

if grep -En 'TypeError|ReferenceError|Cannot assign|failed to load' "$shell_log"; then
  cat "$shell_log" >&2
  echo "client lifecycle check: QML runtime error" >&2
  exit 1
fi

badi_stop_private_process_group "$shell_pid" "$test_root"
shell_pid=""
echo "Badi Omarchy client mutation lifecycle check passed"
