import contextlib
import json
import os
import pathlib
import signal
import socket
import sys
import tempfile
import time
from types import SimpleNamespace
import types
import unittest
import unittest.mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from contract import Denied, MAX_FRAME, SCHEMA, Observer
import daemon as observer_daemon
import desktop as observer_desktop
from daemon import APPS, Daemon, unique_object
from desktop import WindowEvents, application_entry, per_user_executable, verify_process
from field import MAX_RICH_BLOCKS, FieldBackend, browser_interface, rich_text
from geometry import CALIBRATED, calibrated_geometry, gecko_calibrated_geometry, visual_direction
from test_contract import FakeBackend


class FakeGLib:
    IO_IN, IO_HUP, IO_ERR = 1, 2, 4

    @staticmethod
    def io_add_watch(*args):
        return 1

    @staticmethod
    def idle_add(*args):
        return 1


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.temp.name) / "runtime" / "accessibility.sock"
        self.daemon = Daemon(self.path, None, FakeGLib)
        self.daemon.bind()

    def tearDown(self):
        for fd in list(self.daemon.clients):
            self.daemon.close_client(fd)
        self.daemon.server.close()
        if self.daemon.lock_fd is not None:
            os.close(self.daemon.lock_fd)
        self.temp.cleanup()

    def client(self):
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.addCleanup(client.close)
        client.connect(str(self.path))
        before = set(self.daemon.clients)
        self.daemon.accept(None, None)
        return client, (set(self.daemon.clients) - before).pop()

    def test_private_socket_and_parent_modes(self):
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.path.parent.stat().st_mode & 0o777, 0o700)

    def test_duplicate_json_keys_rejected(self):
        with self.assertRaises(ValueError):
            json.loads('{"id":"first","id":"second"}', object_pairs_hook=unique_object)

    def test_oversized_unterminated_frame_closes_connection(self):
        client, fd = self.client()
        client.sendall(b"x" * (MAX_FRAME + 1))
        self.assertFalse(self.daemon.read(fd, FakeGLib.IO_IN))
        self.assertNotIn(fd, self.daemon.clients)

    def test_coalesced_inspect_and_hide_are_sequenced(self):
        client, fd = self.client()
        calls = []
        self.daemon.observer.request = lambda r: calls.append(r["op"]) or {"schema": SCHEMA, "id": r["id"], "ok": True}
        first = {"schema": SCHEMA, "id": "inspect", "op": "inspect", "app_id": "chromium"}
        second = {"schema": SCHEMA, "id": "hide", "op": "hide"}
        client.sendall((json.dumps(first) + "\n" + json.dumps(second) + "\n").encode())
        self.assertTrue(self.daemon.read(fd, FakeGLib.IO_IN))
        self.assertEqual(calls, ["inspect"])
        self.daemon.process_next(fd)
        self.assertEqual(calls, ["inspect", "hide"])
        self.assertEqual([json.loads(line)["id"] for line in client.recv(4096).splitlines()], ["inspect", "hide"])
        self.assertIn(fd, self.daemon.clients)

    def test_complete_frame_preserves_partial_next_frame(self):
        client, fd = self.client()
        first = {"schema": SCHEMA, "id": "hide", "op": "hide"}
        client.sendall((json.dumps(first) + "\n" + '{"schema":').encode())
        self.assertTrue(self.daemon.read(fd, FakeGLib.IO_IN))
        self.assertEqual(self.daemon.clients[fd]["buffer"], b'{"schema":')
        self.assertTrue(json.loads(client.recv(4096))["hidden"])

    def test_preview_position_is_required_and_checked_before_socket_render(self):
        backend = FakeBackend()
        backend.budget = lambda: None  # This backend uses only in-memory fixture metadata.
        self.daemon.backend = backend
        self.daemon.observer = Observer(backend)
        renders = []
        self.daemon.observer.render = lambda *_args: renders.append(True) or True
        client, fd = self.client()
        client.settimeout(1)

        def exchange(request):
            client.sendall(json.dumps({"schema": SCHEMA, "id": "bound", **request}).encode() + b"\n")
            self.assertTrue(self.daemon.read(fd, FakeGLib.IO_IN))
            return json.loads(client.recv(4096))

        focus = exchange({"op": "inspect", "app_id": "chromium"})["focus"]
        focus = exchange({"op": "snapshot", "binding": focus["binding"], "policy_target": focus["target"]})["focus"]
        preview = {"op": "preview", "binding": focus["binding"], "policy_target": focus["target"],
                   "text": " suggestion", "ttl_ms": 2000}
        self.assertEqual(exchange(preview)["error"], "invalid_request")
        self.assertEqual(renders, [])
        preview.update(expected_caret=focus["caret"], expected_total_chars=focus["total_chars"])
        self.assertTrue(exchange(preview)["focus"]["rendered"])
        reads = list(backend.reads)
        backend.meta["total_chars"] += 1  # Missed event; same binding/epoch/caret.
        self.assertEqual(exchange(preview)["error"], "stale_binding")
        self.assertEqual(renders, [True])
        self.assertEqual(backend.reads, reads)

    def test_partial_frame_waits_for_terminator(self):
        client, fd = self.client()
        client.sendall(b'{"schema":')
        self.assertTrue(self.daemon.read(fd, FakeGLib.IO_IN))
        self.assertEqual(self.daemon.clients[fd]["buffer"], b'{"schema":')

    def test_existing_instance_not_replaced(self):
        other = Daemon(self.path, None, FakeGLib)
        try:
            with self.assertRaises(BlockingIOError):
                other.bind()
        finally:
            if other.lock_fd is not None:
                os.close(other.lock_fd)
        self.assertTrue(self.path.is_socket())

    def test_invalidation_contains_no_document_payload(self):
        client, fd = self.client()
        self.daemon.owner = self.daemon.clients[fd]
        self.daemon.observer.tracked = {"app_id": "chromium", "uri": "https://example.test/private"}
        self.daemon.observer.invalidate("field_changed")
        result = json.loads(client.recv(4096))
        self.assertEqual(result, {"schema": SCHEMA, "event": "invalidate", "epoch": 2, "reason": "field_changed", "app_id": "chromium"})

    def test_status_disconnect_preserves_live_owner_preview_and_subscription(self):
        _owner_socket, owner_fd = self.client()
        owner = self.daemon.clients[owner_fd]
        self.daemon.owner = owner
        self.daemon.observer.tracked = {"app_id": "chromium", "process_id": 42}
        state = dict(self.daemon.observer.tracked)
        def forbidden(*_args):
            self.fail("A status client cannot acquire or change another client's authority/UI")
        self.daemon.observer.hide = self.daemon.observer.disarm = forbidden
        self.daemon.backend.metadata = forbidden
        client, fd = self.client()
        client.sendall(json.dumps({"schema": SCHEMA, "id": "health", "op": "status"}).encode() + b"\n")
        self.daemon.read(fd, FakeGLib.IO_IN)
        self.assertEqual(json.loads(client.recv(4096))["status"]["process_id"], os.getpid())
        self.daemon.close_client(fd)
        self.assertIs(self.daemon.owner, owner)
        self.assertEqual(self.daemon.observer.epoch, 1)
        self.assertEqual(self.daemon.observer.tracked, state)
        self.daemon.observer.hide = self.daemon.observer.disarm = lambda: None

    def test_competing_owner_is_denied_and_owner_disconnect_invalidates(self):
        _owner_socket, owner_fd = self.client()
        self.daemon.owner = self.daemon.clients[owner_fd]
        self.daemon.observer.tracked = {"app_id": "chromium"}
        client, fd = self.client()
        client.sendall(json.dumps({"schema": SCHEMA, "id": "rival", "op": "inspect", "app_id": "chromium"}).encode() + b"\n")
        self.daemon.read(fd, FakeGLib.IO_IN)
        self.assertEqual(json.loads(client.recv(4096))["error"], "observer_busy")
        self.assertEqual(self.daemon.observer.epoch, 1)
        hidden = []
        self.daemon.observer.hide = lambda: hidden.append(True)
        self.daemon.close_client(owner_fd)
        self.assertIsNone(self.daemon.owner)
        self.assertIsNone(self.daemon.observer.tracked)
        self.assertEqual(self.daemon.observer.epoch, 2)
        self.assertEqual(hidden, [True])
        client.settimeout(.01)
        with self.assertRaises(TimeoutError):
            client.recv(4096)  # Status/non-owner clients never receive field events.

    def test_callbacks_from_closed_connection_cannot_touch_reused_fd(self):
        _old_socket, old_fd = self.client()
        old = self.daemon.clients[old_fd]
        self.daemon.close_client(old_fd)
        _client, fd = self.client()
        current = self.daemon.clients[fd]
        current["buffer"] = json.dumps({"schema": SCHEMA, "id": "fresh", "op": "status"}).encode() + b"\n"
        current["scheduled"] = True
        before = dict(current)
        self.assertFalse(self.daemon.process_next(fd, old))
        self.assertFalse(self.daemon.read(fd, FakeGLib.IO_HUP, old))
        self.assertEqual(current, before)
        self.assertIs(self.daemon.clients[fd], current)


class PreviewLifecycleTests(unittest.TestCase):
    def setUp(self):
        self.daemon = Daemon("/unused/accessibility.sock", None, FakeGLib)
        self.builds = []

    def fake_module(self, preview):
        def build():
            self.builds.append(True)
            if isinstance(preview, Exception):
                raise preview
            return preview
        return unittest.mock.patch.dict(sys.modules, {"preview": SimpleNamespace(Preview=build)})

    def test_a_failed_construction_is_remembered(self):
        with self.fake_module(RuntimeError("preview_wayland_unavailable")):
            self.assertFalse(self.daemon.warm_preview())
            self.assertFalse(self.daemon.render_preview({}, " text", 1000))
            self.assertFalse(self.daemon.render_preview({}, " text", 1000))
            self.daemon.hide_preview()
        self.assertEqual(self.builds, [True])

    def test_warming_builds_once_and_rendering_reuses_it(self):
        calls = []
        preview = SimpleNamespace(warm=lambda: calls.append("warm"), hide=lambda: calls.append("hide"),
                                  render=lambda focus, text, ttl: calls.append(("render", text, ttl)) or True)
        with self.fake_module(preview):
            self.assertFalse(self.daemon.warm_preview(), "an idle callback runs once")
            self.assertTrue(self.daemon.render_preview({}, " text", 1000))
        self.assertEqual((self.builds, calls), ([True], ["warm", ("render", " text", 1000)]))

    def test_a_render_failure_hides_and_reports_no_preview(self):
        calls = []

        def broken(*_args):
            raise RuntimeError("gtk")
        preview = SimpleNamespace(warm=lambda: None, hide=lambda: calls.append("hide"), render=broken)
        with self.fake_module(preview):
            self.assertFalse(self.daemon.render_preview({}, " text", 1000))
        self.assertEqual(calls, ["hide"])


