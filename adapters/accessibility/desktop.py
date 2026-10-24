"""Which application has the keyboard: Hyprland's active window, the Omarchy
lock, and the exact process behind that window."""
from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import shutil
import socket
import stat
import struct
import subprocess

from contract import Denied

MAX_CMDLINE = 64 * 1024
MAX_IPC_REPLY = 32 * 1024
IPC_SECONDS = .15
LOCK_FLAGS = ("locked", "secure", "pending", "requested", "sessionLocked")
ELECTRON_NON_ENTRY_OPTIONS = frozenset({b"-r", b"--require", b"-i", b"--interactive", b"-repl",
                                        b"-v", b"--version", b"-a", b"--abi"})
DISCORD_BUILD = re.compile(r"app-[0-9]{1,9}(?:\.[0-9]{1,9}){0,3}\Z")
WINDOW_EVENTS = frozenset({b"activewindowv2", b"focusedmon", b"workspacev2", b"movewindowv2", b"closewindow",
                           b"fullscreen", b"changefloatingmode", b"monitoraddedv2", b"monitorremoved"})
MAX_WINDOW_EVENT_BUFFER = 16 * 1024


@dataclass(frozen=True)
class App:
    """One application's exact identity rule."""
    executables: frozenset = frozenset()
    classes: frozenset = frozenset()
    browser: bool = False
    web: bool = False  # Web content (Chromium or Gecko): HTML purpose gates apply.
    gecko: bool = False  # Gecko browser: DocURL, one focused field, window-relative extents.
    entry: str | None = None  # A shared Electron runtime must load exactly this app.
    per_user: bool = False  # Self-updating build under $XDG_CONFIG_HOME/discord.
    web_apps: str | None = None  # Class prefix of this browser's --app windows: <prefix><host>_<path>-<profile>.
    roles: frozenset | None = None  # Only these field roles, when set.
    # A native app on the addon's IME-parity contract (web rules always are);
    # fcitx5/tests/observer-identities.py checks it against the addon's list.
    observed_only: bool = False
    sentence_bounded: bool = False  # Its Fcitx surrounding text is only the caret's sentence.


def hyprland_directory():
    signature = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE", "")
    runtime = os.environ.get("XDG_RUNTIME_DIR", "")
    if not signature or "/" in signature or signature in (".", "..") or not os.path.isabs(runtime):
        raise RuntimeError("hyprland_session_required")
    return Path(runtime) / "hypr" / signature


class Desktop:
    """Hyprland's active window and monitors and the Omarchy lock.

    `remaining(limit)` gives each call the operation's time left, at most
    `limit` seconds, and denies once the operation has timed out.
    """

    def __init__(self, apps, remaining, request_socket=None):
        self.apps = apps
        self.remaining = remaining
        self.request_socket = request_socket
        self.lock_endpoint = LockEndpoint()

    def app(self, app_id):
        app = self.apps.get(app_id)
        if app is None:
            raise Denied("unsupported_app")
        return app

    def window(self, app):
        """The active window, when this user's exact `app` shows it."""
        window = self.active_window()
        if not verify_process(window["pid"], app, os.getuid(), config_home()) or not _shows(window, app):
            raise Denied("app_mismatch")
        return window

    def lock_query(self):
        endpoint = self.lock_endpoint.resolve(self.remaining)
        return SocketLockQuery(self.remaining, self.lock_endpoint, endpoint) if endpoint else LockQuery(self.remaining)

    def active_window(self):
        window = self.request("j/activewindow")
        if not isinstance(window, dict) or type(window.get("pid")) is not int or window["pid"] <= 0:
            raise Denied("focus_unavailable")
        return window

    def monitors(self):
        monitors = self.request("j/monitors")
        return monitors if isinstance(monitors, list) else []

    def request(self, request):
        """Hyprland's JSON reply to one IPC request, within the operation budget."""
        if self.request_socket is None:
            raise Denied("desktop_unavailable")
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                connection.settimeout(self.remaining(IPC_SECONDS))
                connection.connect(str(self.request_socket))
                connection.sendall(request.encode())
                return json.loads(self._reply(connection))
        except (OSError, ValueError):
            raise Denied("desktop_unavailable") from None

    def _reply(self, connection):
        raw = b""
        while part := connection.recv(8192):
            raw += part
            if len(raw) > MAX_IPC_REPLY:
                raise Denied("desktop_unavailable")
            connection.settimeout(self.remaining(IPC_SECONDS))
        return raw


