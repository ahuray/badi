#!/usr/bin/env python3
"""Summarize latency.py output: per model, size and scenario, the server's own counts of
reused (cache_n) and newly evaluated (prompt_n) prompt tokens, TTFT and total time
percentiles, and the uncached-reference verdicts.

Usage: summarize_latency.py results/DIR [> results/DIR/summary.json]
"""
import glob
import json
import statistics
import sys
from pathlib import Path


def pct(values, q):
    values = sorted(v for v in values if v is not None)
    if not values:
        return None
    k = (len(values) - 1) * q
    lo, hi = int(k), min(int(k) + 1, len(values) - 1)
    return round(values[lo] + (values[hi] - values[lo]) * (k - lo), 1)


def stats(rows, key):
    v = [r[key] for r in rows if r.get(key) is not None]
    if not v:
        return None
    return {"n": len(v), "mean": round(statistics.fmean(v), 1),
            "sd": round(statistics.stdev(v), 1) if len(v) > 1 else 0.0,
            "p50": pct(v, 0.5), "p90": pct(v, 0.9), "min": round(min(v), 1), "max": round(max(v), 1)}


out = {}
for f in sorted(glob.glob(str(Path(sys.argv[1]) / "*.jsonl"))):
    rows = [json.loads(line) for line in open(f)]
    meta = next((r for r in rows if r.get("scenario") == "server"), {})
    label = meta.get("label") or Path(f).stem
    model = {"load_s": meta.get("load_s"), "first_request_ms": meta.get("first_request_ms"),
             "peak_rss_gb": round(meta.get("peak_rss_bytes", 0) / 1e9, 2),
             "memory_guard_tripped": meta.get("memory_guard_tripped"), "cmd": meta.get("cmd"),
             "groups": {}}
    for r in rows:
        if r.get("scenario") in (None, "server"):
            continue
        key = f"{r['lang']}/{r['size']}"
        g = model["groups"].setdefault(key, {})
        name = r["scenario"]
        if name.endswith(":uncached_reference"):
            d = r.get("divergence") or {}
            g.setdefault("references", []).append({
                "scenario": name.split(":")[0], "match": r["text_match"],
                "class": None if r["text_match"] else d.get("class"),
                "prob_gap": d.get("prob_gap")})
            continue
        s = g.setdefault(name, {"rows": []})
        s["rows"].append(r)
    for g in model["groups"].values():
        for name, s in list(g.items()):
            if name == "references":
                continue
            rs = s.pop("rows")
            s["n"] = len(rs)
            s["ttft_ms"] = stats(rs, "ttft_ms")
            s["total_ms"] = stats(rs, "total_ms")
            s["prompt_n"] = [r.get("prompt_n") for r in rs]
            s["cache_n"] = [r.get("cache_n") for r in rs]
            s["gen_tok_s"] = stats(rs, "predicted_per_second")
            s["pp_tok_s"] = stats(rs, "prompt_per_second")
    out[label] = model
print(json.dumps(out, indent=1))
