"""Fast checks for the Writer extension's text guards and protocol client.

The client runs against a scripted broker on a private temporary socket, so
every reply shape can be exercised without LibreOffice or the Rust broker.
"""

import json
import os
from pathlib import Path
import socket
import struct
import sys
import tempfile
import threading
import unittest
import unittest.mock

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "adapters" / "libreoffice" / "extension" / "pythonpath"))

from badi_libreoffice import broker, text  # noqa: E402

CAPABILITIES = list(broker.CAPABILITIES)
ALLOWED = {"authority_epoch": 3, "settings_revision": 1, "paused": False, "activation": "always",
           "context_allowed": True, "display_allowed": True, "suggestions_allowed": True,
           "learning_allowed": False, "reason": "matched_rule"}


class TextTest(unittest.TestCase):
    def test_shared_joiner_corpus_and_context_whitespace(self):
        corpus = json.loads((ROOT / "protocol" / "orthographic-joiner-fixtures.json").read_text())
        for fixture in corpus["fixtures"]:
            self.assertEqual(not text.has_unsafe_text(fixture["text"]), fixture["valid"], fixture["name"])
        self.assertFalse(text.has_unsafe_text("normal\ncontext\ttext", True))
        self.assertTrue(text.has_unsafe_text("normal\ncontext\ttext"))
        self.assertTrue(text.has_unsafe_text("unsafe‮context", True))

    def test_corrections_follow_the_shared_spelling_replacement_fixture(self):
        corpus = json.loads((ROOT / "protocol" / "spelling-replacement-fixtures.json").read_text())
        for fixture in corpus["fixtures"]:
            original, replacement = fixture["original"], fixture["replacement"]
            self.assertEqual(text.valid_correction("Fix " + original, original, replacement), fixture["valid"],
                             fixture["name"])

    def test_corrections_own_one_exact_word_and_its_typed_space(self):
        # The same cases as adapters/shared/text-safety.test.mjs.
        for case in (("Use this adress", "adress", "address"),
                     ("Use this adress ", "adress ", "address "), ("adress ", "adress ", "address "),
                     ("Ins Hauss ", "Hauss ", "Haus "), ("muchas grasias ", "grasias ", "gracias ")):
            self.assertTrue(text.valid_correction(*case), case)
        for case in (("badadress ", "adress ", "address "), ("adress ", "adress ", "address"),
                     ("adress", "adress", "address "), ("adress ", "adress ", "adress "),
                     ("adress  ", "adress  ", "address  "), ("adress\n", "adress\n", "address\n"),
                     ("adress ", "adress ", "address\n"), ("adress\t", "adress\t", "address\t"),
                     ("adress ", "other ", "address "), ("x2 ", "x2 ", "xz "),
                     ("mot-clef ", "mot-clef ", "motclef "),
                     ("adress ", "adress ", "address; ")):
            self.assertFalse(text.valid_correction(*case), case)

    def test_language_and_bounded_context(self):
        self.assertEqual(text.language_tag("en", "US"), "en-US")
        self.assertEqual(text.language_tag("de"), "de")
        for language, country in (("", ""), ("zxx", ""), ("en", "U S"), ("x" * 40, "")):
            self.assertIsNone(text.language_tag(language, country))
        before, after = text.bounded_context("a" * 600 + " adress ", "")
        self.assertEqual(len(before), 512)
        self.assertTrue(before.endswith(" adress "))
        self.assertIsNone(text.bounded_context("see ￼ adress ", ""))
        self.assertIsNone(text.bounded_context("   ", ""))
        self.assertIsNone(text.bounded_context("bidi ‮ adress", ""))


