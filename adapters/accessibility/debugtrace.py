"""The observer's lines in the opt-in debug trace behind `badi debug on`.

While the controller's `debug-control.json` is valid and unexpired, each event
appends one JSON line to `debug-trace.jsonl` in the private runtime directory.
The observer never creates the trace: `badi debug on` does, and `badi debug
off` deletes it, and BADI_DEBUG_TRACE=off turns it off for a process. Fields
must be content-free; `text` fields are kept only when
the run says `"text": true` (`badi debug on --text`).
"""

import json
import os
import stat
import time
import uuid

CONTROL = "debug-control.json"
TRACE = "debug-trace.jsonl"
MAX_TRACE_BYTES = 16 * 1024 * 1024
MAX_RUN_SECONDS = 900
RECHECK_SECONDS = 1.0

_checked = None
_run = None


def _directory():
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    return os.path.join(runtime, "badi") if runtime and os.path.isabs(runtime) else None


def _open_private(path, flags, limit):
    descriptor = os.open(path, flags | os.O_NOFOLLOW | os.O_CLOEXEC)
    info = os.fstat(descriptor)
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077
            or info.st_size > limit):
        os.close(descriptor)
        return None
    return descriptor


def _read_run(directory):
    """`{"text": bool}` for a valid, unexpired run, else None."""
    try:
        descriptor = _open_private(os.path.join(directory, CONTROL), os.O_RDONLY, 512)
    except OSError:
        return None
    if descriptor is None:
        return None
    with os.fdopen(descriptor) as stream:
        try:
            control = json.load(stream)
            expires_at, identity = control["expires_at"], control["id"]
            uuid.UUID(identity)
        except (ValueError, KeyError, TypeError, AttributeError):
            return None
    now = time.time()
    if type(expires_at) is not int or not now < expires_at <= now + MAX_RUN_SECONDS:
        return None
    return {"text": control.get("text") is True}


def active():
    """The current run, rechecked at most once a second."""
    global _checked, _run
    now = time.monotonic()
    if _checked is None or now - _checked >= RECHECK_SECONDS:
        directory = _directory()
        _run = _read_run(directory) if directory else None
        _checked = now
    return _run


def emit(event, fields=None, text=None):
    """Appends one observer event while a debug run is active. Test runs set
    BADI_DEBUG_TRACE=off so they never write into a live run's trace."""
    if os.environ.get("BADI_DEBUG_TRACE") == "off":
        return
    run = active()
    directory = _directory()
    if run is None or directory is None:
        return
    line = {"t": int(time.time() * 1000), "src": "observer", "ev": event, **(fields or {})}
    if run["text"] and text:
        line.update(text)
    try:
        descriptor = _open_private(os.path.join(directory, TRACE), os.O_WRONLY | os.O_APPEND,
                                   MAX_TRACE_BYTES)
    except OSError:
        return
    if descriptor is None:
        return
    try:
        # One write per line: O_APPEND keeps concurrent writers' lines whole.
        os.write(descriptor, (json.dumps(line, ensure_ascii=False) + "\n").encode())
    except OSError:
        pass
    finally:
        os.close(descriptor)
