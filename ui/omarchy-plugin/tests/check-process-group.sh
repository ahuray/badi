#!/usr/bin/env bash

set -euo pipefail

tests_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck disable=SC1090,SC1091
source "$tests_dir/process-group.sh"

test_root=$(mktemp -d "${TMPDIR:-/tmp}/badi-process-group.XXXXXX")
test_home="$test_root/home"
pid_log="$test_root/pids"
group_pid=""

cleanup() {
  if [[ $group_pid =~ ^[1-9][0-9]*$ ]]; then
    badi_stop_private_process_group "$group_pid" "$test_home" || true
  fi
  rm -rf -- "$test_root"
}
trap cleanup EXIT

mkdir -p "$test_home"
setsid env \
  "HOME=$test_home" \
  "BADI_GROUP_PID_LOG=$pid_log" \
  "$tests_dir/fake-bin/term-ignoring-group" &
group_pid=$!
badi_assert_private_process_group "$group_pid"

for _ in $(seq 1 80); do
  [[ -s $pid_log ]] && break
  kill -0 "$group_pid" 2>/dev/null || break
  sleep 0.05
done
[[ $(wc -l <"$pid_log") == 2 ]] || {
  echo "isolated process-group check: nested TERM-ignoring fixture did not start" >&2
  exit 1
}
mapfile -t live_members < <(badi_live_process_group_members "$group_pid")
((${#live_members[@]} >= 2)) || {
  echo "isolated process-group check: fixture did not create group descendants" >&2
  exit 1
}

badi_stop_private_process_group "$group_pid" "$test_home"
[[ $BADI_PROCESS_GROUP_ESCALATED == 1 ]] || {
  echo "isolated process-group check: TERM-ignoring fixture did not exercise SIGKILL" >&2
  exit 1
}
mapfile -t live_members < <(badi_live_process_group_members "$group_pid")
((${#live_members[@]} == 0)) || {
  echo "isolated process-group check: live watcher survived cleanup" >&2
  exit 1
}
group_pid=""

echo "Badi isolated private process-group cleanup check passed"
