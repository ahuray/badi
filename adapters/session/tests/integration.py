#!/usr/bin/env python3
"""The session helper against a real phrase broker and a fake desktop bus.

Starts a private dbus-daemon with fake logind, UPower, power-profiles and
portal services, a fake pw-dump, the broker built by cargo (``--provider
phrase``) in disposable XDG directories, and the helper itself. Nothing
touches the user's desktop, keyring, power profile, PipeWire or real broker.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
BROKER = Path(os.environ.get("BADI_TEST_BROKER", ROOT / "target/debug/badi-broker"))
BADICTL = BROKER.with_name("badictl")
SESSION_PATH = "/org/freedesktop/login1/session/_31"
AUDIO = {"id": 52, "type": "PipeWire:Interface:Node",
         "info": {"props": {"media.class": "Stream/Output/Audio", "node.name": "quickshell"}}}
# What xdg-desktop-portal-hyprland 1.4.1 creates for a shared screen.
SCREENCAST = {"id": 123, "type": "PipeWire:Interface:Node",
              "info": {"props": {"media.class": "Video/Source", "node.name": "xdg-desktop-portal-hyprland",
                                 "media.name": "xdph-streaming-789568"}}}


def wait_for(predicate, timeout=6.0, message="condition"):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.05)
    raise AssertionError(f"timed out waiting for {message}")


class SessionHelperIntegration(unittest.TestCase):
    def setUp(self):
        if not BROKER.is_file() or not BADICTL.is_file():
            self.fail(f"build the broker first: cargo build -p badi-broker --bins --locked ({BROKER})")
        if shutil.which("dbus-daemon") is None:
            self.fail("dbus-daemon is required for the session helper integration test")
        self.temporary = tempfile.TemporaryDirectory(prefix="badi-session-")
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.runtime = root / "runtime"
        self.runtime.mkdir(mode=0o700)
        self.record = root / "controller.log"
        controller = root / "controller.py"
        controller.write_text(
            "import sys\n"
            f"open({str(self.record)!r}, 'a').write(' '.join(sys.argv[1:]) + '\\n')\n"
            "print('Predictions paused.' if sys.argv[1:] == ['pause', 'toggle'] else '')\n")
        self.environment = {
            **{key: value for key, value in os.environ.items() if not key.startswith(("DBUS_", "BADI_"))},
            "XDG_RUNTIME_DIR": str(self.runtime),
            "XDG_CONFIG_HOME": str(root / "config"), "XDG_DATA_HOME": str(root / "data"),
            "XDG_STATE_HOME": str(root / "state"), "HOME": str(root / "home"),
        }
        (root / "home").mkdir()
        # The fake pw-dump's controls: a graph without any screencast to start.
        self.pipewire = root / "pipewire"
        self.pipewire.mkdir()
        (self.pipewire / "graph.json").write_text(json.dumps([AUDIO]))
        bus = subprocess.Popen(["dbus-daemon", "--session", "--nofork", "--print-address=1"],
                               stdout=subprocess.PIPE, text=True, env=self.environment)
        self.addCleanup(self.stop, bus)
        address = bus.stdout.readline().strip()
        self.assertTrue(address.startswith("unix:"), "private bus address")
        self.environment.update(DBUS_SESSION_BUS_ADDRESS=address, BADI_SESSION_SYSTEM_BUS_ADDRESS=address)
        self.keyring = None
        if shutil.which("gnome-keyring-daemon"):
            # An isolated keyring on the private bus, in this test's own HOME.
            self.keyring = subprocess.Popen(
                ["gnome-keyring-daemon", "--unlock", "--components=secrets", "--foreground"],
                stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                env=self.environment)
            self.keyring.stdin.write(b"isolated-test-password")
            self.keyring.stdin.close()
            self.addCleanup(self.stop, self.keyring)
        self.fake = subprocess.Popen([sys.executable, "-B", str(HERE / "fake_desktop.py")],
                                     stdout=subprocess.PIPE, text=True, env=self.environment)
        self.addCleanup(self.stop, self.fake)
        self.assertEqual(self.fake.stdout.readline().strip(), "ready")
        self.start_broker()
        self.helper_log = root / "helper.log"
        with self.helper_log.open("w") as log:
            self.helper = subprocess.Popen(
                [sys.executable, "-B", str(ROOT / "adapters/session/daemon.py")],
                env={**self.environment, "BADI_SESSION_CONTROLLER": str(controller),
                     "BADI_SESSION_NOTIFICATIONS": "off",
                     "BADI_SESSION_PIPEWIRE_DUMP": str(HERE / "fake_pw_dump.py"),
                     "BADI_TEST_PW_DUMP_DIR": str(self.pipewire)},
                stdout=subprocess.DEVNULL, stderr=log)
        self.addCleanup(self.stop, self.helper)

    def start_broker(self):
        self.broker = subprocess.Popen([str(BROKER), "--provider", "phrase"], env=self.environment,
                                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                       stderr=subprocess.DEVNULL)
        self.addCleanup(self.stop, self.broker)
        wait_for(lambda: (self.runtime / "badi/broker.sock").exists(), message="broker socket")

    def stop(self, process):
        if process.poll() is None:
            process.send_signal(signal.SIGTERM)
            try:
                process.wait(5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        for stream in (process.stdout, process.stderr):
            if stream is not None:
                stream.close()

    def health(self):
        result = subprocess.run([str(BADICTL), "status"], env=self.environment, capture_output=True,
                                text=True, timeout=5)
        return json.loads(result.stdout) if result.returncode == 0 else None

    def desktop(self):
        health = self.health()
        return health and health.get("desktop")

    def fake_call(self, method, signature=None, *values):
        if not hasattr(self, "control_bus"):
            self.control_bus = Gio.DBusConnection.new_for_address_sync(
                self.environment["DBUS_SESSION_BUS_ADDRESS"],
                Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
                None, None)
            self.addCleanup(self.control_bus.close_sync, None)
        arguments = None if signature is None else GLib.Variant(f"({signature})", values)
        reply = self.control_bus.call_sync("org.badi.Test", "/org/badi/Test", "org.badi.Test", method, arguments,
                                           None, Gio.DBusCallFlags.NONE, 5000, None)
        return reply.unpack() if reply is not None else None

    def state(self):
        return self.fake_call("State")[0]

    def wait_desktop(self, message, **expected):
        def matches():
            desktop = self.desktop()
            return desktop if desktop and all(desktop.get(key) == value for key, value in expected.items()) else None
        return wait_for(matches, message=message)

    def test_session_power_sleep_shortcuts_and_restarts(self):
        # Startup: on battery (the fake's initial state), session active, shortcuts bound.
        self.wait_desktop("initial report", observed=True, session="active", low_power=True, automatic="deferred")
        wait_for(lambda: self.state()["inhibitors_held"] == 1, message="delay inhibitor")
        state = wait_for(lambda: (state := self.state())["bound"] and state, message="bound shortcuts")
        self.assertEqual(state["bound"], ["open-settings", "pause-toggle", "snooze"])
        self.assertEqual(state["registered"], "io.github.ahuray.badi")
        self.assertTrue(state["register_was_first"], "host registration precedes every other portal call")
        self.assertEqual(state["inhibit_mode"], "delay")

        # Power: mains, then the power-saver profile; neither changes authority.
        epoch = self.health()["authority_epoch"]
        self.fake_call("SetOnBattery", "b", False)
        self.wait_desktop("mains power", low_power=False)
        self.fake_call("SetProfile", "s", "power-saver")
        self.wait_desktop("power saver", low_power=True)
        self.fake_call("SetProfile", "s", "balanced")
        self.wait_desktop("balanced profile", low_power=False)
        self.assertEqual(self.health()["authority_epoch"], epoch)

        # Sleep: the broker holds every grant before the inhibitor is released.
        self.fake_call("PrepareForSleep", "b", True)
        wait_for(lambda: self.state()["inhibitors_held"] == 0, message="inhibitor release")
        health = self.health()
        self.assertEqual(health["desktop"]["session"], "sleeping")
        self.assertTrue(health["paused"])
        self.assertGreater(health["authority_epoch"], epoch)
        self.fake_call("PrepareForSleep", "b", False)
        self.wait_desktop("resume", session="active")
        wait_for(lambda: self.state()["inhibitors_held"] == 1, message="inhibitor after resume")
        self.assertEqual(self.state()["inhibitors_taken"], 2)

        # Session switch away and back.
        self.fake_call("SetActive", "b", False)
        self.wait_desktop("inactive session", session="inactive")
        self.fake_call("SetActive", "b", True)
        self.wait_desktop("active session", session="active")

        # A restarted logind forgets the inhibitor; the helper takes a new one.
        self.fake_call("RestartLogind")
        wait_for(lambda: self.state()["inhibitors_taken"] == 3 and self.state()["inhibitors_held"] == 1,
                 message="inhibitor after logind restart")

        # Shortcuts run the controller; nothing else.
        self.fake_call("Activate", "s", "pause-toggle")
        self.fake_call("Activate", "s", "snooze")
        self.fake_call("Activate", "s", "open-settings")
        self.fake_call("Activate", "s", "unknown-shortcut")
        wait_for(lambda: self.record.exists() and len(self.record.read_text().splitlines()) == 3,
                 message="controller runs")
        self.assertEqual(sorted(self.record.read_text().splitlines()), ["pause toggle", "settings", "snooze"])

        # A broker restart: the helper reconnects and reports the current state again.
        self.fake_call("SetOnBattery", "b", True)
        self.wait_desktop("battery again", low_power=True)
        self.stop(self.broker)
        self.start_broker()
        self.wait_desktop("report after broker restart", observed=True, session="active", low_power=True)

        # Stopping the helper releases its hold, inhibitor and portal session.
        self.fake_call("SetActive", "b", False)
        self.wait_desktop("inactive before stop", session="inactive")
        self.stop(self.helper)
        self.wait_desktop("helper gone", observed=False, session="active")
        self.assertFalse(self.health()["paused"] and self.health()["desktop"]["session"] != "active")
        state = self.state()
        self.assertEqual(state["inhibitors_held"], 0)
        self.assertEqual(state["sessions_closed"], 1)
        log = self.helper_log.read_text()
        self.assertNotIn("Traceback", log)


    def pipewire_change(self, *entries):
        self.changes = getattr(self, "changes", 0) + 1
        (self.pipewire / f"change-{self.changes:06d}.json").write_text(json.dumps(list(entries)))

    def settings(self, change):
        shown = subprocess.run([str(BADICTL), "settings", "show", "--json"], env=self.environment,
                               capture_output=True, text=True, timeout=5, check=True)
        document = json.loads(shown.stdout)
        revision = document["revision"]
        change(document)
        document["revision"] = revision + 1
        subprocess.run([str(BADICTL), "settings", "replace", "--if-revision", str(revision), "--json", "-"],
                       env=self.environment, input=json.dumps(document), capture_output=True, text=True,
                       timeout=5, check=True)

    def test_a_screen_share_holds_suggestions_until_it_stops(self):
        self.wait_desktop("initial report", observed=True, screen_share="none")
        # A new broker's settings start paused; only a hold may pause it here.
        self.settings(lambda document: document.update(paused=False))
        self.assertFalse(self.health()["paused"])
        epoch = self.health()["authority_epoch"]

        # A portal screencast appears: every grant is held, the session stays active.
        self.pipewire_change(SCREENCAST)
        self.wait_desktop("screen shared", screen_share="held", session="active")
        health = self.health()
        self.assertTrue(health["paused"])
        self.assertGreater(health["authority_epoch"], epoch)

        # The user keeps suggestions while sharing, then holds them again.
        self.settings(lambda document: document.update(screen_share="suggest"))
        self.wait_desktop("suggesting while shared", screen_share="suggesting")
        self.assertFalse(self.health()["paused"])
        self.settings(lambda document: document.pop("screen_share"))
        self.wait_desktop("held again", screen_share="held")
        self.assertTrue(self.health()["paused"])

        # The stream ends: the hold ends with it.
        self.pipewire_change({"id": 123, "info": None})
        self.wait_desktop("sharing stopped", screen_share="none")
        self.assertFalse(self.health()["paused"])

        # A helper that exits while the screen is shared takes the hold with it.
        self.pipewire_change(SCREENCAST)
        self.wait_desktop("shared again", screen_share="held")
        self.stop(self.helper)
        self.wait_desktop("helper gone", observed=False, screen_share="none")
        self.assertFalse(self.health()["paused"])
        log = self.helper_log.read_text()
        self.assertIn("screen_shared=true", log)
        self.assertNotIn("xdph-streaming", log, "stream names stay out of the journal")
        self.assertNotIn("Traceback", log)

    @unittest.skipUnless(shutil.which("gnome-keyring-daemon"), "gnome-keyring-daemon is not installed")
    def test_the_keyring_style_key_reaches_the_broker_and_is_never_logged(self):
        self.wait_desktop("initial report", observed=True)
        status = self.style_status()
        self.assertEqual(status["key"], "absent", "no key is stored yet")
        stored = subprocess.run(
            [sys.executable, "-B", "-c",
             "import sys; sys.path.insert(0, sys.argv[1]); import style_key; print(' '.join(style_key.ensure()))",
             str(ROOT / "adapters/session")],
            env=self.environment, capture_output=True, text=True, timeout=10)
        self.assertEqual(stored.returncode, 0, stored.stderr)
        key_id, key = stored.stdout.split()
        # The helper hands the key to each new broker connection.
        self.stop(self.broker)
        self.start_broker()
        wait_for(lambda: self.style_status() and self.style_status()["key"] == "ready", message="key delivery")
        log = self.helper_log.read_text()
        self.assertIn("style key delivered", log)
        self.assertNotIn(key, log)
        self.assertNotIn(key_id, log)

    def style_status(self):
        result = subprocess.run([str(BADICTL), "style", "status"], env=self.environment, capture_output=True,
                                text=True, timeout=5)
        return json.loads(result.stdout) if result.returncode == 0 else None


if __name__ == "__main__":
    unittest.main(verbosity=2)
