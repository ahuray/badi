#!/usr/bin/env python3
"""Prove the GNOME backend (ADR 0016) against a real headless GNOME Shell.

Starts a private system bus with a minimal stand-in for GDM and logind (GNOME
Shell builds its screen shield only when both answer), a private session bus,
and `gnome-shell --headless` with Badi's extension enabled in a throwaway
profile. Then it opens two GTK 4 Wayland windows with known app ids and checks
that the observer's GNOME backend reports each focused window's process and
app id, that the identity rule accepts only the focused one, that a locked
session denies (the extension is disabled and the shield answers true), that
logind's Unlock brings the answers back, and that a disabled extension denies.

Needs gnome-shell 46 or later, dbus-daemon, python3-gi with GTK 4, and
/run/systemd/seats (GNOME Shell's test for logind; a systemd host has it, a
container needs it created). Disposable windows only: nothing is typed or read.
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
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE.parent))
UUID = "badi-focus@ahuray.github.io"
DISPLAY_NAME = "wayland-badi-gnome"
SESSION_ID = "badi"
CLIENT = """
import sys
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gtk
app = Gtk.Application(application_id=sys.argv[1])
app.connect("activate", lambda app: Gtk.ApplicationWindow(application=app, title="disposable").present())
app.run([])
"""
# Test only, never shipped: GNOME starts in the Activities overview, which
# holds focus until a user leaves it; this headless desktop has no user.
NO_OVERVIEW_UUID = "badi-test-no-overview@localhost"
NO_OVERVIEW = """import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
export default class extends Extension {
    enable() {
        if (Main.layoutManager._startingUp)
            this._id = Main.layoutManager.connect('startup-complete', () => Main.overview.hide());
        else
            Main.overview.hide();
    }
    disable() {
        if (this._id)
            Main.layoutManager.disconnect(this._id);
    }
}
"""
NO_OVERVIEW_METADATA = ('{"uuid": "badi-test-no-overview@localhost", "name": "Test: leave the overview", '
                        '"description": "Headless test only", "shell-version": ["46", "47", "48", "49", "50"]}')
SYSTEM_BUS = ("<busconfig><type>system</type><listen>unix:path={socket}</listen><auth>EXTERNAL</auth>"
              "<policy context='default'><allow send_destination='*'/><allow receive_sender='*'/>"
              "<allow own='*'/><allow user='*'/></policy></busconfig>")
FAKE_XML = """<node>
<interface name="org.gnome.DisplayManager.Manager">
  <method name="RegisterSession"><arg type="a{sv}" direction="in"/></method>
  <property name="Version" type="s" access="read"/>
</interface>
<interface name="org.freedesktop.login1.Manager">
  <method name="GetSession"><arg type="s" direction="in"/><arg type="o" direction="out"/></method>
  <method name="GetSessionByPID"><arg type="u" direction="in"/><arg type="o" direction="out"/></method>
  <method name="GetUser"><arg type="u" direction="in"/><arg type="o" direction="out"/></method>
  <method name="ListSessions"><arg type="a(susso)" direction="out"/></method>
  <method name="CanSuspend"><arg type="s" direction="out"/></method>
  <method name="CanRebootToBootLoaderMenu"><arg type="s" direction="out"/></method>
  <method name="Inhibit"><arg type="s" direction="in"/><arg type="s" direction="in"/>
    <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="h" direction="out"/></method>
  <signal name="PrepareForSleep"><arg type="b"/></signal>
  <signal name="SessionRemoved"><arg type="s"/><arg type="o"/></signal>
</interface>
<interface name="org.freedesktop.login1.Session">
  <method name="SetLockedHint"><arg type="b" direction="in"/></method>
  <method name="Activate"/>
  <property name="Id" type="s" access="read"/>
  <property name="Active" type="b" access="read"/>
  <property name="LockedHint" type="b" access="read"/>
  <property name="Remote" type="b" access="read"/>
  <property name="Class" type="s" access="read"/>
  <property name="Type" type="s" access="read"/>
  <property name="State" type="s" access="read"/>
  <signal name="Lock"/>
  <signal name="Unlock"/>
