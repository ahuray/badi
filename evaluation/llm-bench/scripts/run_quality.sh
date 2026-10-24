#!/usr/bin/env bash
# run_quality.sh [LABEL...] — quality.py for each model in models.json (all if none given), one at a time.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
data=$(python3 -I "$here/bench_paths.py" data)
bin="$data/builds/x64/llama-b11719/llama-server"
labels=("$@")
[[ ${#labels[@]} -gt 0 ]] || mapfile -t labels < <(python3 -I -c 'import json,sys; [print(m["label"]) for m in json.load(open(sys.argv[1]))]' "$here/models.json")
for label in "${labels[@]}"; do
  path=$(python3 -I "$here/bench_paths.py" model "$label")
  [[ -f "$path" ]] || { echo "skip $label: not downloaded"; continue; }
  python3 "$here/quality.py" --server "$bin" --model "$path" --label "$label" \
    --out "$data/results/quality/$label.jsonl" || echo "FAILED $label"
done
