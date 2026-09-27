#!/usr/bin/env python3
"""Private, non-mutating AT-SPI focus observer for Badi's native adapter."""
from __future__ import annotations

import argparse
import contextlib
from dataclasses import dataclass
import fcntl
import json
import math
import os
from pathlib import Path
import re
import socket
import stat
import struct
import subprocess
import time
from urllib.parse import urlsplit
import warnings

from contract import Denied, MAX_FRAME, Observer, SCHEMA, canonical_origin


@dataclass(frozen=True)
class App:
    executables: frozenset = frozenset()
    classes: frozenset = frozenset()
    browser: bool = False
    web: bool = False  # Web content (Chromium or Gecko): HTML purpose gates apply.
    gecko: bool = False  # Gecko browser: DocURL, one focused field, window-relative extents.
    entry: str | None = None  # A shared Electron runtime must load exactly this app.
    per_user: bool = False  # Self-updating build under $XDG_CONFIG_HOME/discord.


# These executable/class rules were inspected on the supported workstation.
# An unrecognized executable never inherits another application's permission.
APPS = {
    "chromium": App(frozenset({"/usr/lib/chromium/chromium"}), frozenset({"chromium"}), browser=True, web=True),
    # /usr/bin/chromium exports CHROME_DESKTOP=chromium.desktop; started any
    # other way, the same executable's Wayland app id is chromium-browser.
    "chromium-browser": App(frozenset({"/usr/lib/chromium/chromium"}), frozenset({"chromium-browser"}),
                            browser=True, web=True),
    "brave-origin": App(frozenset({"/opt/brave-origin-bin/brave"}), frozenset({"brave-origin"}), browser=True, web=True),
    # /usr/bin/zen-browser execs this binary; its Wayland app id and Fcitx program() are both "zen".
    "zen": App(frozenset({"/opt/zen-browser-bin/zen-bin"}), frozenset({"zen"}), browser=True, web=True, gecko=True),
    "chatgpt": App(frozenset({"/usr/lib/chatgpt/ChatGPT"}), frozenset({"chatgpt"}), web=True),
    "code": App(frozenset({"/usr/share/code/code"}), frozenset({"code"}), web=True),
    "cursor": App(frozenset({"/usr/lib/electron42/electron"}), frozenset({"cursor"}), web=True,
                  entry="/usr/share/cursor/resources/app/cursor.mjs"),
    "discord": App(classes=frozenset({"discord"}), web=True, per_user=True),
    "telegram": App(frozenset({"/usr/bin/Telegram"}), frozenset({"org.telegram.desktop"})),
    "omawrite": App(frozenset({"/usr/bin/omawrite"}), frozenset({"omawrite"})),
}
OPERATION_SECONDS = 0.35
MAX_CMDLINE = 64 * 1024
MAX_HYPRLAND_REPLY = 32 * 1024
LOCK_FLAGS = ("locked", "secure", "pending", "requested", "sessionLocked")
ELECTRON_NON_ENTRY_OPTIONS = frozenset({b"-r", b"--require", b"-i", b"--interactive", b"-repl",
                                        b"-v", b"--version", b"-a", b"--abi"})
DISCORD_BUILD = re.compile(r"app-[0-9]{1,9}(?:\.[0-9]{1,9}){0,3}\Z")
CALIBRATED = "atspi_frame_calibrated"
SCALE_TOLERANCE = 0.01
EDGE_TOLERANCE = 1
# Gecko's glyph extents are device pixels. Only integer scales keep device
# pixels an exact multiple of logical ones; other scales use the Fcitx panel.
GECKO_SCALES = (1, 2)
# A rich editor root exposes each block as one U+FFFC embedded object. Larger
# documents are unsupported rather than slow; each block costs a few calls.
EMBEDDED_OBJECT = "\ufffc"
MAX_RICH_BLOCKS = 64
# Chromium's text-input-v3 surrounding text ends every <p> with two line
# breaks when anything is rendered after the editor, and only one after a
# paragraph whose text already ends in a line break.
PARAGRAPH_END = "\n\n"
# Producer interest requested from the armed application. Its events reach
# this helper only through a match on the armed field's exact sender and path.
FIELD_EVENTS = ("object:text-changed", "object:text-caret-moved", "object:text-selection-changed",
                "object:state-changed:editable", "object:state-changed:showing",
                "object:state-changed:defunct", "object:property-change:accessible-role")