</interface>
<interface name="org.freedesktop.login1.User">
  <property name="Display" type="(so)" access="read"/>
  <property name="Sessions" type="a(so)" access="read"/>
</interface>
<interface name="org.badi.Test">
  <method name="Unlock"/>
</interface>
</node>"""
SESSION_PATH = "/org/freedesktop/login1/session/" + SESSION_ID


def fake_system():
    """GDM's version and enough of logind for GNOME Shell's screen shield."""
    from gi.repository import Gio, GLib
    node = Gio.DBusNodeInfo.new_for_xml(FAKE_XML)
    bus = Gio.DBusConnection.new_for_address_sync(
        os.environ["DBUS_SYSTEM_BUS_ADDRESS"],
        Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
    uid = os.getuid()
    user_path = f"/org/freedesktop/login1/user/_{uid}"
    properties = {
        "org.gnome.DisplayManager.Manager": {"Version": GLib.Variant("s", "48.0")},
        "org.freedesktop.login1.Session": {
            "Id": GLib.Variant("s", SESSION_ID), "Active": GLib.Variant("b", True),
            "LockedHint": GLib.Variant("b", False), "Remote": GLib.Variant("b", False),
            "Class": GLib.Variant("s", "user"), "Type": GLib.Variant("s", "wayland"),
            "State": GLib.Variant("s", "active")},
        "org.freedesktop.login1.User": {
            "Display": GLib.Variant("(so)", (SESSION_ID, SESSION_PATH)),
            "Sessions": GLib.Variant("a(so)", [(SESSION_ID, SESSION_PATH)])},
    }

    def call(connection, _sender, _path, interface, method, parameters, invocation):
        replies = {"GetSession": GLib.Variant("(o)", (SESSION_PATH,)),
                   "GetSessionByPID": GLib.Variant("(o)", (SESSION_PATH,)),
                   "GetUser": GLib.Variant("(o)", (user_path,)),
                   "ListSessions": GLib.Variant("(a(susso))", ([(SESSION_ID, uid, "tester", "seat0", SESSION_PATH)],)),
                   "CanSuspend": GLib.Variant("(s)", ("no",)),
                   "CanRebootToBootLoaderMenu": GLib.Variant("(s)", ("no",))}
        if method == "Unlock":
            connection.emit_signal(None, SESSION_PATH, "org.freedesktop.login1.Session", "Unlock", None)
            invocation.return_value(None)
        elif method == "SetLockedHint":
            properties["org.freedesktop.login1.Session"]["LockedHint"] = GLib.Variant("b", parameters.unpack()[0])
            invocation.return_value(None)
        elif method in replies:
            invocation.return_value(replies[method])
        elif method in ("RegisterSession", "Activate"):
            invocation.return_value(None)
        else:
            invocation.return_dbus_error("org.freedesktop.DBus.Error.NotSupported", "not in this test")

    def get(_connection, _sender, _path, interface, name):
        return properties[interface][name]

    objects = (("/org/gnome/DisplayManager/Manager", "org.gnome.DisplayManager.Manager"),
               ("/org/freedesktop/login1", "org.freedesktop.login1.Manager"),
               (SESSION_PATH, "org.freedesktop.login1.Session"),
               (user_path, "org.freedesktop.login1.User"),
               ("/org/badi/Test", "org.badi.Test"))
    for path, interface in objects:
        bus.register_object(path, node.lookup_interface(interface), call, get, None)
    for name in ("org.gnome.DisplayManager", "org.freedesktop.login1"):
        bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName",
                      GLib.Variant("(su)", (name, 4)), None, Gio.DBusCallFlags.NONE, 2000, None)
    print("fake system services ready", flush=True)
    GLib.MainLoop().run()


def wait(condition, seconds, message):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = condition()
        if value:
            return value
        time.sleep(.1)
    raise AssertionError(message)


def session(root):
    from contract import Denied
    from desktop import App, SourceDesktop
    from gnome import GnomeFocus
    from sessionbus import GioBus

    data = root / "data"
    extension = data / "gnome-shell/extensions" / UUID
    shutil.copytree(ROOT / "adapters/gnome-shell" / UUID, extension)
    helper = data / "gnome-shell/extensions" / NO_OVERVIEW_UUID
    helper.mkdir(parents=True)
    (helper / "extension.js").write_text(NO_OVERVIEW)
    (helper / "metadata.json").write_text(NO_OVERVIEW_METADATA)
    environment = dict(os.environ, XDG_DATA_HOME=str(data), XDG_CURRENT_DESKTOP="GNOME", XDG_SESSION_TYPE="wayland",
                       XDG_SESSION_ID=SESSION_ID, LANG="C.UTF-8", GSETTINGS_BACKEND="keyfile",
                       XDG_CONFIG_HOME=str(root / "config"))
    subprocess.run(["gsettings", "set", "org.gnome.shell", "enabled-extensions", f"['{UUID}', '{NO_OVERVIEW_UUID}']"],
                   env=environment, check=True)
    log = (root / "shell.log").open("w")
    shell = subprocess.Popen(["gnome-shell", "--headless", "--virtual-monitor", "1280x800", "--wayland-display",
                              DISPLAY_NAME], env=environment, stdout=log, stderr=log)
    clients = []
    try:
        bus = GioBus()

        def ask(path, interface, method, reply):
            try:
                return bus.call("org.gnome.Shell", path, interface, method, None, None, reply, 1)
            except Exception:
                return None

        wait(lambda: ask("/io/github/ahuray/badi/Focus", "io.github.ahuray.badi.Focus1", "GetFocus", "(ubtussb)"),
             40, "the extension never answered GetFocus")
        print("gnome-shell up, extension answering")
        focus = GnomeFocus(bus)
        focus.start()
        print(f"shell verified as {focus.shell}")
        remaining = lambda limit: limit
        python = os.path.realpath(sys.executable)
        apps = {"one": App(frozenset({python}), frozenset({"org.badi.HeadlessOne"})),
                "two": App(frozenset({python}), frozenset({"org.badi.HeadlessTwo"}))}
        desktop = SourceDesktop(apps, remaining, focus)
        client_environment = dict(environment, WAYLAND_DISPLAY=DISPLAY_NAME, GDK_BACKEND="wayland")
        for name, app_id in (("one", "org.badi.HeadlessOne"), ("two", "org.badi.HeadlessTwo")):
            client = subprocess.Popen([sys.executable, "-c", CLIENT, app_id], env=client_environment,
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            clients.append(client)
            reported = wait(lambda: focused(focus, remaining, client.pid), 20, f"{app_id} never had focus")
            assert reported["class"] == app_id, reported
            assert desktop.window(desktop.app(name))["pid"] == client.pid
            other = "two" if name == "one" else "one"
            denied(lambda: desktop.window(desktop.app(other)), "app_mismatch", Denied)
            print(f"focused window: pid {client.pid}, app id {reported['class']} (identity rule {name!r} accepted)")
        with focus.lock_query(remaining):
            pass
        print("lock answer: unlocked (explicit false from gnome-shell's own /org/gnome/ScreenSaver)")
        ask("/org/gnome/ScreenSaver", "org.gnome.ScreenSaver", "Lock", None)
        wait(lambda: ask("/org/gnome/ScreenSaver", "org.gnome.ScreenSaver", "GetActive", "(b)") == (True,), 20,
             "the session did not lock")

        def locked_read():
            with focus.lock_query(remaining):
                desktop.window(desktop.app("two"))
        denied(locked_read, "desktop_locked", Denied)
        # GNOME disables a user-mode extension in the unlock-dialog mode.
        wait(lambda: ask("/io/github/ahuray/badi/Focus", "io.github.ahuray.badi.Focus1", "GetFocus",
                         "(ubtussb)") is None, 20, "the extension kept answering while locked")
        denied(lambda: focus.active_window(remaining), "desktop_unavailable", Denied)
        print("locked session: denied as desktop_locked; the extension stops answering while locked")
        subprocess.run(["gdbus", "call", "--system", "--dest", "org.freedesktop.login1", "--object-path",
                        "/org/badi/Test", "--method", "org.badi.Test.Unlock"], check=True, stdout=subprocess.DEVNULL)
        wait(lambda: ask("/org/gnome/ScreenSaver", "org.gnome.ScreenSaver", "GetActive", "(b)") == (False,), 20,
             "logind's Unlock did not unlock the session")
        wait(lambda: answers(focus, remaining), 20, "the extension did not answer again after unlock")
        with focus.lock_query(remaining):
            desktop.window(desktop.app("two"))
        print("after logind's Unlock: the extension answers and the focused field's window passes again")
        subprocess.run(["gsettings", "set", "org.gnome.shell", "enabled-extensions", f"['{NO_OVERVIEW_UUID}']"],
                       env=environment, check=True)
        wait(lambda: not answers(focus, remaining), 20, "the extension kept answering after it was disabled")
        denied(lambda: desktop.window(desktop.app("two")), "desktop_unavailable", Denied)
        print("disabled extension: denied as desktop_unavailable")
        bus.close()
        return 0
    finally:
        for client in clients:
            client.terminate()
            client.wait(5)
        shell.terminate()
        try:
            shell.wait(10)
        except subprocess.TimeoutExpired:
            shell.kill()
        log.close()


def focused(focus, remaining, pid):
    try:
        window = focus.active_window(remaining)
    except Exception:
        return None
    return window if window["pid"] == pid else None


def answers(focus, remaining):
    try:
        focus.active_window(remaining)
        return True
    except Exception as error:
        return str(error) not in ("desktop_unavailable", "desktop_locked")


def denied(action, expected, error_type):
    expected = (expected,) if isinstance(expected, str) else expected
    try:
        action()
    except error_type as error:
        assert str(error) in expected, (str(error), expected)
        return
    raise AssertionError(f"expected {expected}")


def main():
    if sys.argv[1:] == ["--fake-system"]:
        return fake_system()
    if sys.argv[1:] == ["--session"]:
        return session(Path(os.environ["BADI_GNOME_TEST_ROOT"]))
    if sys.argv[1:]:
        raise SystemExit("This runner takes no arguments")
    for command in ("gnome-shell", "dbus-daemon", "dbus-run-session", "gsettings", "gdbus"):
        if not shutil.which(command):
            raise SystemExit(f"Install {command} before running the headless GNOME lane")
    if not os.path.isdir("/run/systemd/seats"):
        raise SystemExit("GNOME Shell builds its screen shield only where /run/systemd/seats exists; create it "
                         "(a container lacks it) before running this lane")
    with tempfile.TemporaryDirectory(prefix="badi-gnome-") as temporary:
        root = Path(temporary)
        runtime = root / "runtime"
        runtime.mkdir(mode=0o700)
        config = root / "system-bus.conf"
        socket = root / "system_bus_socket"
        config.write_text(SYSTEM_BUS.format(socket=socket))
        environment = {key: value for key, value in os.environ.items()
                       if key not in ("WAYLAND_DISPLAY", "DISPLAY", "DBUS_SESSION_BUS_ADDRESS",
                                      "HYPRLAND_INSTANCE_SIGNATURE", "AT_SPI_BUS_ADDRESS")}
        environment.update(XDG_RUNTIME_DIR=str(runtime), BADI_GNOME_TEST_ROOT=str(root), PYTHONDONTWRITEBYTECODE="1",
                           HOME=str(root), DBUS_SYSTEM_BUS_ADDRESS=f"unix:path={socket}")
        processes = []
        try:
            processes.append(subprocess.Popen(["dbus-daemon", f"--config-file={config}", "--nofork"],
                                              env=environment, start_new_session=True))
            wait(socket.exists, 10, "the private system bus did not start")
            fake = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--fake-system"],
                                    env=environment, stdout=subprocess.PIPE, text=True, start_new_session=True)
            processes.append(fake)
            if fake.stdout.readline().strip() != "fake system services ready":
                raise SystemExit("the GDM and logind stand-ins did not start")
            child = subprocess.Popen(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                                      "--session"], env=environment, start_new_session=True)
            processes.append(child)
            status = child.wait(timeout=180)
            if status and (root / "shell.log").exists():
                print((root / "shell.log").read_text()[-3000:], file=sys.stderr)
            return status
        finally:
            for process in reversed(processes):
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass


if __name__ == "__main__":
    raise SystemExit(main())
