"""The observer's own session-bus connection and how it recognizes a
compositor's bus name (ADR 0015, ADR 0016).

A compositor is trusted by the connection that owns its well-known name: the
bus daemon must say it runs as this user, and the kernel must name its
process as expected. Distributions give kwin_wayland CAP_SYS_NICE, which
makes it undumpable, so the name comes from /proc/PID/status, which stays
readable, not from its exe link. Same-UID processes are trusted (ADR 0001).
"""
from __future__ import annotations

import os
from pathlib import Path
import re

UNIQUE_NAME = re.compile(r":[0-9]+\.[0-9]+\Z")


class OwnerError(RuntimeError):
    """"absent" (nobody owns the name), "unavailable" (the bus failed) or
    "unverified" (the owner is not this user's expected process)."""


def verified_owner(bus, name, process, timeout, proc=Path("/proc")):
    """The unique name that owns `name`, when it is this user's `process`."""
    try:
        owner = dbus(bus, "GetNameOwner", "(s)", (name,), "(s)", timeout)[0]
    except Exception as error:
        raise OwnerError("absent" if "NameHasNoOwner" in str(error) else "unavailable") from None
    try:
        credentials = dbus(bus, "GetConnectionCredentials", "(s)", (owner,), "(a{sv})", timeout)[0]
    except Exception:
        raise OwnerError("unavailable") from None
    if (not isinstance(owner, str) or not UNIQUE_NAME.fullmatch(owner) or not isinstance(credentials, dict)
            or credentials.get("UnixUserID") != os.getuid()
            or not is_process(process_name(credentials.get("ProcessID"), proc), process)):
        raise OwnerError("unverified")
    return owner


def dbus(bus, method, signature, args, reply, timeout):
    return bus.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", method,
                    signature, args, reply, timeout)


def app_id(value):
    """The window's app id when it is plain printable ASCII, else ''."""
    if isinstance(value, str) and 0 < len(value) <= 255 and all(" " < char < "\x7f" and char != "/" for char in value):
        return value
    return ""


def process_name(pid, proc=Path("/proc")):
    """The kernel's name (at most 15 bytes) for process `pid` when all its
    user ids are this user's, else None. /proc/PID/status stays readable
    for an undumpable process, unlike its exe link."""
    if type(pid) is not int or pid <= 0:
        return None
    try:
        with (proc / str(pid) / "status").open("rb") as stream:
            lines = stream.read(16384).decode("utf-8", "replace").splitlines()
    except OSError:
        return None
    fields = dict(line.split(":\t", 1) for line in lines if ":\t" in line)
    uids = fields.get("Uid", "").split()
    if len(uids) != 4 or any(uid != str(os.getuid()) for uid in uids):
        return None
    return fields.get("Name")


def is_process(name, expected):
    """Whether a kernel process name is `expected`, which the kernel cuts to 15 bytes."""
    return name is not None and name == expected[:15]


class GioBus:
    """A private session-bus connection of the observer's own."""

    def __init__(self):
        from gi.repository import Gio, GLib
        self.Gio, self.GLib = Gio, GLib
        address = Gio.dbus_address_get_for_bus_sync(Gio.BusType.SESSION, None)
        self.connection = Gio.DBusConnection.new_for_address_sync(
            address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
            None, None)
        self.unique_name = self.connection.get_unique_name()
        self.filter_id = None
        self.subscriptions = []

    def call(self, destination, path, interface, method, signature, args, reply, timeout):
        parameters = self.GLib.Variant(signature, args) if signature else None
        reply_type = self.GLib.VariantType.new(reply) if reply else None
        result = self.connection.call_sync(destination, path, interface, method, parameters, reply_type,
                                           self.Gio.DBusCallFlags.NO_AUTO_START, max(1, round(timeout * 1000)), None)
        return result.unpack() if result is not None else ()

    def serve(self, path, interface, handler):
        """Answer method calls to `path` and `interface` with `handler`'s verdict.

        A filter runs on GDBus's message thread in arrival order, before any
        reply that follows a report reaches a waiting call.
        """
        Gio = self.Gio

        def filter_message(connection, message, incoming):
            if (not incoming or message.get_message_type() != Gio.DBusMessageType.METHOD_CALL
                    or message.get_path() != path or message.get_interface() != interface):
                return message
            body = message.get_body()
            arguments = body.unpack() if body is not None and body.get_type_string() == "(ss)" else None
            try:
                accepted = handler(message.get_sender(), message.get_member(), arguments)
            except Exception:
                accepted = False
            if accepted:
                answer = Gio.DBusMessage.new_method_reply(message)
            else:
                answer = Gio.DBusMessage.new_method_error_literal(message, "org.freedesktop.DBus.Error.AccessDenied",
                                                                  "refused")
            connection.send_message(answer, Gio.DBusSendMessageFlags.NONE)
            return None

        self.filter_id = self.connection.add_filter(filter_message)

    def watch_owner(self, name, callback):
        def changed(_connection, _sender, _path, _interface, _signal, parameters):
            _name, _old, new = parameters.unpack()
            callback(new)

        self.subscriptions.append(self.connection.signal_subscribe(
            "org.freedesktop.DBus", "org.freedesktop.DBus", "NameOwnerChanged", "/org/freedesktop/DBus", name,
            self.Gio.DBusSignalFlags.NONE, changed))

    def subscribe(self, sender, path, interface, member, callback):
        """Deliver `member` signals from exactly `sender` (a unique name) at
        `path`; `callback` gets the unpacked arguments on the main loop."""
        def received(_connection, signal_sender, _path, _interface, _signal, parameters):
            if signal_sender == sender:
                callback(parameters.unpack())

        self.subscriptions.append(self.connection.signal_subscribe(
            sender, interface, member, path, None, self.Gio.DBusSignalFlags.NONE, received))

    def close(self):
        if self.filter_id is not None:
            self.connection.remove_filter(self.filter_id)
        for subscription in self.subscriptions:
            self.connection.signal_unsubscribe(subscription)
        if not self.connection.is_closed():
            self.connection.close_sync(None)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate_key")
        result[key] = value
    return result
