"""The GNOME backend (ADR 0016) against a fake session bus, and the
extension's own files, without GNOME Shell."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from contract import Denied
import daemon as observer_daemon
from desktop import App, NoDesktopBackend, SourceDesktop
import gnome
from gnome import GnomeFocus, GnomeLockQuery, ShellUnavailable, parse_focus

ROOT = Path(__file__).resolve().parents[3]
EXTENSION = ROOT / "adapters/gnome-shell" / gnome.EXTENSION
SHELL = ":1.3"


def focus_reply(window=42, pid=os.getpid(), app_id="org.badi.Test", client="wayland", minimized=False,
                locked=False, version=1):
    return (version, locked, window, pid, app_id, client, minimized)


class FakeBus:
    def __init__(self, shell_pid=777):
        self.unique_name = ":1.50"
        self.calls = []
        self.shell_pid = shell_pid
        self.shell_uid = os.getuid()
        self.focus = focus_reply()
        self.active = (False,)
        self.failing = set()
        self.owner_callback = None
        self.subscriptions = []

    def call(self, destination, path, interface, method, signature, args, reply, timeout):
        self.calls.append((destination, path, method))
        if method in self.failing:
            raise RuntimeError("GDBus.Error:org.freedesktop.DBus.Error.UnknownObject")
        if method == "GetNameOwner":
            return (SHELL,)
        if method == "GetConnectionCredentials":
            return ({"UnixUserID": self.shell_uid, "ProcessID": self.shell_pid},)
        if method == "GetFocus":
            return self.focus
        if method == "GetActive":
            return self.active
        raise AssertionError(method)

    def watch_owner(self, name, callback):
        assert name == gnome.SHELL_NAME
        self.owner_callback = callback

    def subscribe(self, sender, path, interface, member, callback):
        self.subscriptions.append((sender, path, interface, member, callback))


class GnomeFixture(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.proc = Path(self.temporary.name)
        self.process(777, "gnome-shell")
        self.changes, self.losses = [], []

    def process(self, pid, name, uid=None):
        uid = os.getuid() if uid is None else uid
        (self.proc / str(pid)).mkdir()
        (self.proc / str(pid) / "status").write_text(f"Name:\t{name}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n")

    def focus(self, bus=None):
        bus = bus or FakeBus()
        focus = GnomeFocus(bus, on_change=lambda: self.changes.append(True),
                           on_lost=lambda: self.losses.append(True), proc=self.proc)
        focus.start()
        return focus, bus


class ParseTests(unittest.TestCase):
    def test_the_focused_window_in_the_desktop_shape(self):
        self.assertEqual(parse_focus(focus_reply()),
                         {"pid": os.getpid(), "class": "org.badi.Test", "mapped": True, "hidden": False,
                          "address": "42", "xwayland": None})
        self.assertIs(parse_focus(focus_reply(client="x11"))["xwayland"], True)
        self.assertIs(parse_focus(focus_reply(minimized=True))["hidden"], True)
        self.assertEqual(parse_focus(focus_reply(app_id="has space"))["class"], "")

    def test_every_other_answer_fails_closed(self):
        cases = ((focus_reply(locked=True), "desktop_locked"), (focus_reply(window=0), "focus_unavailable"),
                 (focus_reply(pid=0), "focus_unavailable"), (focus_reply(client=""), "focus_unavailable"),
                 (focus_reply(version=2), "desktop_unavailable"), (focus_reply(client="mir"), "desktop_unavailable"),
                 (focus_reply(locked=1), "desktop_unavailable"), (focus_reply(pid=True), "desktop_unavailable"),
                 (focus_reply()[:6], "desktop_unavailable"), (None, "desktop_unavailable"))
        for reply, error in cases:
            with self.subTest(reply=reply), self.assertRaisesRegex(Denied, error):
                parse_focus(reply)


class StartTests(GnomeFixture):
    def test_verifies_the_shell_and_follows_its_signal_only(self):
        focus, bus = self.focus()
        self.assertEqual(focus.shell, SHELL)
        self.assertEqual([call[2] for call in bus.calls], ["GetNameOwner", "GetConnectionCredentials"])
        (sender, path, interface, member, callback), = bus.subscriptions
        self.assertEqual((sender, path, interface, member),
                         (SHELL, gnome.FOCUS_PATH, gnome.FOCUS_INTERFACE, "FocusChanged"))
        callback((7,))
        self.assertEqual(self.changes, [True])

    def test_the_shell_must_be_this_users_gnome_shell(self):
        for pid, name, uid in ((900, "python3", None), (901, "gnome-shell", os.getuid() + 1)):
            self.process(pid, name, uid)
            with self.subTest(name=name, uid=uid), self.assertRaisesRegex(ShellUnavailable, "shell_unverified"):
                self.focus(FakeBus(shell_pid=pid))
        bus = FakeBus()
        bus.shell_uid = os.getuid() + 1
        with self.assertRaisesRegex(ShellUnavailable, "shell_unverified"):
            self.focus(bus)
        bus = FakeBus()
        bus.failing.add("GetNameOwner")
        with self.assertRaisesRegex(ShellUnavailable, "shell_unavailable"):
            self.focus(bus)

    def test_a_gnome_named_desktop_without_gnome_shell_has_no_backend(self):
        # Budgie names itself Budgie:GNOME and runs no gnome-shell.
        class NoShell(FakeBus):
            def call(self, destination, path, interface, method, signature, args, reply, timeout):
                if method == "GetNameOwner":
                    raise RuntimeError("GDBus.Error:org.freedesktop.DBus.Error.NameHasNoOwner: no such name")
                return super().call(destination, path, interface, method, signature, args, reply, timeout)

        waited = []
        focus = GnomeFocus(NoShell(), proc=self.proc)
        with self.assertRaisesRegex(NoDesktopBackend, "gnome_shell_absent"):
            focus.start(wait=.05, sleep=waited.append)
        self.assertTrue(waited, "a slow login gets a moment to show gnome-shell")

        class LateShell(NoShell):
            asked = 0

            def call(self, destination, path, interface, method, signature, args, reply, timeout):
                if method == "GetNameOwner":
                    LateShell.asked += 1
                    if LateShell.asked > 2:
                        return (SHELL,)
                return super().call(destination, path, interface, method, signature, args, reply, timeout)

        focus = GnomeFocus(LateShell(), proc=self.proc)
        focus.start(wait=5, sleep=lambda _seconds: None)
        self.assertEqual(focus.shell, SHELL)

    def test_a_missing_extension_does_not_stop_the_observer(self):
        bus = FakeBus()
        bus.failing.add("GetFocus")
        focus, _bus = self.focus(bus)
        with self.assertRaisesRegex(Denied, "desktop_unavailable"):
            focus.active_window(lambda limit: limit)
        bus.failing.clear()
        self.assertEqual(focus.active_window(lambda limit: limit)["pid"], os.getpid())


class ReadTests(GnomeFixture):
    def test_each_read_asks_the_shells_unique_name_afresh(self):
        focus, bus = self.focus()
        bus.calls.clear()
        focus.active_window(lambda limit: limit)
        bus.focus = focus_reply(window=43)
        self.assertEqual(focus.active_window(lambda limit: limit)["address"], "43")
        self.assertEqual(bus.calls, [(SHELL, gnome.FOCUS_PATH, "GetFocus")] * 2)

    def test_a_new_shell_loses_the_desktop(self):
        focus, bus = self.focus()
        bus.owner_callback(SHELL)
        self.assertEqual(self.losses, [])
        bus.owner_callback(":1.99")
        bus.owner_callback("")
        self.assertEqual(self.losses, [True])
        with self.assertRaisesRegex(Denied, "desktop_unavailable"):
            focus.active_window(lambda limit: limit)

    def test_a_late_answer_keeps_the_operation_timeout(self):
        focus, _bus = self.focus()

        def expired(_limit):
            raise Denied("operation_timeout")
        with self.assertRaisesRegex(Denied, "operation_timeout"):
            focus.active_window(expired)


class LockTests(GnomeFixture):
    def test_only_the_shields_explicit_false_unlocks(self):
        focus, bus = self.focus()
        with GnomeLockQuery(focus, lambda limit: limit):
            pass
        self.assertEqual(bus.calls[-1], (SHELL, "/org/gnome/ScreenSaver", "GetActive"))
        for answer, error in (((True,), "desktop_locked"), ((), "desktop_unavailable"),
                              ((1,), "desktop_unavailable"), (None, "desktop_unavailable")):
            bus.active = answer
            with self.subTest(answer=answer), self.assertRaisesRegex(Denied, error):
                with GnomeLockQuery(focus, lambda limit: limit):
                    pass
        bus.active = (False,)
        bus.failing.add("GetActive")
        with self.assertRaisesRegex(Denied, "desktop_unavailable"):
            with focus.lock_query(lambda limit: limit):
                pass

    def test_the_lock_denial_replaces_the_fields_result(self):
        focus, bus = self.focus()
        bus.active = (True,)
        with self.assertRaisesRegex(Denied, "desktop_locked"):
            with GnomeLockQuery(focus, lambda limit: limit):
                raise Denied("unsupported_field")


class DesktopTests(GnomeFixture):
    def test_the_identity_rule_applies_unchanged(self):
        focus, bus = self.focus()
        app = App(frozenset({os.path.realpath(f"/proc/{os.getpid()}/exe")}), frozenset({"org.badi.Test"}))
        desktop = SourceDesktop({"test": app}, lambda limit: limit, focus)
        self.assertEqual(desktop.window(app)["pid"], os.getpid())
        self.assertEqual(desktop.monitors(), [])
        for reply in (focus_reply(app_id="org.badi.Other"), focus_reply(minimized=True), focus_reply(pid=1)):
            bus.focus = reply
            with self.subTest(reply=reply), self.assertRaisesRegex(Denied, "app_mismatch"):
                desktop.window(app)


class DaemonTests(unittest.TestCase):
    def test_gnome_follows_the_extension_and_reports_it_in_status(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        idle = []
        glib = type("GLib", (), {"idle_add": lambda *args: idle.append(args) or 1})
        daemon = observer_daemon.Daemon(Path(temporary.name) / "accessibility.sock", None, glib)
        daemon.connect_event_bus = lambda: None
        listener = type("Listener", (), {"register": lambda self, name: None})()
        daemon.A = type("A", (), {"EventListener": type("E", (), {"new": staticmethod(lambda *a: listener)})})
        started = []

        class Focus:
            def __init__(self, bus, on_change, on_lost):
                self.on_change, self.on_lost = on_change, on_lost

            def start(self):
                started.append(True)

        environment = {"XDG_CURRENT_DESKTOP": "ubuntu:GNOME", "WAYLAND_DISPLAY": "wayland-0",
                       "HYPRLAND_INSTANCE_SIGNATURE": "", "KDE_SESSION_VERSION": ""}
        with patch.dict(os.environ, environment), patch("sessionbus.GioBus", lambda: object()), \
                patch("gnome.GnomeFocus", Focus):
            daemon.watch()
        self.assertEqual(started, [True])
        self.assertIsInstance(daemon.backend.desktop, SourceDesktop)
        status = daemon.observer.request({"schema": "badi.accessibility.v1", "id": "s", "op": "status"})
        self.assertEqual(status["status"]["desktop"], "gnome")
        daemon.focus_source.on_change()
        daemon.focus_source.on_lost()
        self.assertEqual([call[0] for call in idle], [daemon.source_window_changed, daemon.lose_desktop])


class ExtensionFileTests(unittest.TestCase):
    """The extension cannot run here; its contract with the observer can be read."""

    def test_metadata_names_this_extension_and_the_user_session_only(self):
        metadata = json.loads((EXTENSION / "metadata.json").read_text())
        self.assertEqual(metadata["uuid"], EXTENSION.name)
        # GNOME disables an extension outside its session modes, so a locked
        # screen unexports GetFocus.
        self.assertEqual(metadata["session-modes"], ["user"])
        self.assertTrue(set(metadata["shell-version"]) >= {"46", "48"}, "Ubuntu 24.04 and Debian 13")
        self.assertEqual(sorted(path.name for path in EXTENSION.iterdir()), ["extension.js", "metadata.json"])

    def test_the_extension_exports_what_the_observer_calls(self):
        source = (EXTENSION / "extension.js").read_text()
        self.assertIn(f"const PATH = '{gnome.FOCUS_PATH}';", source)
        self.assertIn(f'<interface name="{gnome.FOCUS_INTERFACE}">', source)
        self.assertIn(f"const VERSION = {gnome.VERSION};", source)
        out = [line.split('type="')[1][0] for line in source.splitlines() if 'direction="out"' in line]
        self.assertEqual("(" + "".join(out) + ")", gnome.FOCUS_REPLY)
        # No title, content or window list leaves the Shell.
        code = "\n".join(line for line in source.splitlines() if not line.lstrip().startswith("//"))
        for forbidden in ("title", "get_buffer", "list_all_windows", "get_window_actors", "get_frame_rect"):
            self.assertNotIn(forbidden, code)

    def test_the_extension_parses(self):
        node = shutil.which("node")
        if node is None:
            self.skipTest("node is not installed")
        result = subprocess.run([node, "--check", str(EXTENSION / "extension.js")], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
