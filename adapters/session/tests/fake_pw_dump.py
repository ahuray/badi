#!/usr/bin/env python3
"""A stand-in for ``pw-dump --monitor`` that a test drives through files.

``BADI_TEST_PW_DUMP_DIR`` holds the controls: each start appends a line to
``starts``; ``refuse`` makes it exit at once without output, as pw-dump does
when PipeWire is down; otherwise it prints ``graph.json`` as the full dump,
then each ``change-*.json`` (removed once printed) as one update; ``exit``
(removed) ends it as a crash would. Arrays are printed as pw-dump prints them.
"""
import json
import os
from pathlib import Path
import sys
import time

controls = Path(os.environ["BADI_TEST_PW_DUMP_DIR"])
with (controls / "starts").open("a") as starts:
    starts.write("start\n")
if (controls / "refuse").exists():
    sys.exit(1)


def emit(entries):
    sys.stdout.write((json.dumps(entries, indent=2) if entries else "[\n]") + "\n")
    sys.stdout.flush()


emit(json.loads((controls / "graph.json").read_text()))
while True:
    if (controls / "exit").exists():
        (controls / "exit").unlink()
        sys.exit(0)
    for change in sorted(controls.glob("change-*.json")):
        emit(json.loads(change.read_text()))
        change.unlink()
    time.sleep(0.05)
