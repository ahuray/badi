#!/usr/bin/env python3
"""Short cold prompts (a new sentence after a different one) on two llama-server
arms, alternating, to isolate a runtime setting's cost when nothing is reused.

Each arm gets the same sequence: every corpus sentence's first N words, in turn,
with n_predict 8 at temperature 0 and cache_prompt on, as Badi sends them.
Reports TTFT and the server's prompt_ms / prompt_n / cache_n per arm.

Usage: cold_starts.py --server BIN --model GGUF --out FILE --arm NAME="EXTRA ARGS"... [--rounds 3] [--words 6] [--after-long]
"""

import argparse
import json
import shlex
from pathlib import Path

from latency import Server, complete

BENCH = Path(__file__).resolve().parent.parent
BASE = ["-c", "512", "-np", "1", "-t", "4", "-tb", "4", "-b", "16", "-ub", "16", "--cache-reuse", "16",
        "--cache-ram", "128", "--no-mmap", "-ngl", "0", "--no-webui"]


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--arm", action="append", required=True)
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--words", type=int, default=6)
    ap.add_argument("--after-long", action="store_true",
                    help="send the previous whole sentence first, as typing does before a new sentence")
    args = ap.parse_args()
    sentences = [s for lang in ("en", "de") for s in (BENCH / "corpus" / f"{lang}.txt").read_text().strip().split("\n")]
    starts = [" ".join(s.split()[: args.words]) for s in sentences]
    arms = [a.split("=", 1) for a in args.arm]
    with open(args.out, "w") as out:
        for rnd in range(args.rounds):
            for name, extra in arms:
                server = Server(args.server, args.model, BASE + shlex.split(extra),
                                Path(args.out).with_suffix(f".{name}{rnd}.log"))
                try:
                    complete(server, "Hello", cache=False, n_predict=2)
                    for i, text in enumerate(starts):
                        if args.after_long:
                            complete(server, sentences[i - 1].rstrip("."), n_predict=8)
                        r = complete(server, text, n_predict=8)
                        t = r["timings"] or {}
                        out.write(json.dumps({"arm": name, "round": rnd, "i": i, "ttft_ms": round(r["ttft_s"] * 1000, 2) if r["ttft_s"] else None,
                                              "total_ms": round(r["total_s"] * 1000, 2), "prompt_n": t.get("prompt_n"),
                                              "cache_n": t.get("cache_n"), "prompt_ms": t.get("prompt_ms"),
                                              "predicted_ms": t.get("predicted_ms"), "text": r["text"]}, ensure_ascii=False) + "\n")
                finally:
                    server.stop()


if __name__ == "__main__":
    main()