class EventTests(unittest.TestCase):
    def setUp(self):
        self.daemon = Daemon("/unused/accessibility.sock", None, FakeGLib)
        self.tracked = {"app_id": "chromium", "process_id": 42,
                        "bus": ":1.7", "path": "/field"}
        self.daemon.observer.tracked = dict(self.tracked)
        self.notifications, self.hidden, self.disarmed = [], [], []
        self.daemon.observer.notify = self.notifications.append
        self.daemon.observer.hide = lambda: self.hidden.append(True)
        self.daemon.observer.disarm = lambda: self.disarmed.append(True)

    def assert_invalidated(self, reason):
        self.assertIsNone(self.daemon.observer.tracked)
        self.assertEqual(self.daemon.observer.epoch, 2)
        self.assertEqual(self.notifications, [{"schema": SCHEMA, "event": "invalidate",
                         "epoch": 2, "reason": reason, "app_id": "chromium"}])
        self.assertEqual((self.hidden, self.disarmed), ([True], [True]))

    def test_any_focus_change_invalidates_without_querying_its_source(self):
        class Event:
            def __getattr__(self, name):
                raise AssertionError("A focus event is never inspected: " + name)
        self.daemon.focus_event(Event())
        self.assert_invalidated("focus_changed")

    def test_focus_changes_without_a_tracked_field_are_ignored(self):
        self.daemon.observer.tracked = None
        self.daemon.focus_event(object())
        self.assertEqual((self.daemon.observer.epoch, self.notifications), (1, []))

    def test_field_events_invalidate_without_unpacking_their_payload(self):
        class Payload:
            def __getattr__(self, name):
                raise AssertionError("A field event payload is never read: " + name)
        self.daemon.field_event(None, ":1.7", "/field", "org.a11y.atspi.Event.Object", "TextChanged", Payload(), None)
        self.assert_invalidated("field_changed")

    def test_only_the_focus_listener_is_global(self):
        registered = []
        listener = SimpleNamespace(register=registered.append)
        self.daemon.A = SimpleNamespace(EventListener=SimpleNamespace(new=lambda callback, _data: listener))
        self.daemon.connect_event_bus = lambda: None
        with tempfile.TemporaryDirectory() as runtime, unittest.mock.patch.dict(
                os.environ, {"XDG_RUNTIME_DIR": runtime, "HYPRLAND_INSTANCE_SIGNATURE": "private"}), \
                self.assertRaises(OSError):
            self.daemon.watch()  # Stops at the missing private Hyprland event socket.
        self.assertIsNone(self.daemon.window_events)
        self.assertEqual(registered, ["object:state-changed:focused"])
        self.assertEqual(self.daemon.backend.desktop.request_socket, pathlib.Path(runtime) / "hypr/private/.socket.sock")

    def test_hyprland_session_must_be_exact(self):
        for environment in ({"XDG_RUNTIME_DIR": "/run/user/1", "HYPRLAND_INSTANCE_SIGNATURE": ""},
                            {"XDG_RUNTIME_DIR": "/run/user/1", "HYPRLAND_INSTANCE_SIGNATURE": "../other"},
                            {"XDG_RUNTIME_DIR": "/run/user/1", "HYPRLAND_INSTANCE_SIGNATURE": ".."},
                            {"XDG_RUNTIME_DIR": "relative", "HYPRLAND_INSTANCE_SIGNATURE": "private"}):
            with self.subTest(environment=environment), unittest.mock.patch.dict(os.environ, environment), \
                    self.assertRaisesRegex(RuntimeError, "hyprland_session_required"):
                observer_desktop.hyprland_directory()


class FieldSubscriptionTests(unittest.TestCase):
    """Producer interest follows the armed application; the match follows the field."""

    class Bus:
        def __init__(self):
            self.calls, self.subscriptions = [], []

        def is_closed(self):
            return False

        def call_sync(self, *args):
            self.calls.append(("sync", args[3], args[4].unpack()))

        def call(self, *args):
            self.calls.append(("async", args[3], args[4].unpack()))

        def signal_subscribe(self, sender, interface, member, path, arg0, _flags, _callback, _data):
            self.subscriptions.append((sender, interface, member, path, arg0))
            return len(self.subscriptions)

        def signal_unsubscribe(self, subscription):
            self.subscriptions[subscription - 1] = None

    def setUp(self):
        from gi.repository import GLib
        self.daemon = Daemon("/unused/accessibility.sock", None, GLib)
        self.bus = self.daemon.event_bus = self.Bus()

    def binding(self, bus=":1.7", path="/field", epoch=1):
        return {"epoch": epoch, "bus": bus, "path": path, "process_id": 42, "app_id": "chromium", "uri": ""}

    def test_arming_registers_the_application_once_and_matches_only_the_field(self):
        self.daemon.arm_field_events(self.binding())
        self.assertEqual(self.bus.calls, [("sync", "RegisterEvent", (event, [], ":1.7")) for event in observer_daemon.FIELD_EVENTS])
        self.assertEqual(self.bus.subscriptions, [(":1.7", "org.a11y.atspi.Event.Object", None, "/field", None)])
        self.daemon.disarm_field_events()
        self.assertEqual(self.bus.subscriptions, [None])
        self.daemon.arm_field_events(self.binding(path="/other", epoch=2))
        self.assertEqual(len(self.bus.calls), len(observer_daemon.FIELD_EVENTS), "same application: no new registration")
        self.assertEqual(self.bus.subscriptions[-1], (":1.7", "org.a11y.atspi.Event.Object", None, "/other", None))

    def test_another_application_or_window_withdraws_producer_interest(self):
        self.daemon.arm_field_events(self.binding())
        self.daemon.arm_field_events(self.binding(bus=":1.8", epoch=2))
        count = len(observer_daemon.FIELD_EVENTS)
        self.assertEqual(self.bus.calls[count:2 * count], [("async", "DeregisterEvent", (event, ":1.7"))
                                                           for event in observer_daemon.FIELD_EVENTS])
        self.assertEqual(self.bus.calls[2 * count:], [("sync", "RegisterEvent", (event, [], ":1.8"))
                                                      for event in observer_daemon.FIELD_EVENTS])
        self.assertEqual(self.bus.subscriptions, [None, (":1.8", "org.a11y.atspi.Event.Object", None, "/field", None)])
        self.daemon.window_events = WindowEvents(SimpleNamespace(recv=lambda _size: b"activewindowv2>>55\n"))
        self.daemon.loop = None
        self.daemon.observer.tracked = {"app_id": "chromium"}
        self.assertTrue(self.daemon.window_event(None, 0))
        self.assertIsNone(self.daemon.observer.tracked)
        self.assertEqual(self.bus.calls[3 * count:], [("async", "DeregisterEvent", (event, ":1.8"))
                                                      for event in observer_daemon.FIELD_EVENTS])
        self.assertIsNone(self.daemon.producer)
        self.assertEqual(self.bus.subscriptions[-1], None, "invalidation removed the field match")


