"""Metadata-only readiness probe shared by installation and desktop diagnostics."""
from __future__ import annotations

import json
import os
from pathlib import Path
import socket
import stat
import struct
import time

SCHEMA = "badi.accessibility.v1"
MAX_STATUS_FRAME = 1024


class ProbeError(Exception):
    """A fixed diagnostic code; never contains application text or paths."""


def _private(path, kind, mode):
    info = path.lstat()
    if not kind(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != mode:
        raise ProbeError("unsafe_socket")
    return info


def _object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError
        result[key] = value
    return result


def probe(endpoint, expected_pid):
    """Verify the private socket peer and a pure status reply within 500 ms.

    The caller supplies the systemd MainPID and must recheck the service after
    this function. Ready means the observer event loop responds; it does not
    establish accessibility permission, an eligible field, or app coverage.
    """
    if type(expected_pid) is not int or expected_pid <= 0:
        raise ProbeError("service_unavailable")
    endpoint = Path(endpoint)
    if not endpoint.is_absolute():
        raise ProbeError("unsafe_socket")
    try:
        parent = _private(endpoint.parent, stat.S_ISDIR, 0o700)
        initial = _private(endpoint, stat.S_ISSOCK, 0o600)
        deadline = time.monotonic() + .5
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(.5)
            connection.connect(str(endpoint))
            pid, uid, _gid = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
            if pid != expected_pid or uid != os.getuid():
                raise ProbeError("unexpected_peer")
            request = {"schema": SCHEMA, "id": "health", "op": "status"}
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise ProbeError("timeout")
            connection.settimeout(remaining)
            connection.sendall((json.dumps(request, separators=(",", ":")) + "\n").encode())
            raw = b""
            while b"\n" not in raw:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ProbeError("timeout")
                connection.settimeout(remaining)
                part = connection.recv(MAX_STATUS_FRAME + 1 - len(raw))
                if not part or len(raw) + len(part) > MAX_STATUS_FRAME:
                    raise ProbeError("invalid_response")
                raw += part
            if raw.count(b"\n") != 1 or not raw.endswith(b"\n"):
                raise ProbeError("invalid_response")
            response = json.loads(raw, object_pairs_hook=_object)
            expected = {"schema": SCHEMA, "id": "health", "ok": True,
                        "status": {"ready": True, "process_id": expected_pid, "protocol_version": 1}}
            if (response != expected or response["ok"] is not True or response["status"]["ready"] is not True
                    or type(response["status"]["process_id"]) is not int or type(response["status"]["protocol_version"]) is not int):
                raise ProbeError("invalid_response")
        final = _private(endpoint, stat.S_ISSOCK, 0o600)
        final_parent = _private(endpoint.parent, stat.S_ISDIR, 0o700)
        if (initial.st_dev, initial.st_ino, parent.st_dev, parent.st_ino) != (final.st_dev, final.st_ino, final_parent.st_dev, final_parent.st_ino):
            raise ProbeError("socket_changed")
        return response["status"]
    except ProbeError:
        raise
    except TimeoutError:
        raise ProbeError("timeout") from None
    except (FileNotFoundError, ConnectionRefusedError):
        raise ProbeError("service_unavailable") from None
    except (ValueError, UnicodeError, TypeError, KeyError):
        raise ProbeError("invalid_response") from None
    except OSError:
        raise ProbeError("transport_unavailable") from None
