"""The KDE Plasma backend (ADR 0015) against a fake session bus, without KWin."""
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from contract import Denied
import daemon as observer_daemon
from desktop import App, SourceDesktop, NoDesktopBackend, desktop_backend
import kwin
from kwin import KWinFocus, KWinLockQuery, KWinUnavailable, parse_window, script_source

KWIN = ":1.4"
OBSERVER = ":1.42"
WINDOW_A = "{08b35194-97dd-4d93-903d-294ad89f8b76}"
WINDOW_B = "{a75b8d0f-de60-4ac1-bb63-cec66a8592bf}"


def window(pid=os.getpid(), app_id="org.badi.Test", identifier=WINDOW_A, **flags):
    return {"pid": pid, "app_id": app_id, "id": identifier, "minimized": False, "hidden": False,
            "managed": True, "deleted": False, **flags}


class FakeBus:
    """The calls KWin and the bus daemon answer, and the hooks KWinFocus sets."""

    def __init__(self, supported=True, script_id=3, kwin_pid=4321, loaded=True, locked=False, hello=True):
        self.unique_name = OBSERVER
        self.calls = []
        self.handler = None
        self.owner_callback = None
        self.supported, self.script_id, self.kwin_pid = supported, script_id, kwin_pid
        self.kwin_uid = os.getuid()
        self.loaded, self.locked, self.hello = loaded, locked, hello
        self.failing = set()

    def call(self, destination, path, interface, method, signature, args, reply, timeout):
        self.calls.append((destination, path, method, args))
        if method in self.failing:
            raise RuntimeError("bus_error")
        if method == "GetNameOwner":
            return (KWIN,)
        if method == "GetConnectionCredentials":
            return ({"UnixUserID": self.kwin_uid, "ProcessID": self.kwin_pid},)
        if method == "unloadScript":
            return (True,)
        if method == "loadScript":
            return (self.script_id,)
        if method == "run":
            # KWin runs the script, which says hello and reports at once.
            if self.hello:
                self.send("Hello", {"supported": self.supported})
                if self.supported:
                    self.send("Focus", {"serial": 1, "window": window()})
            return ()
        if method == "isScriptLoaded":
            return (self.loaded,)
        if method == "GetActive":
            return self.locked if isinstance(self.locked, tuple) else (self.locked,)
        raise AssertionError(method)

    def serve(self, path, interface, handler):
        assert (path, interface) == (kwin.REPORT_PATH, kwin.REPORT_INTERFACE)
        self.handler = handler

    def watch_owner(self, name, callback):
        assert name == kwin.KWIN_NAME
        self.owner_callback = callback

    def send(self, member, document, sender=KWIN, token=None, raw=None):
        token = token if token is not None else self.focus.token
        return self.handler(sender, member, (token, raw if raw is not None else json.dumps(document)))


