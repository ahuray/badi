#!/usr/bin/env python3
"""Bytes read per generated token and the bandwidth ceiling for each model on disk.

llama-roofline counts every tensor as streamed per token. Gemma 4's per-layer
token embedding (`per_layer_token_embd`) is a row lookup, so its bytes are
subtracted for the adjusted ceiling. Usage: ceilings.py PEAK_GBS > DATA/results/ceilings.json
"""
import json
import re
import subprocess
import sys

from bench_paths import DATA, model_path, models

ROOFLINE = str(DATA / ".venv" / "bin" / "llama-roofline")
peak = float(sys.argv[1])
out = {}
for m in models():
    path = model_path(m)
    if not path.exists():
        continue
    info = json.loads(subprocess.run([ROOFLINE, "inspect", "--json", str(path)],
                                     capture_output=True, text=True, check=True).stdout)["models"][0]
    tensors = subprocess.run([ROOFLINE, "inspect", "--tensors", str(path)],
                             capture_output=True, text=True, check=True).stdout
    lookup = sum(float(mb) * 1e6 for mb in re.findall(r"per_layer_token_embd\.weight\s+\S+\s+\S+\s+([\d.]+) MB", tensors))
    streamed = info["streamed_bytes"]
    out[m["label"]] = {"file_bytes": info["file_bytes"], "architecture": info.get("architecture"),
                       "bits_per_weight": info.get("bits_per_weight"),
                       "bytes_per_token": streamed - lookup, "lookup_bytes_excluded": lookup,
                       "ceiling_tok_s": round(peak * 1e9 / (streamed - lookup), 2)}
print(json.dumps({"peak_read_GBs": peak, "models": out}, indent=1))
