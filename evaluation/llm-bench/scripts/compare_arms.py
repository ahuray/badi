#!/usr/bin/env python3
"""Compare two arms of latency.py runs (for example default against --swa-full).

Each arm is a label prefix; every result file whose label starts with it is one
repetition. Per scenario this reports newly evaluated prompt tokens (server
counter), TTFT and total time percentiles, and whether the two arms produced the
same greedy text for the same prompt.

Usage: compare_arms.py DIR PREFIX_A PREFIX_B [> DIR/compare.json]
"""
import glob
import json
import sys
from collections import defaultdict
from pathlib import Path

from summarize_latency import pct

directory, arm_a, arm_b = sys.argv[1], sys.argv[2], sys.argv[3]
rows = defaultdict(list)  # (arm, scenario) -> rows
texts = defaultdict(dict)  # (rep, lang, size, scenario, step) -> {arm: text}
references = defaultdict(lambda: defaultdict(int))
for f in sorted(glob.glob(str(Path(directory) / "*.jsonl"))):
    label = Path(f).stem
    # The longest matching prefix decides, so a `gemma3-swa` run is never
    # filed under a `gemma3` arm.
    matches = [prefix for prefix in (arm_a, arm_b) if label.startswith(prefix)]
    if not matches:
        continue
    arm = max(matches, key=len)
    rep = label[len(arm):]
    for line in open(f):
        r = json.loads(line)
        scenario = r.get("scenario")
        if scenario in (None, "server"):
            continue
        if scenario.endswith(":uncached_reference"):
            d = r.get("divergence") or {}
            references[arm]["match" if r["text_match"] else d.get("class", "?")] += 1
            continue
        rows[(arm, scenario)].append(r)
        rows[(arm, "all_but_cold")].extend([r] if scenario != "a_cold" else [])
        texts[(rep, r["lang"], r["size"], scenario, r["step"])][arm] = r["text"]


def summary(rs):
    return {"n": len(rs),
            "prompt_n_mean": round(sum(r["prompt_n"] or 0 for r in rs) / len(rs), 1) if rs else None,
            "ttft_p50": pct([r["ttft_ms"] for r in rs], 0.5), "ttft_p90": pct([r["ttft_ms"] for r in rs], 0.9),
            "total_p50": pct([r["total_ms"] for r in rs], 0.5), "total_p90": pct([r["total_ms"] for r in rs], 0.9)}


scenarios = sorted({s for _, s in rows})
same = sum(1 for v in texts.values() if len(v) == 2 and v[arm_a] == v[arm_b])
paired = sum(1 for v in texts.values() if len(v) == 2)
print(json.dumps({
    "arms": [arm_a, arm_b],
    "scenarios": {s: {arm: summary(rows[(arm, s)]) for arm in (arm_a, arm_b)} for s in scenarios},
    "same_text_same_prompt": {"identical": same, "paired": paired},
    "uncached_reference": {arm: dict(references[arm]) for arm in (arm_a, arm_b)},
}, indent=1))
