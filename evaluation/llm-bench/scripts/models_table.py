#!/usr/bin/env python3
"""One row per model from latency.py runs at Badi's launch: decode speed against the
bandwidth ceiling, TTFT and 8-token totals by scenario, peak RSS, and pass/fail.

Pass needs warm typing TTFT p90 < 300 ms and the 8-token typing total p90 within
Badi's 550 ms automatic budget. Labels are matched as LABEL-SUFFIX against
models.json labels.

Usage: models_table.py RESULTS_DIR CEILINGS_JSON [> RESULTS_DIR/models.json]
"""
import glob
import json
import statistics
import sys
from pathlib import Path

from bench_paths import models
from summarize_latency import pct

BUDGET_MS = 550
TTFT_TARGET_MS = 300


def scenario(rows, *names):
    return [r for r in rows if r["scenario"] in names]


def main() -> None:
    directory, ceilings = Path(sys.argv[1]), json.loads(Path(sys.argv[2]).read_text())["models"]
    labels = sorted((m["label"] for m in models()), key=len, reverse=True)
    grouped: dict[str, list] = {}
    meta: dict[str, list] = {}
    for f in sorted(glob.glob(str(directory / "*.jsonl"))):
        stem = Path(f).stem
        label = next((lab for lab in labels if stem.startswith(lab + "-") or stem == lab), None)
        if label is None:
            continue
        for line in open(f):
            r = json.loads(line)
            if r.get("scenario") == "server":
                meta.setdefault(label, []).append(r)
            elif r.get("scenario") and not r["scenario"].endswith("uncached_reference"):
                grouped.setdefault(label, []).append(r)
    table = {}
    for label, rows in grouped.items():
        typing = scenario(rows, "b_typing")
        warm = [r for r in rows if r["scenario"] not in ("a_cold", "e_cancelled", "d_edit_early")]
        per_token = [r["predicted_ms"] / r["predicted_n"] for r in typing if r.get("predicted_n")]
        decode_ms = statistics.median(per_token) if per_token else None
        ceiling = ceilings.get(label, {}).get("ceiling_tok_s")
        tok_s = 1000 / decode_ms if decode_ms else None
        row = {
            "runs": len(meta.get(label, [])),
            "decode_ms_per_token_p50": round(decode_ms, 1) if decode_ms else None,
            "decode_tok_s": round(tok_s, 1) if tok_s else None,
            "ceiling_tok_s": ceiling,
            "pct_of_ceiling": round(100 * tok_s / ceiling) if tok_s and ceiling else None,
            "cold_ttft_p50": pct([r["ttft_ms"] for r in scenario(rows, "a_cold")], 0.5),
            "typing_ttft_p50": pct([r["ttft_ms"] for r in typing], 0.5),
            "typing_ttft_p90": pct([r["ttft_ms"] for r in typing], 0.9),
            "typing_total_p50": pct([r["total_ms"] for r in typing], 0.5),
            "typing_total_p90": pct([r["total_ms"] for r in typing], 0.9),
            "warm_total_p90": pct([r["total_ms"] for r in warm], 0.9),
            "reject_ttft_p90": pct([r["ttft_ms"] for r in scenario(rows, "c_reject_retype")], 0.9),
            "typing_prompt_tokens_mean": round(statistics.fmean(r["prompt_n"] or 0 for r in typing), 1),
            "typing_over_budget": sum(r["total_ms"] > BUDGET_MS for r in typing),
            "typing_n": len(typing),
            "peak_rss_gb": round(max(m["peak_rss_bytes"] for m in meta[label]) / 1e9, 2) if label in meta else None,
        }
        row["pass"] = bool(row["typing_ttft_p90"] is not None and row["typing_ttft_p90"] < TTFT_TARGET_MS
                           and row["typing_total_p90"] is not None and row["typing_total_p90"] <= BUDGET_MS)
        table[label] = row
    print(json.dumps(table, indent=1))


if __name__ == "__main__":
    main()