def browser_interface(uri):
    """Chromium renders parts of its own interface (such as the omnibox popup) as top-chrome WebUI."""
    try:
        parsed = urlsplit(uri)
        return parsed.scheme == "chrome" and (parsed.hostname or "").endswith(".top-chrome")
    except ValueError:
        return False


def config_home():
    value = os.environ.get("XDG_CONFIG_HOME", "")
    return Path(value) if os.path.isabs(value) else Path.home() / ".config"


def application_entry(cmdline):
    """The app Electron loads: its first non-switch argument, as Chromium parses it.

    Electron's default_app also honours options that load other code instead
    of, or before, that argument (--app=, -r/--require) or run no app at all
    (REPL, version, ABI). Any of them before the entry means no entry.
    """
    arguments = cmdline.split(b"\0")
    if arguments and arguments[-1] == b"":
        arguments.pop()
    # The Cursor GUI process rewrites its title into one space-joined argument.
    # A path with spaces then yields a wrong first token, which cannot equal
    # the pinned entry, so this fails closed.
    if len(arguments) == 1 and b" " in arguments[0]:
        arguments = [argument for argument in arguments[0].split(b" ") if argument]
    remaining = iter(arguments[1:])
    for argument in remaining:
        if argument in ELECTRON_NON_ENTRY_OPTIONS or argument.startswith(b"--app="):
            return None
        if argument == b"--":
            return next(remaining, None)
        if not argument.startswith(b"-"):
            return argument
    return None


def per_user_executable(executable, running, config, uid):
    """Discord's own updater installs <config>/discord/app-<version>/Discord.

    The kernel reports a canonical executable path. Recheck the current tree
    so no symlinked or foreign-owned component, and no replaced file, passes.
    """
    try:
        if not os.path.isabs(executable) or os.path.normpath(executable) != executable:
            return False
        base = Path(os.path.realpath(config)) / "discord"
        path = Path(executable)
        if path.name != "Discord" or not DISCORD_BUILD.fullmatch(path.parent.name) or path.parent.parent != base:
            return False
        for item, kind in ((base, stat.S_ISDIR), (path.parent, stat.S_ISDIR), (path, stat.S_ISREG)):
            info = item.lstat()
            if not kind(info.st_mode) or info.st_uid != uid or info.st_mode & 0o022:
                return False
        return (info.st_dev, info.st_ino) == (running.st_dev, running.st_ino)
    except (OSError, ValueError):
        return False


def verify_process(pid, app, uid, config, proc=Path("/proc")):
    process = proc / str(pid)
    try:
        if process.stat().st_uid != uid:
            return False
        executable = os.readlink(process / "exe")
        if app.per_user:
            return per_user_executable(executable, os.stat(process / "exe"), config, uid)
        if executable not in app.executables:
            return False
        if app.entry is None:
            return True
        with (process / "cmdline").open("rb") as stream:
            cmdline = stream.read(MAX_CMDLINE + 1)
        return len(cmdline) <= MAX_CMDLINE and application_entry(cmdline) == os.fsencode(app.entry)
    except OSError:
        return False


