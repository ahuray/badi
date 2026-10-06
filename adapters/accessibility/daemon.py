#!/usr/bin/env python3
"""Private, non-mutating AT-SPI focus observer for Badi's native adapter."""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import socket
import stat
import struct
import sys
import time
import warnings

from contract import Denied, MAX_FRAME, Observer, SCHEMA
from desktop import App, WindowEvents, hyprland_directory
from field import FieldBackend

# An unrecognized executable never inherits another application's permission.
APPS = {
    "chromium": App(frozenset({"/usr/lib/chromium/chromium"}), frozenset({"chromium"}), browser=True, web=True),
    # /usr/bin/chromium exports CHROME_DESKTOP=chromium.desktop; started any
    # other way, the same executable's Wayland app id is chromium-browser.
    "chromium-browser": App(frozenset({"/usr/lib/chromium/chromium"}), frozenset({"chromium-browser"}),
                            browser=True, web=True),
    # Omarchy web apps open as Brave --app windows; the addon maps them to this id.
    "brave-origin": App(frozenset({"/opt/brave-origin-bin/brave"}), frozenset({"brave-origin"}), browser=True, web=True,
                        web_apps="brave-"),
    # /usr/bin/zen-browser execs this binary; its Wayland app id and Fcitx program() are both "zen".
    "zen": App(frozenset({"/opt/zen-browser-bin/zen-bin"}), frozenset({"zen"}), browser=True, web=True, gecko=True),
    "chatgpt": App(frozenset({"/usr/lib/chatgpt/ChatGPT"}), frozenset({"chatgpt"}), web=True),
    # VS Code 1.140 renamed its Wayland app id; the addon maps both to "code".
    "code": App(frozenset({"/usr/share/code/code"}), frozenset({"code", "com.microsoft.VSCode"}), web=True),
    "cursor": App(frozenset({"/usr/lib/electron42/electron"}), frozenset({"cursor"}), web=True,
                  entry="/usr/share/cursor/resources/app/cursor.mjs"),
    "discord": App(classes=frozenset({"discord"}), web=True, per_user=True),
    "grok-bot": App(frozenset({"/opt/Grok Bot/grok-bot"}), frozenset({"grok-bot"}), web=True),
    "telegram": App(frozenset({"/usr/bin/Telegram"}), frozenset({"org.telegram.desktop"})),
    "omawrite": App(frozenset({"/usr/bin/omawrite"}), frozenset({"omawrite"})),
    # Writer document paragraphs only: Calc, Impress and dialog fields share
    # LibreOffice's Fcitx program id and fail closed here.
    "libreoffice": App(frozenset({"/usr/lib/libreoffice/program/soffice.bin"}), frozenset({"libreoffice-writer"}),
                       roles=frozenset({"paragraph"}), observed_only=True, sentence_bounded=True),
}
OPERATION_SECONDS = 0.35
MAX_CLIENTS = 4
MAX_BUFFER = MAX_FRAME * 4
# Only the owning connection may acquire or draw; status is open to any client.
OWNER_OPERATIONS = frozenset({"inspect", "snapshot", "preview", "hide"})
# Producer interest requested from the armed application. Its events reach
# this helper only through a match on the armed field's exact sender and path.
FIELD_EVENTS = ("object:text-changed", "object:text-caret-moved", "object:text-selection-changed",
                "object:state-changed:editable", "object:state-changed:showing",
                "object:state-changed:defunct", "object:property-change:accessible-role")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate_key")
        result[key] = value
    return result


def private(info, kind, mode):
    return kind(info.st_mode) and info.st_uid == os.getuid() and stat.S_IMODE(info.st_mode) == mode


def within_frame_limits(buffer):
    return len(buffer) <= MAX_BUFFER and all(len(part) + 1 <= MAX_FRAME for part in buffer.split(b"\n"))