class IdentityTests(unittest.TestCase):
    CURSOR = "/usr/share/cursor/resources/app/cursor.mjs"

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.uid = os.getuid()

    def process(self, executable, cmdline=None, pid=4242):
        # A task-owned stand-in for /proc/PID: exe is a symlink, as in procfs.
        folder = self.root / "proc" / str(pid)
        folder.mkdir(parents=True)
        (folder / "exe").symlink_to(executable)
        if cmdline is not None:
            (folder / "cmdline").write_bytes(cmdline)
        return pid

    def verify(self, pid, app_id, uid=None, config=None):
        return verify_process(pid, APPS[app_id], self.uid if uid is None else uid,
                              config or self.root / "config", self.root / "proc")

    def discord_tree(self, version="app-1.0.9212", config=None):
        base = (config or self.root / "config") / "discord" / version
        base.mkdir(parents=True)
        binary = base / "Discord"
        binary.write_bytes(b"ELF")
        binary.chmod(0o755)
        return binary

    def test_supported_ids_match_the_fcitx_addon(self):
        self.assertEqual(set(APPS), {"chromium", "chromium-browser", "brave-origin", "zen", "chatgpt", "code",
                                     "cursor", "discord", "telegram", "omawrite"})
        self.assertEqual({app for app, rule in APPS.items() if rule.browser},
                         {"chromium", "chromium-browser", "brave-origin", "zen"})
        self.assertEqual({app for app, rule in APPS.items() if rule.gecko}, {"zen"})
        self.assertEqual((APPS["zen"].executables, APPS["zen"].classes), ({"/opt/zen-browser-bin/zen-bin"}, {"zen"}))
        self.assertEqual({app for app, rule in APPS.items() if not rule.web}, {"telegram", "omawrite"})
        self.assertEqual(APPS["telegram"].classes, {"org.telegram.desktop"})

    def test_exact_executables_and_owner(self):
        for app_id, executable in (("chromium", "/usr/lib/chromium/chromium"), ("brave-origin", "/opt/brave-origin-bin/brave"),
                                   ("zen", "/opt/zen-browser-bin/zen-bin"),
                                   ("chatgpt", "/usr/lib/chatgpt/ChatGPT"), ("code", "/usr/share/code/code"),
                                   ("telegram", "/usr/bin/Telegram"), ("omawrite", "/usr/bin/omawrite")):
            with self.subTest(app_id=app_id):
                self.setUp()
                pid = self.process(executable)
                self.assertTrue(self.verify(pid, app_id))
                self.assertFalse(self.verify(pid, app_id, uid=self.uid + 1), "another user's process")
                others = [other for other in APPS if other != app_id and not APPS[other].per_user
                          and not APPS[other].executables & APPS[app_id].executables]
                self.assertFalse(any(self.verify(pid, other) for other in others), "no inherited permission")
        for executable in ("/usr/lib/chromium/chromium (deleted)", "/usr/lib/chromium/chromium-browser",
                           "/opt/brave-origin-bin/brave-wrapper", "/usr/bin/code", "/tmp/usr/share/code/code",
                           # Zen's shell wrapper and launcher are not the running browser.
                           "/usr/bin/zen-browser", "/opt/zen-browser-bin/zen", "/opt/zen-browser-bin/zen-bin (deleted)"):
            with self.subTest(executable=executable):
                self.setUp()
                pid = self.process(executable)
                self.assertFalse(any(self.verify(pid, app_id) for app_id in APPS))
        self.assertFalse(self.verify(999999, "chromium"), "missing process")

    def test_directly_launched_chromium_is_the_same_executable_under_its_own_class(self):
        # Without /usr/bin/chromium's CHROME_DESKTOP the Wayland app id is
        # chromium-browser; the alias shares only the executable.
        self.assertEqual(APPS["chromium-browser"].executables, APPS["chromium"].executables)
        self.assertEqual(APPS["chromium-browser"].classes, {"chromium-browser"})
        self.assertEqual(APPS["chromium"].classes, {"chromium"})
        self.assertTrue(APPS["chromium-browser"].browser and APPS["chromium-browser"].web)
        pid = self.process("/usr/lib/chromium/chromium")
        self.assertTrue(self.verify(pid, "chromium-browser"))
        self.assertFalse(self.verify(pid, "brave-origin"))

    def test_cursor_requires_its_app_on_the_shared_electron_runtime(self):
        electron = "/usr/lib/electron42/electron"
        good = [b"%s\0%s\0" % (electron.encode(), self.CURSOR.encode()),
                b"%s\0--ozone-platform=wayland\0--enable-wayland-ime\0%s\0--new-window\0" % (electron.encode(), self.CURSOR.encode()),
                b"%s\0--\0%s\0" % (electron.encode(), self.CURSOR.encode())]
        bad = [b"%s\0/usr/lib/other/app.mjs\0" % electron.encode(),
               b"%s\0--ozone-platform=wayland\0" % electron.encode(),
               b"%s\0--user-data-dir\0/tmp/profile\0%s\0" % (electron.encode(), self.CURSOR.encode()),
               b"%s\0cursor.mjs\0" % electron.encode(),
               b"%s\0%s/../evil.mjs\0" % (electron.encode(), self.CURSOR.encode()),
               b"%s\0--\0-evil.mjs\0%s\0" % (electron.encode(), self.CURSOR.encode()),
               b"%s\0%s\0%s\0" % (electron.encode(), b"--x=" + b"y" * (64 * 1024), self.CURSOR.encode()),
               # Electron's default_app loads --app= and preloads -r/--require
               # modules; REPL, version and ABI options run no app.
               b"%s\0--app=/tmp/other/main.js\0%s\0" % (electron.encode(), self.CURSOR.encode()),
               b"%s\0-r\0%s\0/tmp/other/main.js\0" % (electron.encode(), self.CURSOR.encode()),
               b"%s\0--require\0/tmp/preload.js\0%s\0" % (electron.encode(), self.CURSOR.encode()),
               *(b"%s\0%s\0%s\0" % (electron.encode(), option, self.CURSOR.encode())
                 for option in (b"-i", b"--interactive", b"-repl", b"-v", b"--version", b"-a", b"--abi")),
               b"%s\0--\0--app=/tmp/other/main.js\0%s\0" % (electron.encode(), self.CURSOR.encode()),
               b""]
        for index, cmdline in enumerate(good + bad):
            with self.subTest(index=index):
                self.setUp()
                pid = self.process(electron, cmdline)
                self.assertEqual(self.verify(pid, "cursor"), index < len(good))
                self.assertFalse(self.verify(pid, "code"))
        self.setUp()
        self.assertFalse(self.verify(self.process(electron), "cursor"), "unreadable cmdline")
        self.setUp()
        self.assertFalse(self.verify(self.process("/usr/lib/electron41/electron", good[0]), "cursor"))
        self.assertIsNone(application_entry(b"electron\0--flag\0"))
        self.assertIsNone(application_entry(b"electron\0--app=/tmp/other/main.js\0" + self.CURSOR.encode()))
        self.assertIsNone(application_entry(b"electron\0-r\0" + self.CURSOR.encode() + b"\0/tmp/other/main.js"))
        self.assertEqual(application_entry(b"electron\0--no-help\0--test-type=webdriver\0" + self.CURSOR.encode()),
                         self.CURSOR.encode(), "default_app options that change no loaded code are ordinary switches")
        # The running Cursor GUI rewrites its title into one space-joined argument.
        title = (b"/usr/lib/electron42/electron --user-data-dir=/tmp/p --new-window --disable-extensions "
                 + self.CURSOR.encode() + b" /tmp/scratch.txt \0")
        self.assertEqual(application_entry(title), self.CURSOR.encode())
        self.setUp()
        self.assertTrue(self.verify(self.process(electron, title), "cursor"))
        self.assertIsNone(application_entry(b"/usr/lib/electron42/electron --app=/tmp/x.js " + self.CURSOR.encode()))
        self.assertNotEqual(application_entry(b"/usr/lib/electron42/electron /tmp/other app/main.js\0"),
                            self.CURSOR.encode(), "a path with spaces yields a non-matching token")

    def test_discord_accepts_only_its_own_per_user_build(self):
        binary = self.discord_tree()
        pid = self.process(str(binary))
        self.assertTrue(self.verify(pid, "discord"))
        self.assertFalse(self.verify(pid, "discord", uid=self.uid + 1))
        self.assertFalse(self.verify(pid, "discord", config=self.root / "other-config"), "another configuration home")
        self.assertFalse(self.verify(pid, "chromium"))

    def test_discord_path_tricks_fail_closed(self):
        binary = self.discord_tree()
        config = self.root / "config"
        running = binary.stat()

        def check(executable, running=running, uid=self.uid):
            return per_user_executable(str(executable), running, config, uid)

        self.assertTrue(check(binary))
        base = config / "discord"
        for executable in (f"{base}/app-1.0.9212/../app-1.0.9212/Discord", f"{base}/./app-1.0.9212/Discord",
                           f"{base}//app-1.0.9212/Discord", f"{base}/app-1.0.9212/Discord (deleted)",
                           f"{base}/app-1.0.9212/discord", f"{base}/app-1.0.x/Discord", f"{base}/app-/Discord",
                           f"{base}/app-1.2.3.4.5/Discord", f"{base}/Discord", f"{base}/app-1.0.9212/sub/Discord",
                           "discord/app-1.0.9212/Discord"):
            self.assertFalse(check(executable), executable)
        self.assertFalse(check(binary, uid=self.uid + 1), "foreign owner")
        self.assertFalse(check(binary, running=(self.root).stat()), "replaced after launch")
        binary.chmod(0o775)
        self.assertFalse(check(binary), "group-writable build")
        binary.chmod(0o755)
        # A symlinked version directory or binary may point anywhere.
        elsewhere = self.root / "elsewhere"
        elsewhere.mkdir()
        (elsewhere / "Discord").write_bytes(b"ELF")
        (elsewhere / "Discord").chmod(0o755)
        (base / "app-9.9.9").symlink_to(elsewhere, target_is_directory=True)
        self.assertFalse(check(base / "app-9.9.9/Discord", running=(elsewhere / "Discord").stat()))
        (base / "app-2.0.0").mkdir()
        (base / "app-2.0.0/Discord").symlink_to(elsewhere / "Discord")
        self.assertFalse(check(base / "app-2.0.0/Discord", running=(elsewhere / "Discord").stat()))
        # The kernel reports the resolved path of a symlinked tree; it is outside the base.
        self.assertFalse(check(elsewhere / "Discord", running=(elsewhere / "Discord").stat()))
        linked = self.root / "linked-config"
        linked.mkdir()
        (linked / "discord").symlink_to(base, target_is_directory=True)
        self.assertFalse(per_user_executable(str(linked / "discord/app-1.0.9212/Discord"), running, linked, self.uid))
        self.assertFalse(per_user_executable(str(binary), running, linked, self.uid))

    def test_symlinked_configuration_home_resolves_like_the_kernel_path(self):
        real = self.root / "real-config"
        binary = self.discord_tree(config=real)
        alias = self.root / "alias-config"
        alias.symlink_to(real, target_is_directory=True)
        self.assertTrue(per_user_executable(str(binary), binary.stat(), alias, self.uid))


class WindowTests(unittest.TestCase):
    def backend(self, window):
        backend = FieldBackend(None, APPS).desktop
        backend.request = lambda request: self.assertEqual(request, "j/activewindow") or window
        return backend

    def test_class_uid_and_executable_rules_all_apply(self):
        window = {"pid": 42, "class": "org.telegram.desktop", "mapped": True, "hidden": False}
        checked = []
        with unittest.mock.patch.object(observer_desktop, "verify_process", side_effect=lambda pid, app, uid, config: checked.append((pid, app, uid)) or True):
            self.assertIs(self.backend(window).app("telegram"), APPS["telegram"])
            self.assertEqual(self.backend(window).window(APPS["telegram"]), window)
            self.assertEqual(checked, [(42, APPS["telegram"], os.getuid())])
            for change in ({"class": "telegram"}, {"class": "code"}, {"mapped": False}, {"hidden": True}):
                with self.assertRaisesRegex(Denied, "app_mismatch"):
                    self.backend({**window, **change}).window(APPS["telegram"])
            with self.assertRaisesRegex(Denied, "unsupported_app"):
                self.backend(window).app("org.telegram.desktop")
            chromium = {"pid": 42, "class": "chromium-browser", "mapped": True, "hidden": False}
            self.assertEqual(self.backend(chromium).window(APPS["chromium-browser"]), chromium)
            for app_id, window_class in (("chromium", "chromium-browser"), ("chromium-browser", "chromium")):
                with self.assertRaisesRegex(Denied, "app_mismatch"):
                    self.backend({**chromium, "class": window_class}).window(APPS[app_id])
            zen = {"pid": 42, "class": "zen", "mapped": True, "hidden": False}
            self.assertEqual(self.backend(zen).window(APPS["zen"]), zen)
            for window_class in ("zen-browser", "app.zen_browser.zen", "firefox", "Zen"):
                with self.assertRaisesRegex(Denied, "app_mismatch"):
                    self.backend({**zen, "class": window_class}).window(APPS["zen"])
            for app_id in ("zen-browser", "firefox"):
                with self.assertRaisesRegex(Denied, "unsupported_app"):
                    self.backend(zen).app(app_id)
            with self.assertRaisesRegex(Denied, "focus_unavailable"):
                self.backend({**window, "pid": 0}).window(APPS["telegram"])
            with self.assertRaisesRegex(Denied, "focus_unavailable"):
                self.backend([window]).window(APPS["telegram"])
        with unittest.mock.patch.object(observer_desktop, "verify_process", return_value=False):
            with self.assertRaisesRegex(Denied, "app_mismatch"):
                self.backend(window).window(APPS["telegram"])


UNLOCKED = {key: False for key in observer_desktop.LOCK_FLAGS}


class LockReplyTests(unittest.TestCase):
    LOCK_QUERY = ("qs", "ipc", "-n", "-p", "/usr/share/omarchy/shell", "call", "--", "lock", "status")

    def setUp(self):
        environment = unittest.mock.patch.dict(os.environ, {"OMARCHY_PATH": "/usr/share/omarchy"})
        environment.start()
        self.addCleanup(environment.stop)
        self.backend = FieldBackend(None, APPS)
        self.backend.deadline = time.monotonic() + 1

    def check_lock(self, reply=b"", returncode=0, failure=None):
        process = unittest.mock.Mock(returncode=returncode)
        process.communicate.side_effect = failure
        process.communicate.return_value = (reply, None)
        with unittest.mock.patch.object(observer_desktop.subprocess, "Popen", return_value=process) as command:
            try:
                with self.backend.desktop.lock_query():
                    pass
            finally:
                self.assertEqual(command.call_args.args[0], self.LOCK_QUERY)
                self.assertLessEqual(process.communicate.call_args.kwargs["timeout"], .15)
                process.kill.assert_called_once_with()
                process.wait.assert_called_once_with()

    def test_every_lock_flag_denies_and_only_an_explicit_unlock_passes(self):
        self.check_lock(json.dumps(UNLOCKED).encode())
        for state in ({**UNLOCKED, "requested": True}, {**UNLOCKED, "pending": True},
                      {**UNLOCKED, "sessionLocked": True}, {**UNLOCKED, "secure": None},
                      {key: False for key in observer_desktop.LOCK_FLAGS[1:]}, [], "unlocked"):
            with self.subTest(state=state), self.assertRaisesRegex(Denied, "desktop_locked"):
                self.check_lock(json.dumps(state).encode())

    def test_an_unanswered_or_unexpected_lock_query_fails_closed(self):
        # The replies omarchy-shell turns into failures arrive on stdout with exit 0.
        for reply in (b"Target not found.\n", b"Function not found.\n", b"Not ready to accept queries yet\n", b""):
            with self.subTest(reply=reply), self.assertRaisesRegex(Denied, "desktop_unavailable"):
                self.check_lock(reply)
        with self.assertRaisesRegex(Denied, "desktop_locked"):
            self.check_lock(b"{" + b" " * observer_desktop.MAX_IPC_REPLY + b"}")
        with self.assertRaisesRegex(Denied, "desktop_unavailable"):
            self.check_lock(json.dumps(UNLOCKED).encode(), returncode=1)
        for failure in (observer_desktop.subprocess.TimeoutExpired(self.LOCK_QUERY, .15), OSError("pipe")):
            with self.subTest(failure=type(failure).__name__), self.assertRaisesRegex(Denied, "desktop_unavailable"):
                self.check_lock(failure=failure)
        with unittest.mock.patch.object(observer_desktop.subprocess, "Popen", side_effect=FileNotFoundError("qs")), \
             self.assertRaisesRegex(Denied, "desktop_unavailable"):
            with self.backend.desktop.lock_query():
                self.fail("no field is read without a lock query")
        for path in ("", "relative/omarchy"):
            with self.subTest(path=path), unittest.mock.patch.dict(os.environ, {"OMARCHY_PATH": path}), \
                 unittest.mock.patch.object(observer_desktop.subprocess, "Popen") as command, \
                 self.assertRaisesRegex(Denied, "desktop_unavailable"):
                with self.backend.desktop.lock_query():
                    self.fail("no field is read without a lock query")
            command.assert_not_called()

    def test_only_inspect_checks_the_session_lock(self):
        backend = FieldBackend(None, APPS)
        backend.deadline = time.monotonic() + 1
        backend.field_metadata = lambda app_id, app, calibrate: {"app_id": app_id}
        queries = []
        backend.desktop.lock_query = lambda: queries.append(True) or contextlib.nullcontext()
        backend.metadata("telegram")
        backend.metadata("telegram", True)
        self.assertEqual(queries, [])
        backend.metadata("telegram", check_lock=True)
        self.assertEqual(queries, [True])
        with self.assertRaisesRegex(Denied, "unsupported_app"):
            backend.metadata("org.telegram.desktop", check_lock=True)
        self.assertEqual(queries, [True], "an unsupported app starts no query")


