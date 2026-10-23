"""Fast checks for the session helper: broker link, reports and sleep order."""
import json
import os
from pathlib import Path
import socket
import struct
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import daemon  # noqa: E402
import link  # noqa: E402

STATUS = {"observed": True, "session": "active", "low_power": False, "automatic": "normal"}


def frame(message):
    body = json.dumps(message).encode()
    return struct.pack("<I", len(body)) + body


def read_frame(connection):
    header = b""
    while len(header) < 4:
        chunk = connection.recv(4 - len(header))
        if not chunk:
            return None
        header += chunk
    (length,) = struct.unpack("<I", header)
    body = b""
    while len(body) < length:
        body += connection.recv(length - len(body))
    return json.loads(body)


class FakeBroker:
    """Accepts one connection and answers with ``respond(message)``."""

    def __init__(self, directory, respond, mode=0o600):
        self.path = Path(directory) / "broker.sock"
        self.listener = socket.socket(socket.AF_UNIX)
        self.listener.bind(str(self.path))
        self.path.chmod(mode)
        self.listener.listen(1)
        self.received = []
        self.respond = respond
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def serve(self):
        try:
            connection, _ = self.listener.accept()
        except OSError:
            return
        with connection:
            while (message := read_frame(connection)) is not None:
                self.received.append(message)
                reply = self.respond(message)
                if reply is None:
                    return
                connection.sendall(frame(reply))

    def close(self):
        if self.thread.is_alive():
            # Closing a listener does not wake a blocked accept(); a connection does.
            with socket.socket(socket.AF_UNIX) as poke:
                try:
                    poke.connect(str(self.path))
                except OSError:
                    pass
        self.thread.join(2)
        self.listener.close()


def well_behaved(message):
    if message["type"] == "hello":
        return {"v": 2, "id": message["id"], "type": "hello.ack", "mono_ms": 1, "payload": {
            "selected_v": 2, "connection_id": "c:1", "enabled_capabilities": message["payload"]["capabilities"],
            "max_frame_bytes": 65536, "max_before_chars": 512, "max_after_chars": 128,
            "max_suggestion_chars": 64, "max_suggestion_words": 8, "paused": False}}
    payload = message["payload"]
    low = payload["on_battery"] or payload["power_saver"]
    return {"v": 2, "id": message["id"], "type": "desktop.status", "mono_ms": 2, "payload": {
        "observed": True, "session": payload["session"], "low_power": low,
        "automatic": "deferred" if low else "normal"}}


class LinkTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="bs")
        self.addCleanup(self.temporary.cleanup)
        os.chmod(self.temporary.name, 0o700)

    def broker(self, respond=well_behaved, mode=0o600):
        broker = FakeBroker(self.temporary.name, respond, mode)
        self.addCleanup(broker.close)
        return broker

    def test_negotiates_only_desktop_signals_and_returns_the_validated_status(self):
        broker = self.broker()
        connection = link.Link(broker.path)
        self.addCleanup(connection.close)
        connection.connect()
        status = connection.send_report("sleeping", True, False)
        self.assertEqual(status, {"observed": True, "session": "sleeping", "low_power": True, "automatic": "deferred"})
        hello, report = broker.received
        self.assertEqual(hello["payload"]["adapter"]["kind"], "session")
        self.assertEqual(hello["payload"]["capabilities"], ["desktop_signals", "style_key"])
        self.assertEqual((hello["payload"]["min_v"], hello["payload"]["max_v"]), (2, 2))
        self.assertEqual(report["type"], "desktop.state")
        self.assertEqual(report["payload"], {"session": "sleeping", "on_battery": True, "power_saver": False})
        self.assertNotIn("session_id", report)

    def test_refuses_a_public_socket_or_a_directory_others_can_enter(self):
        broker = self.broker(mode=0o666)
        with self.assertRaisesRegex(link.LinkError, "unsafe_socket"):
            link.Link(broker.path).connect()
        broker.path.chmod(0o600)
        os.chmod(self.temporary.name, 0o755)
        with self.assertRaisesRegex(link.LinkError, "unsafe_socket"):
            link.Link(broker.path).connect()
        os.chmod(self.temporary.name, 0o700)
        with self.assertRaisesRegex(link.LinkError, "broker_offline"):
            link.Link(Path(self.temporary.name) / "missing.sock").connect()

    def test_an_older_broker_or_a_malformed_status_is_rejected(self):
        def older(message):
            reply = well_behaved(message)
            if message["type"] == "hello":
                reply["payload"]["enabled_capabilities"] = ["health"]
            return reply
        with self.assertRaisesRegex(link.LinkError, "incompatible_broker"):
            link.Link(self.broker(older).path).connect()

    def test_malformed_status_closes_the_link(self):
        def malformed(message):
            reply = well_behaved(message)
            if message["type"] == "desktop.state":
                reply["payload"]["automatic"] = "deferred"
                reply["payload"]["low_power"] = False
            return reply
        connection = link.Link(self.broker(malformed).path)
        connection.connect()
        with self.assertRaisesRegex(link.LinkError, "invalid_status"):
            connection.send_report("active", False, False)
        self.assertFalse(connection.connected)

    def test_a_closed_broker_is_noticed(self):
        replies = iter([True])

        def once(message):
            return well_behaved(message) if next(replies, False) else None
        connection = link.Link(self.broker(once).path)
        connection.connect()
        with self.assertRaises(link.LinkError):
            connection.send_report("active", False, False)
        self.assertFalse(connection.connected)
        self.assertTrue(connection.closed_by_peer())

    def test_status_validation_matches_the_protocol_rules(self):
        self.assertTrue(link.valid_status(STATUS))
        self.assertTrue(link.valid_status({**STATUS, "observed": False}))
        self.assertFalse(link.valid_status({**STATUS, "observed": False, "session": "sleeping"}))
        self.assertFalse(link.valid_status({**STATUS, "automatic": "explicit_only"}))
        self.assertTrue(link.valid_status({**STATUS, "low_power": True, "automatic": "explicit_only"}))
        self.assertFalse(link.valid_status({**STATUS, "extra": 1}))
        with self.assertRaises(link.LinkError):
            link.report("locked", False, False)
        with self.assertRaises(link.LinkError):
            link.report("active", 1, False)