class Daemon:
    def __init__(self, path, atspi, glib):
        self.GLib = glib
        self.A = atspi
        self.path = Path(path)
        self.backend = FieldBackend(atspi, APPS)
        self.observer = Observer(self.backend)
        self.clients = {}
        self.owner = None
        self.observer.notify = self.broadcast
        self.server = None
        self.socket_identity = None
        self.lock_fd = None
        self.window_events = None
        self.failed = False
        self.preview = None
        self.preview_unavailable = False
        self.observer.render = self.render_preview
        self.observer.hide = self.hide_preview
        self.observer.authorize = self.arm_field_events
        self.observer.disarm = self.disarm_field_events
        self.event_bus = None
        self.producer = None
        self.field_subscription = None
        self.field_binding = None

    def connect_event_bus(self):
        # Connection/authentication happens before serving requests; individual
        # field operations never perform an unbounded synchronous connection.
        from gi.repository import Gio
        session = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        address = session.call_sync("org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus", "GetAddress", None,
                                    self.GLib.VariantType.new("(s)"), Gio.DBusCallFlags.NONE, 50, None).unpack()[0]
        self.event_bus = Gio.DBusConnection.new_for_address_sync(
            address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
            None, None)

    def registry(self, method, signature, arguments, sync):
        from gi.repository import Gio
        call = ("org.a11y.atspi.Registry", "/org/a11y/atspi/registry", "org.a11y.atspi.Registry", method,
                self.GLib.Variant(signature, arguments), None, Gio.DBusCallFlags.NONE, 50, None)
        if sync:
            self.event_bus.call_sync(*call)
        else:
            # A reply is expected (and discarded) so the bus never rejects it.
            self.event_bus.call(*call, lambda *_result: None, None)

    def arm_field_events(self, binding):
        """Receive the granted field's own events until the next invalidation.

        Producer interest is first requested after a caller grant in this
        application and lasts while its window stays active. The match is
        narrowed to this unique sender and object path, so other fields'
        events, including their typed text, never arrive.
        """
        from gi.repository import Gio
        if self.field_binding == binding:
            return
        self.disarm_field_events()
        if self.event_bus is None or self.event_bus.is_closed():
            raise Denied("accessibility_unavailable")
        if self.producer != binding["bus"]:
            self.withdraw_producer()
            self.register_producer(binding["bus"])
        self.field_binding = dict(binding)
        self.field_subscription = self.event_bus.signal_subscribe(
            binding["bus"], "org.a11y.atspi.Event.Object", None, binding["path"], None,
            Gio.DBusSignalFlags.NONE, self.field_event, None)

    def field_event(self, _connection, _sender, _path, _interface, _signal, _parameters, _data):
        # Any event on the field ends its epoch. The payload is never unpacked:
        # a text change carries the typed text itself.
        self.observer.invalidate("field_changed")

    def disarm_field_events(self):
        if self.field_subscription is not None:
            self.event_bus.signal_unsubscribe(self.field_subscription)
            self.field_subscription = None
        self.field_binding = None

    def register_producer(self, bus):
        self.producer = bus
        for event in FIELD_EVENTS:
            self.registry("RegisterEvent", "(sass)", (event, [], bus), sync=True)

    def withdraw_producer(self):
        if self.producer is not None:
            for event in FIELD_EVENTS:
                self.registry("DeregisterEvent", "(ss)", (event, self.producer), sync=False)
            self.producer = None

    def load_preview(self):
        """The GTK preview, built at most once; None if it cannot be built here."""
        if self.preview is None and not self.preview_unavailable:
            try:
                from preview import Preview
                self.preview = Preview()
            except Exception:
                self.preview_unavailable = True
        return self.preview

    def warm_preview(self):
        """Build the preview and load its fonts before the first request needs them."""
        preview = self.load_preview()
        if preview is not None:
            # Only an optimization: a broken preview reports itself on render.
            with contextlib.suppress(Exception):
                preview.warm()
        return False

    def render_preview(self, focus, text, ttl_ms):
        preview = self.load_preview()
        if preview is None:
            return False
        try:
            return preview.render(focus, text, ttl_ms)
        except Exception:
            preview.hide()
            return False

    def hide_preview(self):
        if self.preview is not None:
            self.preview.hide()

    def bind(self):
        """Listen on the private socket as this user's only observer."""
        self.lock_instance()
        self.remove_stale_socket()
        self.listen()

    def lock_instance(self):
        parent = self.path.parent
        parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if not private(parent.lstat(), stat.S_ISDIR, 0o700):
            raise RuntimeError("unsafe_runtime_directory")
        self.lock_fd = os.open(str(parent / ".accessibility.lock"), os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        if not private(os.fstat(self.lock_fd), stat.S_ISREG, 0o600):
            raise RuntimeError("unsafe_runtime_lock")
        fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def remove_stale_socket(self):
        """Unlink an earlier instance's socket once a refused connection proves it dead."""
        try:
            old = self.path.lstat()
        except FileNotFoundError:
            return
        if not private(old, stat.S_ISSOCK, 0o600):
            raise RuntimeError("unsafe_existing_socket")
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
            probe.settimeout(.05)
            try:
                probe.connect(str(self.path))
            except ConnectionRefusedError:
                current = self.path.lstat()
                if (old.st_dev, old.st_ino) != (current.st_dev, current.st_ino):
                    raise RuntimeError("socket_changed")
                self.path.unlink()
                return
        raise RuntimeError("observer_already_running")

    def listen(self):
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.setblocking(False)
        previous_umask = os.umask(0o177)
        try:
            self.server.bind(str(self.path))
        finally:
            os.umask(previous_umask)
        os.chmod(self.path, 0o600)
        self.socket_identity = self.path.stat()
        self.server.listen(MAX_CLIENTS)
        self.GLib.io_add_watch(self.server.fileno(), self.GLib.IO_IN, self.accept)

    def accept(self, _fd, _condition):
        try:
            client, _ = self.server.accept()
        except BlockingIOError:
            return True
        _pid, uid, _gid = struct.unpack("3i", client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        if uid != os.getuid() or len(self.clients) >= MAX_CLIENTS:
            client.close()
            return True
        client.setblocking(False)
        state = {"socket": client, "buffer": b"", "scheduled": False}
        self.clients[client.fileno()] = state
        self.GLib.io_add_watch(client.fileno(), self.GLib.IO_IN | self.GLib.IO_HUP | self.GLib.IO_ERR, self.read, state)
        return True

    def close_client(self, fd):
        client = self.clients.pop(fd, None)
        if client:
            if self.owner is client:
                self.owner = None
                self.observer.invalidate("client_disconnected")
            client["socket"].close()

    def disconnect(self, fd):
        """Close a client; the False also removes its GLib watch."""
        self.close_client(fd)
        return False

    def send(self, fd, document):
        client = self.clients.get(fd)
        if client is None:
            return
        raw = (json.dumps(document, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
        if len(raw) > MAX_FRAME:
            self.close_client(fd)
            return
        try:
            # Replies are tiny and bounded; a slow reader loses its connection
            # rather than blocking Fcitx or retaining typed prose in a queue.
            if client["socket"].send(raw) != len(raw):
                self.close_client(fd)
        except OSError:
            self.close_client(fd)

    def broadcast(self, document):
        if self.owner is not None:
            self.send(self.owner["socket"].fileno(), document)

    def read(self, fd, condition, expected=None):
        """Buffer a client's bytes and answer its first complete frame."""
        if expected is not None and self.clients.get(fd) is not expected:
            return False
        client = self.clients.get(fd)
        if condition & (self.GLib.IO_HUP | self.GLib.IO_ERR) or client is None:
            return self.disconnect(fd)
        try:
            data = client["socket"].recv(MAX_BUFFER + 1)
        except BlockingIOError:
            return True
        except OSError:
            return self.disconnect(fd)
        if not data:
            return self.disconnect(fd)
        client["buffer"] += data
        if not within_frame_limits(client["buffer"]):
            return self.disconnect(fd)
        if b"\n" in client["buffer"] and not client["scheduled"]:
            self.process_next(fd)
        return fd in self.clients

    def process_next(self, fd, expected=None):
        """Answer one buffered frame, then yield to GLib before the next.

        Valid stream frames may coalesce (for example hide + inspect). One per
        dispatch lets queued authority events run between expensive
        acquisitions, and the untouched partial tail stays buffered.
        """
        client = self.clients.get(fd)
        if client is None or (expected is not None and client is not expected):
            return False
        client["scheduled"] = False
        if b"\n" not in client["buffer"]:
            return False
        raw, client["buffer"] = client["buffer"].split(b"\n", 1)
        if not self.answer(fd, client, raw):
            return self.disconnect(fd)
        client = self.clients.get(fd)
        if client is not None and b"\n" in client["buffer"]:
            client["scheduled"] = True
            self.GLib.idle_add(self.process_next, fd, client)
        return False

    def answer(self, fd, client, raw):
        """Reply to one frame; False when it is not a well-formed JSON object."""
        request = None
        try:
            request = json.loads(raw, object_pairs_hook=unique_object)
            self.backend.deadline = time.monotonic() + OPERATION_SECONDS
            response = self.respond(fd, client, request)
            self.backend.budget()
            if response is not None:
                self.send(fd, response)
        except Denied as error:
            self.observer.invalidate("operation_timeout" if error.timed_out else "field_unavailable")
            self.send(fd, self.failure(request, str(error)))
        except (ValueError, UnicodeError):
            return False
        except Exception:
            self.observer.invalidate("accessibility_unavailable")
            self.send(fd, self.failure(request, "accessibility_unavailable"))
        return True

    def respond(self, fd, client, request):
        """The observer's response; None once a second acquisition client is told the observer is busy."""
        op = request.get("op") if isinstance(request, dict) else None
        if op in OWNER_OPERATIONS:
            if self.owner is not None and self.owner is not client:
                self.send(fd, self.failure(request, "observer_busy"))
                return None
            self.owner = client
        return self.observer.request(request)

    def failure(self, request, error):
        return {"schema": SCHEMA, "id": request.get("id") if isinstance(request, dict) else None, "ok": False,
                "error": error, "epoch": self.observer.epoch}

    def focus_event(self, _event, *_args):
        # The only global listener: any focus change invalidates, without querying its source.
        if self.observer.tracked is not None:
            self.observer.invalidate("focus_changed")

    def watch(self):
        directory = hyprland_directory()
        self.backend.desktop.request_socket = directory / ".socket.sock"
        self.connect_event_bus()
        self.listener = self.A.EventListener.new(self.focus_event, None)
        self.listener.register("object:state-changed:focused")
        self.window_events = WindowEvents.connect(directory)
        self.GLib.io_add_watch(self.window_events.fileno(), self.GLib.IO_IN | self.GLib.IO_HUP | self.GLib.IO_ERR,
                               self.window_event)

    def window_event(self, _fd, condition):
        if condition & (self.GLib.IO_HUP | self.GLib.IO_ERR):
            return self.lose_desktop()
        try:
            changed = self.window_events.changed()
        except OSError:
            return self.lose_desktop()
        if changed:
            if self.observer.tracked:
                self.observer.invalidate("window_changed")
            self.withdraw_producer()
        return True

    def lose_desktop(self):
        self.observer.invalidate("desktop_unavailable")
        self.failed = True
        self.loop.quit()
        return False

    def run(self):
        self.loop = self.GLib.MainLoop()
        try:
            self.bind()
            self.watch()
            self.GLib.idle_add(self.warm_preview)
            self.loop.run()
        finally:
            self.shutdown()

    def shutdown(self):
        for fd in list(self.clients):
            self.close_client(fd)
        self.disarm_field_events()
        if self.event_bus is not None and not self.event_bus.is_closed():
            self.withdraw_producer()
            self.event_bus.flush_sync(None)
            self.event_bus.close_sync(None)
        if self.window_events is not None:
            self.window_events.close()
        if self.server is not None:
            self.server.close()
        self.remove_own_socket()
        if self.lock_fd is not None:
            os.close(self.lock_fd)

    def remove_own_socket(self):
        """Unlink the socket only while it is still the one this instance bound."""
        if self.socket_identity is None:
            return
        with contextlib.suppress(FileNotFoundError):
            current = self.path.lstat()
            if (current.st_dev, current.st_ino) == (self.socket_identity.st_dev, self.socket_identity.st_ino):
                self.path.unlink()


def quiet_missing_cache(domain, level, message, _data=None):
    # libatspi warns once per application without an AT-SPI cache object and
    # then reads its nodes directly; any other warning is kept.
    from gi.repository import GLib
    if not message.startswith("AT-SPI: Error in GetItems"):
        GLib.log_default_handler(domain, level, message, None)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", default=str(Path(os.environ.get("XDG_RUNTIME_DIR", "/nonexistent")) / "badi/accessibility.sock"))
    args = parser.parse_args()
    # The preview draws one label: software rendering avoids loading GPU
    # drivers, and the helper never takes text input through an input method.
    os.environ.update(GSK_RENDERER="cairo", GDK_DISABLE="gl,vulkan", GTK_IM_MODULE="gtk-im-context-simple")
    # PyGObject binds this name to the deprecated C alias of
    # atspi_document_get_document_attribute_value; both send GetAttributeValue.
    warnings.filterwarnings("ignore", message=r"Atspi\.Document\.get_document_attribute_value is deprecated",
                            category=DeprecationWarning)
    import gi
    gi.require_version("Atspi", "2.0")
    from gi.repository import Atspi, GLib, GLibUnix
    GLib.log_set_handler("dbind", GLib.LogLevelFlags.LEVEL_WARNING, quiet_missing_cache, None)
    Atspi.set_timeout(50, 50)
    Atspi.init()
    daemon = Daemon(args.socket, Atspi, GLib)
    GLibUnix.signal_add(GLib.PRIORITY_DEFAULT, 15, lambda *_args: daemon.loop.quit() or False)
    GLibUnix.signal_add(GLib.PRIORITY_DEFAULT, 2, lambda *_args: daemon.loop.quit() or False)
    try:
        daemon.run()
        if daemon.failed:
            return 1
    except Exception:
        print("Badi accessibility observer: startup_or_runtime_unavailable", file=sys.stderr)
        return 1
    finally:
        Atspi.exit()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
