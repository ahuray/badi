#!/usr/bin/env bash
# step4_backends.sh LABEL THREADS [NGL_LIST] — same file: CPU build vs Vulkan (full, partial, none,
# and no op-offload), plus Badi's shape. Logs keep the device/offload lines as evidence.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
data=$(python3 -I "$here/bench_paths.py" data); cd "$data"
label=$1 threads=$2 ngls=${3:-0,8,16,24,99}
model=$(python3 -I "$here/bench_paths.py" model "$label")
cpu=builds/x64/llama-b11719/llama-bench vk=builds/vulkan-x64/llama-b11719/llama-bench
out=results/step4; mkdir -p $out logs
"$here/conditions.sh" "step4-start-$label" >/dev/null
$cpu -m "$model" -ngl 0 -t $threads -p 512 -n 64 -r 5 -o jsonl 2>logs/step4-$label-cpu.stderr >$out/${label}_cpu.jsonl
$vk -m "$model" -ngl $ngls -t $threads -p 512 -n 64 -r 5 -o jsonl 2>logs/step4-$label-vulkan.stderr >$out/${label}_vulkan.jsonl
$vk -m "$model" -ngl 0 -nopo 1 -t $threads -p 512 -n 64 -r 5 -o jsonl 2>>logs/step4-$label-vulkan.stderr >>$out/${label}_vulkan.jsonl
# Badi's shape on each backend.
$cpu -m "$model" -ngl 0 -t $threads -p 48 -n 8 -ub 16 -b 16 -r 10 -o jsonl 2>>logs/step4-$label-cpu.stderr >>$out/${label}_cpu.jsonl
$vk -m "$model" -ngl 99 -t $threads -p 48 -n 8 -ub 16 -b 16 -r 10 -o jsonl 2>>logs/step4-$label-vulkan.stderr >>$out/${label}_vulkan.jsonl
"$here/conditions.sh" "step4-end-$label" >/dev/null
