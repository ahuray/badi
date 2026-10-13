"""Protocol v2 client for one Writer correction attempt.

Each attempt owns one private broker connection and one session: hello,
authority acknowledgement, exact policy, context, an explicit request and at
most one one-shot grant. Closing the attempt closes the connection, which
retires the session and any grant in the broker. Errors carry content-free
codes only; no text is logged or stored.
"""

import hashlib
import json
import os
import re
import socket
import stat
import struct
import threading
import time
import uuid

from . import text as badi_text

FRAME_LIMIT = 65536
MAX_SAFE_COUNTER = 9007199254740991
CAPABILITIES = ("context", "suggestion", "commit.applied", "text_replacement", "control",
                "policy", "long_reading_window")
ADAPTER = {"kind": "libreoffice", "name": "badi-libreoffice", "version": "0.1.0"}
TARGET_KIND = "libreoffice"
APP_ID = "libreoffice-writer"
RPC_TIMEOUT = 2.0
# The broker gives an explicit request 1.25 s; allow it to answer before giving up.
SUGGEST_TIMEOUT = 3.0
MAX_TTL_MS = 15000
_OPAQUE_ID = re.compile(r"[\x21-\x7e]{1,128}")
_POLICY_FLAGS = ("paused", "context_allowed", "display_allowed", "suggestions_allowed",
                 "learning_allowed")


class BrokerError(Exception):
    """A failed attempt. `code` is one of offline, blocked, protocol, timeout,
    rejected, revoked or expired, and names no text."""

    def __init__(self, code):
        super().__init__(code)
        self.code = code


def _counter(value):
    return isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= MAX_SAFE_COUNTER


def socket_path(environ=None):
    runtime = (os.environ if environ is None else environ).get("XDG_RUNTIME_DIR", "")
    if not runtime or not os.path.isabs(runtime) or os.path.normpath(runtime) != runtime:
        raise BrokerError("offline")
    return os.path.join(runtime, "badi", "broker.sock")


def check_private_socket(path):
    """The broker socket and its directory belong to this user alone."""
    uid = os.geteuid()
    try:
        directory = os.lstat(os.path.dirname(path))
        endpoint = os.lstat(path)
    except OSError as error:
        raise BrokerError("offline") from error
    if (not stat.S_ISDIR(directory.st_mode) or directory.st_uid != uid
            or directory.st_mode & 0o077 or not stat.S_ISSOCK(endpoint.st_mode)
            or endpoint.st_uid != uid or stat.S_IMODE(endpoint.st_mode) != 0o600):
        raise BrokerError("offline")


class Proposal:
    """A shown correction: replace exactly `replace_before` with `text`."""

    def __init__(self, suggestion_id, replace_before, text, ttl_ms):
        self.suggestion_id = suggestion_id
        self.replace_before = replace_before
        self.text = text
        self.ttl_ms = ttl_ms


