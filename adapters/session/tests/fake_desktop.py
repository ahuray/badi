#!/usr/bin/env python3
"""Fake logind, UPower, power-profiles-daemon and desktop portal for tests.

Runs on a private bus given by DBUS_SESSION_BUS_ADDRESS. The test drives it
through ``org.badi.Test`` and reads what the helper did from ``State``.
"""
import os
from pathlib import Path
import sys
import warnings

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from daemon import add_fd, add_signal  # noqa: E402

SESSION_PATH = "/org/freedesktop/login1/session/_31"
XML = f"""
<node>
  <interface name="org.freedesktop.login1.Manager">
    <method name="Inhibit">
      <arg type="s" direction="in"/><arg type="s" direction="in"/>
      <arg type="s" direction="in"/><arg type="s" direction="in"/>
      <arg type="h" direction="out"/>
    </method>
    <signal name="PrepareForSleep"><arg type="b"/></signal>
  </interface>
  <interface name="org.freedesktop.login1.User">
    <property name="Display" type="(so)" access="read"/>
  </interface>
  <interface name="org.freedesktop.login1.Session">
    <property name="Active" type="b" access="read"/>
  </interface>
  <interface name="org.freedesktop.UPower">
    <property name="OnBattery" type="b" access="read"/>
  </interface>
  <interface name="org.freedesktop.UPower.PowerProfiles">
    <property name="ActiveProfile" type="s" access="read"/>
  </interface>
  <interface name="org.freedesktop.host.portal.Registry">
    <method name="Register">
      <arg type="s" direction="in"/><arg type="a{{sv}}" direction="in"/>
    </method>
  </interface>
  <interface name="org.freedesktop.portal.GlobalShortcuts">
    <method name="CreateSession">
      <arg type="a{{sv}}" direction="in"/><arg type="o" direction="out"/>
    </method>
    <method name="BindShortcuts">
      <arg type="o" direction="in"/><arg type="a(sa{{sv}})" direction="in"/>
      <arg type="s" direction="in"/><arg type="a{{sv}}" direction="in"/>
      <arg type="o" direction="out"/>
    </method>
    <property name="version" type="u" access="read"/>
    <signal name="Activated">
      <arg type="o"/><arg type="s"/><arg type="t"/><arg type="a{{sv}}"/>
    </signal>
  </interface>
  <interface name="org.freedesktop.portal.Session">
    <method name="Close"/>
  </interface>
  <interface name="org.badi.Test">
    <method name="SetActive"><arg type="b" direction="in"/></method>
    <method name="SetOnBattery"><arg type="b" direction="in"/></method>
    <method name="SetProfile"><arg type="s" direction="in"/></method>
    <method name="PrepareForSleep"><arg type="b" direction="in"/></method>
    <method name="Activate"><arg type="s" direction="in"/></method>
    <method name="RestartLogind"/>
    <method name="State"><arg type="a{{sv}}" direction="out"/></method>
  </interface>
</node>
"""
NODE = Gio.DBusNodeInfo.new_for_xml(XML)


def interface(name):
    return NODE.lookup_interface(name)


def register(bus, path, info, call, get):
    # The closure-based replacement is missing from older PyGObject.
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        return bus.register_object(path, info, call, get, None)


