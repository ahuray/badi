#!/usr/bin/env bash

set -euo pipefail

tests_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
fake_badictl="$tests_dir/fake-bin/badictl"
test_root=$(mktemp -d "${TMPDIR:-/tmp}/badi-term-ignoring.XXXXXX")
pid_log="$test_root/pids"
child_pid=""

cleanup() {
  if [[ -n $child_pid ]] && kill -0 "$child_pid" 2>/dev/null; then
    kill -KILL "$child_pid" 2>/dev/null || true
    wait "$child_pid" 2>/dev/null || true
  fi
  rm -rf -- "$test_root"
}
trap cleanup EXIT

BADI_FAKE_SCENARIO=term-ignoring \
  BADI_FAKE_PID_LOG="$pid_log" \
  "$fake_badictl" overview --json &
child_pid=$!

for ((attempt = 0; attempt < 100; attempt += 1)); do
  [[ -s $pid_log ]] && break
  sleep 0.01
done
[[ -s $pid_log ]] || {
  echo "TERM-ignoring fixture did not report its process ID" >&2
  exit 1
}

recorded_pid=$(head -n 1 "$pid_log")
[[ $recorded_pid == "$child_pid" ]] || {
  echo "TERM-ignoring fixture changed process ID before exec" >&2
  exit 1
}

kill -TERM "$child_pid"
sleep 0.1
kill -0 "$child_pid" 2>/dev/null || {
  echo "TERM-ignoring fixture unexpectedly exited on SIGTERM" >&2
  exit 1
}

kill -KILL "$child_pid"
wait "$child_pid" 2>/dev/null || true
if kill -0 "$child_pid" 2>/dev/null; then
  echo "TERM-ignoring fixture survived SIGKILL" >&2
  exit 1
fi
child_pid=""

echo "Badi Omarchy TERM-ignoring process fixture check passed"