# Versions whose IPC wire matches the pinned subset: 0.3.2 (checked at
# revision 2b4501a) changed neither ipccommand.hpp nor ipccomm.cpp since 0.3.1.
PINNED_QUICKSHELL = (b"Quickshell 0.3.1 (", b"Quickshell 0.3.2 (")


class LockEndpoint:
    """Cache discovery, never an unlocked answer. The wire format of
    Quickshell 0.3.1 and 0.3.2 is pinned; other versions keep using the
    supported CLI.

    Creating or removing any shell instance, or replacing qs, invalidates the
    cache. The same stamp is checked after each query, including a query that
    overlapped a shell restart. Peer credentials bind it to the discovered PID.
    """

    def __init__(self):
        self.stamp = None
        self.endpoint = None

    def forget(self):
        """Drop the discovered endpoint after it failed, so the next query
        discovers it again instead of failing until the shell restarts."""
        self.stamp, self.endpoint = None, None

    @staticmethod
    def current_stamp():
        runtime = os.environ.get("XDG_RUNTIME_DIR", "")
        shell = os.environ.get("OMARCHY_PATH", "")
        executable = shutil.which("qs")
        if not os.path.isabs(runtime) or not os.path.isabs(shell) or not executable:
            raise Denied("desktop_unavailable")
        instances = Path(runtime) / "quickshell/by-id"
        def identity(path):
            info = os.stat(path)
            return info.st_dev, info.st_ino, info.st_mtime_ns
        return runtime, shell, executable, identity(executable), identity(instances)

    def resolve(self, remaining):
        try:
            stamp = self.current_stamp()
            if stamp == self.stamp:
                return self.endpoint
            self.forget()
            runtime, shell, executable, *_ = stamp
            def run(*args):
                return subprocess.run((executable, *args), stdin=subprocess.DEVNULL,
                                      stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                      check=True, timeout=remaining(IPC_SECONDS)).stdout
            if run("--version").startswith(PINNED_QUICKSHELL):
                instances = json.loads(run("list", "-j", "-n", "-p", os.path.join(shell, "shell")))
                if not isinstance(instances, list) or len(instances) != 1:
                    raise Denied("desktop_unavailable")
                instance = instances[0]
                identifier, pid = instance.get("id"), instance.get("pid")
                if (not isinstance(identifier, str) or not re.fullmatch(r"[a-z0-9]{1,64}", identifier)
                        or type(pid) is not int or pid <= 0
                        or instance.get("config_path") != os.path.realpath(os.path.join(shell, "shell/shell.qml"))):
                    raise Denied("desktop_unavailable")
                self.endpoint = Path(runtime) / "quickshell/by-id" / identifier / "ipc.sock", pid
            if self.current_stamp() != stamp:
                self.endpoint = None
                raise Denied("desktop_unavailable")
            self.stamp = stamp
            return self.endpoint
        except (OSError, ValueError, TypeError, AttributeError, subprocess.SubprocessError):
            raise Denied("desktop_unavailable") from None