class FakeBroker:
    """A scripted v2 broker: `script(message, send)` answers each frame."""

    def __init__(self, script):
        self.directory = tempfile.mkdtemp(prefix="badi-lo-unit-")
        os.chmod(self.directory, 0o700)
        self.path = os.path.join(self.directory, "broker.sock")
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(self.path)
        os.chmod(self.path, 0o600)
        self.server.listen(1)
        self.script = script
        self.received = []
        self.arrived = threading.Condition()
        self.connection = None
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def send(self, message, raw=None):
        body = raw if raw is not None else json.dumps({"v": 2, "mono_ms": 1, **message}).encode()
        self.connection.sendall(struct.pack("<I", len(body)) + body)

    def serve(self):
        try:
            self.connection, _ = self.server.accept()
            buffer = b""
            while True:
                chunk = self.connection.recv(65536)
                if not chunk:
                    return
                buffer += chunk
                while len(buffer) >= 4:
                    (length,) = struct.unpack("<I", buffer[:4])
                    if len(buffer) < 4 + length:
                        break
                    message = json.loads(buffer[4:4 + length])
                    buffer = buffer[4 + length:]
                    with self.arrived:
                        self.received.append(message)
                        self.arrived.notify_all()
                    self.script(message, self.send)
        except OSError:
            return

    def wait_for(self, kind):
        with self.arrived:
            if not self.arrived.wait_for(
                    lambda: any(message["type"] == kind for message in self.received), 2):
                raise AssertionError(f"no {kind} frame")

    def close(self):
        for item in (self.connection, self.server):
            if item is not None:
                try:
                    item.close()
                except OSError:
                    pass
        try:
            os.unlink(self.path)
        except OSError:
            pass
        os.rmdir(self.directory)


def standard(policy=None, show=None, grant=None, after_show=None):
    """A broker that answers hello, policy, suggest and accept requests."""

    def script(message, send):
        kind, message_id = message["type"], message.get("id")
        coordinates = {key: message[key] for key in ("session_id", "focus_epoch", "revision")
                       if key in message}
        if kind == "hello":
            send({"id": message_id, "type": "hello.ack", "payload": {
                "selected_v": 2, "connection_id": "c:1", "enabled_capabilities": CAPABILITIES,
                "max_frame_bytes": 65536, "max_before_chars": 512, "max_after_chars": 128,
                "max_suggestion_chars": 64, "max_suggestion_words": 8, "paused": False}})
            send({"type": "authority.changed", "payload": {"authority_epoch": 3,
                                                           "settings_revision": 1, "paused": False}})
        elif kind == "policy.query":
            send({"id": message_id, "type": "policy.status", "payload": policy or ALLOWED})
        elif kind == "context.changed":
            script.fingerprint = message["payload"]["fingerprint"]
        elif kind == "suggest.request":
            payload = {"fingerprint": script.fingerprint, "suggestion_id": "s:1", "ttl_ms": 15000,
                       "text": "address ", "accept_word": "address ", "replace_before": "adress ",
                       "provider": "phrase_v1"}
            payload.update(show or {})
            send({"id": message_id, "type": "suggestion.show", **coordinates,
                  "payload": {key: value for key, value in payload.items() if value is not None}})
            if after_show:
                # After the client has the proposal, as a user reads it.
                threading.Timer(0.1, after_show, (send, coordinates)).start()
        elif kind == "control.request" and message["payload"]["action"] == "accept_all":
            send({"id": message_id, "type": "control.result", "payload": {
                "action": "accept_all", "accepted": True, "reason": "accepted", "paused": False}})
            payload = {"fingerprint": script.fingerprint, "suggestion_id": "s:1", "text": "address ",
                       "acceptance": "all", "replace_before": "adress "}
            payload.update(grant or {})
            send({"id": message_id, "type": "commit.prepare", **coordinates, "payload": payload})

    script.fingerprint = None
    return script