class FakeLink:
    def __init__(self, failures=0):
        self.connected = False
        self.reports = []
        self.keys = []
        self.failures = failures

    def send_style_key(self, key_id, key):
        self.keys.append((key_id, key))
        return "ready"

    def connect(self):
        if self.failures:
            self.failures -= 1
            raise link.LinkError("broker_offline")
        self.connected = True

    def fileno(self):
        return os.open(os.devnull, os.O_RDONLY)

    def send_report(self, session, on_battery, power_saver):
        self.reports.append((session, on_battery, power_saver))
        low = on_battery or power_saver
        return {"observed": True, "session": session, "low_power": low, "automatic": "normal"}

    def close(self):
        self.connected = False


class ReporterTests(unittest.TestCase):
    def reporter(self, fake):
        reporter = daemon.Reporter(fake)
        reporter._watch_link = lambda: None
        self.addCleanup(reporter.close)
        return reporter

    def test_reports_only_changes_and_sleep_outranks_an_inactive_session(self):
        fake = FakeLink()
        reporter = self.reporter(fake)
        self.assertTrue(reporter.publish())
        self.assertTrue(reporter.publish())
        reporter.session_active = False
        reporter.publish()
        reporter.sleeping = True
        reporter.publish()
        reporter.on_battery = True
        reporter.publish()
        self.assertEqual(fake.reports, [("active", False, False), ("inactive", False, False),
                                        ("sleeping", False, False), ("sleeping", True, False)])

    def test_a_failed_report_is_retried_with_backoff_and_resent_after_reconnect(self):
        fake = FakeLink(failures=1)
        reporter = self.reporter(fake)
        self.assertFalse(reporter.publish())
        self.assertIsNotNone(reporter._retry)
        self.assertIsNone(reporter.acknowledged)
        reporter._retry_now()
        self.assertEqual(fake.reports, [("active", False, False)])
        self.assertEqual(reporter.acknowledged, ("active", False, False))


class StyleKeyTests(unittest.TestCase):
    def reporter(self, lookup):
        fake = FakeLink()
        reporter = daemon.Reporter(fake, lookup)
        reporter._watch_link = lambda: None
        self.addCleanup(reporter.close)
        return fake, reporter

    def test_a_stored_key_reaches_every_new_broker_connection(self):
        key = ("ab" * 16, "cd" * 32)
        fake, reporter = self.reporter(lambda: key)
        reporter.publish()
        reporter.publish()
        self.assertEqual(fake.keys, [key], "once per connection")
        fake.connected = False
        reporter.publish()
        self.assertEqual(fake.keys, [key, key], "again after a reconnect")

    def test_a_locked_keyring_is_retried_later_and_never_prompts(self):
        attempts = []

        def locked():
            attempts.append(1)
            raise RuntimeError("keyring_locked")
        fake, reporter = self.reporter(locked)
        self.assertTrue(reporter.publish(), "the session report still goes through")
        self.assertEqual(fake.keys, [])
        self.assertIsNotNone(reporter._key_retry)
        self.assertEqual(len(attempts), 1)

    def test_no_stored_key_sends_nothing(self):
        fake, reporter = self.reporter(lambda: None)
        reporter.publish()
        self.assertEqual(fake.keys, [])
        self.assertIsNone(reporter._key_retry)

    def test_stored_values_are_exact_hex_pairs(self):
        try:
            import style_key
        except (ImportError, ValueError):
            self.skipTest("libsecret introspection data is not installed")
        self.assertEqual(style_key.parse("ab" * 16 + " " + "cd" * 32), ("ab" * 16, "cd" * 32))
        for value in ("", "AB" * 16 + " " + "cd" * 32, "ab" * 16 + "  " + "cd" * 32, "secret words"):
            with self.assertRaises(style_key.KeyringError):
                style_key.parse(value)