class LockQueryProcessTests(unittest.TestCase):
    """The lock query as a real child process: a task-owned `qs` first on PATH."""

    FIELD = {**FakeBackend().meta, "app_id": "telegram", "uri": "", "browser": False, "web": False}

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.bin = pathlib.Path(temp.name)
        for patch in (unittest.mock.patch.dict(os.environ, {"OMARCHY_PATH": "/usr/share/omarchy",
                                                            "PATH": os.pathsep.join((temp.name, "/usr/bin", "/bin"))}),
                      # Generous, so only a query that never answers can be late.
                      unittest.mock.patch.object(observer_desktop, "IPC_SECONDS", 10)):
            patch.start()
            self.addCleanup(patch.stop)
        self.backend = FieldBackend(None, APPS)
        self.backend.deadline = time.monotonic() + 30
        self.queries = []
        start = self.backend.desktop.lock_query
        self.backend.desktop.lock_query = lambda: self.queries.append(start()) or self.queries[-1]

    def answer(self, script):
        qs = self.bin / "qs"
        qs.write_text(f"#!/bin/sh\n{script}\n")
        qs.chmod(0o700)

    def reply(self, state):
        self.answer(f"printf '%s' '{json.dumps(state)}'")

    def never_answer(self):
        self.answer("exec sleep 30")
        return unittest.mock.patch.object(observer_desktop, "IPC_SECONDS", .05)

    def assert_reaped(self, returncode):
        process = self.queries[-1].process
        self.assertEqual(process.returncode, returncode)
        with self.assertRaises(ChildProcessError):
            os.waitpid(process.pid, os.WNOHANG)

    def test_the_query_runs_while_the_field_is_read_and_answers_after_it(self):
        go = self.bin / "go"
        self.answer(f"until [ -e '{go}' ]; do sleep .01; done\nprintf '%s' '{json.dumps(UNLOCKED)}'")

        def read_field(app_id, _app, _calibrate):
            self.assertIsNone(self.queries[-1].process.poll(), "the query is still running")
            go.touch()
            return {"app_id": app_id}

        self.backend.field_metadata = read_field
        for attempt in range(2):
            go.unlink(missing_ok=True)
            self.assertEqual(self.backend.metadata("telegram", check_lock=True), {"app_id": "telegram"})
            self.assert_reaped(0)
        self.assertEqual(len(self.queries), 2, "every inspect asks again")

    def test_a_locked_session_never_yields_a_binding_however_fine_the_field(self):
        observer = Observer(self.backend)
        events = []
        observer.notify = events.append
        fields = []
        self.backend.field_metadata = lambda app_id, _app, _calibrate: fields.append(app_id) or dict(self.FIELD)
        inspect = {"schema": SCHEMA, "id": "locked", "op": "inspect", "app_id": "telegram"}
        self.reply({**UNLOCKED, "locked": True})
        reply = observer.request(inspect)
        self.assertEqual((reply["ok"], reply["error"]), (False, "desktop_locked"))
        self.assertNotIn("focus", reply)
        self.assertEqual(fields, ["telegram"])
        self.assertIsNone(observer.tracked)
        self.assertEqual(events, [])
        self.assert_reaped(0)

        def unavailable(*_args):
            raise Denied("focus_unavailable")
        self.backend.field_metadata = unavailable
        self.assertEqual(observer.request(inspect)["error"], "desktop_locked", "the lock verdict comes first")
        self.assert_reaped(0)

        self.backend.field_metadata = lambda app_id, _app, _calibrate: dict(self.FIELD)
        self.reply(UNLOCKED)
        self.assertEqual(observer.request(inspect)["focus"]["binding"]["app_id"], "telegram")

    def test_a_late_query_is_killed_and_reaped(self):
        self.backend.field_metadata = lambda app_id, _app, _calibrate: dict(self.FIELD)
        with self.never_answer(), self.assertRaisesRegex(Denied, "desktop_unavailable"):
            self.backend.metadata("telegram", check_lock=True)
        self.assert_reaped(-signal.SIGKILL)

    def test_a_failed_field_read_still_reaps_the_query(self):
        def broken(*_args):
            raise RuntimeError("field")

        def unavailable(*_args):
            raise Denied("focus_unavailable")

        def out_of_time(*_args):
            self.backend.deadline = time.monotonic()
            raise Denied("operation_timeout")

        self.reply(UNLOCKED)
        for failure, error, reason in ((broken, RuntimeError, "field"), (unavailable, Denied, "focus_unavailable")):
            self.backend.field_metadata = failure
            with self.subTest(reason=reason), self.assertRaisesRegex(error, reason):
                self.backend.metadata("telegram", check_lock=True)
            self.assert_reaped(0)
        with self.never_answer():
            self.backend.field_metadata = broken
            with self.assertRaisesRegex(Denied, "desktop_unavailable"):
                self.backend.metadata("telegram", check_lock=True)
            self.assert_reaped(-signal.SIGKILL)
            self.backend.field_metadata = out_of_time
            with self.assertRaisesRegex(Denied, "operation_timeout"):
                self.backend.metadata("telegram", check_lock=True)
            self.assert_reaped(-signal.SIGKILL)


class FakeStates:
    def __init__(self, states):
        self.states = set(states)

    def contains(self, state):
        return state in self.states


class FakeNode:
    def __init__(self, states=(), pid=42, children=(), uri=None, role="entry", parent=None, interfaces=()):
        self.states, self.pid, self.children, self.uri, self.role = states, pid, list(children), uri, role
        self.parent, self.interfaces = parent, list(interfaces)

    def get_state_set(self):
        return FakeStates(self.states)

    def get_process_id(self):
        return self.pid

    def get_interfaces(self):
        return self.interfaces

    def get_child_count(self):
        return len(self.children)

    def get_child_at_index(self, index):
        return self.children[index]

    def get_parent(self):
        return self.parent

    def get_role_name(self):
        return self.role


FAKE_ATSPI = SimpleNamespace(StateType=SimpleNamespace(FOCUSED="focused", EDITABLE="editable", SHOWING="showing"),
                             get_desktop=None)


class FocusSelectionTests(unittest.TestCase):
    PAGE = "https://example.test/"
    POPUP = "chrome://omnibox-popup.top-chrome/omnibox_popup_aim.html"

    def select(self, nodes, browser=True):
        atspi = SimpleNamespace(**vars(FAKE_ATSPI))
        atspi.get_desktop = lambda _index: FakeNode(children=[FakeNode(children=nodes)])
        backend = FieldBackend(atspi, APPS)
        backend.budget = lambda: None

        def uri(node):
            # None: no document ancestor at all, like Chromium's omnibox.
            if node.uri == "unreadable":
                raise Denied("origin_unavailable")
            return node.uri
        backend.nearest_document_uri = uri
        return backend.focused(42, browser)

    def field(self, uri=PAGE, states=("focused", "editable", "showing"), **kwargs):
        return FakeNode(set(states), uri=uri, **kwargs)

    def test_chromium_browser_ui_focus_is_ignored_for_the_one_page_field(self):
        # Chromium 152, verified live: with the page focused, the omnibox
        # popup's WebUI combo box also reports focused+editable+showing; with
        # the omnibox focused, the omnibox entry and the page field both do.
        field = self.field()
        popup = self.field(self.POPUP, role="combo box")
        omnibox = self.field(None)
        for nodes in ([popup, field], [field, popup], [omnibox, field, popup], [self.field(None, ("focused", "editable")), field]):
            with self.subTest(nodes=[node.uri for node in nodes]):
                self.assertIs(self.select(nodes), field)
        self.assertIs(self.select([self.field(None, ("focused", "showing")), field]), field, "non-editable focus is not a field")

    def test_zero_or_several_page_fields_fail_closed(self):
        field = self.field()
        for nodes in ([], [self.field(None)], [self.field(self.POPUP, role="combo box")],
                      [field, self.field("https://other.test/")],
                      [field, self.field(self.PAGE, ("focused", "editable"))],
                      # Other documents are page content, never browser UI.
                      [field, self.field("devtools://devtools/bundled/devtools_app.html")],
                      [field, self.field("chrome://settings/")],
                      [field, self.field("chrome-extension://abcdefghijklmnop/popup.html")],
                      [field, self.field("about:blank")],
                      [field, self.field("unreadable")]):
            with self.subTest(nodes=[node.uri for node in nodes]), self.assertRaisesRegex(Denied, "focus_unavailable"):
                self.select(nodes)

    def test_the_one_page_field_must_have_a_web_origin_in_this_process(self):
        for field in (self.field("chrome://newtab/"), self.field("chrome://new-tab-page/"), self.field("about:blank"),
                      self.field("file:///tmp/page.html"), self.field("unreadable"), self.field(pid=43)):
            with self.subTest(uri=field.uri, pid=field.pid), self.assertRaisesRegex(Denied, "focus_unavailable"):
                self.select([field])
        self.assertIs(self.select([field := self.field("http://127.0.0.1:8080/page")]), field)

    def test_only_top_chrome_webui_is_browser_interface(self):
        for uri in ("chrome://omnibox-popup.top-chrome/", self.POPUP, "chrome://tab-search.top-chrome/"):
            self.assertTrue(browser_interface(uri), uri)
        for uri in ("chrome://settings/", "chrome://top-chrome/", "chrome://evil.top-chrome.example/",
                    "chrome-untrusted://x.top-chrome/", "https://omnibox-popup.top-chrome/", "devtools://devtools/",
                    "http://[::1", ""):
            self.assertFalse(browser_interface(uri), uri)

    def test_desktop_apps_require_exactly_one_focused_editable(self):
        field = FakeNode({"focused", "editable", "showing"})
        self.assertIs(self.select([field], browser=False), field)
        with self.assertRaisesRegex(Denied, "focus_unavailable"):
            self.select([FakeNode({"focused", "editable"}), field], browser=False)
        with self.assertRaisesRegex(Denied, "focus_unavailable"):
            self.select([FakeNode({"focused", "editable", "showing"}, pid=43)], browser=False)

    def test_origin_timeout_is_not_mistaken_for_browser_ui(self):
        atspi = SimpleNamespace(**vars(FAKE_ATSPI))
        atspi.get_desktop = lambda _index: FakeNode(children=[FakeNode(children=[FakeNode({"focused", "editable", "showing"})])])
        backend = FieldBackend(atspi, APPS)
        backend.budget = lambda: None
        def timeout(_node):
            raise Denied("operation_timeout")
        backend.nearest_document_uri = timeout
        with self.assertRaisesRegex(Denied, "operation_timeout"):
            backend.focused(42, True)

    def test_nearest_document_uri_distinguishes_no_document_from_unreadable(self):
        atspi = SimpleNamespace(Document=SimpleNamespace(get_document_attribute_value=lambda document, _name: document))
        backend = FieldBackend(atspi, APPS)
        backend.budget = lambda: None

        class Document(FakeNode):
            def get_document_iface(self):
                return self.uri
        application = FakeNode(role="application")
        page = Document(uri=self.PAGE, role="document web", parent=application, interfaces=["Document"])
        blank = Document(uri="", role="document web", parent=page, interfaces=["Document"])
        self.assertEqual(backend.nearest_document_uri(FakeNode(parent=blank)), self.PAGE, "an empty URI is skipped")
        self.assertIsNone(backend.nearest_document_uri(FakeNode(parent=application)))
        with self.assertRaisesRegex(Denied, "origin_unavailable"):
            backend.document_uri(FakeNode(parent=application))
        huge = Document(uri="https://example.test/" + "a" * 4096, role="document web", parent=application, interfaces=["Document"])
        with self.assertRaisesRegex(Denied, "origin_unavailable"):
            backend.nearest_document_uri(FakeNode(parent=huge))
        chain = application
        for _ in range(30):
            chain = FakeNode(role="section", parent=chain)
        with self.assertRaisesRegex(Denied, "origin_unavailable"):
            backend.nearest_document_uri(chain)
        backend.nearest_document_uri = lambda _node: (_ for _ in ()).throw(Denied("origin_unavailable"))
        self.assertTrue(backend.page_content(FakeNode()), "an unreadable document is never browser UI")

    def test_frame_and_nearest_document_ancestors(self):
        backend = FieldBackend(None, APPS)
        backend.budget = lambda: None
        application = FakeNode(role="application")
        frame = FakeNode(role="frame", parent=application)
        outer = FakeNode(role="document web", parent=FakeNode(role="panel", parent=frame))
        inner = FakeNode(role="document web", parent=FakeNode(role="section", parent=outer))
        field = FakeNode(parent=FakeNode(role="paragraph", parent=inner))
        self.assertEqual(backend.frame_and_document(field), (frame, inner))
        self.assertEqual(backend.frame_and_document(FakeNode(parent=None)), (None, None))
        self.assertEqual(backend.frame_and_document(FakeNode(parent=frame)), (frame, None))
        chain = FakeNode(role="section")
        for _ in range(100):
            chain = FakeNode(role="section", parent=chain)
        self.assertEqual(backend.frame_and_document(FakeNode(parent=chain)), (None, None))


