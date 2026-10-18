#!/usr/bin/env python3
"""Where the bench keeps things. Source (scripts, corpus, quality set) lives in
evaluation/llm-bench; everything large or local (builds, models, venv, results,
logs) lives in the ignored data directory, output/llm-bench unless LLM_BENCH_DATA
says otherwise.

CLI: bench_paths.py data | model LABEL | fetch-spec LABEL
"""
import json
import os
import sys
from pathlib import Path

BENCH = Path(__file__).resolve().parent.parent
REPO = BENCH.parent.parent
DATA = Path(os.environ.get("LLM_BENCH_DATA", REPO / "output" / "llm-bench"))


def models() -> list[dict]:
    return json.loads((BENCH / "scripts" / "models.json").read_text())


def model_path(entry: dict) -> Path:
    """A downloaded model sits in DATA/models/<repo with __>/<file>; others name a path
    relative to the repository or the home directory."""
    if "repo" in entry:
        return DATA / "models" / entry["repo"].replace("/", "__") / entry["file"]
    path = Path(entry["path"]).expanduser()
    return path if path.is_absolute() else REPO / path


def entry(label: str) -> dict:
    for m in models():
        if m["label"] == label:
            return m
    raise SystemExit(f"unknown model label {label!r}; see {BENCH / 'scripts' / 'models.json'}")


if __name__ == "__main__":
    match sys.argv[1:]:
        case ["data"]:
            print(DATA)
        case ["model", label]:
            print(model_path(entry(label)))
        case ["fetch-spec", label]:
            e = entry(label)
            if "repo" not in e:
                raise SystemExit(f"{label} is not a downloadable model")
            print(e["repo"], e["file"])
        case _:
            raise SystemExit(__doc__)
