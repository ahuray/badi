#!/usr/bin/env python3
"""Badi desktop session helper.

Tells the broker, without any text, whether the login session is in the
foreground or the machine is going to sleep (logind), and whether it runs on
battery or in the power-saver profile (UPower, power-profiles-daemon). It also
offers pause, snooze and settings shortcuts through the GlobalShortcuts portal.

It never reads a field, opens a suggestion session or edits anything. A
report can only withhold authority: an inactive or sleeping session holds
every grant, and the broker drops this helper's report when it disconnects.
"""
from __future__ import annotations

import os
import re
from pathlib import Path
import shutil
import signal
import sys

import gi

gi.require_version("Gio", "2.0")
gi.require_version("GLib", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

try:
    gi.require_version("GLibUnix", "2.0")
    from gi.repository import GLibUnix  # noqa: E402
    # PyGObject 3.52+ names the full-argument form signal_add.
    add_signal = getattr(GLibUnix, "signal_add", None) or GLibUnix.signal_add_full
    add_fd = GLibUnix.fd_add_full
except (ImportError, AttributeError, ValueError):
    # Older PyGObject (the pinned CI Arch snapshot) predates the GLibUnix override.
    add_signal, add_fd = GLib.unix_signal_add_full, GLib.unix_fd_add_full

sys.path.insert(0, str(Path(__file__).resolve().parent))
from link import Link, LinkError  # noqa: E402

try:
    import style_key  # noqa: E402
except (ImportError, ValueError):
    # Without libsecret's introspection data there is no style key to hand over.
    style_key = None

APP_ID = "io.github.ahuray.badi"
LOGIND = "org.freedesktop.login1"
LOGIND_PATH = "/org/freedesktop/login1"
UPOWER = "org.freedesktop.UPower"
PROFILES = (("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
            ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"))
PORTAL = "org.freedesktop.portal.Desktop"
PORTAL_PATH = "/org/freedesktop/portal/desktop"
SHORTCUTS_IFACE = "org.freedesktop.portal.GlobalShortcuts"
SHORTCUTS = (
    ("pause-toggle", "Pause or resume Badi suggestions"),
    ("snooze", "Snooze Badi in the focused app or site for 10 minutes"),
    ("open-settings", "Open Badi settings"),
)
# The controller is installed beside this directory; in the checkout it is
# scripts/badi-desktop.py.
INSTALLED_CONTROLLER = Path(__file__).resolve().parent.parent / "badi-desktop.py"
CHECKOUT_CONTROLLER = Path(__file__).resolve().parents[2] / "scripts/badi-desktop.py"
ACTIONS = {
    "pause-toggle": ("pause", "toggle"),
    "snooze": ("snooze",),
    "open-settings": ("settings",),
}
BACKOFF_SECONDS = (1, 2, 5, 10, 30)
DBUS_TIMEOUT_MS = 2000
# While the keyring is locked, look again this often; never prompt.
KEY_RETRY_SECONDS = 120


def log(message):
    """Fixed, content-free diagnostics for the user journal."""
    print(f"badi-session: {message}", file=sys.stderr, flush=True)


def first_line(text, limit=200):
    """The first line of a controller message, bounded for a notification."""
    return ((text or "").strip().splitlines() or [""])[0][:limit]


def controller_path(environment=os.environ):
    configured = environment.get("BADI_SESSION_CONTROLLER")
    if configured:
        return Path(configured)
    return INSTALLED_CONTROLLER if INSTALLED_CONTROLLER.is_file() else CHECKOUT_CONTROLLER


def bus_for(kind, environment=os.environ):
    """The system or session bus; tests point both at a private daemon."""
    override = environment.get("BADI_SESSION_SYSTEM_BUS_ADDRESS" if kind == "system" else "DBUS_SESSION_BUS_ADDRESS")
    if kind == "system" and not override:
        return Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
    address = override or Gio.dbus_address_get_for_bus_sync(Gio.BusType.SESSION, None)
    # A private connection: the portal's host registration must be the first
    # portal call this connection makes.
    return Gio.DBusConnection.new_for_address_sync(
        address,
        Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
        None, None)


def read_property(bus, name, path, interface, prop, kind):
    """One property, read fresh; ``None`` when the service or value is absent."""
    try:
        value = bus.call_sync(
            name, path, "org.freedesktop.DBus.Properties", "Get", GLib.Variant("(ss)", (interface, prop)),
            GLib.VariantType("(v)"), Gio.DBusCallFlags.NO_AUTO_START, DBUS_TIMEOUT_MS, None).unpack()[0]
    except GLib.Error:
        return None
    return value if type(value) is kind else None


class Watched:
    """Calls ``changed`` when a service appears or vanishes, or when one
    interface on one object reports a property change."""

    def __init__(self, bus, name, path, interface, changed):
        self.bus = bus
        self._changes = bus.signal_subscribe(
            name, "org.freedesktop.DBus.Properties", "PropertiesChanged", path, interface,
            Gio.DBusSignalFlags.NONE, lambda *_arguments: changed())
        self._owner = Gio.bus_watch_name_on_connection(
            bus, name, Gio.BusNameWatcherFlags.NONE, lambda *_arguments: changed(),
            lambda *_arguments: changed())

    def close(self):
        self.bus.signal_unsubscribe(self._changes)
        Gio.bus_unwatch_name(self._owner)


class Reporter:
    """Keeps the broker told about the current state, reconnecting with backoff."""

    def __init__(self, link, lookup_key=None):
        self.link = link
        self.lookup_key = lookup_key
        self._key_retry = None
        self.session_active = True
        self.sleeping = False
        self.on_battery = False
        self.power_saver = False
        self.acknowledged = None
        self._watch = None
        self._retry = None
        self._attempt = 0
        self.problem = None

    def state(self):
        session = "sleeping" if self.sleeping else "active" if self.session_active else "inactive"
        return session, self.on_battery, self.power_saver

    def publish(self):
        """Sends the current state unless the broker already has it; returns
        whether the broker acknowledged it."""
        state = self.state()
        if self.link.connected and self.acknowledged == state:
            return True
        try:
            if not self.link.connected:
                self.link.connect()
                self._watch_link()
                self._deliver_key()
            status = self.link.send_report(*state)
        except LinkError as error:
            self._lost(str(error))
            return False
        if self.acknowledged != state:
            log(f"reported session={state[0]} on_battery={str(state[1]).lower()} "
                f"power_saver={str(state[2]).lower()}; automatic={status['automatic']}")
        self.acknowledged = state
        self._attempt = 0
        self.problem = None
        return True

    def _deliver_key(self):
        """Hands the keyring's style key to a newly connected broker, which
        keeps it in memory only. A locked keyring is retried later."""
        if self.lookup_key is None:
            return
        if self._key_retry is not None:
            GLib.source_remove(self._key_retry)
            self._key_retry = None
        try:
            key = self.lookup_key()
        except Exception as error:  # style_key.KeyringError, or a missing service
            reason = str(error) if str(error).startswith("keyring_") else "keyring_unavailable"
            log(f"style key not delivered ({reason}); retrying in {KEY_RETRY_SECONDS} s")
            self._key_retry = GLib.timeout_add_seconds(KEY_RETRY_SECONDS, self._retry_key)
            return
        if key is None:
            return
        state = self.link.send_style_key(*key)
        log(f"style key delivered; broker key state {state}")

    def _retry_key(self):
        self._key_retry = None
        if self.link.connected:
            try:
                self._deliver_key()
            except LinkError as error:
                self._lost(str(error))
        return GLib.SOURCE_REMOVE

    def close(self):
        self._unwatch()
        for source in (self._retry, self._key_retry):
            if source is not None:
                GLib.source_remove(source)
        self._retry = self._key_retry = None
        self.link.close()

    def _watch_link(self):
        self._unwatch()
        self._watch = add_fd(
            GLib.PRIORITY_DEFAULT, self.link.fileno(),
            GLib.IOCondition.IN | GLib.IOCondition.HUP | GLib.IOCondition.ERR, self._readable)

    def _unwatch(self):
        if self._watch is not None:
            GLib.source_remove(self._watch)
            self._watch = None

    def _readable(self, _fd, _condition):
        if not self.link.closed_by_peer():
            return GLib.SOURCE_CONTINUE
        self._watch = None
        self._lost("broker_disconnected")
        return GLib.SOURCE_REMOVE

    def _lost(self, reason):
        self._unwatch()
        self.link.close()
        self.acknowledged = None
        if reason != self.problem:
            log(f"broker unavailable ({reason}); retrying")
            self.problem = reason
        if self._retry is None:
            delay = BACKOFF_SECONDS[min(self._attempt, len(BACKOFF_SECONDS) - 1)]
            self._attempt += 1
            self._retry = GLib.timeout_add_seconds(delay, self._retry_now)

    def _retry_now(self):
        self._retry = None
        self.publish()
        return GLib.SOURCE_REMOVE


class Logind:
    """The display session's foreground state and sleep preparation.

    A delay inhibitor makes logind wait (at most its InhibitDelayMaxSec) until
    the broker has withdrawn every grant; the lock is released right after
    that report, or after its bounded failure. ``LockedHint`` is never used:
    it is a hint, not proof of unlock, and the observer keeps its own checks.
    """

    def __init__(self, bus, reporter):
        self.bus = bus
        self.reporter = reporter
        self.inhibitor = None
        self.session_path = None
        self._session_watch = None
        self._sleep = bus.signal_subscribe(
            LOGIND, f"{LOGIND}.Manager", "PrepareForSleep", LOGIND_PATH, None,
            Gio.DBusSignalFlags.NONE, self._prepare_for_sleep)
        self._owner = Gio.bus_watch_name_on_connection(
            bus, LOGIND, Gio.BusNameWatcherFlags.NONE, self._appeared, self._vanished)

    def _appeared(self, *_arguments):
        display = read_property(self.bus, LOGIND, f"{LOGIND_PATH}/user/self", f"{LOGIND}.User", "Display", tuple)
        path = display[1] if display is not None and len(display) == 2 else "/"
        if path == "/":
            log("logind reports no display session; reporting an active session")
        self._follow_session(path)
        self.take_inhibitor()

    def _vanished(self, *_arguments):
        # A restarted logind forgets every inhibitor; take one when it returns.
        self.release_inhibitor()
        self._follow_session("/")

    def _follow_session(self, path):
        if self._session_watch is not None:
            self._session_watch.close()
            self._session_watch = None
        self.session_path = path if path and path != "/" else None
        if self.session_path is not None:
            self._session_watch = Watched(self.bus, LOGIND, self.session_path, f"{LOGIND}.Session",
                                          self._session_changed)
        self._session_changed()

    def _session_changed(self):
        active = None
        if self.session_path is not None:
            active = read_property(self.bus, LOGIND, self.session_path, f"{LOGIND}.Session", "Active", bool)
        self.reporter.session_active = True if active is None else active
        self.reporter.publish()

    def _prepare_for_sleep(self, _bus, _sender, _path, _interface, _signal, parameters):
        (starting,) = parameters.unpack()
        self.reporter.sleeping = bool(starting)
        # Withdraw before letting sleep proceed; never hold it past the bounded report.
        self.reporter.publish()
        if starting:
            self.release_inhibitor()
        else:
            self.take_inhibitor()

    def take_inhibitor(self):
        if self.inhibitor is not None:
            return
        try:
            reply, fds = self.bus.call_with_unix_fd_list_sync(
                LOGIND, LOGIND_PATH, f"{LOGIND}.Manager", "Inhibit",
                GLib.Variant("(ssss)", ("sleep", "Badi", "Withdraw writing suggestions before sleep", "delay")),
                GLib.VariantType("(h)"), Gio.DBusCallFlags.NO_AUTO_START, DBUS_TIMEOUT_MS, None, None)
            self.inhibitor = fds.get(reply.unpack()[0])
        except GLib.Error:
            log("sleep inhibitor unavailable; grants are still revoked when sleep is announced")

    def release_inhibitor(self):
        if self.inhibitor is not None:
            os.close(self.inhibitor)
            self.inhibitor = None

    def close(self):
        self.bus.signal_unsubscribe(self._sleep)
        Gio.bus_unwatch_name(self._owner)
        if self._session_watch is not None:
            self._session_watch.close()
        self.release_inhibitor()


class Power:
    """Battery power and the power-saver profile, read only. Badi never
    changes the system power profile."""

    def __init__(self, bus, reporter):
        self.bus = bus
        self.reporter = reporter
        self._watches = [Watched(bus, UPOWER, "/org/freedesktop/UPower", UPOWER, self.update)]
        self._watches += [Watched(bus, name, path, name, self.update) for name, path in PROFILES]
        self.update()

    def update(self):
        self.reporter.on_battery = read_property(
            self.bus, UPOWER, "/org/freedesktop/UPower", UPOWER, "OnBattery", bool) is True
        self.reporter.power_saver = any(
            read_property(self.bus, name, path, name, "ActiveProfile", str) == "power-saver"
            for name, path in PROFILES)
        self.reporter.publish()

    def close(self):
        for watch in self._watches:
            watch.close()


# The OSD icon for a snooze: Material Design's "sleep" in Omarchy's Nerd Font.
SNOOZE_GLYPH = "\U000f04b2"


def snooze_message(line):
    """The on-screen display's short form of the controller's snooze line:
    the display shows about twenty characters before it cuts the text."""
    match = re.match(r"Badi is snoozed in (.+) for \d+ minutes?\b", line or "")
    return f"Snoozed in {match.group(1)}" if match else "Badi snoozed"


class Shortcuts:
    """Registers pause, snooze and settings shortcuts with the GlobalShortcuts
    portal.

    The compositor decides the keys (Hyprland: ``hl.dsp.global``). A shortcut
    runs the same controller command as the terminal and grants no access to
    any application or field; a snooze only withholds, in the app or site the
    broker already follows.
    """

    def __init__(self, bus, controller, notify=True):
        self.bus = bus
        self.controller = controller
        self.notify = notify and shutil.which("notify-send") is not None
        # Omarchy 4.5 shows brief confirmations on its own on-screen display,
        # as it does for volume or the touchpad; elsewhere they stay toasts.
        self.osd = notify and shutil.which("omarchy-osd") is not None
        self.session_handle = None
        self.version = None
        self.bound = ()
        self._running = set()
        self._subscriptions = []
        self._serial = 0
        self._activated = bus.signal_subscribe(
            PORTAL, SHORTCUTS_IFACE, "Activated", PORTAL_PATH, None,
            Gio.DBusSignalFlags.NONE, self._on_activated)
        self._owner = Gio.bus_watch_name_on_connection(
            bus, PORTAL, Gio.BusNameWatcherFlags.NONE, self._appeared, self._vanished)

    def _appeared(self, *_arguments):
        self._register_host()
        try:
            version = self.bus.call_sync(
                PORTAL, PORTAL_PATH, "org.freedesktop.DBus.Properties", "Get",
                GLib.Variant("(ss)", (SHORTCUTS_IFACE, "version")), GLib.VariantType("(v)"),
                Gio.DBusCallFlags.NONE, DBUS_TIMEOUT_MS, None).unpack()[0]
        except GLib.Error:
            log("the desktop portal offers no global shortcuts")
            return
        # Version 1 has no ConfigureShortcuts; binding works the same in both.
        self.version = version
        self._request("CreateSession", lambda token: GLib.Variant("(a{sv})", ({
            "handle_token": GLib.Variant("s", token),
            "session_handle_token": GLib.Variant("s", f"badi_shortcuts_{os.getpid()}_{self._serial}"),
        },)), self._created)

    def _vanished(self, *_arguments):
        self.session_handle = None
        self.bound = ()

    def _register_host(self):
        try:
            self.bus.call_sync(
                PORTAL, PORTAL_PATH, "org.freedesktop.host.portal.Registry", "Register",
                GLib.Variant("(sa{sv})", (APP_ID, {})), None, Gio.DBusCallFlags.NONE, DBUS_TIMEOUT_MS, None)
        except GLib.Error:
            # Older portals: shortcuts register without an application id.
            pass

    def _request(self, method, arguments, done):
        self._serial += 1
        token = f"badi_{os.getpid()}_{self._serial}"
        sender = self.bus.get_unique_name().lstrip(":").replace(".", "_")
        path = f"{PORTAL_PATH}/request/{sender}/{token}"
        subscription = None

        def responded(_bus, _sender, _path, _interface, _signal, parameters):
            self.bus.signal_unsubscribe(subscription)
            self._subscriptions.remove(subscription)
            response, results = parameters.unpack()
            done(response, results)

        subscription = self.bus.signal_subscribe(
            PORTAL, "org.freedesktop.portal.Request", "Response", path, None,
            Gio.DBusSignalFlags.NONE, responded)
        self._subscriptions.append(subscription)
        try:
            self.bus.call_sync(PORTAL, PORTAL_PATH, SHORTCUTS_IFACE, method, arguments(token),
                               GLib.VariantType("(o)"), Gio.DBusCallFlags.NONE, DBUS_TIMEOUT_MS, None)
        except GLib.Error:
            self.bus.signal_unsubscribe(subscription)
            self._subscriptions.remove(subscription)
            log(f"global shortcuts unavailable ({method} failed)")

    def _created(self, response, results):
        if response != 0 or "session_handle" not in results:
            log("global shortcut session refused")
            return
        self.session_handle = results["session_handle"]
        shortcuts = [(shortcut_id, {"description": GLib.Variant("s", description)})
                     for shortcut_id, description in SHORTCUTS]
        self._request("BindShortcuts", lambda token: GLib.Variant("(oa(sa{sv})sa{sv})", (
            self.session_handle, shortcuts, "", {"handle_token": GLib.Variant("s", token)})), self._bound)

    def _bound(self, response, results):
        if response != 0:
            log("global shortcuts were not bound")
            return
        self.bound = tuple(sorted(shortcut for shortcut, _properties in results.get("shortcuts", [])))
        log(f"global shortcuts registered: {', '.join(self.bound) or 'none'}")

    def _on_activated(self, _bus, _sender, _path, _interface, _signal, parameters):
        session_handle, shortcut_id, _timestamp, _options = parameters.unpack()
        if session_handle != self.session_handle or shortcut_id not in ACTIONS:
            return
        if shortcut_id in self._running:
            return
        self.run(shortcut_id)

    def run(self, shortcut_id):
        self._running.add(shortcut_id)
        try:
            process = Gio.Subprocess.new(
                [sys.executable, "-B", str(self.controller), *ACTIONS[shortcut_id]],
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE)
        except GLib.Error:
            self._running.discard(shortcut_id)
            log(f"{shortcut_id} could not start the controller")
            return
        process.communicate_utf8_async(None, None, self._finished, shortcut_id)

    def _finished(self, process, result, shortcut_id):
        self._running.discard(shortcut_id)
        try:
            _ok, output, error = process.communicate_utf8_finish(result)
        except GLib.Error:
            output = error = ""
        if shortcut_id == "snooze":
            # The controller's own line names the app or site, or why nothing
            # was snoozed (no focused field Badi follows); never any text.
            if not process.get_successful():
                log("snooze refused or failed")
            line = first_line(output if process.get_successful() else error)
            if process.get_successful():
                self._confirm(SNOOZE_GLYPH, snooze_message(line), "Badi snoozed",
                              line or "Run badi doctor in a terminal.")
            else:
                self._notification("Badi did not snooze", line or "Run badi doctor in a terminal.")
            return
        if not process.get_successful():
            log(f"{shortcut_id} failed")
            self._notification("Badi could not change pause", "Run badi doctor in a terminal.")
            return
        if shortcut_id == "pause-toggle":
            paused = (output or "").startswith("Predictions paused")
            self._confirm("media-pause" if paused else "media-play", "Badi paused" if paused else "Badi resumed",
                          "Badi paused" if paused else "Badi resumed",
                          "Suggestions stay off until you resume." if paused else "Suggestions are on again.")

    def _confirm(self, icon, message, summary, body):
        """A brief confirmation: Omarchy's on-screen display where it runs,
        otherwise a toast with `summary` and `body`."""
        if not (self.osd and self._osd(icon, message)):
            self._notification(summary, body)

    def _osd(self, icon, message):
        try:
            Gio.Subprocess.new(["omarchy-osd", "-i", icon, "-m", message],
                               Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE)
        except GLib.Error:
            return False
        return True

    def _notification(self, summary, body):
        if not self.notify:
            return
        try:
            Gio.Subprocess.new(["notify-send", "--app-name=Badi", f"--icon={APP_ID}", summary, body],
                               Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE)
        except GLib.Error:
            pass

    def close(self):
        Gio.bus_unwatch_name(self._owner)
        self.bus.signal_unsubscribe(self._activated)
        for subscription in self._subscriptions:
            self.bus.signal_unsubscribe(subscription)
        self._subscriptions.clear()
        if self.session_handle is not None:
            try:
                self.bus.call_sync(PORTAL, self.session_handle, "org.freedesktop.portal.Session", "Close",
                                   None, None, Gio.DBusCallFlags.NONE, DBUS_TIMEOUT_MS, None)
            except GLib.Error:
                pass
            self.session_handle = None


def main(arguments):
    shortcuts_enabled = "--no-shortcuts" not in arguments
    unknown = [argument for argument in arguments if argument != "--no-shortcuts"]
    if unknown:
        print("usage: daemon.py [--no-shortcuts]", file=sys.stderr)
        return 2
    loop = GLib.MainLoop()
    reporter = Reporter(Link(), style_key.lookup if style_key is not None else None)
    parts = []
    try:
        system = bus_for("system")
        parts.append(Logind(system, reporter))
        parts.append(Power(system, reporter))
    except GLib.Error:
        log("system bus unavailable; reporting an active session on mains power")
        reporter.publish()
    if shortcuts_enabled:
        try:
            parts.append(Shortcuts(bus_for("session"), controller_path(),
                                   notify=os.environ.get("BADI_SESSION_NOTIFICATIONS") != "off"))
        except GLib.Error:
            log("session bus unavailable; no global shortcuts")

    def stop(*_arguments):
        loop.quit()
        return GLib.SOURCE_REMOVE

    for signum in (signal.SIGTERM, signal.SIGINT):
        add_signal(GLib.PRIORITY_HIGH, signum, stop)
    try:
        loop.run()
    finally:
        for part in reversed(parts):
            part.close()
        reporter.close()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