class GeckoDocument(FakeNode):
    """A Gecko Document: DocURL carries the address and URI is always empty."""

    def __init__(self, doc_url, role="document web", uri="", **kwargs):
        super().__init__(role=role, interfaces=["Component", "Document"], **kwargs)
        self.attributes = {"DocURL": doc_url, "URI": uri, "MimeType": "text/html"}

    def get_document_iface(self):
        return self


GECKO_ATSPI = SimpleNamespace(**{**vars(FAKE_ATSPI), "Document": SimpleNamespace(
    get_document_attribute_value=lambda document, name: document.attributes.get(name))})
CHROME_URL = "chrome://browser/content/browser.xhtml"


def gecko_window():
    """Zen 1.22.3b's live ancestry: application > frame (itself the chrome Document)."""
    application = FakeNode(role="application")
    return application, GeckoDocument(CHROME_URL, role="frame", parent=application)


def gecko_page(frame, url):
    # document web > internal frame > scroll pane > panel > frame, as probed.
    return GeckoDocument(url, parent=FakeNode(role="internal frame", parent=FakeNode(
        role="scroll pane", parent=FakeNode(role="panel", parent=frame))))


def gecko_field(parent, role="entry", states=("focused", "editable", "showing"), pid=42):
    return FakeNode(set(states), pid=pid, role=role, parent=parent, interfaces=["Text", "Component"])


def gecko_urlbar(frame):
    # Role combo box, tag input; section > panel > frame, no document web.
    return gecko_field(FakeNode(role="section", parent=FakeNode(role="panel", parent=frame)), role="combo box")


class GeckoFocusTests(unittest.TestCase):
    PAGE = "http://127.0.0.1:47811/token/fixture"

    def setUp(self):
        self.application, self.frame = gecko_window()
        self.document = gecko_page(self.frame, self.PAGE)
        self.backend = FieldBackend(GECKO_ATSPI, APPS)
        self.backend.budget = lambda: None

    def select(self, nodes):
        self.backend.atspi = SimpleNamespace(**vars(GECKO_ATSPI))
        self.backend.atspi.get_desktop = lambda _index: FakeNode(children=[FakeNode(children=nodes)])
        return self.backend.focused(42, True, True)

    def test_the_one_focused_page_field_is_selected(self):
        field = gecko_field(self.document)
        self.assertIs(self.select([field]), field)
        nested = gecko_field(FakeNode(role="section", parent=FakeNode(role="form", parent=self.document)))
        self.assertIs(self.select([nested]), nested)
        self.assertIs(self.select([FakeNode({"focused", "showing"}), field]), field, "non-editable focus is not a field")
        self.assertEqual(self.backend.gecko_document_url(field), self.PAGE)

    def test_the_urlbar_is_denied_by_its_document_without_a_url_purpose(self):
        # Zen's urlbar shares the page's Fcitx context (0x72, no Url purpose):
        # the addon cannot deny it, so the observer must.
        for node in (gecko_urlbar(self.frame), gecko_field(self.frame),
                     gecko_field(FakeNode(role="tool bar", parent=self.frame), role="entry")):
            with self.subTest(role=node.role), self.assertRaisesRegex(Denied, "unsupported_field"):
                self.select([node])

    def test_any_second_focused_field_is_ambiguous(self):
        # Unlike Chromium nothing is filtered: Gecko reported one focused field.
        field = gecko_field(self.document)
        other = gecko_page(self.frame, "https://other.test/")
        for nodes in ([], [gecko_urlbar(self.frame), field], [field, gecko_urlbar(self.frame)],
                      [field, gecko_field(self.document)], [field, gecko_field(other)],
                      [field, gecko_field(self.document, states=("focused", "editable"))],
                      [gecko_field(self.document, pid=43)]):
            with self.subTest(count=len(nodes)), self.assertRaisesRegex(Denied, "focus_unavailable"):
                self.select(nodes)

    def test_only_an_http_web_document_is_page_content(self):
        for url in ("about:blank", "about:newtab", "about:preferences", "about:reader?url=https://example.test/",
                    "moz-extension://4f6d0c8e/popup.html", "resource://pdf.js/web/viewer.html", CHROME_URL,
                    "chrome://devtools/content/devtools.xhtml", "file:///tmp/page.html", "data:text/html,x",
                    "view-source:https://example.test/", "https://user:secret@example.test/", "", None):
            with self.subTest(url=url), self.assertRaisesRegex(Denied, "unsupported_field"):
                self.select([gecko_field(gecko_page(self.frame, url))])
        for url in ("https://example.test/page", "http://[::1]:8080/"):
            field = gecko_field(gecko_page(self.frame, url))
            self.assertIs(self.select([field]), field)
            self.assertEqual(self.backend.gecko_document_url(field), url)

    def test_docurl_is_read_and_uri_is_not(self):
        document = GeckoDocument("", uri=self.PAGE, parent=self.frame)
        with self.assertRaisesRegex(Denied, "unsupported_field"):
            self.select([gecko_field(document)])

    def test_only_the_nearest_document_counts(self):
        # A frame's own document decides, never an outer page's address.
        loading = GeckoDocument("", parent=FakeNode(role="internal frame", parent=self.document))
        blank = GeckoDocument("about:blank", parent=FakeNode(role="internal frame", parent=self.document))
        for document in (loading, blank):
            with self.assertRaisesRegex(Denied, "unsupported_field"):
                self.select([gecko_field(document)])
        framed = GeckoDocument("https://frame.example.test/", parent=FakeNode(role="internal frame", parent=self.document))
        self.assertEqual(self.backend.gecko_document_url(gecko_field(framed)), "https://frame.example.test/")

    def test_missing_unbounded_or_oversized_documents_fail_closed(self):
        with self.assertRaisesRegex(Denied, "origin_unavailable"):
            self.select([gecko_field(self.application)])
        chain = self.document
        for _ in range(30):
            chain = FakeNode(role="section", parent=chain)
        with self.assertRaisesRegex(Denied, "origin_unavailable"):
            self.select([gecko_field(chain)])
        huge = gecko_page(self.frame, "https://example.test/" + "a" * 4096)
        with self.assertRaisesRegex(Denied, "origin_unavailable"):
            self.select([gecko_field(huge)])

    def test_deadline_propagates(self):
        field = gecko_field(self.document)
        self.backend.budget = lambda: (_ for _ in ()).throw(Denied("operation_timeout"))
        with self.assertRaisesRegex(Denied, "operation_timeout"):
            self.backend.gecko_document_url(field)

    def test_frame_and_document_follow_gecko_ancestry(self):
        field = gecko_field(self.document)
        self.assertEqual(self.backend.frame_and_document(field), (self.frame, self.document))


def rect(x, y, width, height):
    return {"x": x, "y": y, "width": width, "height": height}


