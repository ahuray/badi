#!/usr/bin/env bash

set -euo pipefail

scenario=${1:-healthy}
summon_cycles=${BADI_SUMMON_CYCLES:-1}
case "$scenario" in
healthy | unavailable | degraded | stale | capacity | memory-repair | malformed | timeout | term-ignoring) ;;
*)
  echo "usage: $0 [healthy|unavailable|degraded|stale|capacity|memory-repair|malformed|timeout|term-ignoring]" >&2
  exit 64
  ;;
esac
[[ $summon_cycles =~ ^[1-9][0-9]*$ && $summon_cycles -le 1000 ]] || {
  echo "BADI_SUMMON_CYCLES must be an integer from 1 through 1000" >&2
  exit 64
}

artifact_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
tests_dir="$artifact_dir/tests"
# shellcheck disable=SC1090,SC1091
source "$tests_dir/process-group.sh"
omarchy_root=${BADI_OMARCHY_ROOT:-/usr/share/omarchy}
[[ -f $omarchy_root/shell/shell.qml ]] || {
  echo "isolated plugin check: Omarchy shell not found at $omarchy_root" >&2
  exit 1
}

test_root=$(mktemp -d /tmp/badi-omarchy-plugin.XXXXXX)
shell_copy="$test_root/shell"
test_home="$test_root/home"
runtime_dir="$test_root/runtime"
plugin_target="$test_home/.config/omarchy/plugins/io.github.ahuray.badi"
call_log="$test_root/badictl.calls"
pid_log="$test_root/badictl.pids"
shell_log="$test_root/shell.log"
shell_pid=""

stop_shell_group() {
  badi_stop_private_process_group "$shell_pid" "$test_home"
  shell_pid=""
}

cleanup() {
  stop_shell_group || true
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
  if [[ ${BADI_KEEP_ISOLATED_ROOT:-0} == 1 ]]; then
    echo "isolated plugin check preserved: $test_root"
  else
    rm -rf -- "$test_root"
  fi
}
trap cleanup EXIT

mkdir -p "$plugin_target" "$runtime_dir" \
  "$test_root/config/omarchy" \
  "$test_home/.config/omarchy" "$test_home/.local/share" \
  "$test_home/.cache" "$test_home/.local/state"
chmod 700 "$runtime_dir" "$test_home/.config" "$test_home/.config/omarchy"
cp -a -- "$omarchy_root/shell" "$shell_copy"
cp -- "$omarchy_root/config/omarchy/shell.json" \
  "$test_root/config/omarchy/shell.json"
cp -a -- "$artifact_dir/." "$plugin_target/"

# Keep Quickshell's IPC/log namespace isolated while connecting to the current
# graphical session through explicit socket links owned by this temporary tree.
host_runtime_dir=${XDG_RUNTIME_DIR:-}
wayland_display=${WAYLAND_DISPLAY:-}
[[ -n $host_runtime_dir && -n $wayland_display \
  && -S $host_runtime_dir/$wayland_display ]] || {
  echo "isolated plugin check: an active Wayland session is required" >&2
  exit 1
}
ln -s -- "$host_runtime_dir/$wayland_display" "$runtime_dir/$wayland_display"
if [[ -e $host_runtime_dir/$wayland_display.lock ]]; then
  ln -s -- "$host_runtime_dir/$wayland_display.lock" \
    "$runtime_dir/$wayland_display.lock"
fi
if [[ -d $host_runtime_dir/hypr ]]; then
  ln -s -- "$host_runtime_dir/hypr" "$runtime_dir/hypr"
fi

jq -n '{
  version: 1,
  badiIsolatedSentinel: {preserve: true},
  bar: {
    position: "top",
    transparent: false,
    centerAnchor: "",
    layout: {left: [], center: [], right: []}
  },
  plugins: []
}' >"$test_home/.config/omarchy/shell.json"

runtime_env=(
  env
  "HOME=$test_home"
  "XDG_CONFIG_HOME=$test_home/.config"
  "XDG_DATA_HOME=$test_home/.local/share"
  "XDG_CACHE_HOME=$test_home/.cache"
  "XDG_STATE_HOME=$test_home/.local/state"
  "XDG_RUNTIME_DIR=$runtime_dir"
  "OMARCHY_PATH=$test_root"
  "PATH=$artifact_dir/tests/fake-bin:$PATH"
  "BADI_FAKE_SCENARIO=$scenario"
  "BADI_FAKE_CALL_LOG=$call_log"
  "BADI_FAKE_PID_LOG=$pid_log"
  "QT_QPA_PLATFORM=wayland"
  "NO_COLOR=1"
)

