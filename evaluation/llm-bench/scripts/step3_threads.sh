#!/usr/bin/env bash
# step3_threads.sh LABEL — CPU-only thread sweep (brief Step 3) plus Badi-shaped and pinned variants.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
data=$(python3 -I "$here/bench_paths.py" data); cd "$data"
label=${1:-granite4-micro-base}
model=$(python3 -I "$here/bench_paths.py" model "$label")
bench=builds/x64/llama-b11719/llama-bench
out=results/step3; mkdir -p $out logs
"$here/conditions.sh" "step3-start-$label" >/dev/null
# The brief's sweep.
$bench -m "$model" -ngl 0 -t 4,6,8,10,14,20 -p 512 -n 64 -r 5 -o jsonl 2>logs/step3-sweep.stderr >$out/sweep_$label.jsonl
# Badi's shape: ~48-token prompt in 16-token ubatches, 8 generated tokens.
$bench -m "$model" -ngl 0 -t 2,4,6,8,10 -p 48 -n 8 -ub 16 -b 16 -r 10 -o jsonl 2>>logs/step3-sweep.stderr >$out/badi_shape_$label.jsonl
# Pinning: one thread per P-core (CPUs 0,2,..,10), all 12 P-core threads, and 4 P-cores.
for spec in "6 0x555" "12 0xfff" "4 0x55"; do
  set -- $spec
  $bench -m "$model" -ngl 0 -t $1 -C $2 --cpu-strict 1 -p 512 -n 64 -r 5 -o jsonl 2>>logs/step3-sweep.stderr >>$out/pinned_$label.jsonl
done
"$here/conditions.sh" "step3-end-$label" >/dev/null