class KWinFixture(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.proc = Path(self.temporary.name) / "proc"
        self.process(4321, "kwin_wayland")
        self.changes, self.losses = [], []

    def process(self, pid, name, uid=None):
        """A /proc entry as an undumpable process shows it: status, no exe link."""
        uid = os.getuid() if uid is None else uid
        entry = self.proc / str(pid)
        entry.mkdir(parents=True)
        (entry / "status").write_text(f"Name:\t{name[:15]}\nUmask:\t0022\nState:\tS (sleeping)\n"
                                      f"Uid:\t{uid}\t{uid}\t{uid}\t{uid}\n")

    def focus(self, bus=None, timeout=.2):
        bus = bus or FakeBus()
        focus = KWinFocus(bus, Path(self.temporary.name) / "kwin-field-observer.js",
                          on_change=lambda: self.changes.append(True), on_lost=lambda: self.losses.append(True),
                          proc=self.proc)
        bus.focus = focus
        focus.start(timeout=timeout)
        self.changes.clear()
        return focus, bus


class BackendSelectionTests(unittest.TestCase):
    def test_hyprland_first_then_plasma_6_then_gnome(self):
        plasma = {"XDG_CURRENT_DESKTOP": "KDE", "KDE_SESSION_VERSION": "6", "WAYLAND_DISPLAY": "wayland-0"}
        self.assertEqual(desktop_backend({"HYPRLAND_INSTANCE_SIGNATURE": "x", **plasma}), "hyprland")
        self.assertEqual(desktop_backend(plasma), "kwin")
        self.assertEqual(desktop_backend({**plasma, "XDG_CURRENT_DESKTOP": "foo:KDE"}), "kwin")
        self.assertEqual(desktop_backend({**plasma, "XDG_CURRENT_DESKTOP": "GNOME"}), "gnome")
        self.assertEqual(desktop_backend({**plasma, "XDG_CURRENT_DESKTOP": "ubuntu:GNOME"}), "gnome")
        for environment in ({**plasma, "KDE_SESSION_VERSION": "5"}, {**plasma, "KDE_SESSION_VERSION": ""},
                            {**plasma, "XDG_CURRENT_DESKTOP": "KDE-ish"}, {**plasma, "XDG_CURRENT_DESKTOP": "sway"},
                            {**plasma, "WAYLAND_DISPLAY": ""}, {**plasma, "XDG_CURRENT_DESKTOP": "GNOME",
                                                                "WAYLAND_DISPLAY": ""}, {}):
            with self.subTest(environment=environment), self.assertRaisesRegex(NoDesktopBackend, "no_desktop_backend"):
                desktop_backend(environment)

    def test_the_installer_and_doctor_decide_the_same(self):
        sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "scripts"))
        import badi_install
        for environment in ({"HYPRLAND_INSTANCE_SIGNATURE": "x"},
                            {"XDG_CURRENT_DESKTOP": "KDE", "KDE_SESSION_VERSION": "6", "WAYLAND_DISPLAY": "w"},
                            {"XDG_CURRENT_DESKTOP": "KDE", "KDE_SESSION_VERSION": "7", "WAYLAND_DISPLAY": "w"},
                            {"XDG_CURRENT_DESKTOP": "KDE", "KDE_SESSION_VERSION": "5", "WAYLAND_DISPLAY": "w"},
                            {"XDG_CURRENT_DESKTOP": "ubuntu:GNOME", "WAYLAND_DISPLAY": "w"},
                            {"XDG_CURRENT_DESKTOP": "GNOME"}, {"XDG_CURRENT_DESKTOP": "sway", "WAYLAND_DISPLAY": "w"}):
            try:
                expected = desktop_backend(environment)
            except NoDesktopBackend:
                expected = None
            with self.subTest(environment=environment):
                self.assertEqual(badi_install.observer_desktop(environment), expected)


class StartTests(KWinFixture):
    def test_loads_a_private_script_for_this_connection_then_removes_it(self):
        written = {}
        original = KWinFocus.write_script

        def capture(focus):
            original(focus)
            info = focus.script_path.stat()
            written.update(mode=stat.S_IMODE(info.st_mode), text=focus.script_path.read_text())

        with patch.object(KWinFocus, "write_script", capture):
            focus, bus = self.focus()
        self.assertEqual(written["mode"], 0o600)
        self.assertIn(f'const TARGET = "{OBSERVER}";', written["text"])
        self.assertIn(f'const TOKEN = "{focus.token}";', written["text"])
        self.assertFalse(focus.script_path.exists())
        self.assertEqual([call[2] for call in bus.calls],
                         ["GetNameOwner", "GetConnectionCredentials", "unloadScript", "loadScript", "run"])
        self.assertEqual(bus.calls[4][1], "/Scripting/Script3")
        self.assertEqual(focus.window["pid"], os.getpid())

    def test_the_script_template_takes_only_a_unique_name_and_a_token(self):
        self.assertIn('"@TARGET@"', kwin.SCRIPT)
        for target, token in (("org.kde.KWin", "0" * 32), (':1.2"; evil', "0" * 32), (":1.2", "x" * 32), (":1.2", "")):
            with self.subTest(target=target, token=token), self.assertRaises(ValueError):
                script_source(target, token)

    def test_kwin_must_be_this_users_kwin_wayland(self):
        bus = FakeBus(kwin_pid=999)
        self.process(999, "python3")
        with self.assertRaisesRegex(KWinUnavailable, "kwin_unverified"):
            self.focus(bus)
        bus = FakeBus(kwin_pid=998)
        self.process(998, "kwin_wayland", uid=os.getuid() + 1)
        with self.assertRaisesRegex(KWinUnavailable, "kwin_unverified"):
            self.focus(bus)
        bus = FakeBus()
        bus.kwin_uid = os.getuid() + 1
        with self.assertRaisesRegex(KWinUnavailable, "kwin_unverified"):
            self.focus(bus)
        bus = FakeBus(kwin_pid=997)
        with self.assertRaisesRegex(KWinUnavailable, "kwin_unverified"):
            self.focus(bus)  # No such process.
        bus = FakeBus()
        bus.failing.add("GetNameOwner")
        with self.assertRaisesRegex(KWinUnavailable, "kwin_unavailable"):
            self.focus(bus)

    def test_a_refused_or_silent_script_is_a_retryable_failure(self):
        with self.assertRaisesRegex(KWinUnavailable, "kwin_script_refused"):
            self.focus(FakeBus(script_id=-1))
        # KWin accepts a script it cannot read and fails only when it runs.
        with self.assertRaisesRegex(KWinUnavailable, "kwin_script_silent"):
            self.focus(FakeBus(hello=False), timeout=.05)
        self.assertFalse((Path(self.temporary.name) / "kwin-field-observer.js").exists())

    def test_a_kwin_without_the_plasma_6_api_is_no_backend(self):
        with self.assertRaisesRegex(NoDesktopBackend, "kwin_scripting_unsupported"):
            self.focus(FakeBus(supported=False))


