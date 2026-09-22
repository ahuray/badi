import json
import os
import pathlib
import socket
import sys
import tempfile
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from contract import MAX_FRAME, SCHEMA, Observer
from daemon import Daemon, unique_object
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


class EventTests(unittest.TestCase):
    def setUp(self):
        self.daemon = Daemon("/unused/accessibility.sock", None, FakeGLib)
        self.tracked = {"app_id": "chromium", "process_id": 42,
                        "bus": ":1.7", "path": "/field"}
        self.daemon.observer.tracked = dict(self.tracked)
        self.cached = object()
        self.daemon.backend.cached = self.cached
        self.notifications, self.hidden, self.disarmed = [], [], []
        self.daemon.observer.notify = self.notifications.append
        self.daemon.observer.hide = lambda: self.hidden.append(True)
        self.daemon.observer.disarm = lambda: self.disarmed.append(True)

    def event(self, *, bus=":1.7", path="/unrelated", pid=42, sender=None,
              event_type="object:state-changed:defunct", detail1=1, detail2=0):
        source = SimpleNamespace(app=SimpleNamespace(bus_name=bus), path=path,
                                 get_process_id=lambda: pid)
        class Event(SimpleNamespace):
            @property
            def any_data(self):
                raise AssertionError("Metadata events must never read text payloads")
        return Event(type=event_type, source=source, sender=sender,
                     detail1=detail1, detail2=detail2)

    def assert_preserved(self):
        self.assertEqual(self.daemon.observer.tracked, self.tracked)
        self.assertEqual(self.daemon.observer.epoch, 1)
        self.assertIs(self.daemon.backend.cached, self.cached)
        self.assertEqual((self.notifications, self.hidden, self.disarmed), ([], [], []))

    def assert_invalidated(self, reason="field_changed"):
        self.assertIsNone(self.daemon.observer.tracked)
        self.assertIsNone(self.daemon.backend.cached)
        self.assertEqual(self.daemon.observer.epoch, 2)
        self.assertEqual(self.notifications, [{"schema": SCHEMA, "event": "invalidate",
                         "epoch": 2, "reason": reason, "app_id": "chromium"}])
        self.assertEqual((self.hidden, self.disarmed), ([True], [True]))

    def test_unrelated_local_proxy_disposal_preserves_focused_authority(self):
        event = self.event()
        def no_remote_pid_query():
            self.fail("Local unrelated proxy disposal needs no remote query")
        event.source.get_process_id = no_remote_pid_query
        self.daemon.event(event)
        self.assert_preserved()

    def test_same_path_in_another_bus_is_an_unrelated_local_proxy(self):
        self.daemon.event(self.event(bus=":1.8", path="/field"))
        self.assert_preserved()

    def test_tracked_local_proxy_disposal_invalidates(self):
        event = self.event(path="/field")
        def no_remote_pid_query():
            self.fail("Tracked proxy identity suffices even during disposal")
        event.source.get_process_id = no_remote_pid_query
        self.daemon.event(event)
        self.assert_invalidated()

    def test_remote_unrelated_defunct_in_tracked_process_still_invalidates(self):
        self.daemon.event(self.event(sender=object()))
        self.assert_invalidated()

    def test_remote_tracked_defunct_still_invalidates(self):
        self.daemon.event(self.event(path="/field", sender=object()))
        self.assert_invalidated()

    def test_incomplete_local_proxy_identity_cannot_bypass_invalidation(self):
        self.daemon.event(self.event(bus=None, pid=99))
        self.assert_invalidated()

    def test_unreadable_local_proxy_identity_fails_closed(self):
        event = self.event()
        event.source.app = None
        self.daemon.event(event)
        self.assert_invalidated("accessibility_unavailable")

    def test_sender_null_is_not_a_general_event_exemption(self):
        self.daemon.event(self.event(event_type="object:children-changed"))
        self.assert_invalidated()

    def test_defunct_state_clear_is_not_a_proxy_disposal(self):
        self.daemon.event(self.event(detail1=0))
        self.assert_invalidated()

    def test_focus_change_from_another_process_still_invalidates(self):
        self.daemon.event(self.event(pid=99, event_type="object:state-changed:focused"))
        self.assert_invalidated("focus_changed")


if __name__ == "__main__":
    unittest.main()
