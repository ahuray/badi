#!/usr/bin/env python3
"""Private, non-mutating AT-SPI focus observer for Badi's native adapter."""
from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import socket
import stat
import struct
import subprocess
import time

from contract import Denied, MAX_FRAME, Observer, SCHEMA, canonical_origin

# These exact executable/class pairs were inspected on the supported workstation.
# An unrecognized executable never inherits another application's permission.
APPS = {
    "chromium": ({"/usr/lib/chromium/chromium"}, {"chromium"}, True),
    "brave-origin": ({"/opt/brave-origin-bin/brave"}, {"brave-origin"}, True),
    "chatgpt": ({"/usr/lib/chatgpt/ChatGPT"}, {"chatgpt"}, False),
    "omawrite": ({"/usr/bin/omawrite"}, {"omawrite"}, False),
}
OPERATION_SECONDS = 0.35


class DesktopBackend:
    def __init__(self, atspi):
        self.atspi = atspi
        self.cached = None
        self.deadline = 0
        self.chromium_version_verified = None
        self.chromium_version_identity = None

    def budget(self):
        if time.monotonic() >= self.deadline:
            raise Denied("operation_timeout")

    def command(self, *args):
        self.budget()
        try:
            raw = subprocess.check_output(args, timeout=min(.15, max(.001, self.deadline-time.monotonic())), stderr=subprocess.DEVNULL)
            if len(raw) > 32 * 1024:
                raise Denied("desktop_unavailable")
            return json.loads(raw)
        except (OSError, ValueError, subprocess.SubprocessError):
            raise Denied("desktop_unavailable") from None

    def window(self, app_id):
        if app_id not in APPS:
            raise Denied("unsupported_app")
        locked = self.command("omarchy-shell", "lock", "status")
        if any(locked.get(k) is not False for k in ("locked", "secure", "pending", "requested", "sessionLocked")):
            raise Denied("desktop_locked")
        window = self.command("hyprctl", "-j", "activewindow")
        pid = window.get("pid")
        if type(pid) is not int or pid <= 0:
            raise Denied("focus_unavailable")
        paths, classes, browser = APPS[app_id]
        try:
            process = Path(f"/proc/{pid}")
            if process.stat().st_uid != os.getuid() or str((process / "exe").resolve()) not in paths:
                raise Denied("app_mismatch")
        except OSError:
            raise Denied("app_mismatch") from None
        if window.get("class") not in classes or window.get("mapped") is not True or window.get("hidden") is not False:
            raise Denied("app_mismatch")
        return window, browser

    def focused(self, pid, browser=False):
        A = self.atspi
        desktop = A.get_desktop(0)
        focused = []
        for index in range(min(desktop.get_child_count(), 64)):
            self.budget()
            app = desktop.get_child_at_index(index)
            if app.get_process_id() != pid:
                continue
            # Match in the application's process. No document text is visited.
            if "Collection" in app.get_interfaces():
                states = A.StateSet.new([A.StateType.FOCUSED, A.StateType.EDITABLE])
                rule = A.MatchRule.new(states, A.CollectionMatchType.ALL, {}, A.CollectionMatchType.ALL,
                                       [], A.CollectionMatchType.ALL, [], A.CollectionMatchType.ALL, False)
                focused.extend(app.get_collection_iface().get_matches(rule, A.CollectionSortOrder.CANONICAL, 8, True))
            else:
                queue = [app]
                visited = 0
                while queue:
                    self.budget()
                    node = queue.pop(0)
                    visited += 1
                    if visited > 192:
                        raise Denied("tree_limit")
                    if node.get_state_set().contains(A.StateType.FOCUSED):
                        focused.append(node)
                    queue.extend(node.get_child_at_index(i) for i in range(min(node.get_child_count(), 128)))
        focused = [node for node in focused if node.get_state_set().contains(A.StateType.EDITABLE)]
        if browser:
            scoped = []
            for node in focused:
                try:
                    canonical_origin(self.document_uri(node))
                    scoped.append(node)
                except Denied:
                    continue
            focused = scoped
        if len(focused) != 1 or focused[0].get_process_id() != pid:
            raise Denied("focus_unavailable")
        self.cached = focused[0]
        return self.cached

    def document_uri(self, node):
        ancestor = node
        for _ in range(24):
            self.budget()
            if ancestor is None:
                break
            if "Document" in ancestor.get_interfaces():
                uri = self.atspi.Document.get_document_attribute_value(ancestor.get_document_iface(), "URI") or ""
                if uri:
                    if len(uri.encode()) > 4096:
                        raise Denied("origin_unavailable")
                    return uri
            ancestor = ancestor.get_parent()
        raise Denied("origin_unavailable")

    def metadata(self, app_id):
        self.budget()
        A = self.atspi
        window, browser = self.window(app_id)
        node = self.focused(window["pid"], browser)
        node.clear_cache()
        flags = node.get_state_set()
        role = node.get_role_name()
        interfaces = node.get_interfaces()
        if "Text" not in interfaces:
            raise Denied("unsupported_field")
        attributes = node.get_attributes()
        # Never fetch a password's caret, extent or text. All other purpose gates
        # likewise precede the first call to Text.GetText in contract.py.
        if role == "password text" or attributes.get("text-input-type") == "password":
            raise Denied("sensitive_field")
        uri = ""
        if browser:
            uri = self.document_uri(node)
        text = node.get_text_iface()
        self.budget()
        caret, count, selections = text.get_caret_offset(), text.get_character_count(), text.get_n_selections()
        geometry = None
        if count and caret >= 0 and flags.contains(A.StateType.SHOWING):
            rect = text.get_character_extents(min(max(caret - 1, 0), count - 1), A.CoordType.SCREEN)
            geometry = {"raw_character": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height},
                        "character_offset": min(max(caret - 1, 0), count - 1),
                        "requested_space": "atspi_screen", "normalized": False,
                        "window": {k: window[k] for k in ("pid", "address", "at", "size", "monitor", "xwayland")},
                        "note": "coordinate_space_requires_adapter_validation"}
        if geometry and app_id == "chromium" and window.get("xwayland") is False:
            try:
                executable = Path("/usr/lib/chromium/chromium").stat()
                version_identity = (executable.st_dev, executable.st_ino, executable.st_size, executable.st_mtime_ns)
            except OSError:
                version_identity = None
            if self.chromium_version_identity != version_identity:
                self.chromium_version_verified = None
                self.chromium_version_identity = version_identity
            if self.chromium_version_verified is None:
                try:
                    version = subprocess.check_output(["/usr/lib/chromium/chromium", "--version"], timeout=.1, stderr=subprocess.DEVNULL).decode()
                    self.chromium_version_verified = version.startswith("Chromium 151.")
                except (OSError, UnicodeError, subprocess.SubprocessError):
                    self.chromium_version_verified = False
            if self.chromium_version_verified:
                monitors = self.command("hyprctl", "-j", "monitors")
                matching = [monitor for monitor in monitors if monitor.get("id") == window["monitor"]]
                if len(matching) == 1:
                    monitor = matching[0]
                    scale = monitor.get("scale")
                    if isinstance(scale, (int, float)) and 0.5 <= scale <= 4 and monitor.get("transform") == 0:
                        geometry.update(coordinate_convention="chromium151_wayland_window_physical", scale=scale,
                                        monitor={k: monitor[k] for k in ("name", "x", "y", "width", "height", "scale", "transform")})
        direction = ""
        if count and caret >= 0:
            run = A.Text.get_attribute_run(text, min(max(caret - 1, 0), count - 1), True)
            direction = run[0].get("direction", "")
        return {"direction": direction, "bus": node.app.bus_name, "path": node.path, "process_id": window["pid"],
                "app_id": app_id, "uri": uri, "browser": browser, "role": role,
                "tag": attributes.get("tag", ""), "input_type": attributes.get("text-input-type", ""),
                "focused": flags.contains(A.StateType.FOCUSED), "editable": flags.contains(A.StateType.EDITABLE),
                "showing": flags.contains(A.StateType.SHOWING), "visible": flags.contains(A.StateType.VISIBLE),
                "enabled": flags.contains(A.StateType.ENABLED), "sensitive": False,
                "caret": caret, "total_chars": count, "selection_count": selections,
                "geometry": geometry, "node": node}

    def text(self, metadata, start, end):
        self.budget()
        return self.atspi.Text.get_text(metadata["node"].get_text_iface(), start, end)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate_key")
        result[key] = value
    return result