class ReportTests(KWinFixture):
    def test_only_kwin_with_this_runs_token_may_report(self):
        focus, bus = self.focus()
        report = {"serial": 2, "window": window(identifier=WINDOW_B)}
        for sender, token, member in ((":1.99", None, "Focus"), (KWIN, "0" * 32, "Focus"), (KWIN, None, "Other")):
            with self.subTest(sender=sender, token=token, member=member):
                self.assertFalse(bus.send(member, report, sender=sender, token=token))
        self.assertEqual(focus.window["id"], WINDOW_A)
        self.assertEqual(self.changes, [])
        self.assertTrue(bus.send("Focus", report))
        self.assertEqual((focus.window["id"], self.changes), (WINDOW_B, [True]))

    def test_malformed_calls_change_nothing(self):
        focus, bus = self.focus()
        for raw in ("not json", json.dumps({"serial": 2}), '{"serial": 2, "serial": 3, "window": null}',
                    json.dumps({"serial": "2", "window": None}), json.dumps({"serial": 2, "window": None, "x": 1}),
                    json.dumps({"serial": 2, "window": None, "pad": "x" * kwin.MAX_REPORT})):
            with self.subTest(raw=raw[:40]):
                self.assertFalse(bus.send("Focus", None, raw=raw))
        self.assertFalse(bus.handler(KWIN, "Focus", ("only one",)))
        self.assertFalse(bus.handler(KWIN, "Focus", None))
        self.assertEqual((focus.serial, focus.window["id"], self.changes), (1, WINDOW_A, []))

    def test_reports_apply_once_in_order(self):
        focus, bus = self.focus()
        self.assertTrue(bus.send("Focus", {"serial": 3, "window": window(identifier=WINDOW_B)}))
        self.assertFalse(bus.send("Focus", {"serial": 2, "window": window()}))
        self.assertFalse(bus.send("Focus", {"serial": 3, "window": window()}))
        self.assertEqual(focus.window["id"], WINDOW_B)
        # The same window again (a class re-report) is no focus change.
        self.assertTrue(bus.send("Focus", {"serial": 4, "window": window(identifier=WINDOW_B)}))
        self.assertEqual(self.changes, [True])

    def test_an_unknown_window_shape_from_kwin_means_no_window(self):
        focus, bus = self.focus()
        serial = 1
        for value in (window(pid=0), window(pid=True), window(identifier="not-a-uuid"), window(minimized="no"),
                      {**window(), "extra": 1}, [1]):
            with self.subTest(value=value):
                serial += 2
                self.assertTrue(bus.send("Focus", {"serial": serial, "window": window()}))
                self.assertIsNotNone(focus.window)
                self.assertTrue(bus.send("Focus", {"serial": serial + 1, "window": value}))
                self.assertIsNone(focus.window)
                with self.assertRaisesRegex(Denied, "focus_unavailable"):
                    focus.active_window(lambda limit: limit)

    def test_parse_window_normalizes_flags_and_drops_odd_app_ids(self):
        self.assertEqual(parse_window(window(minimized=True))["hidden"], True)
        self.assertEqual(parse_window(window(deleted=True))["mapped"], False)
        self.assertEqual(parse_window(window(app_id="has space"))["app_id"], "")
        self.assertEqual(parse_window(window(app_id="a/b"))["app_id"], "")
        self.assertIsNone(parse_window(None))


