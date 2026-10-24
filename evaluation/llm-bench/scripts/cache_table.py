#!/usr/bin/env python3
"""Cache table from latency.py output: newly evaluated tokens per scenario, from server counters.
Usage: cache_table.py RESULTS_DIR"""
import glob, json, sys
from pathlib import Path
print(f"{'model':30} {'size':>4} | {'typing new/appended':>20} | {'reject new (cached)':>26} | {'edit10% new':>11} | {'cancel→repl new':>15} | {'switch back new':>15} | refs")
for f in sorted(glob.glob(str(Path(sys.argv[1]) / "*.jsonl"))):
    rows = [json.loads(l) for l in open(f)]
    label = Path(f).stem
    for size in sorted({r["size"] for r in rows if "size" in r}):
        R = [r for r in rows if r.get("size") == size]
        by = lambda s: [r for r in R if r["scenario"] == s]
        if not by("f_switch_other"):
            continue  # incomplete (run in progress or failed)
        typ = by("b_typing")
        prev = [by("a_cold")[0]["expected_prompt_tokens"]] + [r["expected_prompt_tokens"] for r in typ[:-1]]
        appended = sum(r["expected_prompt_tokens"] - p for r, p in zip(typ, prev))
        new = sum(r["prompt_n"] for r in typ)
        rej = by("c_reject_retype")
        refs = [r for r in R if r["scenario"].endswith("uncached_reference")]
        bad = [r["scenario"].split(":")[0] + "=" + ((r.get("divergence") or {}).get("class") or "?") for r in refs if not r["text_match"]]
        print(f"{label:30} {size:>4} | {new:>8}/{appended:<11} | {' '.join(f'{r['prompt_n']}({r['cache_n']})' for r in rej):>26} | "
              f"{by('d_edit_early')[0]['prompt_n']:>11} | {by('e_replacement')[0]['prompt_n']:>15} | {by('f_switch_back')[0]['prompt_n']:>15} | "
              f"{len(refs)-len(bad)}/{len(refs)} match {' '.join(bad)}")