def visual_direction(previous, last):
    """Run direction from the two glyphs before the caret, or "".

    Gecko exposes no "direction" text attribute, so the caret edge follows
    glyph order instead: on one line, a later glyph to the right means
    left-to-right. Missing, empty or wrapped glyphs give "".
    """
    try:
        boxes = [(float(box.x), float(box.y), float(box.width), float(box.height)) for box in (previous, last)]
    except (AttributeError, TypeError, ValueError):
        return ""
    (x1, y1, w1, h1), (x2, y2, w2, h2) = boxes
    if min(w1, w2, h1, h2) <= 0 or abs(y1 - y2) > min(h1, h2) / 2 or x1 == x2:
        return ""
    return "ltr" if x2 > x1 else "rtl"


def _rect(value):
    return (isinstance(value, dict) and all(type(value.get(key)) in (int, float) and math.isfinite(value[key])
            and abs(value[key]) <= 10**7 for key in ("x", "y", "width", "height"))
            and value["width"] >= 0 and value["height"] >= 0)


def _inside(inner, outer):
    return (inner["x"] >= outer["x"] - EDGE_TOLERANCE and inner["y"] >= outer["y"] - EDGE_TOLERANCE and
            inner["x"] + inner["width"] <= outer["x"] + outer["width"] + EDGE_TOLERANCE and
            inner["y"] + inner["height"] <= outer["y"] + outer["height"] + EDGE_TOLERANCE)


def _output(frame, document, field, glyph, window, monitors, caret):
    """(monitor, scale, window width, window height) when the inputs are usable."""
    if not all(_rect(value) for value in (frame, document, field, glyph)) or type(caret) is not int or caret < 1:
        return None
    matching = [item for item in monitors if isinstance(item, dict) and item.get("id") == window["monitor"]]
    if len(matching) != 1 or window["xwayland"] is not False:
        return None
    monitor = matching[0]
    scale, (width, height) = monitor["scale"], window["size"]
    if (type(scale) not in (int, float) or not .5 <= scale <= 4 or monitor["transform"] != 0
            or not all(type(value) in (int, float) and 1 <= value <= 32768 for value in (width, height))
            or frame["width"] < 1 or glyph["height"] < 1):
        return None
    return monitor, scale, width, height


def _calibrated(output, caret, x, y, line, field, window):
    """The geometry reply; the caret must lie inside Hyprland's window size."""
    monitor, scale, width, height = output
    if not (0 <= x <= width and 0 <= y and y + line <= height):
        return None
    return {"coordinate_convention": CALIBRATED, "scale": scale, "character_offset": caret - 1,
            "caret": {"x": x, "y": y, "height": line}, "field": field,
            "window": {key: window[key] for key in ("pid", "address", "at", "size", "monitor", "xwayland")},
            "monitor": {key: monitor[key] for key in ("name", "x", "y", "width", "height", "scale", "transform")}}


def calibrated_geometry(frame, document, field, glyph, window, monitors, caret):
    """Window-local logical caret geometry for this request, or None.

    Chromium and Electron report document, field and glyph SCREEN extents in
    physical pixels offset by the frame's SCREEN origin times the scale; the
    frame itself is logical and Electron's origin is arbitrary. The document
    to frame width ratio must equal the monitor scale for this convention.
    """
    try:
        output = _output(frame, document, field, glyph, window, monitors, caret)
        if output is None:
            return None
        scale = output[1]
        if abs(document["width"] / frame["width"] - scale) > SCALE_TOLERANCE:
            return None
        if not _inside(glyph, field) or not _inside(field, document):
            return None
        left, top = frame["x"] * scale, frame["y"] * scale

        def local(value, origin):
            return round((value - origin) / scale, 2)

        return _calibrated(output, caret, local(glyph["x"] + glyph["width"], left), local(glyph["y"], top),
                           round(glyph["height"] / scale, 2),
                           {"x": local(field["x"], left), "y": local(field["y"], top),
                            "width": round(field["width"] / scale, 2), "height": round(field["height"] / scale, 2)},
                           window)
    except (KeyError, TypeError, ValueError, ZeroDivisionError):
        return None


