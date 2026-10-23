"""The session helper's broker connection: one negotiated capability, no text.

The helper reports only the login session, power state and whether the screen
is shared, and hands over the keyring's style-learning key. It opens no session, reads no field and
holds no editing grant, so it negotiates only ``desktop_signals`` and
``style_key`` on protocol v2.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import socket
import stat
import struct
import time

FRAME_LIMIT = 65536
SESSIONS = ("active", "inactive", "sleeping")
AUTOMATIC = ("normal", "deferred", "explicit_only")
SCREEN_SHARE = ("none", "held", "suggesting")
CAPABILITIES = ["desktop_signals", "style_key"]
STYLE_KEYS = ("absent", "ready", "mismatch")


class LinkError(Exception):
    """A fixed, content-free reason the broker could not take a report."""


def default_endpoint(environment=os.environ):
    runtime = environment.get("XDG_RUNTIME_DIR", "")
    if not runtime or not os.path.isabs(runtime):
        raise LinkError("runtime_directory_unavailable")
    return Path(runtime) / "badi" / "broker.sock"


def _private(path, kind, mode):
    info = path.lstat()
    if not kind(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != mode:
        raise LinkError("unsafe_socket")


def _object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result


def encode(message):
    body = json.dumps(message, separators=(",", ":"), ensure_ascii=False).encode()
    if not 1 <= len(body) <= FRAME_LIMIT:
        raise LinkError("frame_too_large")
    return struct.pack("<I", len(body)) + body


def report(session, on_battery, power_saver, screen_shared):
    if (session not in SESSIONS or type(on_battery) is not bool or type(power_saver) is not bool
            or type(screen_shared) is not bool):
        raise LinkError("invalid_report")
    return {"session": session, "on_battery": on_battery, "power_saver": power_saver,
            "screen_shared": screen_shared}


def valid_status(payload):
    """The broker's combined view, checked like the protocol schema does."""
    if (type(payload) is not dict
            or set(payload) != {"observed", "session", "low_power", "automatic", "screen_share"}
            or type(payload["observed"]) is not bool or payload["session"] not in SESSIONS
            or type(payload["low_power"]) is not bool or payload["automatic"] not in AUTOMATIC
            or payload["screen_share"] not in SCREEN_SHARE):
        return False
    if not payload["observed"] and (payload["session"] != "active" or payload["low_power"]
                                    or payload["screen_share"] != "none"):
        return False
    return payload["low_power"] or payload["automatic"] == "normal"


class Link:
    """A blocking, bounded connection. Every call either completes within the
    timeout or raises ``LinkError``; the caller reconnects and resends."""

    def __init__(self, endpoint=None, timeout=1.5):
        self.endpoint = Path(endpoint) if endpoint is not None else default_endpoint()
        self.timeout = timeout
        self.socket = None
        self._buffer = b""
        self._serial = 0

    @property
    def connected(self):
        return self.socket is not None

    def fileno(self):
        return self.socket.fileno()

    def connect(self):
        self.close()
        if not self.endpoint.is_absolute():
            raise LinkError("unsafe_socket")
        try:
            _private(self.endpoint.parent, stat.S_ISDIR, 0o700)
            _private(self.endpoint, stat.S_ISSOCK, 0o600)
            connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM | socket.SOCK_CLOEXEC)
        except FileNotFoundError:
            raise LinkError("broker_offline") from None
        except OSError:
            raise LinkError("broker_offline") from None
        try:
            connection.settimeout(self.timeout)
            connection.connect(str(self.endpoint))
            _pid, uid, _gid = struct.unpack("3i", connection.getsockopt(
                socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i")))
            if uid != os.getuid():
                raise LinkError("unexpected_peer")
        except LinkError:
            connection.close()
            raise
        except OSError:
            connection.close()
            raise LinkError("broker_offline") from None
        self.socket = connection
        self._buffer = b""
        try:
            hello = self._exchange("hello", {
                "min_v": 2, "max_v": 2,
                "adapter": {"kind": "session", "name": "badi-session", "version": "0.1.0"},
                "capabilities": CAPABILITIES,
            }, "hello.ack")
            value = hello["payload"]
            if (value.get("selected_v") != 2 or value.get("enabled_capabilities") != CAPABILITIES
                    or value.get("max_frame_bytes") != FRAME_LIMIT):
                raise LinkError("incompatible_broker")
        except LinkError:
            self.close()
            raise

    def send_report(self, session, on_battery, power_saver, screen_shared):
        """Returns the broker's validated combined status."""
        if self.socket is None:
            raise LinkError("broker_offline")
        try:
            reply = self._exchange("desktop.state", report(session, on_battery, power_saver, screen_shared),
                                   "desktop.status")
        except LinkError:
            self.close()
            raise
        if not valid_status(reply["payload"]):
            self.close()
            raise LinkError("invalid_status")
        return reply["payload"]

    def send_style_key(self, key_id, key):
        """Delivers the keyring key; returns the broker's style key state."""
        if self.socket is None:
            raise LinkError("broker_offline")
        try:
            reply = self._exchange("style.key", {"key_id": key_id, "key": key}, "style.status")
        except LinkError:
            self.close()
            raise
        payload = reply["payload"]
        if (type(payload) is not dict
                or set(payload) != {"enabled", "available", "key", "samples", "subjects"}
                or payload["key"] not in STYLE_KEYS):
            self.close()
            raise LinkError("invalid_status")
        return payload["key"]

    def closed_by_peer(self):
        """Called when the socket turns readable. The broker sends nothing
        unsolicited here, so readable means closed, broken or misbehaving;
        a spurious wake-up keeps the connection."""
        if self.socket is None:
            return True
        try:
            self.socket.setblocking(False)
            try:
                self.socket.recv(1)
            finally:
                self.socket.settimeout(self.timeout)
        except BlockingIOError:
            return False
        except OSError:
            pass
        self.close()
        return True

    def close(self):
        if self.socket is not None:
            try:
                self.socket.close()
            except OSError:
                pass
        self.socket = None
        self._buffer = b""

    def _exchange(self, message_type, payload, expected):
        self._serial += 1
        request_id = f"session.{self._serial}"
        message = {"v": 2, "id": request_id, "type": message_type,
                   "mono_ms": int(time.monotonic() * 1000), "payload": payload}
        try:
            self.socket.sendall(encode(message))
            deadline = time.monotonic() + self.timeout
            while True:
                reply = self._read(deadline)
                if reply.get("id") != request_id:
                    raise LinkError("unexpected_reply")
                if reply["type"] == "error":
                    raise LinkError("broker_rejected")
                if reply["type"] != expected:
                    raise LinkError("unexpected_reply")
                return reply
        except (OSError, ValueError):
            raise LinkError("broker_offline") from None

    def _read(self, deadline):
        while True:
            if len(self._buffer) >= 4:
                (length,) = struct.unpack_from("<I", self._buffer)
                if not 1 <= length <= FRAME_LIMIT:
                    raise LinkError("invalid_frame")
                if len(self._buffer) >= 4 + length:
                    body = self._buffer[4:4 + length]
                    self._buffer = self._buffer[4 + length:]
                    message = json.loads(body.decode(), object_pairs_hook=_object)
                    if (type(message) is not dict or message.get("v") != 2
                            or type(message.get("type")) is not str
                            or type(message.get("payload")) is not dict):
                        raise LinkError("invalid_frame")
                    return message
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise LinkError("timeout")
            self.socket.settimeout(remaining)
            chunk = self.socket.recv(FRAME_LIMIT)
            if not chunk:
                raise LinkError("broker_offline")
            self._buffer += chunk
