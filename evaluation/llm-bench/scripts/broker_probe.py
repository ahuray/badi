#!/usr/bin/env python3
"""Replay a typing sequence through a running broker with `badictl probe`.

For each corpus sentence the text grows one word at a time from the sixth word;
two words before the end it backspaces three words and types the start of
another sentence instead. Each probe goes through the broker's real provider
(display checks included) and records the broker's own latency, so two broker
builds can be compared on identical requests. Disposable corpus text only.

Usage: broker_probe.py --badictl BIN --socket PATH --label NAME --out FILE [--langs en,de] [--sentences 6]
"""

import argparse
import json
import subprocess
from pathlib import Path

BENCH = Path(__file__).resolve().parent.parent


def probe(badictl: str, socket: str, lang: str, text: str) -> dict:
    run = subprocess.run([badictl, "--socket", socket, "probe", "--language", lang, "-"],
                         input=text, capture_output=True, text=True, timeout=30, check=False)
    if run.returncode != 0:
        return {"outcome": "error", "error": run.stderr.strip()[:200]}
    return json.loads(run.stdout)


def sequence(sentences: list[str]):
    """(kind, text) pairs: appends, then a three-word backspace and retype."""
    for i, sentence in enumerate(sentences):
        words = sentence.rstrip(".?").split()
        other = sentences[(i + 1) % len(sentences)].split()
        for k in range(6, len(words) - 1):
            yield "append" if k > 6 else "first", " ".join(words[:k])
        kept = words[: len(words) - 4]
        for j in range(1, 4):
            yield "after_backspace" if j == 1 else "append_after_backspace", " ".join(kept + other[:j])


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--badictl", required=True)
    ap.add_argument("--socket", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--langs", default="en,de")
    ap.add_argument("--sentences", type=int, default=6)
    args = ap.parse_args()
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    with open(args.out, "w") as out:
        for lang in args.langs.split(","):
            sentences = (BENCH / "corpus" / f"{lang}.txt").read_text().strip().split("\n")[: args.sentences]
            for step, (kind, text) in enumerate(sequence(sentences)):
                result = probe(args.badictl, args.socket, lang, text)
                out.write(json.dumps({"label": args.label, "lang": lang, "step": step, "kind": kind,
                                      "chars": len(text), **result}, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
