#!/usr/bin/env python3
"""Exercise the installed addon through synthetic Fcitx D-Bus input contexts.

Opt-in desktop integration check; requires Gio, a loaded Badi addon and the
unpaused local-model broker with both native app policies enabled. Only synthetic
text is sent. This verifies input-method routing, not visible editor insertion.
"""

import json
import subprocess
import time

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib


PREFIX = "Please find attached the"
TAB, ESCAPE = 0xff09, 0xff1b
CAPABILITIES = (1 << 1) | (1 << 6) | (1 << 35) | (1 << 39)


class InputContext:
    def __init__(self, app, extra_capabilities=0):
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self.events = []
        self.path = self.call(
            "/org/freedesktop/portal/inputmethod", "org.fcitx.Fcitx.InputMethod1",
            "CreateInputContext", "(a(ss))", ([('program', app)],))[0]
        self.subscription = self.bus.signal_subscribe(
            "org.fcitx.Fcitx5", "org.fcitx.Fcitx.InputContext1", None,
            self.path, None, Gio.DBusSignalFlags.NONE, self.on_signal, None)
        self.ic("SetCapability", "(t)", (CAPABILITIES | extra_capabilities,))
        self.ic("FocusIn")

    def call(self, path, interface, method, signature=None, values=None):
        return self.bus.call_sync(
            "org.fcitx.Fcitx5", path, interface, method,
            GLib.Variant(signature, values) if signature else None, None,
            Gio.DBusCallFlags.NONE, 3000, None).unpack()

    def ic(self, method, signature=None, values=None):
        return self.call(self.path, "org.fcitx.Fcitx.InputContext1", method, signature, values)

    def on_signal(self, bus, sender, path, interface, name, parameters, data):
        self.events.append((name, parameters.unpack()))

    def close(self):
        try:
            self.ic("FocusOut")
            self.ic("DestroyIC")
        finally:
            self.bus.signal_unsubscribe(self.subscription)

    def text(self, value, cursor=None, anchor=None):
        cursor = len(value) if cursor is None else cursor
        self.ic("SetSurroundingText", "(suu)",
                (value, cursor, cursor if anchor is None else anchor))

    def key(self, symbol, modifiers=0):
        handled, = self.ic("ProcessKeyEvent", "(uuubu)", (symbol, 0, modifiers, False, 0))
        self.ic("ProcessKeyEvent", "(uuubu)", (symbol, 0, modifiers, True, 0))
        return handled

    def candidate(self):
        for name, values in reversed(self.events):
            if name == "UpdateClientSideUI":
                return values[4][0][1] if values[4] else None
        return None

    def commits(self):
        return [values[0] for name, values in self.events if name == "CommitString"]


def pump():
    while GLib.MainContext.default().pending():
        GLib.MainContext.default().iteration(False)


def wait_for(predicate, timeout=3):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        pump()
        if predicate():
            return
        time.sleep(.01)
    raise AssertionError("Timed out waiting for the installed Fcitx/model result")


def check_app(app):
    context = InputContext(app)
    try:
        context.text("")
        assert not context.key(TAB), "Empty fields must retain Tab"
        context.text(PREFIX, len(PREFIX) - 1)
        assert not context.key(TAB), "Mid-text Tab must pass through"
        context.text(PREFIX, anchor=0)
        assert not context.key(TAB), "Selection must retain Tab"
        context.text(PREFIX)
        assert context.key(TAB), "Tab must invoke the model"
        wait_for(lambda: context.candidate() is not None)
        expected = context.candidate()
        assert isinstance(expected, str) and expected.strip(), "The model must display a nonempty candidate"
        assert len(expected.split()) <= 4, "The native preview remains bounded"
        assert context.key(TAB), "Second Tab must accept"
        wait_for(lambda: bool(context.commits()))
        assert context.commits() == [expected], "Acceptance must dispatch the exact visible candidate once"

        context.text(PREFIX)
        assert context.key(TAB)
        wait_for(lambda: context.candidate() is not None)
        assert context.key(ESCAPE)
        wait_for(lambda: context.candidate() is None)
        assert context.commits() == [expected], "Escape must not commit"

        context.text(PREFIX)
        assert context.key(TAB)
        wait_for(lambda: context.candidate() is not None)
        context.text(PREFIX + " x")
        wait_for(lambda: context.candidate() is None)
        assert not context.key(ord('Y'), 5), "Stale candidate must not be accepted"
        assert context.commits() == [expected]
    finally:
        context.close()
    print(f"{app}: Tab request/accept, empty/selected/mid-text pass-through, Escape and stale text passed")


def check_denied(app, extra_capabilities=0):
    context = InputContext(app, extra_capabilities)
    try:
        context.text(PREFIX)
        assert not context.key(TAB), "Unsupported/sensitive fields must retain Tab"
        pump()
        assert context.candidate() is None and not context.commits()
    finally:
        context.close()


if __name__ == "__main__":
    lock = json.loads(subprocess.run(["omarchy-shell", "lock", "status"],
        capture_output=True, text=True, check=True, timeout=4).stdout)
    assert all(lock.get(key) is False for key in ("locked", "secure", "requested", "pending", "sessionLocked")), \
        "Run the installed-input-method test only after normal desktop unlock"
    for app in ("omawrite", "com.github.xournalpp.xournalpp"):
        check_app(app)
    for app in ("obsidian", "chromium", "com.mitchellh.ghostty"):
        check_denied(app)
    check_denied("omawrite", 1 << 3)  # Password
    check_denied("omawrite", 1 << 32)  # Terminal capability
    print("Unsupported and sensitive contexts passed. Visible editor insertion remains a separate test.")