class CalibrationTests(unittest.TestCase):
    WINDOW = {"pid": 42, "address": "0x1", "at": [100, 50], "size": [900, 650], "monitor": 0, "xwayland": False}
    MONITOR = {"id": 0, "name": "eDP-1", "x": 0, "y": 0, "width": 2880, "height": 1800, "scale": 2, "transform": 0}

    def calibrate(self, frame, document, field, glyph, scale=2, window=None, monitors=None, caret=12):
        monitor = {**self.MONITOR, "scale": scale}
        return calibrated_geometry(frame, document, field, glyph, window or dict(self.WINDOW),
                                   [monitor] if monitors is None else monitors, caret)

    def test_chromium_scale_two_maps_physical_glyphs_to_logical_window_coordinates(self):
        result = self.calibrate(rect(0, 0, 900, 650), rect(0, 170, 1800, 1130), rect(100, 200, 1200, 80), rect(400, 220, 20, 40))
        self.assertEqual(result["coordinate_convention"], CALIBRATED)
        self.assertEqual(result["caret"], {"x": 210, "y": 110, "height": 20})
        self.assertEqual(result["field"], {"x": 50, "y": 100, "width": 600, "height": 40})
        self.assertEqual((result["scale"], result["character_offset"]), (2, 11))
        self.assertEqual(result["window"], self.WINDOW)
        self.assertEqual(result["monitor"], {key: self.MONITOR[key] for key in ("name", "x", "y", "width", "height", "scale", "transform")})

    def test_scale_one_and_electron_fake_frame_origin(self):
        one = self.calibrate(rect(0, 0, 900, 650), rect(0, 85, 900, 565), rect(50, 100, 600, 40), rect(200, 110, 10, 20), scale=1)
        self.assertEqual(one["caret"], {"x": 210, "y": 110, "height": 20})
        # Electron's frame SCREEN origin is arbitrary, but every descendant
        # carries that origin times the scale, so it cancels.
        electron = self.calibrate(rect(5000, 3000, 900, 650), rect(10000, 6000, 1800, 1300),
                                  rect(10100, 6200, 1200, 80), rect(10400, 6220, 20, 40))
        self.assertEqual(electron["caret"], {"x": 210, "y": 110, "height": 20})
        self.assertEqual(electron["field"], {"x": 50, "y": 100, "width": 600, "height": 40})

    def test_scale_ratio_tolerance(self):
        field, glyph = rect(100, 200, 1200, 80), rect(400, 220, 20, 40)
        self.assertIsNotNone(self.calibrate(rect(0, 0, 900, 650), rect(0, 170, 1808, 1130), field, glyph))
        self.assertIsNone(self.calibrate(rect(0, 0, 900, 650), rect(0, 170, 1810, 1130), field, glyph))
        # A sidebar narrows the web view (Brave): the ratio no longer proves the convention.
        self.assertIsNone(self.calibrate(rect(0, 0, 900, 650), rect(100, 170, 1700, 1130), field, glyph))

    def test_rejections_fail_closed(self):
        frame, document, field, glyph = rect(0, 0, 900, 650), rect(0, 170, 1800, 1130), rect(100, 200, 1200, 80), rect(400, 220, 20, 40)
        cases = {
            "glyph outside field": dict(glyph=rect(1400, 220, 20, 40)),
            "glyph scrolled above field": dict(glyph=rect(400, 150, 20, 40)),
            "field outside document": dict(field=rect(100, 100, 1200, 80), glyph=rect(400, 110, 20, 40)),
            "caret outside window": dict(document=rect(0, 170, 1800, 2000), field=rect(100, 1300, 1200, 80), glyph=rect(400, 1320, 20, 40)),
            "zero glyph (Qt)": dict(glyph=rect(0, 0, 0, 0)),
            "zero frame": dict(frame=rect(0, 0, 0, 0)),
            "nan": dict(glyph=rect(float("nan"), 220, 20, 40)),
            "bool": dict(glyph=rect(True, 220, 20, 40)),
            "negative size": dict(field=rect(100, 200, -5, 80)),
            "missing key": dict(glyph={"x": 1, "y": 2, "width": 3}),
        }
        for name, change in cases.items():
            values = {"frame": frame, "document": document, "field": field, "glyph": glyph, **change}
            self.assertIsNone(self.calibrate(**values), name)
        self.assertIsNone(self.calibrate(frame, document, field, glyph, scale=1), "monitor scale mismatch")
        self.assertIsNone(self.calibrate(frame, document, field, glyph, caret=0))
        self.assertIsNone(self.calibrate(frame, document, field, glyph, window={**self.WINDOW, "xwayland": True}))
        self.assertIsNone(self.calibrate(frame, document, field, glyph, window={**self.WINDOW, "monitor": 1}))
        self.assertIsNone(self.calibrate(frame, document, field, glyph, monitors=[self.MONITOR, self.MONITOR]))
        self.assertIsNone(self.calibrate(frame, document, field, glyph, monitors=[{**self.MONITOR, "transform": 1}]))
        self.assertIsNone(self.calibrate(frame, document, field, glyph, monitors=[{**self.MONITOR, "scale": "2"}]))
        self.assertIsNone(self.calibrate(frame, document, field, glyph, monitors=["not a monitor"]))


class GeckoCalibrationTests(unittest.TestCase):
    # Zen 1.22.3b / Gecko 156.0.1 on eDP-1 at scale 2, live 2026-09-26. The
    # typed textarea's caret followed character 23 of 24; hyprctl clipped the
    # 495-pixel-tall surface to a 687x431 tile at (40, 457).
    FRAME, DOCUMENT = rect(0, 0, 687, 495), rect(8, 8, 671, 479)
    FIELD, GLYPH = rect(26, 112, 635, 55), rect(477, 248, 22, 46)
    WINDOW = {"pid": 3529152, "address": "0x2", "at": [40, 457], "size": [687, 431], "monitor": 0, "xwayland": False}
    MONITOR = CalibrationTests.MONITOR

    def calibrate(self, frame=FRAME, document=DOCUMENT, field=FIELD, glyph=GLYPH, scale=2, window=None, caret=24):
        return gecko_calibrated_geometry(frame, document, field, glyph, window or dict(self.WINDOW),
                                         [{**self.MONITOR, "scale": scale}], caret)

    def test_live_zen_extents_map_device_glyphs_and_logical_fields_to_the_window(self):
        result = self.calibrate()
        self.assertEqual(result["coordinate_convention"], CALIBRATED)
        self.assertEqual(result["caret"], {"x": 249.5, "y": 124, "height": 23})
        self.assertEqual(result["field"], {"x": 26, "y": 112, "width": 635, "height": 55})
        self.assertEqual((result["scale"], result["character_offset"]), (2, 23))
        self.assertEqual(result["window"], self.WINDOW)
        # Hyprland origin plus window-local caret: the marker screenshot's
        # prediction (289.5, 581); the page's own caret was at x 289.77.
        self.assertEqual((40 + result["caret"]["x"], 457 + result["caret"]["y"]), (289.5, 581))

    def test_scale_one_is_the_identity(self):
        result = self.calibrate(glyph=rect(238, 124, 11, 23), scale=1)
        self.assertEqual(result["caret"], {"x": 249, "y": 124, "height": 23})
        self.assertEqual(result["field"], self.FIELD)

    def test_each_convention_rejects_the_other(self):
        # Chromium's document/frame width ratio (671/687) is not the scale.
        self.assertIsNone(calibrated_geometry(self.FRAME, self.DOCUMENT, self.FIELD, self.GLYPH, dict(self.WINDOW),
                                              [self.MONITOR], 24))
        chromium = CalibrationTests()
        frame, document, field, glyph = rect(0, 0, 900, 650), rect(0, 170, 1800, 1130), rect(100, 200, 1200, 80), rect(400, 220, 20, 40)
        self.assertIsNotNone(chromium.calibrate(frame, document, field, glyph))
        self.assertIsNone(gecko_calibrated_geometry(frame, document, field, glyph, dict(CalibrationTests.WINDOW),
                                                    [self.MONITOR], 12))

    def test_unverified_or_inconsistent_extents_fail_closed(self):
        cases = {
            "fractional scale": dict(scale=1.5),
            "unmeasured integer scale": dict(scale=3),
            "frame not window-relative": dict(frame=rect(40, 457, 687, 495)),
            "physical frame": dict(frame=rect(0, 0, 1374, 990)),
            "frame narrower than the window": dict(frame=rect(0, 0, 600, 495)),
            "logical glyph": dict(glyph=rect(238, 124, 11, 23)),
            "glyph outside field": dict(glyph=rect(1400, 248, 22, 46)),
            "field outside document": dict(field=rect(26, 480, 635, 55)),
            "document outside frame": dict(document=rect(8, 8, 900, 479)),
            # Inside Gecko's 495-pixel frame, below Hyprland's 431-pixel tile.
            "caret in the clipped surface": dict(field=rect(26, 400, 635, 55), glyph=rect(477, 820, 22, 46)),
            "unready extents": dict(field=rect(-1, -1, -1, -1)),
            "zero glyph": dict(glyph=rect(0, 0, 0, 0)),
            "nan": dict(glyph=rect(float("nan"), 248, 22, 46)),
            "caret zero": dict(caret=0),
            "xwayland": dict(window={**self.WINDOW, "xwayland": True}),
        }
        for name, change in cases.items():
            with self.subTest(name):
                self.assertIsNone(self.calibrate(**change))
        self.assertIsNotNone(self.calibrate(field=rect(26, 360, 635, 55), glyph=rect(477, 740, 22, 46)),
                             "a caret that ends exactly at the tile's edge is still visible")


class GeometryWiringTests(unittest.TestCase):
    def setUp(self):
        extents = lambda x, y, w, h: SimpleNamespace(get_extents=lambda kind: (self.assertEqual(kind, "screen"),
                                                     SimpleNamespace(x=x, y=y, width=w, height=h))[1])
        application = FakeNode(role="application")
        self.frame = FakeNode(role="frame", parent=application, interfaces=["Component"])
        self.frame.get_component_iface = lambda: extents(5000, 3000, 900, 650)
        document = FakeNode(role="document web", parent=self.frame, interfaces=["Component", "Document"])
        document.get_component_iface = lambda: extents(10000, 6000, 1800, 1300)
        self.field = FakeNode(parent=document, interfaces=["Component", "Text"])
        self.field.get_component_iface = lambda: extents(10100, 6200, 1200, 80)
        self.offsets = []
        self.text = SimpleNamespace(get_character_extents=lambda offset, kind: self.offsets.append((offset, kind)) or
                                    SimpleNamespace(x=10400, y=6220, width=20, height=40))
        self.backend = FieldBackend(SimpleNamespace(CoordType=SimpleNamespace(SCREEN="screen")), APPS)
        self.backend.budget = lambda: None
        self.monitors = [CalibrationTests.MONITOR]
        self.backend.desktop.request = lambda request: self.assertEqual(request, "j/monitors") or self.monitors
        self.window = dict(CalibrationTests.WINDOW)

    def test_every_request_reads_live_extents_and_calibrates(self):
        result = self.backend.geometry(self.field, self.text, 12, self.window)
        self.assertEqual(result["caret"], {"x": 210, "y": 110, "height": 20})
        self.assertEqual(self.offsets, [(11, "screen")])
        self.backend.geometry(self.field, self.text, 12, self.window)
        self.assertEqual(len(self.offsets), 2, "no cached calibration")

    def test_unavailable_geometry_is_not_a_request_failure(self):
        self.frame.interfaces = []
        self.assertIsNone(self.backend.geometry(self.field, self.text, 12, self.window))
        self.setUp()
        def fail(*_args):
            raise RuntimeError("GLib.Error timeout")
        self.text.get_character_extents = fail
        self.assertIsNone(self.backend.geometry(self.field, self.text, 12, self.window))
        self.setUp()
        def unavailable(*_args):
            raise Denied("desktop_unavailable")
        self.backend.desktop.request = unavailable
        self.assertIsNone(self.backend.geometry(self.field, self.text, 12, self.window))
        self.setUp()
        self.monitors = {"not": "a list"}
        self.assertIsNone(self.backend.geometry(self.field, self.text, 12, self.window))

    def test_gecko_fields_use_the_gecko_rule(self):
        # The Chromium-shaped extents here fail Gecko's window-relative frame check.
        self.assertIsNone(self.backend.geometry(self.field, self.text, 12, self.window, True))
        self.frame.get_component_iface = lambda: SimpleNamespace(get_extents=lambda _kind: SimpleNamespace(
            x=0, y=0, width=900, height=700))
        self.field.parent.get_component_iface = lambda: SimpleNamespace(get_extents=lambda _kind: SimpleNamespace(
            x=8, y=8, width=884, height=640))
        self.field.get_component_iface = lambda: SimpleNamespace(get_extents=lambda _kind: SimpleNamespace(
            x=50, y=100, width=600, height=40))
        self.text.get_character_extents = lambda offset, kind: SimpleNamespace(x=400, y=220, width=20, height=40)
        result = self.backend.geometry(self.field, self.text, 12, self.window, True)
        self.assertEqual((result["caret"], result["field"]),
                         ({"x": 210, "y": 110, "height": 20}, {"x": 50, "y": 100, "width": 600, "height": 40}))
        self.assertIsNone(self.backend.geometry(self.field, self.text, 12, self.window), "Chromium rule for Gecko extents")

    def test_operation_deadline_still_fails_the_request(self):
        def expired():
            raise Denied("operation_timeout")
        self.backend.budget = expired
        with self.assertRaisesRegex(Denied, "operation_timeout"):
            self.backend.geometry(self.field, self.text, 12, self.window)


