#!/usr/bin/env python3
"""Per-model objective counts (re-scored from saved text) and subjective means.

Subjective scores live in results/quality/scores.json as {label: {id: 1-5}}; they
judge what Badi would show: the continuation up to the first sentence end, at most
four words, against the prompt's intent. Usage: summarize_quality.py > DATA/results/quality/summary.json
"""
import glob
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bench_paths import DATA  # noqa: E402
from quality import objective  # noqa: E402

scores_path = DATA / "results/quality/scores.json"
scores = json.loads(scores_path.read_text()) if scores_path.exists() else {}
summary = {}
for f in sorted(glob.glob(str(DATA / "results/quality/*.jsonl"))):
    rows = [json.loads(line) for line in open(f)]
    label = rows[0]["label"]
    s = {}
    for lang in ("de", "en"):
        sub = [r for r in rows if r["lang"] == lang]
        checks = [objective(r["lang"], r["prompt"], r["continuation"]) for r in sub]
        s[lang] = {k: sum(c[k] for c in checks) for k in checks[0] if k != "leading_space_ok"}
        s[lang]["n"] = len(sub)
        mine = [scores.get(label, {}).get(r["id"]) for r in sub]
        mine = [m for m in mine if m is not None]
        s[lang]["subjective_mean"] = round(sum(mine) / len(mine), 2) if mine else None
        s[lang]["subjective_n"] = len(mine)
    summary[label] = s
print(json.dumps(summary, indent=1))