class ActiveWindowTests(KWinFixture):
    def test_each_read_waits_for_kwin_and_returns_the_desktop_shape(self):
        focus, bus = self.focus()
        bus.calls.clear()
        self.assertEqual(focus.active_window(lambda limit: limit),
                         {"pid": os.getpid(), "class": "org.badi.Test", "mapped": True, "hidden": False,
                          "address": WINDOW_A, "xwayland": None})
        self.assertEqual(bus.calls, [(KWIN, "/Scripting", "isScriptLoaded", (kwin.PLUGIN,))])

    def test_an_unloaded_script_or_a_new_kwin_loses_the_desktop(self):
        focus, bus = self.focus()
        bus.loaded = False
        with self.assertRaisesRegex(Denied, "desktop_unavailable"):
            focus.active_window(lambda limit: limit)
        self.assertEqual(self.losses, [True])
        focus, bus = self.focus()
        self.losses.clear()
        bus.owner_callback(KWIN)
        self.assertEqual(self.losses, [])
        bus.owner_callback(":1.77")
        bus.owner_callback("")
        self.assertEqual(self.losses, [True])
        with self.assertRaisesRegex(Denied, "desktop_unavailable"):
            focus.active_window(lambda limit: limit)
        # A report from the old KWin connection after that is still refused for the window.
        bus.send("Focus", {"serial": 9, "window": window()})
        with self.assertRaisesRegex(Denied, "desktop_unavailable"):
            focus.active_window(lambda limit: limit)

    def test_a_failed_or_late_barrier_denies(self):
        focus, bus = self.focus()
        bus.failing.add("isScriptLoaded")
        with self.assertRaisesRegex(Denied, "desktop_unavailable"):
            focus.active_window(lambda limit: limit)
        self.assertEqual(self.losses, [])

        def expired(_limit):
            raise Denied("operation_timeout")
        with self.assertRaisesRegex(Denied, "operation_timeout"):
            focus.active_window(expired)

    def test_no_active_window_is_no_focus(self):
        focus, bus = self.focus()
        bus.send("Focus", {"serial": 2, "window": None})
        with self.assertRaisesRegex(Denied, "focus_unavailable"):
            focus.active_window(lambda limit: limit)

    def test_the_lock_screen_greeter_is_denied_as_locked(self):
        focus, bus = self.focus()
        bus.send("Focus", {"serial": 2, "window": window(app_id="kscreenlocker_greet")})
        with self.assertRaisesRegex(Denied, "desktop_locked"):
            focus.active_window(lambda limit: limit)
        # The kernel cuts the greeter's process name to 15 bytes.
        self.process(5555, "kscreenlocker_greet")
        bus.send("Focus", {"serial": 3, "window": window(pid=5555, app_id="org.kde.greeter")})
        with self.assertRaisesRegex(Denied, "desktop_locked"):
            focus.active_window(lambda limit: limit)


class LockTests(KWinFixture):
    def test_only_kwins_explicit_false_unlocks(self):
        focus, bus = self.focus()
        with KWinLockQuery(focus, lambda limit: limit):
            pass
        self.assertEqual(bus.calls[-1], (KWIN, "/ScreenSaver", "GetActive", None))
        for answer, error in ((True, "desktop_locked"), ((), "desktop_unavailable"), ((1,), "desktop_unavailable")):
            bus.locked = answer
            with self.subTest(answer=answer), self.assertRaisesRegex(Denied, error):
                with KWinLockQuery(focus, lambda limit: limit):
                    pass
        bus.locked = False
        bus.failing.add("GetActive")
        with self.assertRaisesRegex(Denied, "desktop_unavailable"):
            with KWinLockQuery(focus, lambda limit: limit):
                pass

    def test_the_lock_denial_replaces_the_fields_result(self):
        focus, bus = self.focus()
        bus.locked = True
        with self.assertRaisesRegex(Denied, "desktop_locked"):
            with KWinLockQuery(focus, lambda limit: limit):
                raise Denied("unsupported_field")


