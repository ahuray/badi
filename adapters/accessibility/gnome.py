"""GNOME's focused window, as Badi's own Shell extension answers it
([ADR 0016](../../docs/decisions/0016-gnome-shell-extension-reports-focus.md)).

`org.gnome.Shell.Introspect` gives no process id and answers only the portal
backends, so the user installs and enables the `badi-focus@ahuray.github.io`
extension. It exports `GetFocus` on gnome-shell's own connection. The
observer verifies the owner of `org.gnome.Shell` (this user's `gnome-shell`,
by the bus daemon's credentials and the kernel's process name) and calls that
unique name, so only gnome-shell can answer. Every inspection asks afresh;
nothing is cached. The extension's `FocusChanged` signal, accepted only from
that unique name, invalidates the tracked field.

A missing or disabled extension (GNOME disables it while the screen is
locked) is `desktop_unavailable`; a locked session, or `org.gnome.ScreenSaver`
on the same unique name not answering an explicit false, is
`desktop_locked`. No geometry is reported: GNOME fields use the input
method's panel.
"""
from __future__ import annotations

from pathlib import Path
import threading
import time

from contract import Denied
from desktop import NoDesktopBackend
from sessionbus import OwnerError, app_id, verified_owner

SHELL_NAME = "org.gnome.Shell"
SHELL_PROCESS = "gnome-shell"
EXTENSION = "badi-focus@ahuray.github.io"
FOCUS_PATH = "/io/github/ahuray/badi/Focus"
FOCUS_INTERFACE = "io.github.ahuray.badi.Focus1"
FOCUS_REPLY = "(ubtussb)"
VERSION = 1
IPC_SECONDS = .15
STARTUP_SECONDS = 3.0
# How long a session that calls itself GNOME may take to show gnome-shell.
SHELL_WAIT_SECONDS = 10.0


class ShellUnavailable(RuntimeError):
    """gnome-shell did not answer or is not this user's; a later start may succeed."""


def parse_focus(reply):
    """The focused window in the Desktop's window shape, or a Denied reason.

    `reply` is GetFocus's (version, locked, window, pid, app_id, client,
    minimized). Anything but exactly that shape is desktop_unavailable.
    """
    if not isinstance(reply, tuple) or len(reply) != 7:
        raise Denied("desktop_unavailable")
    version, locked, window, pid, application, client, minimized = reply
    if (type(version) is not int or type(locked) is not bool or type(window) is not int
            or type(pid) is not int or not isinstance(application, str) or client not in ("wayland", "x11", "")
            or type(minimized) is not bool):
        raise Denied("desktop_unavailable")
    if version != VERSION:
        raise Denied("desktop_unavailable")
    if locked:
        raise Denied("desktop_locked")
    if window == 0:
        raise Denied("focus_unavailable")
    if not 0 < pid < 2**22 or client == "":
        raise Denied("focus_unavailable")
    # An X11 window runs under XWayland: no caret is calibrated for it, and
    # none is for a Wayland window either, since GNOME reports no geometry.
    return {"pid": pid, "class": app_id(application), "mapped": True, "hidden": minimized,
            "address": str(window), "xwayland": True if client == "x11" else None}


class GnomeFocus:
    """GetFocus answers from gnome-shell, called on its verified unique name.

    `bus` is a sessionbus.GioBus or a test double. `on_change` and `on_lost`
    run on the main loop and must only schedule work there.
    """

    def __init__(self, bus, on_change=None, on_lost=None, proc=Path("/proc")):
        self.bus = bus
        self.on_change = on_change or (lambda: None)
        self.on_lost = on_lost or (lambda: None)
        self.proc = proc
        self.shell = None
        self.lost = False
        self.lock = threading.Lock()

    def start(self, timeout=STARTUP_SECONDS, wait=SHELL_WAIT_SECONDS, sleep=time.sleep):
        """Verify gnome-shell and follow its focus signal. The extension may
        be missing or disabled now; each inspection asks again.

        A desktop that lists GNOME but runs no gnome-shell (Budgie names
        itself Budgie:GNOME) has no backend: after `wait` seconds without an
        owner of org.gnome.Shell the observer stops for good (exit 78).
        """
        deadline = time.monotonic() + wait
        while True:
            try:
                self.shell = verified_owner(self.bus, SHELL_NAME, SHELL_PROCESS, timeout, self.proc)
                break
            except OwnerError as error:
                if str(error) != "absent":
                    raise ShellUnavailable(f"shell_{error}") from None
                if time.monotonic() >= deadline:
                    raise NoDesktopBackend("gnome_shell_absent") from None
                sleep(.25)
        self.bus.watch_owner(SHELL_NAME, self.owner_changed)
        self.bus.subscribe(self.shell, FOCUS_PATH, FOCUS_INTERFACE, "FocusChanged", lambda _arguments: self.on_change())

    def stop(self):
        """Nothing to undo: the extension is the user's to enable and disable."""

    def owner_changed(self, owner):
        """gnome-shell restarted or left the bus: stop trusting its answers."""
        if owner != self.shell:
            with self.lock:
                if self.lost:
                    return
                self.lost = True
            self.on_lost()

    def lock_query(self, remaining):
        return GnomeLockQuery(self, remaining)

    def active_window(self, remaining):
        """The focused window as gnome-shell answers it now."""
        return parse_focus(self.ask("GetFocus", FOCUS_PATH, FOCUS_INTERFACE, FOCUS_REPLY, remaining))

    def require_unlocked(self, remaining):
        """gnome-shell's own screen shield must answer an explicit false."""
        active = self.ask("GetActive", "/org/gnome/ScreenSaver", "org.gnome.ScreenSaver", "(b)", remaining)
        if not isinstance(active, tuple) or len(active) != 1 or type(active[0]) is not bool:
            raise Denied("desktop_unavailable")
        if active[0]:
            raise Denied("desktop_locked")

    def ask(self, method, path, interface, reply, remaining):
        if self.lost or self.shell is None:
            raise Denied("desktop_unavailable")
        try:
            return self.bus.call(self.shell, path, interface, method, None, None, reply, remaining(IPC_SECONDS))
        except Denied:
            raise
        except Exception:
            raise Denied("desktop_unavailable") from None

class GnomeLockQuery:
    """The lock answer, asked after the field was read, as on Hyprland."""

    def __init__(self, focus, remaining):
        self.focus, self.remaining = focus, remaining

    def __enter__(self):
        return self

    def __exit__(self, *_exception):
        self.focus.require_unlocked(self.remaining)
