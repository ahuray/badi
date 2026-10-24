import contextlib
import json
import os
import pathlib
import socket
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from health import MAX_STATUS_FRAME, ProbeError, SCHEMA, probe


class HealthTests(unittest.TestCase):
    @contextlib.contextmanager
    def server(self, response=None, mutate=None):
        with tempfile.TemporaryDirectory() as temporary:
            endpoint = pathlib.Path(temporary) / "accessibility.sock"
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(endpoint))
                endpoint.chmod(0o600)
                listener.listen(1)
                received = []
                errors = []
                def respond():
                    try:
                        with listener.accept()[0] as connection:
                            connection.settimeout(1)
                            data = connection.recv(2048)
                            received.append(data)
                            if not data:
                                return
                            if mutate:
                                mutate(endpoint)
                            value = response if response is not None else {"schema": SCHEMA, "id": "health", "ok": True,
                                "status": {"ready": True, "process_id": os.getpid(), "protocol_version": 1}}
                            raw = value if isinstance(value, bytes) else json.dumps(value).encode() + b"\n"
                            connection.sendall(raw)
                    except Exception as error:
                        errors.append(error)
                thread = threading.Thread(target=respond, daemon=True)
                thread.start()
                try:
                    yield endpoint, received
                finally:
                    thread.join(2)
                    self.assertFalse(thread.is_alive())
                    self.assertEqual(errors, [])

    def test_real_peer_and_status_request(self):
        with self.server() as (endpoint, received):
            self.assertEqual(probe(endpoint, os.getpid()), {"ready": True, "process_id": os.getpid(), "protocol_version": 1})
        self.assertEqual([json.loads(raw) for raw in received], [{"schema": SCHEMA, "id": "health", "op": "status"}])

    def test_wrong_peer_rejected_before_any_request(self):
        with self.server() as (endpoint, received):
            with self.assertRaisesRegex(ProbeError, "unexpected_peer"):
                probe(endpoint, os.getpid() + 1)
        self.assertEqual(received, [b""])

    def test_invalid_protocol_and_oversized_frames_reject(self):
        valid = {"schema": SCHEMA, "id": "health", "ok": True,
                 "status": {"ready": True, "process_id": os.getpid(), "protocol_version": 1}}
        for response in ({**valid, "id": "wrong"}, {**valid, "ok": 1}, {**valid, "extra": True},
                         {**valid, "status": {**valid["status"], "ready": 1}},
                         {**valid, "status": {**valid["status"], "protocol_version": True}},
                         b'{"id":"health","id":"duplicate"}\n', b"[]\n", b"{}\n{}\n", b"x" * (MAX_STATUS_FRAME + 1)):
            with self.subTest(response=response), self.server(response) as (endpoint, _received):
                with self.assertRaisesRegex(ProbeError, "invalid_response"):
                    probe(endpoint, os.getpid())

    def test_socket_mode_change_is_detected_after_reply(self):
        with self.server(mutate=lambda endpoint: endpoint.chmod(0o666)) as (endpoint, _received):
            with self.assertRaisesRegex(ProbeError, "unsafe_socket"):
                probe(endpoint, os.getpid())

    def test_absent_unsafe_and_symlink_endpoints(self):
        with tempfile.TemporaryDirectory() as temporary:
            endpoint = pathlib.Path(temporary) / "accessibility.sock"
            with self.assertRaisesRegex(ProbeError, "service_unavailable"):
                probe(endpoint, os.getpid())
            endpoint.write_text("not a socket")
            endpoint.chmod(0o600)
            with self.assertRaisesRegex(ProbeError, "unsafe_socket"):
                probe(endpoint, os.getpid())
            endpoint.unlink()
            endpoint.symlink_to(endpoint.parent / "missing")
            with self.assertRaisesRegex(ProbeError, "unsafe_socket"):
                probe(endpoint, os.getpid())


if __name__ == "__main__":
    unittest.main()
