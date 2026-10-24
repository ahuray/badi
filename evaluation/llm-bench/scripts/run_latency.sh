#!/usr/bin/env bash
# run_latency.sh OUTDIR "LATENCY_ARGS" LABEL[:VARIANT_EXTRA]... — latency.py per model, one at a time,
# under a logind idle inhibitor so Omarchy's idle handler cannot drop the power profile mid-run.
# A variant is written as label@name=extra-args, e.g. "qwen3.5-2b-base@cms32=-cms 32".
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
data=$(python3 -I "$here/bench_paths.py" data)
outdir=$1 common=$2; shift 2
bin=${LATENCY_BIN:-$data/builds/x64/llama-b11719/llama-server}
for spec in "$@"; do
  label=${spec%%@*} variant="" extra=""
  if [[ $spec == *@* ]]; then rest=${spec#*@}; variant=${rest%%=*}; extra=${rest#*=}; fi
  path=$(python3 -I "$here/bench_paths.py" model "$label")
  name=$label${variant:+-$variant}
  "$here/conditions.sh" "latency-start-$name" >/dev/null
  systemd-inhibit --what=idle:sleep --who=llm-bench --why="benchmark run" --mode=block \
    python3 "$here/latency.py" --server "$bin" --model "$path" --label "$name" \
    --out "$data/$outdir/$name.jsonl" $common --extra="$extra" || echo "FAILED $name"
  "$here/conditions.sh" "latency-end-$name" >/dev/null
done
