import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import debugtrace


class DebugTraceTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.directory = Path(self.root.name) / "badi"
        self.directory.mkdir(mode=0o700)
        self.environment = patch.dict(os.environ, {"XDG_RUNTIME_DIR": self.root.name})
        self.environment.start()
        os.environ.pop("BADI_DEBUG_TRACE", None)
        debugtrace._checked = None

    def tearDown(self):
        self.environment.stop()
        self.root.cleanup()
        debugtrace._checked = None

    def start(self, text=False, expires_in=600, trace=True):
        control = {"id": "550e8400-e29b-41d4-a716-446655440000", "expires_at": int(time.time()) + expires_in}
        if text:
            control["text"] = True
        path = self.directory / "debug-control.json"
        path.write_text(json.dumps(control))
        path.chmod(0o600)
        if trace:
            (self.directory / "debug-trace.jsonl").touch(mode=0o600)
        debugtrace._checked = None

    def lines(self):
        return [json.loads(line) for line in (self.directory / "debug-trace.jsonl").read_text().splitlines()]

    def test_an_events_only_run_drops_text_fields(self):
        self.start()
        debugtrace.emit("preview drawn", {"placement": "chip"}, {"text": " next words"})
        [line] = self.lines()
        self.assertEqual((line["src"], line["ev"], line["placement"]), ("observer", "preview drawn", "chip"))
        self.assertNotIn("text", line)

    def test_a_text_run_keeps_text_fields(self):
        self.start(text=True)
        debugtrace.emit("preview drawn", {"placement": "inline"}, {"text": " next words"})
        self.assertEqual(self.lines()[0]["text"], " next words")

    def test_nothing_is_written_off_expired_or_without_the_trace_file(self):
        debugtrace.emit("preview drawn", {})
        self.assertFalse((self.directory / "debug-trace.jsonl").exists())
        self.start(trace=False)
        debugtrace.emit("preview drawn", {})
        self.assertFalse((self.directory / "debug-trace.jsonl").exists())
        self.start(expires_in=-1)
        debugtrace.emit("preview drawn", {})
        self.assertEqual(self.lines(), [])
        self.start(expires_in=3600)
        debugtrace.emit("preview drawn", {})
        self.assertEqual(self.lines(), [])

    def test_a_trace_readable_by_others_is_left_untouched(self):
        self.start()
        (self.directory / "debug-trace.jsonl").chmod(0o644)
        debugtrace.emit("preview drawn", {})
        self.assertEqual(self.lines(), [])

    def test_the_environment_switch_keeps_a_process_out_of_a_live_trace(self):
        self.start()
        os.environ["BADI_DEBUG_TRACE"] = "off"
        debugtrace.emit("preview drawn", {})
        self.assertEqual(self.lines(), [])

    def test_a_disabled_run_is_rechecked_at_most_once_a_second(self):
        debugtrace.emit("preview drawn", {})
        self.start()
        debugtrace._checked = time.monotonic()
        debugtrace._run = None
        debugtrace.emit("preview drawn", {})
        self.assertEqual(self.lines(), [])


if __name__ == "__main__":
    unittest.main()
