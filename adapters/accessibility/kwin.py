"""KDE Plasma's focused window, as a KWin script of the observer's own reports
it ([ADR 0015](../../docs/decisions/0015-kwin-script-reports-plasma-focus.md)).

KWin keeps `org_kde_plasma_window_management` from clients that no desktop
file grants it, so the observer loads a small script into KWin instead. The
script sends the active window's process id, app id and window id to this
observer's own D-Bus connection on every activation. A report counts only
from the connection that owns `org.kde.KWin`, which the bus daemon says runs
as this user in a process the kernel names `kwin_wayland`, carrying this
run's random token. (Distributions give kwin_wayland CAP_SYS_NICE, which
makes it undumpable: its /proc exe link cannot be read, so the name comes
from /proc/PID/status. Same-UID processes are trusted, ADR 0001.)

Each inspection first asks KWin, on its main thread, whether the script is
still loaded: KWin answers after sending every report from activations it
already handled, so the window read afterwards is current. The lock answer
comes from KWin's own `org.freedesktop.ScreenSaver` object and must be an
explicit false. Anything else fails closed. The script reads no window title
or content, and no geometry: Plasma fields use the Fcitx panel.
"""
from __future__ import annotations

import hmac
import json
import os
from pathlib import Path
import re
import secrets
import threading

from contract import Denied
from desktop import NoDesktopBackend
from sessionbus import UNIQUE_NAME, OwnerError, app_id, is_process, process_name, unique_object, verified_owner

KWIN_NAME = "org.kde.KWin"
KWIN_PROCESS = "kwin_wayland"
GREETER = "kscreenlocker_greet"
PLUGIN = "badi-field-observer"
REPORT_PATH = "/org/badi/FieldObserver"
REPORT_INTERFACE = "org.badi.FieldObserver.KWin1"
SCRIPT_NAME = "kwin-field-observer.js"
MAX_REPORT = 4096
IPC_SECONDS = .15
STARTUP_SECONDS = 3.0
STOP_SECONDS = .2
WINDOW_ID = re.compile(r"\{[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\}\Z")
TOKEN = re.compile(r"[0-9a-f]{32}\Z")

SCRIPT = """\
// Badi's field observer (ADR 0015): tells the one observer connection that
// loaded this script which window has focus. No title, content or geometry.
"use strict";
const TARGET = "@TARGET@";
const TOKEN = "@TOKEN@";
const PATH = "/org/badi/FieldObserver";
const INTERFACE = "org.badi.FieldObserver.KWin1";
let serial = 0;
let watched = null;

function describe(window) {
    if (!window) {
        return null;
    }
    return {pid: window.pid, app_id: String(window.resourceClass), id: String(window.internalId),
            minimized: window.minimized === true, hidden: window.hidden === true,
            managed: window.managed === true, deleted: window.deleted === true};
}

function report() {
    const window = workspace.activeWindow;
    if (window !== watched) {
        if (watched && typeof watched.windowClassChanged === "function") {
            watched.windowClassChanged.disconnect(report);
        }
        watched = window;
        if (watched && typeof watched.windowClassChanged === "function") {
            watched.windowClassChanged.connect(report);
        }
    }
    serial += 1;
    callDBus(TARGET, PATH, INTERFACE, "Focus", TOKEN, JSON.stringify({serial: serial, window: describe(window)}));
}

const supported = typeof workspace.windowActivated === "function" && typeof workspace.activeWindow === "object";
callDBus(TARGET, PATH, INTERFACE, "Hello", TOKEN, JSON.stringify({supported: supported}));
if (supported) {
    workspace.windowActivated.connect(report);
    report();
}
"""


class KWinUnavailable(RuntimeError):
    """KWin or its scripting did not answer; a later start may succeed."""


def script_source(target, token):
    if not UNIQUE_NAME.fullmatch(target) or not TOKEN.fullmatch(token):
        raise ValueError("invalid_script_parameters")
    return SCRIPT.replace("@TARGET@", target).replace("@TOKEN@", token)