setsid "${runtime_env[@]}" qs --no-color -p "$shell_copy" >"$shell_log" 2>&1 &
shell_pid=$!
badi_assert_private_process_group "$shell_pid"

ready=0
for _ in $(seq 1 80); do
  if ! kill -0 "$shell_pid" 2>/dev/null; then
    cat "$shell_log" >&2
    echo "isolated plugin check: copied shell exited before IPC was ready" >&2
    exit 1
  fi
  if [[ $("${runtime_env[@]}" qs --no-color -p "$shell_copy" ipc call shell ping \
      2>/dev/null || true) == ok ]]; then
    ready=1
    break
  fi
  sleep 0.05
done
[[ $ready == 1 ]] || {
  cat "$shell_log" >&2
  echo "isolated plugin check: copied shell IPC did not become ready" >&2
  exit 1
}

plugin_state() {
  "${runtime_env[@]}" qs --no-color -p "$shell_copy" \
    ipc call shell listPlugins |
    jq -r --arg id io.github.ahuray.badi \
      '[.[] | select(.id == $id) | (.enabled | tostring)][0] // "missing"'
}

wait_for_plugin_state() {
  local expected=$1
  local state=""
  for _ in $(seq 1 80); do
    state=$(plugin_state)
    [[ $state == "$expected" ]] && return 0
    sleep 0.05
  done
  cat "$shell_log" >&2
  echo "isolated plugin check: expected plugin state $expected, observed $state" >&2
  return 1
}

# A third-party plugin present on disk is discoverable but disabled until the
# owner explicitly enables it. Rescan must retain that default.
wait_for_plugin_state false
disabled_summon=$("${runtime_env[@]}" qs --no-color -p "$shell_copy" \
  ipc call shell summon io.github.ahuray.badi '{}')
[[ $disabled_summon == unknown && ! -e $call_log ]]
"${runtime_env[@]}" qs --no-color -p "$shell_copy" \
  ipc call shell rescanPlugins >/dev/null
wait_for_plugin_state false

"${runtime_env[@]}" omarchy-plugin-enable io.github.ahuray.badi >/dev/null
wait_for_plugin_state true
jq -e '
  .badiIsolatedSentinel == {preserve: true}
  and any(.plugins[]; .id == "io.github.ahuray.badi")
' "$test_home/.config/omarchy/shell.json" >/dev/null

for cycle in $(seq 1 "$summon_cycles"); do
  summoned=$("${runtime_env[@]}" qs --no-color -p "$shell_copy" \
    ipc call shell summon io.github.ahuray.badi '{}')
  [[ $summoned == ok ]]

  if [[ $cycle == 1 ]]; then
    for _ in $(seq 1 80); do
      [[ -s $call_log ]] && break
      sleep 0.05
    done
    grep -Fx 'overview --json' "$call_log" >/dev/null
  fi

  "${runtime_env[@]}" qs --no-color -p "$shell_copy" \
    ipc call shell hide io.github.ahuray.badi >/dev/null

  if [[ $scenario == term-ignoring && $cycle == 1 ]]; then
    first_pid=$(head -n 1 "$pid_log")
    calls_after_close=$(wc -l <"$call_log")
    sleep 0.7
    ! kill -0 "$first_pid" 2>/dev/null || {
      echo "isolated plugin check: first TERM-ignoring probe survived close" >&2
      exit 1
    }
    [[ $(wc -l <"$call_log") == "$calls_after_close" ]] || {
      echo "isolated plugin check: Process.onExited relaunched after close" >&2
      exit 1
    }

    # Start another hanging probe, then hide and immediately reopen before
    # waiting for teardown. The fresh lifecycle must launch one replacement.
    pre_teardown=$("${runtime_env[@]}" qs --no-color -p "$shell_copy" \
      ipc call shell summon io.github.ahuray.badi '{}')
    [[ $pre_teardown == ok ]]
    calls_before_teardown=$calls_after_close
    for _ in $(seq 1 80); do
      calls_during_teardown=$(wc -l <"$call_log")
      ((calls_during_teardown == calls_before_teardown + 1)) && break
      sleep 0.05
    done
    ((calls_during_teardown == calls_before_teardown + 1))
    "${runtime_env[@]}" qs --no-color -p "$shell_copy" \
      ipc call shell hide io.github.ahuray.badi >/dev/null

    calls_before_reopen=$(wc -l <"$call_log")
    reopened=$("${runtime_env[@]}" qs --no-color -p "$shell_copy" \
      ipc call shell summon io.github.ahuray.badi '{}')
    [[ $reopened == ok ]]
    for _ in $(seq 1 80); do
      calls_after_reopen=$(wc -l <"$call_log")
      ((calls_after_reopen == calls_before_reopen + 1)) && break
      sleep 0.05
    done
    ((calls_after_reopen == calls_before_reopen + 1))
    "${runtime_env[@]}" qs --no-color -p "$shell_copy" \
      ipc call shell hide io.github.ahuray.badi >/dev/null
  fi