def gecko_calibrated_geometry(frame, document, field, glyph, window, monitors, caret):
    """Window-local logical caret geometry for a Gecko field, or None.

    Gecko on Wayland reports SCREEN extents relative to its own window: the
    frame sits at (0, 0), and frame, document and field are logical pixels,
    but character extents are device pixels. The frame must start at the
    origin and span the window's width (a physical frame would be twice as
    wide), the document must lie inside the frame, the field inside the
    document and the scaled glyph inside the field. Gecko's surface can be
    taller than the tile Hyprland shows, so the caret is bounded by the window
    size, never by the frame height.
    """
    try:
        output = _output(frame, document, field, glyph, window, monitors, caret)
        if output is None:
            return None
        _monitor, scale, width, _height = output
        if scale not in GECKO_SCALES:
            return None
        if frame["x"] != 0 or frame["y"] != 0 or abs(frame["width"] - width) > EDGE_TOLERANCE:
            return None
        logical = {key: glyph[key] / scale for key in ("x", "y", "width", "height")}
        if not _inside(document, frame) or not _inside(field, document) or not _inside(logical, field):
            return None
        return _calibrated(output, caret, round(logical["x"] + logical["width"], 2), round(logical["y"], 2),
                           round(logical["height"], 2),
                           {key: round(field[key], 2) for key in ("x", "y", "width", "height")}, window)
    except (KeyError, TypeError, ValueError, ZeroDivisionError):
        return None


def hyprland_directory():
    signature = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE", "")
    runtime = os.environ.get("XDG_RUNTIME_DIR", "")
    if not signature or "/" in signature or signature in (".", "..") or not os.path.isabs(runtime):
        raise RuntimeError("hyprland_session_required")
    return Path(runtime) / "hypr" / signature