class Attempt:
    """One explicit correction request and its one-shot acceptance."""

    def __init__(self, path=None, on_revoked=None, clock=time.monotonic):
        self._path = socket_path() if path is None else path
        self._on_revoked = on_revoked
        self._clock = clock
        self._socket = None
        self._send_lock = threading.Lock()
        self._lock = threading.Lock()
        self._pending = {}
        self._epoch = None
        self._authority = threading.Event()
        self._closed = False
        self._revoked = None
        self._session = None
        self._fingerprint = None
        self._before = None
        self._proposal = None
        self._expires = None
        self._grant_id = None

    # Connection -----------------------------------------------------------

    def _connect(self):
        check_private_socket(self._path)
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM | socket.SOCK_CLOEXEC)
        try:
            connection.settimeout(RPC_TIMEOUT)
            connection.connect(self._path)
            credentials = connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED,
                                                struct.calcsize("3i"))
            _pid, peer_uid, _gid = struct.unpack("3i", credentials)
            if peer_uid != os.geteuid():
                raise BrokerError("offline")
            connection.settimeout(None)
        except OSError as error:
            connection.close()
            raise BrokerError("offline") from error
        except BrokerError:
            connection.close()
            raise
        self._socket = connection
        threading.Thread(target=self._read, name="badi-broker-reader", daemon=True).start()
        hello = self._rpc("hello", {"min_v": 2, "max_v": 2, "adapter": ADAPTER,
                                    "capabilities": list(CAPABILITIES)}, ("hello.ack",))
        value = hello["payload"]
        enabled = value.get("enabled_capabilities")
        if (value.get("selected_v") != 2 or value.get("max_frame_bytes") != FRAME_LIMIT
                or value.get("max_before_chars") != badi_text.MAX_BEFORE
                or value.get("max_after_chars") != badi_text.MAX_AFTER
                or not isinstance(enabled, list) or sorted(enabled) != sorted(CAPABILITIES)):
            raise self._fail("protocol")
        if not self._authority.wait(RPC_TIMEOUT):
            raise self._fail("timeout")

    def _fail(self, code):
        """Closes the connection once and returns the error to raise."""
        self.close(code)
        return BrokerError(self._revoked or code)

    def close(self, code="closed"):
        """Closes once; returns whether this call closed the attempt."""
        with self._lock:
            if self._closed:
                return False
            self._closed = True
            self._revoked = self._revoked or code
            pending = list(self._pending.values())
            self._pending.clear()
        for slot in pending:
            slot["error"] = self._revoked
            slot["event"].set()
        if self._socket is not None:
            try:
                self._socket.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self._socket.close()
        return True

    def _revoke(self, code):
        if self.close(code) and self._on_revoked is not None:
            self._on_revoked(code)

    def _send(self, kind, payload, message_id=None, coordinates=None):
        message = {"v": 2, "type": kind,
                   "mono_ms": int(self._clock() * 1000) % (MAX_SAFE_COUNTER + 1), "payload": payload}
        # Authority acknowledgements are the only frames the broker forbids an id on.
        if kind != "authority.ack":
            message["id"] = message_id or str(uuid.uuid4())
        if coordinates is not None:
            message.update(coordinates)
        body = json.dumps(message, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        if not body or len(body) > FRAME_LIMIT:
            raise BrokerError("protocol")
        with self._send_lock:
            if self._closed or self._socket is None:
                raise BrokerError(self._revoked or "offline")
            # The socket stays blocking: a timeout would also end the reader's
            # recv while a proposal waits to be read. Frames are small and the
            # broker reads continuously.
            try:
                self._socket.sendall(struct.pack("<I", len(body)) + body)
            except OSError as error:
                raise self._fail("offline") from error

    def _rpc(self, kind, payload, expected, coordinates=None, timeout=RPC_TIMEOUT):
        message_id = str(uuid.uuid4())
        slot = {"event": threading.Event(), "expected": expected, "coordinates": coordinates,
                "message": None, "error": None}
        with self._lock:
            if self._closed:
                raise BrokerError(self._revoked or "offline")
            if len(self._pending) >= 8:
                raise BrokerError("protocol")
            self._pending[message_id] = slot
        try:
            self._send(kind, payload, message_id, coordinates)
            if not slot["event"].wait(timeout):
                raise self._fail("timeout")
        finally:
            with self._lock:
                self._pending.pop(message_id, None)
        if slot["error"] is not None:
            raise BrokerError(slot["error"])
        return slot["message"]

    def _read(self):
        buffer = b""
        try:
            while True:
                chunk = self._socket.recv(FRAME_LIMIT)
                if not chunk:
                    raise ConnectionError
                buffer += chunk
                while len(buffer) >= 4:
                    (length,) = struct.unpack("<I", buffer[:4])
                    if not 1 <= length <= FRAME_LIMIT:
                        raise ValueError
                    if len(buffer) < 4 + length:
                        break
                    frame, buffer = buffer[4:4 + length], buffer[4 + length:]
                    self._receive(json.loads(frame.decode("utf-8")))
        except (ConnectionError, OSError):
            self._revoke("offline")
        except (ValueError, TypeError, KeyError, UnicodeDecodeError, BrokerError):
            self._revoke("protocol")

    def _receive(self, message):
        if (not isinstance(message, dict) or message.get("v") != 2
                or not isinstance(message.get("type"), str) or not _counter(message.get("mono_ms"))
                or not isinstance(message.get("payload"), dict)):
            raise ValueError
        kind = message["type"]
        if kind == "authority.changed":
            epoch = message["payload"].get("authority_epoch")
            if not _counter(epoch) or (self._epoch is not None and epoch <= self._epoch):
                raise ValueError
            initial = self._epoch is None
            self._epoch = epoch
            self._send("authority.ack", {"authority_epoch": epoch})
            if initial:
                self._authority.set()
            else:
                # Any later epoch (pause, settings) retires this attempt.
                self._revoke("revoked")
            return
        with self._lock:
            slot = self._pending.get(message.get("id"))
        if slot is not None:
            coordinates = slot["coordinates"]
            if (coordinates is not None and kind not in ("control.result", "error")
                    and any(message.get(key) != value for key, value in coordinates.items())):
                raise ValueError
            rejected = kind == "error" or (kind == "control.result"
                                           and message["payload"].get("accepted") is False)
            if kind in slot["expected"] or rejected:
                slot["message"] = message
                slot["error"] = "rejected" if rejected else None
                slot["event"].set()
            return
        if kind in ("suggestion.clear", "commit.revoke") and self._session is not None \
                and message.get("session_id") == self._session["session_id"]:
            self._revoke("expired" if message["payload"].get("reason") == "expired" else "revoked")

    # Correction ---------------------------------------------------------

    def _policy(self):
        target = {"kind": TARGET_KIND, "app_id": APP_ID, "target_id": str(uuid.uuid4())}
        epoch = self._epoch
        policy = self._rpc("policy.query", {"target": target}, ("policy.status",))["payload"]
        if (not _counter(policy.get("authority_epoch")) or not _counter(policy.get("settings_revision"))
                or any(not isinstance(policy.get(key), bool) for key in _POLICY_FLAGS)
                or policy.get("activation") not in ("always", "manual", "never")
                or policy["learning_allowed"]
                or (policy["suggestions_allowed"]
                    and not (policy["context_allowed"] and policy["display_allowed"]))
                or (policy["paused"] and (policy["context_allowed"] or policy["display_allowed"]
                                          or policy["suggestions_allowed"]))):
            raise self._fail("protocol")
        if policy["authority_epoch"] != epoch or epoch != self._epoch:
            raise self._fail("revoked")
        if (policy["paused"] or policy["activation"] == "never" or not policy["context_allowed"]
                or not policy["display_allowed"] or not policy["suggestions_allowed"]):
            raise self._fail("blocked")

    def request(self, before, after, language):
        """Asks for a correction of the word before the caret. Returns a
        Proposal, or None when the broker has no replacement to offer."""
        if self._socket is not None:
            raise BrokerError("protocol")
        bounded = badi_text.bounded_context(before, after)
        if bounded is None or not badi_text.valid_language_tag(language):
            raise BrokerError("protocol")
        before, after = bounded
        self._connect()
        self._policy()
        session_id = str(uuid.uuid4())
        self._session = {"session_id": session_id, "focus_epoch": 1, "revision": 1}
        self._fingerprint = hashlib.sha256(uuid.uuid4().bytes + before.encode("utf-8")).hexdigest()
        self._before = before
        target = {"kind": TARGET_KIND, "app_id": APP_ID, "target_id": session_id}
        self._send("session.open", {"target": target, "activation": "always"},
                   coordinates={**self._session, "revision": 0})
        caret = len(before)
        self._send("context.changed", {
            "fingerprint": self._fingerprint, "before": before, "after": after, "language": language,
            "selection": {"anchor": caret, "head": caret, "unit": "unicode_scalar_values"},
            "field": {"purpose": "normal", "editable": True, "multiline": True, "composing": False,
                      "sensitive": False, "identity_known": True, "focused": True,
                      "lock_screen": False},
            "activation": "manual", "explicit": True,
        }, coordinates=self._session)
        started = self._clock()
        reply = self._rpc("suggest.request", {"fingerprint": self._fingerprint, "explicit": True},
                          ("suggestion.show", "suggestion.clear"), self._session, SUGGEST_TIMEOUT)
        if reply["type"] == "suggestion.clear":
            return None
        value = reply["payload"]
        ttl = value.get("ttl_ms")
        suggestion_id = value.get("suggestion_id")
        original = value.get("replace_before")
        replacement = value.get("text")
        if (value.get("fingerprint") != self._fingerprint or not isinstance(suggestion_id, str)
                or not _OPAQUE_ID.fullmatch(suggestion_id) or not _counter(ttl)
                or not 1 <= ttl <= MAX_TTL_MS):
            raise self._fail("protocol")
        if original is None:
            # A continuation is not a correction; this client never appends.
            return None
        if not badi_text.valid_correction(before, original, replacement) \
                or value.get("accept_word") != replacement:
            raise self._fail("protocol")
        self._proposal = Proposal(suggestion_id, original, replacement, ttl)
        self._expires = started + ttl / 1000
        return self._proposal

    def authorize(self):
        """Takes the one-shot grant for the shown proposal."""
        proposal = self._proposal
        if proposal is None or self._clock() >= self._expires:
            raise BrokerError("revoked")
        self._proposal = None
        reply = self._rpc("control.request", {
            "action": "accept_all", "fingerprint": self._fingerprint,
            "suggestion_id": proposal.suggestion_id,
        }, ("commit.prepare",), self._session)
        grant = reply["payload"]
        if (self._clock() >= self._expires or grant.get("acceptance") != "all"
                or grant.get("text") != proposal.text or grant.get("fingerprint") != self._fingerprint
                or grant.get("suggestion_id") != proposal.suggestion_id
                or grant.get("replace_before") != proposal.replace_before):
            raise self._fail("revoked")
        self._grant_id = reply["id"]
        return proposal

    def report(self, status, proposal):
        """Reports the editor-verified result of the granted edit, once."""
        if status not in ("applied", "stale", "blocked", "failed") or self._grant_id is None:
            raise BrokerError("protocol")
        grant_id, self._grant_id = self._grant_id, None
        try:
            self._send("commit.result", {"fingerprint": self._fingerprint,
                                         "suggestion_id": proposal.suggestion_id, "status": status},
                       grant_id, self._session)
        finally:
            self.finish()

    def dismiss(self):
        """Escape: the broker retires the suggestion before the connection closes."""
        proposal, self._proposal = self._proposal, None
        if proposal is not None and not self._closed:
            try:
                self._send("control.request", {"action": "dismiss", "fingerprint": self._fingerprint,
                                               "suggestion_id": proposal.suggestion_id},
                           coordinates=self._session)
            except BrokerError:
                pass
        self.finish()

    def finish(self):
        if self._session is not None and not self._closed:
            try:
                self._send("session.close", {"reason": "session_closed"}, coordinates=self._session)
            except BrokerError:
                pass
        self.close()
