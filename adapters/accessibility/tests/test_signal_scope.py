"""Optional real session-bus proof using only task-created signal emitters."""
import os
import pathlib
import sys
import time
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from daemon import Daemon


@unittest.skipUnless(os.environ.get("BADI_A11Y_REQUIRE_SESSION_BUS") == "1", "requires explicit real session-bus integration lane")
class SignalScopeTests(unittest.TestCase):
    def test_text_events_are_sender_and_field_scoped(self):
        import gi
        gi.require_version("Atspi", "2.0")
        from gi.repository import Atspi, Gio, GLib
        session = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        address = session.call_sync("org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus", "GetAddress", None,
                                    GLib.VariantType.new("(s)"), Gio.DBusCallFlags.NONE, 100, None).unpack()[0]
        def connection():
            return Gio.DBusConnection.new_for_address_sync(address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
        emitters = [connection(), connection()]
        observer = Daemon("/unused-test-socket", Atspi, GLib)
        events = []
        observer.observer.notify = events.append
        binding = {"epoch": 1, "bus": emitters[0].get_unique_name(), "path": "/badi/fixture/one",
                   "process_id": os.getpid(), "app_id": "chromium", "uri": "https://fixture.invalid/"}
        def pump():
            until = time.monotonic() + .12
            while time.monotonic() < until:
                while GLib.MainContext.default().pending():
                    GLib.MainContext.default().iteration(False)
                time.sleep(.005)
        def emit(sender, path):
            sender.emit_signal(None, path, "org.a11y.atspi.Event.Object", "TextChanged", GLib.Variant("(s)", ("disposable synthetic text",)))
            sender.flush_sync(None)
            pump()
        try:
            observer.observer.tracked = binding
            observer.connect_text_bus()
            observer.arm_text_events(binding)
            observer.text_bus.flush_sync(None)
            pump()
            emit(emitters[0], "/badi/fixture/two")
            emit(emitters[1], "/badi/fixture/one")
            self.assertEqual(events, [])
            emit(emitters[0], "/badi/fixture/one")
            self.assertEqual(len(events), 1)
            self.assertEqual(events[0]["reason"], "field_changed")
            self.assertNotIn("disposable", str(events[0]))
            self.assertIsNone(observer.text_subscription)
            emit(emitters[0], "/badi/fixture/one")
            self.assertEqual(len(events), 1)
        finally:
            observer.disarm_text_events()
            if observer.text_bus:
                observer.text_bus.close_sync(None)
            for emitter in emitters:
                emitter.close_sync(None)


if __name__ == "__main__":
    unittest.main()