def parse_window(value):
    """The reported window as a record, None for no active window.

    Raises ValueError for anything that is not exactly the script's shape.
    """
    if value is None:
        return None
    keys = {"pid", "app_id", "id", "minimized", "hidden", "managed", "deleted"}
    if not isinstance(value, dict) or set(value) != keys:
        raise ValueError("invalid_window")
    pid, identifier = value["pid"], value["id"]
    if (type(pid) is not int or not 0 < pid < 2**22 or not isinstance(identifier, str)
            or not WINDOW_ID.fullmatch(identifier)):
        raise ValueError("invalid_window")
    flags = [value[name] for name in ("minimized", "hidden", "managed", "deleted")]
    if not all(type(flag) is bool for flag in flags):
        raise ValueError("invalid_window")
    return {"pid": pid, "app_id": app_id(value["app_id"]), "id": identifier,
            "hidden": value["minimized"] or value["hidden"], "mapped": value["managed"] and not value["deleted"]}


class KWinFocus:
    """The active window as this observer's KWin script last reported it.

    `bus` is a GioBus or a test double. `on_change` and `on_lost` run on the
    bus's message thread and must only schedule work on the main loop.
    """

    def __init__(self, bus, script_path, on_change=None, on_lost=None, token=None, proc=Path("/proc")):
        self.bus = bus
        self.script_path = Path(script_path)
        self.on_change = on_change or (lambda: None)
        self.on_lost = on_lost or (lambda: None)
        self.token = token or secrets.token_hex(16)
        self.proc = proc
        self.lock = threading.Lock()
        self.hello = threading.Event()
        self.reported = threading.Event()
        self.supported = None
        self.kwin = None
        self.serial = 0
        self.window = None
        self.lost = False

    def start(self, timeout=STARTUP_SECONDS):
        """Load the script into the verified KWin and wait for its first report."""
        self.kwin = self.verified_kwin(timeout)
        self.bus.serve(REPORT_PATH, REPORT_INTERFACE, self.receive)
        self.bus.watch_owner(KWIN_NAME, self.owner_changed)
        self.write_script()
        try:
            self.scripting("unloadScript", "(s)", (PLUGIN,), "(b)", timeout)
            script_id = self.scripting("loadScript", "(ss)", (str(self.script_path), PLUGIN), "(i)", timeout)[0]
            if type(script_id) is not int or script_id < 0:
                raise KWinUnavailable("kwin_script_refused")
            self.call(f"/Scripting/Script{script_id}", "org.kde.kwin.Script", "run", None, None, None, timeout)
            if not self.hello.wait(timeout):
                raise KWinUnavailable("kwin_script_silent")
            if self.supported is not True:
                raise NoDesktopBackend("kwin_scripting_unsupported")
            if not self.reported.wait(timeout):
                raise KWinUnavailable("kwin_script_silent")
        finally:
            # KWin read the file when the script started; the token need not stay on disk.
            self.remove_script()

    def stop(self):
        """Unload the script; best effort, the observer is exiting."""
        self.remove_script()
        if self.kwin is not None and not self.lost:
            try:
                self.scripting("unloadScript", "(s)", (PLUGIN,), "(b)", STOP_SECONDS)
            except Exception:
                pass

    def verified_kwin(self, timeout):
        """The unique bus name of this user's running kwin_wayland."""
        try:
            return verified_owner(self.bus, KWIN_NAME, KWIN_PROCESS, timeout, self.proc)
        except OwnerError as error:
            raise KWinUnavailable(f"kwin_{error}") from None

    def write_script(self):
        flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW | os.O_CLOEXEC
        descriptor = os.open(self.script_path, flags, 0o600)
        with os.fdopen(descriptor, "w") as stream:
            os.fchmod(stream.fileno(), 0o600)
            stream.write(script_source(self.bus.unique_name, self.token))

    def remove_script(self):
        try:
            self.script_path.unlink()
        except FileNotFoundError:
            pass

    def receive(self, sender, member, body):
        """Accept one call to the report path; False refuses it.

        Runs on the bus's message thread, in the order KWin sent the calls.
        """
        if sender != self.kwin or self.kwin is None or member not in ("Hello", "Focus"):
            return False
        if not isinstance(body, tuple) or len(body) != 2 or not all(isinstance(part, str) for part in body):
            return False
        token, raw = body
        if not hmac.compare_digest(token.encode(), self.token.encode()) or len(raw.encode()) > MAX_REPORT:
            return False
        try:
            document = json.loads(raw, object_pairs_hook=unique_object)
        except ValueError:
            document = None
        if member == "Hello":
            self.supported = (isinstance(document, dict) and set(document) == {"supported"}
                              and document["supported"] is True)
            self.hello.set()
            return True
        if not isinstance(document, dict) or set(document) != {"serial", "window"}:
            return False
        serial = document["serial"]
        if type(serial) is not int:
            return False
        try:
            window = parse_window(document["window"])
        except ValueError:
            # KWin's own report in an unknown shape: no window, never a guess.
            window = None
        with self.lock:
            if serial <= self.serial:
                return False
            self.serial = serial
            changed = window != self.window
            self.window = window
        self.reported.set()
        if changed:
            self.on_change()
        return True

    def owner_changed(self, owner):
        """KWin left the bus or another process took its name: stop trusting reports."""
        if owner != self.kwin:
            self.lose()

    def lose(self):
        with self.lock:
            if self.lost:
                return
            self.lost = True
            self.window = None
        self.on_lost()

    def lock_query(self, remaining):
        return KWinLockQuery(self, remaining)

    def active_window(self, remaining):
        """The active window in the Desktop's window shape, current as of this call."""
        self.require_script(remaining)
        with self.lock:
            window, lost = self.window, self.lost
        if lost:
            raise Denied("desktop_unavailable")
        if window is None:
            raise Denied("focus_unavailable")
        if window["app_id"] == GREETER or is_process(process_name(window["pid"], self.proc), GREETER):
            raise Denied("desktop_locked")
        # No geometry: xwayland stays unknown, so no preview is calibrated.
        return {"pid": window["pid"], "class": window["app_id"], "mapped": window["mapped"],
                "hidden": window["hidden"], "address": window["id"], "xwayland": None}

    def require_script(self, remaining):
        """A round trip through KWin's main thread: reports it sent before this
        answer have been received, and the script is still loaded."""
        if self.lost:
            raise Denied("desktop_unavailable")
        try:
            loaded = self.scripting("isScriptLoaded", "(s)", (PLUGIN,), "(b)", remaining(IPC_SECONDS))[0]
        except Denied:
            raise
        except Exception:
            raise Denied("desktop_unavailable") from None
        if loaded is not True:
            self.lose()
            raise Denied("desktop_unavailable")

    def require_unlocked(self, remaining):
        """KWin's own screen locker must answer an explicit false."""
        if self.lost:
            raise Denied("desktop_unavailable")
        try:
            active = self.call("/ScreenSaver", "org.freedesktop.ScreenSaver", "GetActive", None, None, "(b)",
                               remaining(IPC_SECONDS))
        except Denied:
            raise
        except Exception:
            raise Denied("desktop_unavailable") from None
        if not isinstance(active, tuple) or len(active) != 1 or type(active[0]) is not bool:
            raise Denied("desktop_unavailable")
        if active[0]:
            raise Denied("desktop_locked")

    def scripting(self, method, signature, args, reply, timeout):
        return self.call("/Scripting", "org.kde.kwin.Scripting", method, signature, args, reply, timeout)

    def call(self, path, interface, method, signature, args, reply, timeout):
        return self.bus.call(self.kwin, path, interface, method, signature, args, reply, timeout)



class KWinLockQuery:
    """The lock answer, asked after the field was read, as on Hyprland."""

    def __init__(self, focus, remaining):
        self.focus, self.remaining = focus, remaining

    def __enter__(self):
        return self

    def __exit__(self, *_exception):
        self.focus.require_unlocked(self.remaining)