done

# Exercise the host's toggle path separately from direct summon/hide. The
# additional overview call proves that toggle reopened the panel.
calls_before_toggle=$(wc -l <"$call_log")
"${runtime_env[@]}" qs --no-color -p "$shell_copy" \
  ipc call shell toggle io.github.ahuray.badi '{}' >/dev/null
for _ in $(seq 1 80); do
  calls_after_toggle=$(wc -l <"$call_log")
  ((calls_after_toggle > calls_before_toggle)) && break
  sleep 0.05
done
((calls_after_toggle > calls_before_toggle))
"${runtime_env[@]}" qs --no-color -p "$shell_copy" \
  ipc call shell toggle io.github.ahuray.badi '{}' >/dev/null

"${runtime_env[@]}" omarchy-plugin-disable io.github.ahuray.badi >/dev/null
wait_for_plugin_state false
jq -e '
  .badiIsolatedSentinel == {preserve: true}
  and all(.plugins[]; .id != "io.github.ahuray.badi")
' "$test_home/.config/omarchy/shell.json" >/dev/null
disabled_summon=$("${runtime_env[@]}" qs --no-color -p "$shell_copy" \
  ipc call shell summon io.github.ahuray.badi '{}')
[[ $disabled_summon == unknown ]]

# The official remover acts only inside this temporary HOME. A plain copied
# artifact is moved to a recoverable hidden backup and then explicitly
# rescanned out of the copied shell.
"${runtime_env[@]}" omarchy-plugin-remove io.github.ahuray.badi --yes >/dev/null
wait_for_plugin_state missing

[[ $("${runtime_env[@]}" qs --no-color -p "$shell_copy" \
  ipc call shell ping) == ok ]]
jq -e '.badiIsolatedSentinel == {preserve: true}' \
  "$test_home/.config/omarchy/shell.json" >/dev/null

children_reaped=0
for _ in $(seq 1 80); do
  if ! pgrep -f -- "$artifact_dir/tests/fake-bin/badictl" >/dev/null; then
    children_reaped=1
    break
  fi
  sleep 0.05
done
[[ $children_reaped == 1 ]] || {
  echo "isolated plugin check: fake badictl child survived panel teardown" >&2
  exit 1
}

if [[ $scenario == term-ignoring ]]; then
  fake_pids_reaped=0
  for _ in $(seq 1 80); do
    alive=0
    while IFS= read -r pid; do
      if [[ $pid =~ ^[1-9][0-9]*$ ]] && kill -0 "$pid" 2>/dev/null; then
        alive=1
        break
      fi
    done <"$pid_log"
    if [[ $alive == 0 ]]; then
      fake_pids_reaped=1
      break
    fi
    sleep 0.05
  done
  [[ $fake_pids_reaped == 1 ]] || {
    echo "isolated plugin check: TERM-ignoring badictl survived panel teardown" >&2
    exit 1
  }
  [[ $(wc -l <"$pid_log") -ge 3 ]] || {
    echo "isolated plugin check: close/reopen did not launch fresh probes" >&2
    exit 1
  }
fi

if grep -En 'panel plugin io\.github\.ahuray\.badi failed to load|TypeError|ReferenceError' \
    "$shell_log"; then
  cat "$shell_log" >&2
  echo "isolated plugin check: plugin load/runtime error" >&2
  exit 1
fi

stop_shell_group
echo "Badi Omarchy plugin isolated $scenario check passed ($summon_cycles summon/hide cycles)"