class VisualDirectionTests(unittest.TestCase):
    """Gecko has no direction attribute; glyph order decides the caret edge."""

    @staticmethod
    def box(x, y=100, width=10, height=20):
        return types.SimpleNamespace(x=x, y=y, width=width, height=height)

    def test_glyph_order_gives_the_run_direction(self):
        self.assertEqual(visual_direction(self.box(100), self.box(112)), "ltr")
        self.assertEqual(visual_direction(self.box(112), self.box(100)), "rtl")

    def test_ambiguous_glyphs_give_no_direction(self):
        self.assertEqual(visual_direction(self.box(100), self.box(100)), "", "same x")
        self.assertEqual(visual_direction(self.box(300), self.box(10, y=130)), "", "wrapped onto the next line")
        self.assertEqual(visual_direction(self.box(100, width=0), self.box(112)), "", "empty glyph box")
        self.assertEqual(visual_direction(self.box(100, height=0), self.box(112)), "", "zero height")
        self.assertEqual(visual_direction(None, self.box(112)), "", "missing extents")


class MetadataTests(unittest.TestCase):
    def test_only_preview_metadata_calibrates_geometry(self):
        states = {"focused", "editable", "showing", "visible", "enabled"}
        text = SimpleNamespace(get_caret_offset=lambda: 5, get_character_count=lambda: 5, get_n_selections=lambda: 0)
        node = FakeNode(states, interfaces=["Text", "Component"])
        node.clear_cache = lambda: None
        node.get_attributes = lambda: {"tag": "textarea", "text-input-type": "textarea"}
        node.get_text_iface = lambda: text
        node.app, node.path = SimpleNamespace(bus_name=":1.9"), "/field"
        atspi = SimpleNamespace(StateType=SimpleNamespace(FOCUSED="focused", EDITABLE="editable", SHOWING="showing",
                                                          VISIBLE="visible", ENABLED="enabled"),
                                Text=SimpleNamespace(get_attribute_run=lambda *_args: ({"direction": "lr"}, 0, 5)))
        backend = FieldBackend(atspi, APPS)
        backend.budget = lambda: None
        window = {"pid": 42, "xwayland": False}
        backend.desktop.window = lambda _app: window
        backend.focused = lambda pid, browser, gecko: node
        calibrations = []
        backend.geometry = lambda *args: calibrations.append(args) or {"coordinate_convention": CALIBRATED}
        metadata = backend.metadata("code")
        self.assertIsNone(metadata["geometry"])
        self.assertEqual((metadata["browser"], metadata["web"], metadata["uri"]), (False, True, ""))
        self.assertEqual(calibrations, [])
        self.assertEqual(backend.metadata("code", True)["geometry"], {"coordinate_convention": CALIBRATED})
        self.assertEqual(calibrations, [(node, text, 5, window, False, 5)])
        window["xwayland"] = True
        self.assertIsNone(backend.metadata("code", True)["geometry"])
        self.assertEqual(backend.metadata("telegram")["web"], False)


class GeckoMetadataTests(unittest.TestCase):
    STATES = {"focused", "editable", "showing", "visible", "enabled"}

    def setUp(self):
        _application, frame = gecko_window()
        self.document = gecko_page(frame, GeckoFocusTests.PAGE)
        self.text = SimpleNamespace(get_caret_offset=lambda: 24, get_character_count=lambda: 24,
                                    get_n_selections=lambda: 0)
        self.node = self.field(role="entry", attributes={"tag": "textarea", "display": "block"})
        self.node.get_text_iface = lambda: self.text
        self.backend = FieldBackend(SimpleNamespace(**{**vars(GECKO_ATSPI), "StateType": SimpleNamespace(
            FOCUSED="focused", EDITABLE="editable", SHOWING="showing", VISIBLE="visible", ENABLED="enabled"),
            "Text": SimpleNamespace(get_attribute_run=lambda *_args: ({"direction": "lr"}, 0, 24))}), APPS)
        self.backend.budget = lambda: None
        self.window = {"pid": 42, "xwayland": False}
        self.backend.desktop.window = lambda _app: self.window
        self.backend.desktop.lock_query = contextlib.nullcontext
        self.selections = []
        self.backend.focused = lambda pid, browser, gecko: self.selections.append((browser, gecko)) or self.node
        self.calibrations = []
        self.backend.geometry = lambda *args: self.calibrations.append(args) or {"coordinate_convention": CALIBRATED}
        # Gecko serializes block boundaries differently; its fields are never flattened.
        self.backend.rich_layout = lambda *_args: self.fail("Gecko fields are never flattened")

    def field(self, role, attributes):
        node = gecko_field(self.document, role=role, states=self.STATES)
        node.clear_cache = lambda: None
        node.get_attributes = lambda: dict(attributes)
        node.app, node.path = SimpleNamespace(bus_name=":1.1227"), "/org/a11y/atspi/accessible/9"
        node.get_text_iface = lambda: self.fail("a denied field's text interface is never used")
        return node

    def inspect(self):
        return Observer(self.backend).request({"schema": SCHEMA, "id": "zen", "op": "inspect", "app_id": "zen"})

    def test_zen_page_field_binds_its_exact_app_and_shares_browser_origin_policy(self):
        metadata = self.backend.metadata("zen", True)
        self.assertEqual(self.selections, [(True, True)])
        self.assertEqual((metadata["uri"], metadata["browser"], metadata["web"], metadata["tag"], metadata["input_type"]),
                         (GeckoFocusTests.PAGE, True, True, "textarea", ""))
        self.assertEqual(self.calibrations, [(self.node, self.text, 24, self.window, True, 24)])
        focus = self.inspect()["focus"]
        self.assertEqual((focus["binding"]["app_id"], focus["binding"]["uri"]), ("zen", GeckoFocusTests.PAGE))
        self.assertEqual({key: focus["target"][key] for key in ("kind", "app_id", "origin")},
                         {"kind": "browser", "app_id": "chromium",
                          "origin": {"scheme": "http", "host": "127.0.0.1", "port": 47811}})
        for attributes in ({"tag": "input", "text-input-type": "text"}, {"tag": "div", "xml-roles": "textbox"}):
            self.node = self.field("entry", attributes)
            self.node.get_text_iface = lambda: SimpleNamespace(get_caret_offset=lambda: 0, get_character_count=lambda: 0,
                                                               get_n_selections=lambda: 0)
            self.assertTrue(self.inspect()["ok"], attributes)

    def test_passwords_are_denied_before_their_text_interface(self):
        for role, attributes in (("password text", {"text-input-type": "password"}),
                                 ("password text", {}), ("entry", {"tag": "input", "text-input-type": "password"})):
            self.node = self.field(role, attributes)
            with self.subTest(role=role, attributes=attributes), self.assertRaisesRegex(Denied, "sensitive_field"):
                self.backend.metadata("zen", True)
            self.assertEqual(self.inspect()["error"], "sensitive_field")
        self.assertEqual(self.calibrations, [])

    def test_unexposed_or_unsupported_purposes_are_ineligible(self):
        # Gecko's first query of a new document returned no tag.
        for attributes in ({}, {"display": "block"}, {"tag": "input"}, {"tag": "input", "text-input-type": "email"},
                           {"tag": "input", "text-input-type": "search"}, {"tag": "input", "text-input-type": "url"}):
            self.node = self.field("entry", attributes)
            self.node.get_text_iface = lambda: SimpleNamespace(get_caret_offset=lambda: 0, get_character_count=lambda: 0,
                                                               get_n_selections=lambda: 0)
            with self.subTest(attributes=attributes):
                self.assertEqual(self.inspect()["error"], "unsupported_field")
        self.node = self.field("combo box", {"tag": "input"})
        self.node.get_text_iface = lambda: SimpleNamespace(get_caret_offset=lambda: 0, get_character_count=lambda: 0,
                                                           get_n_selections=lambda: 0)
        self.assertEqual(self.inspect()["error"], "unsupported_field")



class FakeText:
    """One accessible's Text interface; every prose read is recorded."""

    def __init__(self, owner, value, caret=-1, selections=0):
        self.owner, self.value, self.caret, self.selections = owner, value, caret, selections
        self.extents = []
        self.change = None

    def get_character_count(self):
        return len(self.value)

    def get_caret_offset(self):
        return self.caret

    def get_n_selections(self):
        return self.selections

    def get_character_extents(self, offset, kind):
        self.extents.append(offset)
        return SimpleNamespace(x=100 + 10 * offset, y=100, width=10, height=20)

    def read(self, start, end):
        self.owner.reads.append((self.owner.path, start, end))
        value = self.value[start:end]
        if self.change:
            self.change()
            self.change = None
        return value


class RichNode(FakeNode):
    STATES = {"focused", "editable", "showing", "visible", "enabled"}

    def __init__(self, reads, value, caret=-1, role="entry", parent=None, path="/composer", pid=42):
        super().__init__(set(self.STATES) if role == "entry" else {"editable", "showing"}, pid=pid, role=role,
                         parent=parent, interfaces=["Text", "Hypertext", "Component"])
        self.reads, self.path = reads, path
        self.text = FakeText(self, value, caret)
        self.links = []
        self.app = SimpleNamespace(bus_name=":1.50")
        self.attributes = {"tag": "div", "xml-roles": "textbox", "class": "ProseMirror"} if role == "entry" else {}

    def get_text_iface(self):
        return self.text

    def get_hypertext_iface(self):
        return self

    def get_attributes(self):
        return dict(self.attributes)

    def clear_cache(self):
        pass

    def embed(self, child, start=None, anchors=1):
        start = len(self.links) if start is None else start
        self.links.append(SimpleNamespace(start=start, end=start + 1, anchors=anchors, obj=child))
        self.children.append(child)
        return child


RICH_ATSPI = SimpleNamespace(
    StateType=SimpleNamespace(FOCUSED="focused", EDITABLE="editable", SHOWING="showing", VISIBLE="visible",
                              ENABLED="enabled"),
    CoordType=SimpleNamespace(SCREEN="screen"),
    Text=SimpleNamespace(get_text=lambda text, start, end: text.read(start, end),
                         get_attribute_run=lambda text, offset, _include: (
                             text.owner.reads.append((text.owner.path, "run", offset)) or {"direction": "lr"}, 0, 1)),
    Hypertext=SimpleNamespace(get_n_links=lambda node: len(node.links), get_link=lambda node, index: node.links[index]),
    Hyperlink=SimpleNamespace(get_start_index=lambda link: link.start, get_end_index=lambda link: link.end,
                              get_n_anchors=lambda link: link.anchors, get_object=lambda link, _index: link.obj))


