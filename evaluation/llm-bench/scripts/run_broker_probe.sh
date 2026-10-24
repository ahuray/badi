#!/usr/bin/env bash
# run_broker_probe.sh HOME_DIR BROKER BADICTL LABEL [PROBE_ARGS...] — start one broker build in an
# isolated home (HOME_DIR/{home,run,data}; data holds runtimes/ and models/, settings unpaused),
# wait for its runtime warm-up, replay broker_probe.py, then stop that broker by its pid.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
data=$(python3 -I "$here/bench_paths.py" data)
iso=$(cd "$1" && pwd) broker=$2 badictl=$3 label=$4; shift 4
socket=$iso/run/broker.sock log=$data/logs/broker-probe-$label.log
rm -f "$socket"
env -i PATH=/usr/bin HOME="$iso/home" XDG_RUNTIME_DIR="$iso/run" XDG_CONFIG_HOME="$iso/home/.config" \
  XDG_STATE_HOME="$iso/home/.local/state" XDG_DATA_HOME="$iso/home/.local/share" \
  "$broker" --socket "$socket" --model-directory "$iso/data" >"$log" 2>&1 &
pid=$!
trap 'kill -TERM $pid 2>/dev/null; wait $pid 2>/dev/null || true' EXIT
for _ in $(seq 240); do
  grep -q 'warm-up completed' "$log" && [[ -S $socket ]] && break
  kill -0 $pid 2>/dev/null || { echo "broker exited; see $log" >&2; exit 1; }
  sleep 0.25
done
grep -q 'warm-up completed' "$log" || { echo "no warm-up in 60 s; see $log" >&2; exit 1; }
"$here/conditions.sh" "broker-probe-start-$label" >/dev/null
systemd-inhibit --what=idle:sleep --who=llm-bench --why="benchmark run" --mode=block \
  python3 "$here/broker_probe.py" --badictl "$badictl" --socket "$socket" --label "$label" \
  --out "$data/results/broker-probe/$label.jsonl" "$@"
"$here/conditions.sh" "broker-probe-end-$label" >/dev/null
echo "$label: $(grep -c . "$data/results/broker-probe/$label.jsonl") probes"
