#!/usr/bin/env python3
"""Print the same-window context the observer would read for the focused field.

A Lab capture tool, not part of the installed observer: production prompts
use no window context until the Lab qualifies it. It applies the observer's
own rules (exact app identity, unlocked desktop, one focused editable field,
no password) and prints the nearest heading, the end of the preceding
paragraph and the field's text before the caret as JSON on stdout, for
building Lab cases from disposable pages. It writes no file and keeps
nothing.

    python3 evaluation/writing/window-context.py chromium --delay 3
"""
import argparse
import json
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "adapters/accessibility"))
from contract import Denied  # noqa: E402
from daemon import APPS  # noqa: E402
from field import FieldBackend  # noqa: E402

MAX_BEFORE = 512


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("app_id", choices=sorted(app for app, rule in APPS.items() if rule.web))
    parser.add_argument("--delay", type=float, default=0, help="Seconds to wait so the field can be focused")
    args = parser.parse_args()
    import gi
    gi.require_version("Atspi", "2.0")
    from gi.repository import Atspi
    Atspi.set_timeout(50, 50)
    Atspi.init()
    time.sleep(max(0.0, args.delay))
    backend = FieldBackend(Atspi, APPS)
    # A capture may walk a little further than one keystroke's budget.
    backend.budget = lambda: None
    try:
        metadata = backend.metadata(args.app_id, check_lock=True)
        if not metadata["editable"] or not metadata["focused"] or metadata["selection_count"]:
            raise Denied("unsupported_field")
        caret = metadata["caret"]
        before = backend.text(metadata, max(0, caret - MAX_BEFORE), caret)
        context = backend.window_context(metadata)
    except Denied as error:
        print(json.dumps({"error": str(error)}))
        return 1
    finally:
        Atspi.exit()
    print(json.dumps({"app_id": args.app_id, "uri": metadata["uri"], "window_context": context,
                      "before": before}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