class Daemon:
    def __init__(self, path, atspi, glib):
        self.GLib = glib
        self.A = atspi
        self.path = Path(path)
        self.backend = DesktopBackend(atspi)
        self.observer = Observer(self.backend)
        self.clients = {}
        self.owner = None
        self.observer.notify = self.broadcast
        self.server = None
        self.hypr = None
        self.hypr_buffer = b""
        self.lock_fd = None
        self.failed = False
        self.preview = None
        self.observer.render = self.render_preview
        self.observer.hide = self.hide_preview
        self.observer.authorize = self.arm_text_events
        self.observer.disarm = self.disarm_text_events
        self.text_bus = None
        self.text_subscription = None
        self.text_binding = None

    def connect_text_bus(self):
        # Connection/authentication happens before serving requests; individual
        # field operations never perform an unbounded synchronous connection.
        from gi.repository import Gio
        session = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        address = session.call_sync("org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus", "GetAddress", None,
                                    self.GLib.VariantType.new("(s)"), Gio.DBusCallFlags.NONE, 50, None).unpack()[0]
        self.text_bus = Gio.DBusConnection.new_for_address_sync(address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)

    def arm_text_events(self, binding):
        from gi.repository import Gio
        if self.text_binding == binding:
            return
        self.disarm_text_events()
        if self.text_bus is None or self.text_bus.is_closed():
            raise Denied("accessibility_unavailable")
        # Register producer interest only after the exact target's caller grant.
        # The separate connection's bus match is narrowed to this unique sender
        # and object path, so unrelated fields' typed signal payloads never arrive.
        self.text_bus.call_sync("org.a11y.atspi.Registry", "/org/a11y/atspi/registry", "org.a11y.atspi.Registry", "RegisterEvent",
                                self.GLib.Variant("(sass)", ("object:text-changed", [], binding["bus"])), None,
                                Gio.DBusCallFlags.NONE, 50, None)
        self.text_binding = dict(binding)
        self.text_subscription = self.text_bus.signal_subscribe(binding["bus"], "org.a11y.atspi.Event.Object", "TextChanged", binding["path"], None,
                                                               Gio.DBusSignalFlags.NONE, self.text_event, None)

    def text_event(self, _connection, _sender, _path, _interface, _signal, _parameters, _data):
        # Never unpack the event's text payload, including for the approved field.
        self.observer.invalidate("field_changed")

    def disarm_text_events(self):
        if self.text_subscription is not None:
            self.text_bus.signal_unsubscribe(self.text_subscription)
            self.text_subscription = None
        if self.text_binding is not None:
            from gi.repository import Gio
            self.text_bus.call("org.a11y.atspi.Registry", "/org/a11y/atspi/registry", "org.a11y.atspi.Registry", "DeregisterEvent",
                               self.GLib.Variant("(ss)", ("object:text-changed", self.text_binding["bus"])), None,
                               Gio.DBusCallFlags.NONE, 50, None, None, None)
            self.text_binding = None

    def render_preview(self, focus, text, ttl_ms):
        try:
            if self.preview is None:
                from preview import Preview
                self.preview = Preview()
            return self.preview.render(focus, text, ttl_ms)
        except Exception:
            self.hide_preview()
            return False

    def hide_preview(self):
        if self.preview is not None:
            self.preview.hide()

    def bind(self):
        parent = self.path.parent
        parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        info = parent.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
            raise RuntimeError("unsafe_runtime_directory")
        self.lock_fd = os.open(str(parent / ".accessibility.lock"), os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        lock_stat = os.fstat(self.lock_fd)
        if not stat.S_ISREG(lock_stat.st_mode) or lock_stat.st_uid != os.getuid() or stat.S_IMODE(lock_stat.st_mode) != 0o600:
            raise RuntimeError("unsafe_runtime_lock")
        fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            old = self.path.lstat()
        except FileNotFoundError:
            old = None
        if old is not None:
            if not stat.S_ISSOCK(old.st_mode) or old.st_uid != os.getuid() or stat.S_IMODE(old.st_mode) != 0o600:
                raise RuntimeError("unsafe_existing_socket")
            probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            probe.settimeout(.05)
            try:
                probe.connect(str(self.path))
            except ConnectionRefusedError:
                current = self.path.lstat()
                if (old.st_dev, old.st_ino) != (current.st_dev, current.st_ino):
                    raise RuntimeError("socket_changed")
                self.path.unlink()
            else:
                raise RuntimeError("observer_already_running")
            finally:
                probe.close()
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.setblocking(False)
        previous_umask = os.umask(0o177)
        try:
            self.server.bind(str(self.path))
        finally:
            os.umask(previous_umask)
        os.chmod(self.path, 0o600)
        self.socket_identity = self.path.stat()
        self.server.listen(4)
        self.GLib.io_add_watch(self.server.fileno(), self.GLib.IO_IN, self.accept)

    def accept(self, _fd, _condition):
        try:
            client, _ = self.server.accept()
        except BlockingIOError:
            return True
        _pid, uid, _gid = struct.unpack("3i", client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        if uid != os.getuid() or len(self.clients) >= 4:
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
        if expected is not None and self.clients.get(fd) is not expected:
            return False
        if condition & (self.GLib.IO_HUP | self.GLib.IO_ERR) or fd not in self.clients:
            self.close_client(fd)
            return False
        client = self.clients[fd]
        try:
            data = client["socket"].recv(MAX_FRAME * 4 + 1)
        except BlockingIOError:
            return True
        except OSError:
            self.close_client(fd)
            return False
        if not data:
            self.close_client(fd)
            return False
        client["buffer"] += data
        parts = client["buffer"].split(b"\n")
        if len(client["buffer"]) > MAX_FRAME * 4 or any(len(part) + 1 > MAX_FRAME for part in parts):
            self.close_client(fd)
            return False
        if b"\n" in client["buffer"] and not client["scheduled"]:
            self.process_next(fd)
        return fd in self.clients

    def process_next(self, fd, expected=None):
        client = self.clients.get(fd)
        if client is None or (expected is not None and client is not expected):
            return False
        client["scheduled"] = False
        if b"\n" not in client["buffer"]:
            return False
        raw, client["buffer"] = client["buffer"].split(b"\n", 1)
        request = None
        try:
            request = json.loads(raw, object_pairs_hook=unique_object)
            self.backend.deadline = time.monotonic() + OPERATION_SECONDS
            op = request.get("op") if isinstance(request, dict) else None
            if op in ("inspect", "snapshot", "preview", "hide"):
                if self.owner is not None and self.owner is not client:
                    self.send(fd, {"schema": SCHEMA, "id": request.get("id"), "ok": False,
                                   "error": "observer_busy", "epoch": self.observer.epoch})
                    response = None
                else:
                    self.owner = client
                    response = self.observer.request(request)
            else:
                response = self.observer.request(request)
            self.backend.budget()
            if response is not None:
                self.send(fd, response)
        except Denied as error:
            self.observer.invalidate("operation_timeout" if str(error) == "operation_timeout" else "field_unavailable")
            self.send(fd, {"schema": SCHEMA, "id": request.get("id") if isinstance(request, dict) else None, "ok": False,
                           "error": str(error), "epoch": self.observer.epoch})
        except (ValueError, UnicodeError):
            self.close_client(fd)
            return False
        except Exception:
            self.observer.invalidate("accessibility_unavailable")
            self.send(fd, {"schema": SCHEMA, "id": request.get("id") if isinstance(request, dict) else None,
                           "ok": False, "error": "accessibility_unavailable", "epoch": self.observer.epoch})
        client = self.clients.get(fd)
        if client is not None and b"\n" in client["buffer"]:
            # Valid stream frames may coalesce (for example hide + inspect).
            # Process one per dispatch so queued authority events run between
            # expensive acquisitions, preserving the untouched partial tail.
            client["scheduled"] = True
            self.GLib.idle_add(self.process_next, fd, client)
        return False

    def event(self, event, *_args):
        tracked = self.observer.tracked
        if tracked is None:
            return
        try:
            local_disposal = (event.type == "object:state-changed:defunct" and event.detail1 == 1 and
                              event.detail2 == 0 and event.sender is None)
            if local_disposal:
                # libatspi 2.60.6 emits sender-null defunct from local proxy
                # disposal, including unrelated nodes visited by inspection.
                # Dropping the focused cache for those events disposes that
                # proxy too and invalidates every snapshot. Ignore only a
                # positively identified different object; the tracked object,
                # unknown identity and all remote events still fail closed.
                bus, path = event.source.app.bus_name, event.source.path
                if (isinstance(bus, str) and bus and isinstance(path, str) and path and
                        (bus, path) != (tracked["bus"], tracked["path"])):
                    return
            source_pid = None if local_disposal else event.source.get_process_id()
            focus_event = event.type.startswith("object:state-changed:focused")
            if local_disposal or focus_event or source_pid == tracked["process_id"]:
                # Invalidate before any later read; never retain event.any_data,
                # which can contain text typed in another application.
                self.backend.cached = None
                self.observer.invalidate("focus_changed" if focus_event else "field_changed")
        except Exception:
            self.backend.cached = None
            self.observer.invalidate("accessibility_unavailable")

    def watch(self):
        self.connect_text_bus()
        self.listener = self.A.EventListener.new(self.event, None)
        for event in ("object:state-changed:focused", "object:text-caret-moved",
                      "object:text-selection-changed", "object:state-changed:editable", "object:state-changed:showing",
                      "object:state-changed:defunct", "object:property-change:accessible-role", "object:children-changed", "window:deactivate", "window:move", "window:resize"):
            self.listener.register(event)
        signature = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE", "")
        if not signature or "/" in signature:
            raise RuntimeError("hyprland_session_required")
        endpoint = Path(os.environ["XDG_RUNTIME_DIR"]) / "hypr" / signature / ".socket2.sock"
        self.hypr = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.hypr.connect(str(endpoint))
        self.hypr.setblocking(False)
        self.GLib.io_add_watch(self.hypr.fileno(), self.GLib.IO_IN | self.GLib.IO_HUP | self.GLib.IO_ERR, self.hypr_event)

    def hypr_event(self, _fd, condition):
        if condition & (self.GLib.IO_HUP | self.GLib.IO_ERR):
            self.observer.invalidate("desktop_unavailable")
            self.failed = True
            self.loop.quit()
            return False
        try:
            data = self.hypr.recv(4096)
            if not data:
                raise OSError
            self.hypr_buffer += data
            if len(self.hypr_buffer) > 16384:
                raise OSError
            while b"\n" in self.hypr_buffer:
                line, self.hypr_buffer = self.hypr_buffer.split(b"\n", 1)
                event = line.split(b">>", 1)[0]
                if self.observer.tracked and event in (b"activewindowv2", b"focusedmon", b"workspacev2", b"movewindowv2", b"closewindow", b"monitoraddedv2", b"monitorremoved"):
                    self.backend.cached = None
                    self.observer.invalidate("window_changed")
        except OSError:
            self.observer.invalidate("desktop_unavailable")
            self.failed = True
            self.loop.quit()
            return False
        return True

    def run(self):
        self.loop = self.GLib.MainLoop()
        try:
            self.bind()
            self.watch()
            self.loop.run()
        finally:
            for fd in list(self.clients):
                self.close_client(fd)
            self.disarm_text_events()
            if self.text_bus is not None:
                self.text_bus.close_sync(None)
            if self.hypr:
                self.hypr.close()
            if self.server:
                self.server.close()
            try:
                current = self.path.lstat()
                if hasattr(self, "socket_identity") and (current.st_dev, current.st_ino) == (self.socket_identity.st_dev, self.socket_identity.st_ino):
                    self.path.unlink()
            except FileNotFoundError:
                pass
            if self.lock_fd is not None:
                os.close(self.lock_fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", default=str(Path(os.environ.get("XDG_RUNTIME_DIR", "/nonexistent")) / "badi/accessibility.sock"))
    args = parser.parse_args()
    import gi
    gi.require_version("Atspi", "2.0")
    from gi.repository import Atspi, GLib, GLibUnix
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
        print("Badi accessibility observer: startup_or_runtime_unavailable", file=__import__("sys").stderr)
        return 1
    finally:
        Atspi.exit()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