class Desktop:
    def __init__(self, bus):
        self.bus = bus
        self.active = True
        self.on_battery = True
        self.profile = "balanced"
        self.inhibitors = {}
        self.watches = {}
        self.inhibitors_taken = 0
        self.registered = ""
        self.register_was_first = None
        self.portal_calls = 0
        self.session_handle = ""
        self.bound = []
        self.sessions_closed = 0
        self.logind_owner = None
        self.objects = []
        for path, name in (
            ("/org/freedesktop/login1", "org.freedesktop.login1.Manager"),
            ("/org/freedesktop/login1/user/self", "org.freedesktop.login1.User"),
            (SESSION_PATH, "org.freedesktop.login1.Session"),
            ("/org/freedesktop/UPower", "org.freedesktop.UPower"),
            ("/org/freedesktop/UPower/PowerProfiles", "org.freedesktop.UPower.PowerProfiles"),
            ("/org/freedesktop/portal/desktop", "org.freedesktop.host.portal.Registry"),
            ("/org/freedesktop/portal/desktop", "org.freedesktop.portal.GlobalShortcuts"),
            ("/org/badi/Test", "org.badi.Test"),
        ):
            self.objects.append(register(bus, path, interface(name), self.call, self.get))
        self.own("org.freedesktop.login1")
        for name in ("org.freedesktop.UPower", "org.freedesktop.UPower.PowerProfiles",
                     "org.freedesktop.portal.Desktop", "org.badi.Test"):
            Gio.bus_own_name_on_connection(bus, name, Gio.BusNameOwnerFlags.NONE, None, None)

    def own(self, name):
        self.logind_owner = Gio.bus_own_name_on_connection(self.bus, name, Gio.BusNameOwnerFlags.NONE, None, None)

    def get(self, _bus, _sender, _path, interface_name, prop):
        values = {
            ("org.freedesktop.login1.User", "Display"): GLib.Variant("(so)", ("31", SESSION_PATH)),
            ("org.freedesktop.login1.Session", "Active"): GLib.Variant("b", self.active),
            ("org.freedesktop.UPower", "OnBattery"): GLib.Variant("b", self.on_battery),
            ("org.freedesktop.UPower.PowerProfiles", "ActiveProfile"): GLib.Variant("s", self.profile),
            ("org.freedesktop.portal.GlobalShortcuts", "version"): GLib.Variant("u", 1),
        }
        return values.get((interface_name, prop))

    def changed(self, path, interface_name, prop, value):
        self.bus.emit_signal(None, path, "org.freedesktop.DBus.Properties", "PropertiesChanged",
                             GLib.Variant("(sa{sv}as)", (interface_name, {prop: value}, [])))

    def respond(self, sender, token, results):
        path = f"/org/freedesktop/portal/desktop/request/{sender.lstrip(':').replace('.', '_')}/{token}"

        def emit():
            self.bus.emit_signal(sender, path, "org.freedesktop.portal.Request", "Response",
                                 GLib.Variant("(ua{sv})", (0, results)))
            return GLib.SOURCE_REMOVE

        GLib.idle_add(emit)
        return path

    def released(self, fd, _condition):
        os.close(fd)
        self.inhibitors.pop(fd, None)
        self.watches.pop(fd, None)
        return GLib.SOURCE_REMOVE

    def call(self, _bus, sender, path, interface_name, method, parameters, invocation):
        arguments = parameters.unpack()
        if interface_name.startswith("org.freedesktop.portal") or interface_name.startswith("org.freedesktop.host"):
            if self.register_was_first is None:
                self.register_was_first = method == "Register"
            self.portal_calls += 1
        if method == "Inhibit":
            read, write = os.pipe()
            fds = Gio.UnixFDList.new()
            index = fds.append(write)
            os.close(write)
            self.inhibitors[read] = arguments
            self.inhibitors_taken += 1
            self.watches[read] = add_fd(GLib.PRIORITY_DEFAULT, read,
                                        GLib.IOCondition.HUP | GLib.IOCondition.ERR, self.released)
            invocation.return_value_with_unix_fd_list(GLib.Variant("(h)", (index,)), fds)
            return
        if method == "Register":
            self.registered = arguments[0]
        elif method == "CreateSession":
            options = arguments[0]
            self.session_handle = (f"/org/freedesktop/portal/desktop/session/"
                                   f"{sender.lstrip(':').replace('.', '_')}/{options['session_handle_token']}")
            self.session_object = register(
                self.bus, self.session_handle, interface("org.freedesktop.portal.Session"), self.call, None)
            invocation.return_value(GLib.Variant("(o)", (self.respond(sender, options["handle_token"], {
                "session_handle": GLib.Variant("s", self.session_handle)}),)))
            return
        elif method == "BindShortcuts":
            session_handle, shortcuts, _parent, options = arguments
            self.bound = sorted(shortcut for shortcut, _properties in shortcuts)
            results = {"shortcuts": GLib.Variant("a(sa{sv})", [
                (shortcut, {"description": GLib.Variant("s", properties["description"]),
                            "trigger_description": GLib.Variant("s", "")})
                for shortcut, properties in shortcuts])}
            invocation.return_value(GLib.Variant("(o)", (self.respond(sender, options["handle_token"], results),)))
            return
        elif method == "Close":
            self.sessions_closed += 1
            self.bus.unregister_object(self.session_object)
        elif method == "SetActive":
            self.active = arguments[0]
            self.changed(SESSION_PATH, "org.freedesktop.login1.Session", "Active", GLib.Variant("b", self.active))
        elif method == "SetOnBattery":
            self.on_battery = arguments[0]
            self.changed("/org/freedesktop/UPower", "org.freedesktop.UPower", "OnBattery",
                         GLib.Variant("b", self.on_battery))
        elif method == "SetProfile":
            self.profile = arguments[0]
            self.changed("/org/freedesktop/UPower/PowerProfiles", "org.freedesktop.UPower.PowerProfiles",
                         "ActiveProfile", GLib.Variant("s", self.profile))
        elif method == "PrepareForSleep":
            self.bus.emit_signal(None, "/org/freedesktop/login1", "org.freedesktop.login1.Manager",
                                 "PrepareForSleep", GLib.Variant("(b)", (arguments[0],)))
        elif method == "Activate":
            self.bus.emit_signal(None, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.GlobalShortcuts",
                                 "Activated", GLib.Variant("(osta{sv})", (self.session_handle, arguments[0], 1, {})))
        elif method == "RestartLogind":
            # A restarted logind forgets every inhibitor it handed out.
            Gio.bus_unown_name(self.logind_owner)
            for fd in list(self.inhibitors):
                GLib.source_remove(self.watches.pop(fd))
                os.close(fd)
            self.inhibitors.clear()
            GLib.timeout_add(100, lambda: self.own("org.freedesktop.login1") or GLib.SOURCE_REMOVE)
        elif method == "State":
            invocation.return_value(GLib.Variant("(a{sv})", ({
                "inhibitors_held": GLib.Variant("u", len(self.inhibitors)),
                "inhibitors_taken": GLib.Variant("u", self.inhibitors_taken),
                "inhibit_mode": GLib.Variant("s", next(iter(self.inhibitors.values()), ("", "", "", ""))[3]),
                "registered": GLib.Variant("s", self.registered),
                "register_was_first": GLib.Variant("b", bool(self.register_was_first)),
                "bound": GLib.Variant("as", self.bound),
                "sessions_closed": GLib.Variant("u", self.sessions_closed),
            },)))
            return
        invocation.return_value(None)


bus = Gio.DBusConnection.new_for_address_sync(
    os.environ["DBUS_SESSION_BUS_ADDRESS"],
    Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
desktop = Desktop(bus)
print("ready", flush=True)
loop = GLib.MainLoop()
add_signal(GLib.PRIORITY_HIGH, 15, lambda *_arguments: loop.quit() or GLib.SOURCE_REMOVE)
loop.run()
sys.exit(0)