class SocketLockQuery:
    """Fresh `lock.status` over the pinned QDataStream wire (see runbook).

    Only StringCall(lock, status, []) is sent, and only Completed(non-void,
    bounded QString) is accepted. The socket overlaps field inspection just
    like the CLI query, but does not spawn a process on the warm path. A
    failed endpoint is forgotten; a locked answer or the operation's own
    timeout says nothing against it.
    """
    REQUEST = b"\x03\x00\x00\x00\x08\x00l\x00o\x00c\x00k\x00\x00\x00\x0c\x00s\x00t\x00a\x00t\x00u\x00s\x00\x00\x00\x00"

    def __init__(self, remaining, cache, endpoint):
        self.remaining, self.cache, self.endpoint = remaining, cache, endpoint
        self.connection = None

    def __enter__(self):
        try:
            self.connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self.connection.settimeout(self.remaining(IPC_SECONDS))
            path, pid = self.endpoint
            self.connection.connect(str(path))
            peer_pid, uid, _ = struct.unpack("3i", self.connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
            if (peer_pid, uid) != (pid, os.getuid()):
                raise Denied("desktop_unavailable")
            self.connection.sendall(self.REQUEST)
            return self
        except (OSError, Denied) as error:
            self.close()
            if not (isinstance(error, Denied) and error.timed_out):
                self.cache.forget()
            raise Denied("desktop_unavailable") from None

    def __exit__(self, *_exception):
        try:
            if self.read(2) != b"\x05\x00":
                raise Denied("desktop_unavailable")
            size = struct.unpack(">I", self.read(4))[0]
            if size > MAX_IPC_REPLY or size % 2:
                raise Denied("desktop_unavailable")
            state = json.loads(self.read(size).decode("utf-16-be"))
            if self.cache.current_stamp() != self.cache.stamp:
                raise Denied("desktop_unavailable")
        except (OSError, ValueError):
            self.cache.forget()
            raise Denied("desktop_unavailable") from None
        except Denied as error:
            if not error.timed_out:
                self.cache.forget()
            raise
        finally:
            self.close()
        require_unlocked(state)

    def read(self, count):
        raw = bytearray()
        while len(raw) < count:
            self.connection.settimeout(self.remaining(IPC_SECONDS))
            part = self.connection.recv(count - len(raw))
            if not part:
                raise Denied("desktop_unavailable")
            raw.extend(part)
        return raw

    def close(self):
        if self.connection is not None:
            self.connection.close()


def require_unlocked(state):
    if not isinstance(state, dict) or any(state.get(flag) is not False for flag in LOCK_FLAGS):
        raise Denied("desktop_locked")


class LockQuery:
    """The Omarchy lock status, asked by a child process while the block runs.

    Leaving the block waits at most IPC_SECONDS more, within the operation
    budget, and denies unless every lock flag is explicitly false. That denial
    replaces the block's result or error, so a locked session never yields a
    binding. A missing shell, a failed or late query and any reply that is not
    the status document fail closed. The child is always reaped, and every
    block asks again.
    """

    def __init__(self, remaining):
        self.remaining = remaining
        self.process = None

    def __enter__(self):
        shell = os.environ.get("OMARCHY_PATH", "")
        if not os.path.isabs(shell):
            raise Denied("desktop_unavailable")
        # The Quickshell IPC call behind `omarchy-shell lock status`, made
        # directly: killing that wrapper on timeout would leave this running.
        try:
            self.process = subprocess.Popen(
                ("qs", "ipc", "-n", "-p", os.path.join(shell, "shell"), "call", "--", "lock", "status"),
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        except (OSError, subprocess.SubprocessError):
            raise Denied("desktop_unavailable") from None
        return self

    def __exit__(self, *_exception):
        try:
            self.require_unlocked()
        finally:
            self.reap()

    def reap(self):
        """Kill the query if it still runs, and collect its exit status."""
        self.process.kill()
        self.process.wait()
        self.process.stdout.close()

    def require_unlocked(self):
        raw = self.reply()
        try:
            state = json.loads(raw) if len(raw) <= MAX_IPC_REPLY else None
        except ValueError:
            raise Denied("desktop_unavailable") from None
        require_unlocked(state)

    def reply(self):
        """The query's output, once it exits successfully."""
        try:
            raw, _ = self.process.communicate(timeout=self.remaining(IPC_SECONDS))
        except (OSError, subprocess.SubprocessError):
            raise Denied("desktop_unavailable") from None
        if self.process.returncode != 0:
            raise Denied("desktop_unavailable")
        return raw


class WindowEvents:
    """Hyprland's event stream, reduced to whether the active window or layout changed."""

    def __init__(self, stream):
        self.stream = stream
        self.buffer = b""
        # Hyprland repeats activewindowv2 for the same window whenever its
        # title changes (an editor's unsaved mark or autosave, a terminal's
        # running command), which is no focus change.
        self.active = None

    @classmethod
    def connect(cls, directory):
        stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            stream.connect(str(directory / ".socket2.sock"))
        except OSError:
            stream.close()
            raise
        stream.setblocking(False)
        return cls(stream)

    def fileno(self):
        return self.stream.fileno()

    def changed(self):
        """Whether the newly received events change focus, windows or monitors.

        OSError means the stream ended or flooded; the observer cannot follow
        the desktop any longer. A wake-up with nothing to read changes nothing.
        """
        try:
            data = self.stream.recv(4096)
        except BlockingIOError:
            return False
        if not data:
            raise OSError("hyprland_events_closed")
        self.buffer += data
        if len(self.buffer) > MAX_WINDOW_EVENT_BUFFER:
            raise OSError("hyprland_events_flooded")
        *lines, self.buffer = self.buffer.split(b"\n")
        changed = False
        for line in lines:
            name, _separator, value = line.partition(b">>")
            if name == b"activewindowv2":
                changed = changed or value != self.active
                self.active = value
            elif name in WINDOW_EVENTS:
                changed = True
        return changed

    def close(self):
        self.stream.close()


def config_home():
    value = os.environ.get("XDG_CONFIG_HOME", "")
    return Path(value) if os.path.isabs(value) else Path.home() / ".config"


def verify_process(pid, app, uid, config, proc=Path("/proc")):
    """Whether `uid` owns process `pid` and it runs exactly `app`."""
    process = proc / str(pid)
    try:
        if process.stat().st_uid != uid:
            return False
        executable = os.readlink(process / "exe")
        if app.per_user:
            return per_user_executable(executable, os.stat(process / "exe"), config, uid)
        if executable not in app.executables:
            return False
        return app.entry is None or _loads_entry(process, app.entry)
    except OSError:
        return False


def _loads_entry(process, entry):
    with (process / "cmdline").open("rb") as stream:
        cmdline = stream.read(MAX_CMDLINE + 1)
    return len(cmdline) <= MAX_CMDLINE and application_entry(cmdline) == os.fsencode(entry)


def application_entry(cmdline):
    """The app Electron loads: its first non-switch argument, as Chromium parses it.

    Electron's default_app also honours options that load other code instead
    of, or before, that argument (--app=, -r/--require) or run no app at all
    (REPL, version, ABI). Any of them before the entry means no entry.
    """
    remaining = iter(_arguments(cmdline)[1:])
    for argument in remaining:
        if argument in ELECTRON_NON_ENTRY_OPTIONS or argument.startswith(b"--app="):
            return None
        if argument == b"--":
            return next(remaining, None)
        if not argument.startswith(b"-"):
            return argument
    return None


def _arguments(cmdline):
    """A NUL-separated command line's arguments.

    The Cursor GUI process rewrites its title into one space-joined argument.
    A path with spaces then yields a wrong first token, which cannot equal
    the pinned entry, so this fails closed.
    """
    arguments = cmdline.split(b"\0")
    if arguments and arguments[-1] == b"":
        arguments.pop()
    if len(arguments) == 1 and b" " in arguments[0]:
        arguments = [argument for argument in arguments[0].split(b" ") if argument]
    return arguments


def per_user_executable(executable, running, config, uid):
    """Discord's own updater installs <config>/discord/app-<version>/Discord.

    The kernel reports a canonical executable path. Recheck the current tree
    so no symlinked or foreign-owned component, and no replaced file, passes.
    """
    try:
        if not os.path.isabs(executable) or os.path.normpath(executable) != executable:
            return False
        base = Path(os.path.realpath(config)) / "discord"
        binary = Path(executable)
        if binary.name != "Discord" or not DISCORD_BUILD.fullmatch(binary.parent.name) or binary.parent.parent != base:
            return False
        if not (_unshared(base, stat.S_ISDIR, uid) and _unshared(binary.parent, stat.S_ISDIR, uid)):
            return False
        info = _unshared(binary, stat.S_ISREG, uid)
        return info is not None and (info.st_dev, info.st_ino) == (running.st_dev, running.st_ino)
    except (OSError, ValueError):
        return False


def _unshared(path, kind, uid):
    """`path`'s own status when it is a `kind` that `uid` owns and nobody else may write."""
    info = path.lstat()
    return info if kind(info.st_mode) and info.st_uid == uid and not info.st_mode & 0o022 else None


def _shows(window, app):
    return _app_class(window.get("class"), app) and window.get("mapped") is True and window.get("hidden") is False


def _app_class(value, app):
    if value in app.classes:
        return True
    prefix = app.web_apps
    return (prefix is not None and isinstance(value, str) and value.startswith(prefix) and len(value) <= 255
            and "__" in value[len(prefix):] and all(" " < char < "\x7f" and char != "/" for char in value))