class DesktopBackend:
    def __init__(self, atspi, hyprland=None):
        self.atspi = atspi
        self.hyprland = hyprland  # Hyprland's request socket
        self.deadline = 0

    def budget(self):
        if time.monotonic() >= self.deadline:
            raise Denied("operation_timeout")

    def remaining(self, limit):
        self.budget()
        return min(limit, max(.001, self.deadline - time.monotonic()))

    def hypr(self, request):
        """Hyprland's JSON reply to one IPC request, within the operation budget."""
        if self.hyprland is None:
            raise Denied("desktop_unavailable")
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                connection.settimeout(self.remaining(.15))
                connection.connect(str(self.hyprland))
                connection.sendall(request.encode())
                raw = b""
                while part := connection.recv(8192):
                    raw += part
                    if len(raw) > MAX_HYPRLAND_REPLY:
                        raise Denied("desktop_unavailable")
                    connection.settimeout(self.remaining(.15))
            return json.loads(raw)
        except (OSError, ValueError):
            raise Denied("desktop_unavailable") from None

    def require_unlocked(self):
        """Deny unless the Omarchy shell reports every lock flag explicitly false.

        This is the Quickshell IPC call `omarchy-shell lock status` makes, made
        directly: that wrapper adds a Bash and a `timeout` process per inspect,
        and a timeout here would kill only the wrapper. Any failure, and any
        reply that is not the status document, fails closed.
        """
        shell = os.environ.get("OMARCHY_PATH", "")
        if not os.path.isabs(shell):
            raise Denied("desktop_unavailable")
        try:
            raw = subprocess.check_output(("qs", "ipc", "-n", "-p", os.path.join(shell, "shell"), "call", "--", "lock", "status"),
                                          timeout=self.remaining(.15), stdin=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            state = json.loads(raw) if len(raw) <= MAX_HYPRLAND_REPLY else None
        except (OSError, ValueError, subprocess.SubprocessError):
            raise Denied("desktop_unavailable") from None
        if not isinstance(state, dict) or any(state.get(flag) is not False for flag in LOCK_FLAGS):
            raise Denied("desktop_locked")

    def window(self, app_id, check_lock=False):
        app = APPS.get(app_id)
        if app is None:
            raise Denied("unsupported_app")
        if check_lock:
            self.require_unlocked()
        window = self.hypr("j/activewindow")
        if not isinstance(window, dict):
            raise Denied("focus_unavailable")
        pid = window.get("pid")
        if type(pid) is not int or pid <= 0:
            raise Denied("focus_unavailable")
        if not verify_process(pid, app, os.getuid(), config_home()):
            raise Denied("app_mismatch")
        if window.get("class") not in app.classes or window.get("mapped") is not True or window.get("hidden") is not False:
            raise Denied("app_mismatch")
        return window, app

    def page_content(self, node):
        """False only for Chromium's own interface; True for any document field.

        The omnibox has no document ancestor, and its popup (like other
        browser panels) is a chrome://*.top-chrome WebUI document. Every other
        document is page content, including a non-web one, and so is a field
        whose document cannot be read: it can never be skipped as browser UI.
        """
        try:
            uri = self.nearest_document_uri(node)
        except Denied as error:
            if str(error) == "operation_timeout":
                raise
            return True
        return uri is not None and not browser_interface(uri)

    def focused(self, pid, browser=False, gecko=False):
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
        if gecko:
            # Gecko reports exactly one focused editable node: the urlbar while
            # it has the keyboard, otherwise the page field. The urlbar shares
            # the page's Fcitx input context without a Url purpose, so this
            # observer, not the addon, must deny it: the one field's nearest
            # Document must be web content with an HTTP(S) DocURL.
            if len(focused) != 1 or focused[0].get_process_id() != pid:
                raise Denied("focus_unavailable")
            self.gecko_document_url(focused[0])
        elif browser:
            # Chromium reports FOCUSED on the page field, the omnibox and its
            # popup at once, so browser UI is ignored and exactly one page field
            # must remain. Picking the page field while the omnibox has the
            # keyboard is safe: the omnibox's Url purpose denies it in the
            # addon, and every display and dispatch requires this field's
            # snapshot to equal Fcitx's live surrounding text and caret.
            focused = [node for node in focused if self.page_content(node)]
            if len(focused) == 1:
                try:
                    canonical_origin(self.document_uri(focused[0]))
                except Denied as error:
                    if str(error) == "operation_timeout":
                        raise
                    raise Denied("focus_unavailable") from None
        if len(focused) != 1 or focused[0].get_process_id() != pid:
            raise Denied("focus_unavailable")
        return focused[0]

    def nearest_document_uri(self, node):
        """The nearest document's URI; None only when no ancestor is a document."""
        ancestor = node
        for _ in range(24):
            self.budget()
            if ancestor is None:
                return None
            if "Document" in ancestor.get_interfaces():
                uri = self.atspi.Document.get_document_attribute_value(ancestor.get_document_iface(), "URI") or ""
                if uri:
                    if len(uri.encode()) > 4096:
                        raise Denied("origin_unavailable")
                    return uri
            ancestor = ancestor.get_parent()
        raise Denied("origin_unavailable")

    def document_uri(self, node):
        uri = self.nearest_document_uri(node)
        if uri is None:
            raise Denied("origin_unavailable")
        return uri

    def gecko_document_url(self, node):
        """The HTTP(S) URL of a Gecko field's nearest Document, which must be a web page.

        Gecko names the attribute DocURL (its URI is empty). Its browser window
        is itself a Document (role frame), so the urlbar and other browser UI
        resolve to a non-web document. Only the nearest Document counts: an
        empty or non-HTTP(S) URL (about:, moz-extension:, file:, a loading
        frame) is never replaced by an outer page's. Such fields are denied as
        unsupported, without reading their text.
        """
        ancestor = node
        for _ in range(24):
            self.budget()
            if ancestor is None:
                break
            if "Document" in ancestor.get_interfaces():
                if ancestor.get_role_name() != "document web":
                    raise Denied("unsupported_field")
                url = self.atspi.Document.get_document_attribute_value(ancestor.get_document_iface(), "DocURL") or ""
                if len(url.encode()) > 4096:
                    raise Denied("origin_unavailable")
                try:
                    canonical_origin(url)
                except Denied:
                    raise Denied("unsupported_field") from None
                return url
            ancestor = ancestor.get_parent()
        raise Denied("origin_unavailable")

    def frame_and_document(self, node):
        """The outermost frame and the field's nearest web document ancestor."""
        ancestor, frame, document = node.get_parent(), None, None
        for _ in range(96):
            self.budget()
            if ancestor is None:
                return frame, document
            role = ancestor.get_role_name()
            if role == "application":
                return frame, document
            if document is None and role == "document web":
                document = ancestor
            if role in ("frame", "window", "dialog"):
                frame = ancestor
            ancestor = ancestor.get_parent()
        return None, None

    def geometry(self, node, text, caret, window, gecko=False, reported=None):
        """Calibrate this request's caret; None lets Fcitx show its own panel.

        `text` and `caret` locate the glyph before the caret: the field's own
        text, or a rich root's caret paragraph. `reported` is the caret in
        the field's (flattened) coordinates, which the reply must name.
        """
        A = self.atspi
        try:
            frame, document = self.frame_and_document(node)
            if frame is None or document is None:
                return None
            rects = {}
            for name, item in (("frame", frame), ("document", document), ("field", node)):
                if "Component" not in item.get_interfaces():
                    return None
                self.budget()
                rects[name] = item.get_component_iface().get_extents(A.CoordType.SCREEN)
            self.budget()
            rects["glyph"] = text.get_character_extents(caret - 1, A.CoordType.SCREEN)
            monitors = self.hypr("j/monitors")
        except Denied as error:
            if str(error) == "operation_timeout":
                raise
            return None
        except Exception:
            return None
        rects = {name: {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height} for name, rect in rects.items()}
        calibrate = gecko_calibrated_geometry if gecko else calibrated_geometry
        return calibrate(rects["frame"], rects["document"], rects["field"], rects["glyph"],
                         window, monitors if isinstance(monitors, list) else [], caret if reported is None else reported)

    def rich_layout(self, node, count, caret):
        """Flattened coordinates of a Chromium rich editor root, or None for a plain field.

        A ProseMirror composer (Codex desktop) is a focused `entry` whose text
        is one U+FFFC per paragraph; the paragraph holds the typed text,
        reports the caret, and the root's caret is the offset of that
        paragraph's embedded object. Only lengths, caret offsets and structure
        are read here, never prose. Anything the flattening cannot resolve
        fails closed: root text that is not solely embedded paragraphs, a
        paragraph with its own embedded objects (placeholder, image, mention,
        link), another role, a foreign parent, or an ambiguous caret.
        """
        A = self.atspi
        if "Hypertext" not in node.get_interfaces():
            return None
        self.budget()
        hypertext = node.get_hypertext_iface()
        links = A.Hypertext.get_n_links(hypertext)
        if links == 0:
            return None
        if not 1 <= links <= MAX_RICH_BLOCKS or links != count:
            raise Denied("unsupported_field")
        blocks, carets, selections, start = [], [], 0, 0
        for index in range(links):
            self.budget()
            link = A.Hypertext.get_link(hypertext, index)
            if (link is None or A.Hyperlink.get_start_index(link) != index or
                    A.Hyperlink.get_end_index(link) != index + 1 or A.Hyperlink.get_n_anchors(link) != 1):
                raise Denied("unsupported_field")
            child = A.Hyperlink.get_object(link, 0)
            parent = child.get_parent() if child is not None else None
            if (child is None or parent is None or child.get_role_name() != "paragraph" or
                    (parent.app.bus_name, parent.path) != (node.app.bus_name, node.path)):
                raise Denied("unsupported_field")
            interfaces = child.get_interfaces()
            if "Text" not in interfaces or "Hypertext" not in interfaces:
                raise Denied("unsupported_field")
            if A.Hypertext.get_n_links(child.get_hypertext_iface()) != 0:
                raise Denied("unsupported_field")
            text = child.get_text_iface()
            length, offset = text.get_character_count(), text.get_caret_offset()
            selections += text.get_n_selections()
            blocks.append({"node": child, "start": start, "length": length})
            carets.append(offset)
            start += length + len(PARAGRAPH_END)
        if not 0 <= caret < links or not 0 <= carets[caret] <= blocks[caret]["length"] or any(
                offset != -1 for index, offset in enumerate(carets) if index != caret):
            raise Denied("invalid_caret")
        block = blocks[caret]
        return {"blocks": blocks, "caret": block["start"] + carets[caret], "total_chars": start,
                "selections": selections, "text": block["node"].get_text_iface(),
                "local_caret": carets[caret], "local_count": block["length"]}

    def metadata(self, app_id, calibrate=False, check_lock=False):
        self.budget()
        A = self.atspi
        window, app = self.window(app_id, check_lock)
        browser = app.browser
        node = self.focused(window["pid"], browser, app.gecko)
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
            uri = self.gecko_document_url(node) if app.gecko else self.document_uri(node)
        # Gecko serializes block boundaries differently and is unverified;
        # its embedded objects stay in the plain text and never agree.
        flatten = app.web and not app.gecko
        caret, count, selections, rich = self.caret_position(node, flatten)
        # The glyph before the caret: in the field, or in a rich root's caret paragraph.
        local, local_caret, local_count = (node.get_text_iface(), caret, count) if rich is None else (
            rich["text"], rich["local_caret"], rich["local_count"])
        geometry = None
        if (calibrate and 1 <= local_caret <= local_count and flags.contains(A.StateType.SHOWING) and
                window.get("xwayland") is False):
            geometry = self.geometry(node, local, local_caret, window, app.gecko, caret)
        direction = ""
        if local_count and local_caret >= 0:
            run = A.Text.get_attribute_run(local, min(max(local_caret - 1, 0), local_count - 1), True)
            direction = run[0].get("direction", "")
        if not direction and 2 <= local_caret <= local_count:
            direction = visual_direction(local.get_character_extents(local_caret - 2, A.CoordType.SCREEN),
                                         local.get_character_extents(local_caret - 1, A.CoordType.SCREEN))
        return {"direction": direction, "bus": node.app.bus_name, "path": node.path, "process_id": window["pid"],
                "app_id": app_id, "uri": uri, "browser": browser, "web": app.web, "role": role,
                "tag": attributes.get("tag", ""), "input_type": attributes.get("text-input-type", ""),
                "focused": flags.contains(A.StateType.FOCUSED), "editable": flags.contains(A.StateType.EDITABLE),
                "showing": flags.contains(A.StateType.SHOWING), "visible": flags.contains(A.StateType.VISIBLE),
                "enabled": flags.contains(A.StateType.ENABLED),
                "caret": caret, "total_chars": count, "selection_count": selections,
                "geometry": geometry, "node": node, "flatten": flatten, "blocks": rich["blocks"] if rich else None}

    def caret_position(self, node, flatten):
        """(caret, length, selections, rich layout or None) in the field's flattened coordinates."""
        self.budget()
        text = node.get_text_iface()
        caret, count, selections = text.get_caret_offset(), text.get_character_count(), text.get_n_selections()
        rich = self.rich_layout(node, count, caret) if flatten else None
        if rich is not None:
            caret, count, selections = rich["caret"], rich["total_chars"], selections + rich["selections"]
        return caret, count, selections, rich

    def position(self, metadata):
        """`metadata` with the same field's caret, length and selections read again."""
        caret, count, selections, rich = self.caret_position(metadata["node"], metadata["flatten"])
        return {**metadata, "caret": caret, "total_chars": count, "selection_count": selections,
                "blocks": rich["blocks"] if rich else None}

    def text(self, metadata, start, end):
        self.budget()
        if not metadata.get("blocks"):
            return self.atspi.Text.get_text(metadata["node"].get_text_iface(), start, end)

        def read(block, first, last):
            self.budget()
            return self.atspi.Text.get_text(block["node"].get_text_iface(), first, last)
        return rich_text(metadata["blocks"], start, end, read)


def rich_text(blocks, start, end, read):
    """Text of [start, end) in a rich root's flattened coordinates.

    Each block is its paragraph text followed by PARAGRAPH_END, exactly as
    Chromium serializes a paragraph that does not end in a line break. One that
    does (an empty ProseMirror paragraph, a trailing hard break) gets only one
    more break from Chromium, which these lengths cannot express without
    reading prose at inspection, so it fails closed whenever its end is in the
    window. `read(block, first, last)` returns block-local text and must not
    contain embedded objects, whose text cannot be resolved.
    """
    parts = []
    for block in blocks:
        first, length = block["start"], block["length"]
        low, high = max(start, first), min(end, first + length)
        value = ""
        if low < high:
            value = read(block, low - first, high - first)
            if not isinstance(value, str) or EMBEDDED_OBJECT in value:
                raise Denied("unsupported_field")
            parts.append(value)
        separator = first + length
        if start < separator + len(PARAGRAPH_END) and separator < end:
            last = value[-1:] if high == separator and value else (read(block, length - 1, length) if length else "")
            if not isinstance(last, str) or len(last) != 1 or last in ("\n", EMBEDDED_OBJECT):
                raise Denied("unsupported_field")
            parts.append(PARAGRAPH_END[max(start - separator, 0):end - separator])
    return "".join(parts)


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
        self.event_bus = Gio.DBusConnection.new_for_address_sync(address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)

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
        """Receive the granted field's own events until the next invalidation."""
        from gi.repository import Gio
        if self.field_binding == binding:
            return
        self.disarm_field_events()
        if self.event_bus is None or self.event_bus.is_closed():
            raise Denied("accessibility_unavailable")
        # Producer interest is first requested after a caller grant in this
        # application and lasts while its window stays active. The match is
        # narrowed to this unique sender and object path, so other fields'
        # events, including their typed text, never arrive.
        if self.producer != binding["bus"]:
            self.withdraw_producer()
            self.producer = binding["bus"]
            for event in FIELD_EVENTS:
                self.registry("RegisterEvent", "(sass)", (event, [], binding["bus"]), sync=True)
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

    def focus_event(self, _event, *_args):
        # Focus moved somewhere: the tracked field may no longer have the keyboard.
        if self.observer.tracked is not None:
            self.observer.invalidate("focus_changed")

    def watch(self):
        directory = hyprland_directory()
        self.backend.hyprland = directory / ".socket.sock"
        self.connect_event_bus()
        self.listener = self.A.EventListener.new(self.focus_event, None)
        self.listener.register("object:state-changed:focused")
        self.hypr = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.hypr.connect(str(directory / ".socket2.sock"))
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
                if event in (b"activewindowv2", b"focusedmon", b"workspacev2", b"movewindowv2", b"closewindow",
                             b"fullscreen", b"changefloatingmode", b"monitoraddedv2", b"monitorremoved"):
                    if self.observer.tracked:
                        self.observer.invalidate("window_changed")
                    self.withdraw_producer()
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
            self.GLib.idle_add(self.warm_preview)
            self.loop.run()
        finally:
            for fd in list(self.clients):
                self.close_client(fd)
            self.disarm_field_events()
            if self.event_bus is not None and not self.event_bus.is_closed():
                self.withdraw_producer()
                self.event_bus.flush_sync(None)
                self.event_bus.close_sync(None)
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
        print("Badi accessibility observer: startup_or_runtime_unavailable", file=__import__("sys").stderr)
        return 1
    finally:
        Atspi.exit()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