class SleepOrderTests(unittest.TestCase):
    def test_grants_are_withdrawn_before_the_inhibitor_is_released(self):
        events = []

        class Recorder:
            sleeping = False

            def publish(self):
                events.append(("publish", self.sleeping))
                return True

        logind = daemon.Logind.__new__(daemon.Logind)
        logind.reporter = Recorder()
        logind.inhibitor = None
        logind.release_inhibitor = lambda: events.append(("release",))
        logind.take_inhibitor = lambda: events.append(("take",))

        class Parameters:
            def __init__(self, starting):
                self.starting = starting

            def unpack(self):
                return (self.starting,)

        logind._prepare_for_sleep(None, None, None, None, None, Parameters(True))
        logind._prepare_for_sleep(None, None, None, None, None, Parameters(False))
        self.assertEqual(events, [("publish", True), ("release",), ("publish", False), ("take",)])


class ShortcutTests(unittest.TestCase):
    def test_actions_are_fixed_controller_commands(self):
        self.assertEqual(daemon.ACTIONS, {"pause-toggle": ("pause", "toggle"), "snooze": ("snooze",),
                                          "open-settings": ("settings",)})
        self.assertEqual({shortcut for shortcut, _description in daemon.SHORTCUTS}, set(daemon.ACTIONS))
        self.assertEqual(daemon.controller_path({"BADI_SESSION_CONTROLLER": "/x/badi-desktop.py"}),
                         Path("/x/badi-desktop.py"))

    def test_a_snooze_notification_repeats_the_controllers_own_line(self):
        notes = []
        shortcuts = daemon.Shortcuts.__new__(daemon.Shortcuts)
        shortcuts._running = {"snooze"}
        shortcuts.osd = False
        shortcuts._notification = lambda summary, body: notes.append((summary, body))

        class Process:
            def __init__(self, successful, output, error):
                self.successful, self.output, self.error = successful, output, error

            def communicate_utf8_finish(self, _result):
                return True, self.output, self.error

            def get_successful(self):
                return self.successful

        shortcuts._finished(Process(True, "Badi is snoozed in telegram for 10 minutes. badi resume ends it sooner.\n",
                                    ""), None, "snooze")
        shortcuts._running.add("snooze")
        shortcuts._finished(Process(False, "", "Nothing to snooze: type in a field where Badi suggests.\n"),
                            None, "snooze")
        shortcuts._running.add("snooze")
        shortcuts._finished(Process(False, "", ""), None, "snooze")
        self.assertEqual(notes, [
            ("Badi snoozed", "Badi is snoozed in telegram for 10 minutes. badi resume ends it sooner."),
            ("Badi did not snooze", "Nothing to snooze: type in a field where Badi suggests."),
            ("Badi did not snooze", "Run badi doctor in a terminal."),
        ])
        self.assertEqual(shortcuts._running, set())

    def test_omarchy_shows_confirmations_on_its_display_and_failures_stay_toasts(self):
        shown, notes = [], []
        shortcuts = daemon.Shortcuts.__new__(daemon.Shortcuts)
        shortcuts._running = set()
        shortcuts.osd = True
        shortcuts._osd = lambda icon, message: shown.append((icon, message)) or True
        shortcuts._notification = lambda summary, body: notes.append((summary, body))

        class Process:
            def __init__(self, successful, output):
                self.successful, self.output = successful, output

            def communicate_utf8_finish(self, _result):
                return True, self.output, ""

            def get_successful(self):
                return self.successful

        for shortcut, process in (("pause-toggle", Process(True, "Predictions paused.\n")),
                                  ("pause-toggle", Process(True, "Predictions resumed.\n")),
                                  ("snooze", Process(True, "Badi is snoozed in telegram for 10 minutes. "
                                                           "badi resume ends it sooner.\n")),
                                  ("pause-toggle", Process(False, ""))):
            shortcuts._running.add(shortcut)
            shortcuts._finished(process, None, shortcut)
        self.assertEqual(shown, [("media-pause", "Badi paused"), ("media-play", "Badi resumed"),
                                 (daemon.SNOOZE_GLYPH, "Snoozed in telegram")])
        self.assertEqual(daemon.snooze_message("Badi is snoozed in mail.example.org for 1 minute. x"),
                         "Snoozed in mail.example.org")
        self.assertEqual(daemon.snooze_message(""), "Badi snoozed")
        self.assertEqual(notes, [("Badi could not change pause", "Run badi doctor in a terminal.")])
        # A display that could not start falls back to the toast.
        shortcuts._osd = lambda icon, message: False
        shortcuts._running.add("pause-toggle")
        shortcuts._finished(Process(True, "Predictions paused.\n"), None, "pause-toggle")
        self.assertEqual(notes[-1], ("Badi paused", "Suggestions stay off until you resume."))


if __name__ == "__main__":
    unittest.main()