class DesktopTests(KWinFixture):
    def app(self, **values):
        return App(frozenset({os.path.realpath(f"/proc/{os.getpid()}/exe")}), frozenset({"org.badi.Test"}), **values)

    def test_the_identity_rule_applies_unchanged(self):
        focus, bus = self.focus()
        desktop = SourceDesktop({"test": self.app()}, lambda limit: limit, focus)
        self.assertEqual(desktop.window(desktop.app("test"))["pid"], os.getpid())
        self.assertEqual(desktop.monitors(), [])
        for serial, value in enumerate((window(app_id="org.badi.Other"), window(minimized=True),
                                        window(deleted=True), window(pid=1)), start=2):
            bus.send("Focus", {"serial": serial, "window": value})
            with self.subTest(value=value), self.assertRaisesRegex(Denied, "app_mismatch"):
                desktop.window(desktop.app("test"))

    def test_a_kwin_session_calibrates_nothing(self):
        # Geometry needs xwayland to be known false; KWin reports no position.
        focus, _bus = self.focus()
        self.assertIsNone(focus.active_window(lambda limit: limit)["xwayland"])


class DaemonTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.idle = []
        glib = type("GLib", (), {"idle_add": lambda *args: self.idle.append(args) or 1})
        self.daemon = observer_daemon.Daemon(Path(self.temporary.name) / "accessibility.sock", None, glib)
        self.daemon.connect_event_bus = lambda: None
        listener = type("Listener", (), {"register": lambda self, name: None})()
        self.daemon.A = type("A", (), {"EventListener": type("E", (), {"new": staticmethod(lambda *a: listener)})})

    def test_plasma_follows_kwin_and_reports_it_in_status(self):
        started = []

        class Focus:
            def __init__(self, bus, path, on_change, on_lost):
                self.on_change, self.on_lost, self.path = on_change, on_lost, path

            def start(self):
                started.append(self.path.name)

        plasma = {"XDG_CURRENT_DESKTOP": "KDE", "KDE_SESSION_VERSION": "6", "WAYLAND_DISPLAY": "wayland-0",
                  "HYPRLAND_INSTANCE_SIGNATURE": ""}
        with patch.dict(os.environ, plasma), patch("sessionbus.GioBus", lambda: object()), patch("kwin.KWinFocus", Focus):
            self.daemon.watch()
        self.assertEqual(started, ["kwin-field-observer.js"])
        self.assertIsInstance(self.daemon.backend.desktop, SourceDesktop)
        self.assertIsNone(self.daemon.window_events)
        self.assertEqual(self.daemon.observer.request({"schema": "badi.accessibility.v1", "id": "s", "op": "status"})
                         ["status"]["desktop"], "kwin")
        # Both callbacks only schedule work on the main loop.
        self.daemon.focus_source.on_change()
        self.daemon.focus_source.on_lost()
        self.assertEqual([call[0] for call in self.idle], [self.daemon.source_window_changed, self.daemon.lose_desktop])

    def test_a_kwin_change_invalidates_the_tracked_field(self):
        notices = []
        self.daemon.observer.notify = notices.append
        self.daemon.observer.tracked = {"app_id": "chromium"}
        self.assertFalse(self.daemon.source_window_changed())
        self.assertEqual(notices[-1]["reason"], "window_changed")
        self.assertIsNone(self.daemon.observer.tracked)

    def test_another_desktop_has_no_backend(self):
        # A Wayland desktop with no backend, set whole so the host's own
        # session (a real WAYLAND_DISPLAY, GNOME or Plasma) cannot select one.
        with patch.dict(os.environ, {"XDG_CURRENT_DESKTOP": "sway", "WAYLAND_DISPLAY": "wayland-0",
                                     "HYPRLAND_INSTANCE_SIGNATURE": "", "KDE_SESSION_VERSION": ""}), \
                self.assertRaisesRegex(NoDesktopBackend, "no_desktop_backend"):
            self.daemon.watch()
        self.assertEqual(observer_daemon.NO_DESKTOP_BACKEND, 78)


class ConcurrencyTests(KWinFixture):
    def test_reports_from_the_message_thread_reach_a_later_read(self):
        focus, bus = self.focus()
        thread = threading.Thread(target=bus.send, args=("Focus", {"serial": 2, "window": window(identifier=WINDOW_B)}))
        thread.start()
        thread.join()
        self.assertEqual(focus.active_window(lambda limit: limit)["address"], WINDOW_B)


if __name__ == "__main__":
    unittest.main()
