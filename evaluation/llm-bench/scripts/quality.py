#!/usr/bin/env python3
"""Greedy continuations of quality_set.jsonl for one model, plus objective checks.

Same settings for every model: raw prompt (base models, no template), temperature 0,
seed 42, n_predict 16, stop at a newline, no grammar. Output is clock-independent,
so this can run while timing work is blocked.

Usage: quality.py --server BIN --model GGUF --label NAME --out results/quality/NAME.jsonl
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from latency import Server  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
STOP = {
    "en": {"the", "and", "to", "of", "is", "are", "was", "we", "you", "it", "that", "for",
           "with", "this", "be", "have", "not", "on", "at", "by", "will", "would", "as",
           "from", "they", "our", "your", "can", "if", "or", "an", "in"},
    "de": {"der", "die", "das", "und", "zu", "ist", "sind", "war", "wir", "sie", "ich",
           "nicht", "mit", "für", "auf", "den", "dem", "ein", "eine", "einen", "es", "auch",
           "noch", "wird", "werden", "haben", "hat", "von", "bei", "dass", "im", "um", "an"},
}
CHAT = re.compile(r"(?i)\b(assistant|user|answer|antwort|question|frage)\s*:|^\s*(sure|of course|natürlich)[,!]"
                  r"|\b(here is|hier ist)\b|<\|im_|<start_of_turn>|<\|")
MARKUP = re.compile(r"[<>`*#_\[\]{}|\\]|\n|https?://")


def objective(lang: str, prompt: str, text: str) -> dict:
    words = re.findall(r"[A-Za-zÄÖÜäöüß']+", text.lower())
    own = sum(w in STOP[lang] for w in words)
    other = sum(w in STOP["de" if lang == "en" else "en"] for w in words)
    grams = [tuple(words[i:i + 2]) for i in range(len(words) - 1)]
    sentence_ends = len(re.findall(r"(?<!\d)[.!?](\s|$)", text))
    tail = prompt.split()[-3:]
    return {
        "empty": not text.strip(),
        "language_switch": other >= 2 and other > own,
        "repetition": len(grams) != len(set(grams)) or any(
            w.lower() == t.lower().strip(",.") for w in text.split()[:3] for t in tail[-1:]),
        "chat_prefix": bool(CHAT.search(text)),
        "rambling": sentence_ends >= 2,
        "broken_format": bool(MARKUP.search(text)),
        "leading_space_ok": text[:1] in {" ", ",", ".", ";", ":", "!", "?"},
    }


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--threads", type=int, default=6)
    ap.add_argument("--set", default=str(ROOT / "quality_set.jsonl"))
    args = ap.parse_args()
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    server = Server(args.server, args.model,
                    ["-c", "512", "-np", "1", "-t", str(args.threads), "-ngl", "0", "--no-webui"],
                    Path(args.out).with_suffix(".server.log"))
    try:
        with open(args.out, "w") as out:
            for line in open(args.set):
                item = json.loads(line)
                status, body = server.request("POST", "/completion", {
                    "prompt": item["prompt"], "n_predict": 16, "temperature": 0.0, "seed": 42,
                    "stop": ["\n"], "cache_prompt": False})
                if status != 200 or not body:
                    raise SystemExit(f"/completion {status} on {item['id']}")
                text = body["content"]
                row = {"label": args.label, "model": Path(args.model).name, **item,
                       "continuation": text, "checks": objective(item["lang"], item["prompt"], text)}
                out.write(json.dumps(row, ensure_ascii=False) + "\n")
    finally:
        server.stop()
    print(f"{args.label}: {sum(1 for _ in open(args.out))} prompts -> {args.out}")


if __name__ == "__main__":
    main()