class RichEditorTests(unittest.TestCase):
    """Codex desktop's ProseMirror composer (Chromium 153, live 2026-09-27).

    The focused `entry` root's text is one U+FFFC per paragraph and its caret
    is that paragraph's embedded-object offset; the paragraph holds the text
    and the caret. Chromium's surrounding text ends each paragraph with
    "\n\n", including the last one.
    """

    PHRASE = "Please find attached the"

    def setUp(self):
        self.reads = []
        self.backend = FieldBackend(RICH_ATSPI, APPS)
        self.backend.budget = lambda: None
        self.window = {"pid": 42, "xwayland": False}
        self.backend.desktop.window = lambda _app: self.window
        self.backend.desktop.lock_query = contextlib.nullcontext
        self.root = None
        self.backend.focused = lambda pid, browser, gecko: self.root
        self.calibrations = []
        self.backend.geometry = lambda *args: self.calibrations.append(args) or {"coordinate_convention": CALIBRATED}
        self.observer = Observer(self.backend)

    def composer(self, paragraphs, root_caret=0, carets=None):
        carets = carets or [-1] * len(paragraphs)
        self.root = RichNode(self.reads, "\ufffc" * len(paragraphs), root_caret)
        for index, value in enumerate(paragraphs):
            self.root.embed(RichNode(self.reads, value, carets[index], role="paragraph", parent=self.root,
                                     path=f"/paragraph/{index}"))
        return self.root

    def inspect(self):
        return self.observer.request({"schema": SCHEMA, "id": "inspect", "op": "inspect", "app_id": "chatgpt"})

    def snapshot(self, focus):
        return self.observer.request({"schema": SCHEMA, "id": "snapshot", "op": "snapshot",
                                      "binding": focus["binding"], "policy_target": focus["target"]})

    def prose_reads(self):
        return [read for read in self.reads if read[1] != "run"]

    def test_live_codex_composer_matches_chromium_surrounding_text(self):
        self.composer([self.PHRASE], 0, [24])
        inspected = self.inspect()
        self.assertTrue(inspected["ok"], inspected)
        focus = inspected["focus"]
        self.assertEqual((focus["caret"], focus["binding"]["path"], focus["target"]["app_id"]),
                         (24, "/composer", "chatgpt"))
        self.assertEqual(self.prose_reads(), [], "inspect reads structure and lengths, never prose")
        snapshot = self.snapshot(focus)["focus"]
        self.assertEqual((snapshot["before"], snapshot["after"], snapshot["caret"], snapshot["total_chars"]),
                         (self.PHRASE, "\n\n", 24, 26))
        self.assertEqual(self.prose_reads(), [("/paragraph/0", 0, 24), ("/paragraph/0", 0, 24)],
                         "only the paragraph text, read twice for the stale recheck")

    def test_paragraphs_end_like_chromium_in_flattened_coordinates(self):
        # Chromium 152 sent "Hello\n\nPlease find attached the" + "\n\n" for two <p>.
        self.composer(["Hello", "world"], 1, [-1, 3])
        focus = self.inspect()["focus"]
        self.assertEqual(focus["caret"], 10)
        snapshot = self.snapshot(focus)["focus"]
        self.assertEqual((snapshot["before"], snapshot["after"], snapshot["total_chars"]),
                         ("Hello\n\nwor", "ld\n\n", 14))

    def test_paragraphs_ending_in_a_line_break_fail_closed(self):
        # Chromium adds one break, not two, after "\n": blank lines and trailing hard breaks.
        for paragraphs, carets in ((["Hello\n", "world"], [-1, 3]), (["Hello", "\n"], [-1, 0]),
                                   (["Hello", ""], [-1, 0])):
            with self.subTest(paragraphs=paragraphs):
                self.composer(paragraphs, 1, carets)
                self.assertEqual(self.snapshot(self.inspect()["focus"])["error"], "unsupported_field")
        self.composer(["Hello\n"] + ["x" * 600], 1, [-1, 600])
        self.assertEqual(self.snapshot(self.inspect()["focus"])["focus"]["after"], "\n\n",
                         "a line-break paragraph outside the window does not matter")

    def test_reads_stay_bounded_to_the_blocks_around_the_caret(self):
        self.composer(["a" * 600, "b" * 600, "tail"], 2, [-1, -1, 4])
        snapshot = self.snapshot(self.inspect()["focus"])["focus"]
        self.assertEqual(snapshot["before"], "b" * 506 + "\n\ntail")
        self.assertEqual(snapshot["after"], "\n\n")
        self.assertEqual({read[0] for read in self.prose_reads()}, {"/paragraph/1", "/paragraph/2"})

    def test_plain_fields_keep_their_own_text_and_caret(self):
        self.root = RichNode(self.reads, "Dear team,\n", 10)
        snapshot = self.snapshot(self.inspect()["focus"])["focus"]
        self.assertEqual((snapshot["before"], snapshot["after"], snapshot["total_chars"]), ("Dear team,", "\n", 11))
        self.assertEqual(self.prose_reads(), [("/composer", 0, 11), ("/composer", 0, 11)])

    def test_unresolvable_structure_fails_closed_before_prose(self):
        def placeholder():
            root = self.composer([""], 0, [0])
            root.children[0].text.value = "\n\ufffc"
            root.children[0].embed(RichNode(self.reads, "Do anything", role="section", parent=root.children[0]))

        def text_in_root():
            self.composer([self.PHRASE], 0, [24]).text.value = "\ufffcx"

        def section():
            self.composer([self.PHRASE], 0, [24]).children[0].role = "section"

        def foreign_parent():
            self.composer([self.PHRASE], 0, [24]).children[0].parent = RichNode(self.reads, "", path="/other")

        def shifted_link():
            self.composer([self.PHRASE], 0, [24]).links[0].start = 1

        def two_anchors():
            self.composer([self.PHRASE], 0, [24]).links[0].anchors = 2

        def too_many():
            self.composer(["x"] * (MAX_RICH_BLOCKS + 1), 0, [1] + [-1] * MAX_RICH_BLOCKS)

        cases = {"unsupported_field": (placeholder, text_in_root, section, foreign_parent, shifted_link, two_anchors,
                                       too_many),
                 "invalid_caret": (lambda: self.composer([self.PHRASE], 1, [24]),
                                   lambda: self.composer([self.PHRASE], 0, [-1]),
                                   lambda: self.composer([self.PHRASE], 0, [25]),
                                   lambda: self.composer(["Hello", "world"], 1, [0, 3])),
                 "selection_present": (lambda: setattr(self.composer(["Hello", "world"], 1, [-1, 3]).children[0].text,
                                                       "selections", 1),)}
        for error, builders in cases.items():
            for build in builders:
                with self.subTest(error=error, case=getattr(build, "__name__", "lambda")):
                    self.reads.clear()
                    build()
                    self.assertEqual(self.inspect()["error"], error)
                    self.assertEqual(self.prose_reads(), [])

    def test_changes_between_reads_fail_closed(self):
        self.composer([self.PHRASE], 0, [24])
        focus = self.inspect()["focus"]
        paragraph = self.root.children[0]
        paragraph.text.change = lambda: setattr(paragraph.text, "value", "Please find attached thx")
        self.assertEqual(self.snapshot(focus)["error"], "stale_binding")
        self.composer([self.PHRASE], 0, [24])
        focus = self.inspect()["focus"]
        paragraph = self.root.children[0]
        paragraph.text.change = lambda: paragraph.text.__setattr__("caret", 23)
        self.assertEqual(self.snapshot(focus)["error"], "stale_binding", "caret moved during the read")
        # An embedded object in paragraph text without a link is never text.
        self.composer(["Please find attached \ufffc"], 0, [22])
        self.assertEqual(self.snapshot(self.inspect()["focus"])["error"], "unsupported_field")

    def test_geometry_and_direction_use_the_caret_paragraph(self):
        self.composer(["Hello", "world"], 1, [-1, 3])
        metadata = self.backend.metadata("chatgpt", True)
        paragraph = self.root.children[1]
        self.assertEqual(self.calibrations, [(self.root, paragraph.text, 3, self.window, False, 10)])
        self.assertIn(("/paragraph/1", "run", 2), self.reads)
        self.assertEqual((metadata["caret"], metadata["total_chars"]), (10, 14))
        self.calibrations.clear()
        self.composer(["Hello", "world"], 1, [-1, 0])
        self.backend.metadata("chatgpt", True)
        self.assertEqual(self.calibrations, [], "no glyph before a paragraph's first character")

    def test_calibrated_offset_names_the_flattened_caret(self):
        del self.backend.geometry
        rich = self.composer(["Hello", "world"], 1, [-1, 3])
        self.backend.frame_and_document = lambda node: (None, None)
        self.assertIsNone(self.backend.metadata("chatgpt", True)["geometry"])
        extents = lambda x, y, w, h: SimpleNamespace(get_extents=lambda kind: SimpleNamespace(x=x, y=y, width=w, height=h))
        frame = FakeNode(role="frame", interfaces=["Component"])
        frame.get_component_iface = lambda: extents(5000, 3000, 900, 650)
        document = FakeNode(role="document web", interfaces=["Component"])
        document.get_component_iface = lambda: extents(10000, 6000, 1800, 1300)
        rich.get_component_iface = lambda: extents(10100, 6200, 1200, 80)
        rich.children[1].text.get_character_extents = lambda offset, kind: SimpleNamespace(
            x=10400, y=6220, width=20, height=40)
        self.backend.frame_and_document = lambda node: (frame, document)
        self.backend.desktop.request = lambda _request: [CalibrationTests.MONITOR]
        self.window.update(CalibrationTests.WINDOW)
        geometry = self.backend.metadata("chatgpt", True)["geometry"]
        self.assertEqual((geometry["character_offset"], geometry["caret"]), (9, {"x": 210, "y": 110, "height": 20}))

    def test_native_toolkits_never_flatten(self):
        self.backend.rich_layout = lambda *_args: self.fail("only Chromium web content is flattened")
        self.root = RichNode(self.reads, "Saved message", 13)
        self.root.attributes = {}
        self.assertEqual(self.backend.metadata("telegram")["caret"], 13)


class RichTextTests(unittest.TestCase):
    BLOCKS = [{"start": 0, "length": 5, "value": "Hello"}, {"start": 7, "length": 5, "value": "world"}]

    def setUp(self):
        self.reads = []

    def read(self, block, first, last):
        self.reads.append((block["start"], first, last))
        return block["value"][first:last]

    def test_every_paragraph_ends_with_two_line_breaks(self):
        for (start, end), expected in {(0, 14): "Hello\n\nworld\n\n", (3, 9): "lo\n\nwo", (5, 6): "\n",
                                       (6, 7): "\n", (12, 14): "\n\n", (7, 12): "world", (4, 4): ""}.items():
            self.assertEqual(rich_text(self.BLOCKS, start, end, self.read), expected, (start, end))

    def test_a_window_starting_at_a_break_still_checks_its_paragraph_end(self):
        self.assertEqual(rich_text(self.BLOCKS, 6, 9, self.read), "\nwo")
        self.assertIn((0, 4, 5), self.reads, "the last character decides the break count")

    def test_embedded_objects_line_break_endings_and_invalid_reads_are_unsupported(self):
        for value in ("Hel\ufffco", "Hell\n", None):
            with self.assertRaisesRegex(Denied, "unsupported_field"):
                rich_text(self.BLOCKS, 0, 14, lambda block, first, last: value)
        with self.assertRaisesRegex(Denied, "unsupported_field"):
            rich_text([{"start": 0, "length": 0, "value": ""}], 0, 2, self.read)


if __name__ == "__main__":
    unittest.main()
