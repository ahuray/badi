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

# Hold the parent stopped so it cannot reap its child. This makes the zombie
# case deterministic on both an ordinary desktop and a container without init.
rm -- "$pid_log"
# Child-shell variables must expand in that child, not in this test driver.
# shellcheck disable=SC2016
setsid env "HOME=$test_home" bash -c '
  sleep 300 &
  child=$!
  printf "%s\n" "$child" >"$1"
  kill -STOP "$$"
  wait "$child" 2>/dev/null || true
' bash "$pid_log" >"$test_root/zombie-parent.log" 2>&1 &
group_pid=$!
badi_assert_private_process_group "$group_pid"
for _ in $(seq 1 80); do
  [[ -s $pid_log && $(ps -o stat= -p "$group_pid") == T* ]] && break
  sleep 0.01
done
[[ -s $pid_log && $(ps -o stat= -p "$group_pid") == T* ]] || {
  echo "isolated process-group check: zombie fixture parent did not stop" >&2
  exit 1
}
child_pid=$(cat -- "$pid_log")
badi_verify_process_group_members "$group_pid" "$test_home"
if badi_process_has_exited "$child_pid"; then
  echo "isolated process-group check: live child was mistaken for an exited process" >&2
  exit 1
fi
kill -KILL "$child_pid"
for _ in $(seq 1 80); do
  [[ $(ps -o stat= -p "$child_pid") == Z* ]] && break
  sleep 0.01
done
[[ $(ps -o stat= -p "$child_pid") == Z* ]]
kill -0 "$child_pid" # Demonstrate why kill -0 alone cannot prove liveness.
badi_process_has_exited "$child_pid" || {
  echo "isolated process-group check: terminated zombie was mistaken for a live child" >&2
  exit 1
}
kill -CONT "$group_pid"
wait "$group_pid"
badi_process_has_exited "$child_pid"
badi_process_has_exited "$group_pid"
group_pid=""

echo "Badi isolated private process-group cleanup check passed"
