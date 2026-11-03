#!/usr/bin/env python3
"""Prove the KWin backend (ADR 0015) against a real headless KWin.

Starts `kwin_wayland --virtual` on a private session bus, loads the
observer's KWin script through the real D-Bus path, opens two GTK4 Wayland
windows with known app ids, and checks that each activation reports that
window's process and app id, that the identity rule accepts only the active
one, that the lock answer denies, and that an unloaded script denies.

It needs kwin_wayland 6, dbus-run-session, python3-gi with GTK 4 and no
graphical session of its own; see the observer runbook for a container.
Disposable windows only: no text is typed or read.
"""
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
SOCKET = "wayland-badi-headless"
CLIENT = """
import sys
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gtk
app = Gtk.Application(application_id=sys.argv[1])
app.connect("activate", lambda app: Gtk.ApplicationWindow(application=app, title="disposable").present())
app.run([])
"""


def wait(condition, seconds, message):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = condition()
        if value:
            return value
        time.sleep(.05)
    raise AssertionError(message)


def session(root):
    from contract import Denied
    from desktop import App, SourceDesktop
    from kwin import KWinFocus, KWinLockQuery, PLUGIN
    from sessionbus import GioBus

    # A new window without an activation token may be refused focus; this
    # private compositor activates each one so every run checks a report.
    config = root / "config"
    config.mkdir(mode=0o700)
    (config / "kwinrc").write_text("[Windows]\nFocusStealingPreventionLevel=0\n")
    environment = dict(os.environ, WAYLAND_DISPLAY=SOCKET, QT_QPA_PLATFORM="offscreen", GDK_BACKEND="wayland",
                       XDG_CURRENT_DESKTOP="KDE", KDE_SESSION_VERSION="6", LANG="C.UTF-8",
                       XDG_CONFIG_HOME=str(config))
    log = (root / "kwin.log").open("w")
    compositor = subprocess.Popen(["kwin_wayland", "--virtual", "--socket", SOCKET, "--width", "1280",
                                   "--height", "800"], env=environment, stdout=log, stderr=log)
    clients = []
    try:
        runtime = Path(os.environ["XDG_RUNTIME_DIR"])
        wait(lambda: (runtime / SOCKET).exists(), 20, "kwin_wayland did not open its Wayland socket")
        bus = GioBus()
        wait(lambda: name_owner(bus, "org.kde.KWin"), 20, "KWin did not claim org.kde.KWin")
        changes = []
        focus = KWinFocus(bus, runtime / "kwin-field-observer.js", on_change=lambda: changes.append(True))
        focus.start()
        print(f"script loaded; KWin is {focus.kwin}, reports go to {bus.unique_name}")
        remaining = lambda limit: limit
        python = os.path.realpath(sys.executable)
        apps = {"one": App(frozenset({python}), frozenset({"org.badi.HeadlessOne"})),
                "two": App(frozenset({python}), frozenset({"org.badi.HeadlessTwo"}))}
        desktop = SourceDesktop(apps, remaining, focus)
        for name, app_id in (("one", "org.badi.HeadlessOne"), ("two", "org.badi.HeadlessTwo")):
            client = subprocess.Popen([sys.executable, "-c", CLIENT, app_id], env=environment,
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            clients.append(client)
            reported = wait(lambda: active(focus, remaining, client.pid), 20, f"no activation report for {app_id}")
            assert reported["class"] == app_id, reported
            assert desktop.window(desktop.app(name))["pid"] == client.pid
            other = "two" if name == "one" else "one"
            denied(lambda: desktop.window(desktop.app(other)), "app_mismatch", Denied)
            print(f"active window: pid {client.pid}, app id {reported['class']} (identity rule {name!r} accepted)")
        with KWinLockQuery(focus, remaining):
            pass
        print("lock answer: unlocked (explicit false from KWin's own /ScreenSaver)")
        bus.call(focus.kwin, "/ScreenSaver", "org.freedesktop.ScreenSaver", "Lock", None, None, None, 2)
        wait(lambda: bus.call(focus.kwin, "/ScreenSaver", "org.freedesktop.ScreenSaver", "GetActive", None, None,
                              "(b)", 1) == (True,), 10, "the session did not lock")

        def locked_read():
            with KWinLockQuery(focus, remaining):
                desktop.window(desktop.app("two"))
        denied(locked_read, "desktop_locked", Denied)
        print("locked session: denied as desktop_locked")
        bus.call(focus.kwin, "/Scripting", "org.kde.kwin.Scripting", "unloadScript", "(s)", (PLUGIN,), "(b)", 2)
        denied(lambda: desktop.window(desktop.app("two")), "desktop_unavailable", Denied)
        print("unloaded script: denied as desktop_unavailable")
        print(f"focus changes seen: {len(changes)}")
        bus.close()
        return 0
    finally:
        for client in clients:
            client.terminate()
            client.wait(5)
        compositor.terminate()
        try:
            compositor.wait(10)
        except subprocess.TimeoutExpired:
            compositor.kill()
        log.close()


def name_owner(bus, name):
    try:
        return bus.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetNameOwner",
                        "(s)", (name,), "(s)", 1)[0]
    except Exception:
        return None


def active(focus, remaining, pid):
    window = focus.active_window(remaining) if focus.window else None
    return window if window and window["pid"] == pid else None


def denied(action, expected, error_type):
    try:
        action()
    except error_type as error:
        assert str(error) == expected, (str(error), expected)
        return
    raise AssertionError(f"expected {expected}")


def main():
    if sys.argv[1:] == ["--session"]:
        return session(Path(os.environ["BADI_KWIN_TEST_ROOT"]))
    if sys.argv[1:]:
        raise SystemExit("This runner takes no arguments")
    for command in ("kwin_wayland", "dbus-run-session"):
        if not shutil.which(command):
            raise SystemExit(f"Install {command} before running the headless KWin lane")
    with tempfile.TemporaryDirectory(prefix="badi-kwin-") as temporary:
        root = Path(temporary)
        runtime = root / "runtime"
        runtime.mkdir(mode=0o700)
        environment = {key: value for key, value in os.environ.items()
                       if key not in ("WAYLAND_DISPLAY", "DISPLAY", "DBUS_SESSION_BUS_ADDRESS",
                                      "HYPRLAND_INSTANCE_SIGNATURE", "AT_SPI_BUS_ADDRESS")}
        environment.update(XDG_RUNTIME_DIR=str(runtime), BADI_KWIN_TEST_ROOT=str(root), PYTHONDONTWRITEBYTECODE="1")
        child = subprocess.Popen(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                                  "--session"], env=environment, start_new_session=True)
        try:
            status = child.wait(timeout=120)
        finally:
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        if status:
            log = root / "kwin.log"
            if log.exists():
                print(log.read_text()[-2000:], file=sys.stderr)
        return status


if __name__ == "__main__":
    raise SystemExit(main())