class ClientTest(unittest.TestCase):
    def broker(self, script):
        fake = FakeBroker(script)
        self.addCleanup(fake.close)
        return fake

    def attempt(self, fake, revoked=None):
        attempt = broker.Attempt(path=fake.path, on_revoked=revoked)
        self.addCleanup(attempt.close)
        return attempt

    def test_request_grant_and_report_bind_the_exact_session(self):
        fake = self.broker(standard())
        attempt = self.attempt(fake)
        proposal = attempt.request("We need an adress ", "", "en-US")
        self.assertEqual((proposal.replace_before, proposal.text, proposal.ttl_ms),
                         ("adress ", "address ", 15000))
        granted = attempt.authorize()
        attempt.report("applied", granted)
        fake.wait_for("session.close")
        kinds = [message["type"] for message in fake.received]
        self.assertEqual(kinds[:4], ["hello", "authority.ack", "policy.query", "session.open"])
        hello = fake.received[0]["payload"]
        self.assertEqual(hello["adapter"]["kind"], "libreoffice")
        self.assertEqual(sorted(hello["capabilities"]), sorted(CAPABILITIES))
        self.assertNotIn("id", fake.received[1], "authority.ack must not carry an id")
        query = fake.received[2]["payload"]["target"]
        self.assertEqual((query["kind"], query["app_id"]), ("libreoffice", "libreoffice-writer"))
        context = next(message for message in fake.received if message["type"] == "context.changed")
        self.assertEqual(context["payload"]["selection"]["anchor"], len("We need an adress "))
        self.assertTrue(context["payload"]["explicit"])
        accept = next(message for message in fake.received if message["type"] == "control.request")
        result = next(message for message in fake.received if message["type"] == "commit.result")
        self.assertEqual(result["id"], accept["id"])
        self.assertEqual(result["payload"]["status"], "applied")
        self.assertEqual(result["session_id"], context["session_id"])
        self.assertRaises(broker.BrokerError, attempt.authorize)

    def test_policy_denial_is_blocked_before_any_context(self):
        for policy in ({**ALLOWED, "suggestions_allowed": False, "display_allowed": False,
                        "context_allowed": False, "activation": "never", "reason": "default_policy"},
                       {**ALLOWED, "paused": True, "context_allowed": False, "display_allowed": False,
                        "suggestions_allowed": False, "activation": "never"}):
            fake = self.broker(standard(policy=policy))
            with self.assertRaises(broker.BrokerError) as raised:
                self.attempt(fake).request("adress ", "", "en")
            self.assertEqual(raised.exception.code, "blocked")
            self.assertNotIn("context.changed", [message["type"] for message in fake.received])

    def test_invalid_policy_shapes_are_protocol_errors(self):
        for policy in ({**ALLOWED, "learning_allowed": True}, {**ALLOWED, "paused": "no"},
                       {**ALLOWED, "context_allowed": False}):
            fake = self.broker(standard(policy=policy))
            with self.assertRaises(broker.BrokerError) as raised:
                self.attempt(fake).request("adress ", "", "en")
            self.assertEqual(raised.exception.code, "protocol")

    def test_continuations_are_not_corrections(self):
        fake = self.broker(standard(show={"text": " for your time", "accept_word": " for",
                                          "replace_before": None}))
        self.assertIsNone(self.attempt(fake).request("thank you", "", "en"))

    def test_replies_outside_the_bound_word_are_rejected(self):
        for show in ({"replace_before": "an adress "}, {"text": "address", "accept_word": "address"},
                     {"replace_before": "dress "}, {"accept_word": "addr"}, {"ttl_ms": 15001},
                     {"fingerprint": "0" * 64}):
            fake = self.broker(standard(show=show))
            with self.assertRaises(broker.BrokerError) as raised:
                self.attempt(fake).request("We need an adress ", "", "en")
            self.assertEqual(raised.exception.code, "protocol", show)

    def test_a_grant_for_other_text_is_never_returned(self):
        for grant in ({"text": "addresses "}, {"replace_before": "an adress "},
                      {"acceptance": "word"}):
            fake = self.broker(standard(grant=grant))
            attempt = self.attempt(fake)
            attempt.request("We need an adress ", "", "en")
            with self.assertRaises(broker.BrokerError):
                attempt.authorize()

    def test_authority_change_and_expiry_revoke_the_attempt(self):
        for frame, expected in (
                (lambda coordinates: {"type": "authority.changed",
                                      "payload": {"authority_epoch": 4, "settings_revision": 2,
                                                  "paused": True}}, "revoked"),
                (lambda coordinates: {"type": "suggestion.clear", **coordinates,
                                      "payload": {"reason": "expired"}}, "expired")):
            codes = []
            revoked = threading.Event()

            def after_show(send, coordinates, frame=frame):
                send(frame(coordinates))

            fake = self.broker(standard(after_show=after_show))
            attempt = self.attempt(fake, lambda code: (codes.append(code), revoked.set()))
            attempt.request("We need an adress ", "", "en")
            self.assertTrue(revoked.wait(2))
            self.assertEqual(codes, [expected])
            with self.assertRaises(broker.BrokerError):
                attempt.authorize()

    def test_malformed_frames_close_the_attempt(self):
        def oversized(send, coordinates):
            send(None, raw=b"x" * 65537)

        codes = []
        revoked = threading.Event()
        fake = self.broker(standard(after_show=oversized))
        attempt = self.attempt(fake, lambda code: (codes.append(code), revoked.set()))
        attempt.request("We need an adress ", "", "en")
        self.assertTrue(revoked.wait(2))
        self.assertEqual(codes, ["protocol"])

    def test_capability_mismatch_and_unsafe_context_fail_before_use(self):
        def partial(message, send):
            if message["type"] == "hello":
                send({"id": message["id"], "type": "hello.ack", "payload": {
                    "selected_v": 2, "connection_id": "c:1", "enabled_capabilities": CAPABILITIES[:-1],
                    "max_frame_bytes": 65536, "max_before_chars": 512, "max_after_chars": 128,
                    "max_suggestion_chars": 64, "max_suggestion_words": 8, "paused": False}})

        fake = self.broker(partial)
        with self.assertRaises(broker.BrokerError) as raised:
            self.attempt(fake).request("adress ", "", "en")
        self.assertEqual(raised.exception.code, "protocol")
        fake = self.broker(standard())
        for before, language in (("bidi ‮ adress", "en"), ("adress ", "en_US"), ("  ", "en")):
            with self.assertRaises(broker.BrokerError):
                self.attempt(fake).request(before, "", language)
        self.assertEqual(fake.received, [], "nothing reaches the broker")

    def test_no_socket_or_a_close_while_connecting_is_an_error_not_a_crash(self):
        fake = self.broker(standard())
        with unittest.mock.patch.object(broker.socket, "socket", side_effect=OSError("no descriptors")), \
                self.assertRaises(broker.BrokerError) as raised:
            self.attempt(fake).request("adress ", "", "en")
        self.assertEqual(raised.exception.code, "offline")
        attempt = self.attempt(fake)
        connect = socket.socket.connect

        def superseded(connection, address):
            attempt.close()  # The owner retires the attempt mid-connect.
            return connect(connection, address)
        with unittest.mock.patch.object(socket.socket, "connect", superseded), \
                self.assertRaises(broker.BrokerError) as raised:
            attempt.request("adress ", "", "en")
        self.assertEqual(raised.exception.code, "closed")
        self.assertIsNone(attempt._socket, "the connection was closed, not kept")

    def test_socket_must_be_private(self):
        self.assertRaises(broker.BrokerError, broker.socket_path, {"XDG_RUNTIME_DIR": "relative"})
        self.assertRaises(broker.BrokerError, broker.socket_path, {"XDG_RUNTIME_DIR": "/run/../tmp"})
        fake = self.broker(standard())
        os.chmod(fake.path, 0o666)
        with self.assertRaises(broker.BrokerError) as raised:
            self.attempt(fake).request("adress ", "", "en")
        self.assertEqual(raised.exception.code, "offline")
        os.chmod(fake.path, 0o600)
        os.chmod(fake.directory, 0o755)
        try:
            with self.assertRaises(broker.BrokerError):
                self.attempt(fake).request("adress ", "", "en")
        finally:
            os.chmod(fake.directory, 0o700)


if __name__ == "__main__":
    unittest.main(verbosity=2)
