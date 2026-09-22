"""A non-interactive Wayland preview; the observer owns text/focus authority."""
from __future__ import annotations

from dataclasses import dataclass
import math
import unicodedata


def _number(value, minimum=-100000, maximum=100000):
    return type(value) in (int, float) and math.isfinite(value) and minimum <= value <= maximum


def _arabic_letter(value):
    return any(low <= ord(value) <= high for low, high in (
        (0x0620, 0x063F), (0x0641, 0x064A), (0x066E, 0x066F),
        (0x0671, 0x06D3), (0x06D5, 0x06D5), (0x06E5, 0x06E6),
        (0x06EE, 0x06EF), (0x06FA, 0x06FC), (0x06FF, 0x06FF)))


def valid_preview_text(text):
    if not isinstance(text, str) or not 1 <= len(text) <= 64 or not text.strip():
        return False
    for index, char in enumerate(text):
        if char == "\u200c":
            if not (0 < index < len(text) - 1 and _arabic_letter(text[index - 1]) and _arabic_letter(text[index + 1])):
                return False
        elif unicodedata.category(char).startswith("C") or (char.isspace() and char != " "):
            return False
    return True


@dataclass(frozen=True)
class CaretGeometry:
    connector: str
    monitor: tuple[float, float, float, float]
    window: tuple[float, float, float, float]
    x: float
    y: float
    line_height: float


def mapped_caret(focus):
    """Normalize only the physically checked Chromium/Wayland convention."""
    try:
        binding, geometry = focus["binding"], focus["geometry"]
        window, monitor, raw = geometry["window"], geometry["monitor"], geometry["raw_character"]
        scale = geometry["scale"]
        if (binding["app_id"] != "chromium" or type(binding["process_id"]) is not int or binding["process_id"] <= 0
                or window["pid"] != binding["process_id"] or window["xwayland"] is not False
                or geometry["coordinate_convention"] != "chromium151_wayland_window_physical"
                or geometry["caret_edge"] != "right"
                or geometry["requested_space"] != "atspi_screen" or geometry["normalized"] is not False
                or not _number(scale, .5, 4) or monitor["scale"] != scale or monitor["transform"] != 0
                or not isinstance(monitor["name"], str) or not monitor["name"] or len(monitor["name"]) > 64
                or type(focus["caret"]) is not int or focus["caret"] < 1
                or type(geometry["character_offset"]) is not int
                or geometry["character_offset"] != focus["caret"] - 1):
            return None
        at, size = window["at"], window["size"]
        if (len(at) != 2 or len(size) != 2 or not all(_number(v) for v in at)
                or not all(_number(v, 1, 32768) for v in size)
                or not all(_number(monitor[key]) for key in ("x", "y"))
                or not all(_number(monitor[key], 1, 32768) for key in ("width", "height"))
                or not all(_number(raw[key], 0, 65536) for key in ("x", "y", "width", "height"))
                or raw["height"] < 1 or raw["x"] + raw["width"] > size[0] * scale
                or raw["y"] + raw["height"] > size[1] * scale):
            return None
        result = CaretGeometry(monitor["name"],
            (monitor["x"], monitor["y"], monitor["width"] / scale, monitor["height"] / scale),
            (at[0], at[1], size[0], size[1]),
            at[0] + (raw["x"] + raw["width"]) / scale,
            at[1] + raw["y"] / scale, raw["height"] / scale)
        mx, my, mw, mh = result.monitor
        if not (mx <= result.x <= mx + mw and my <= result.y < my + mh and 4 <= result.line_height <= 128):
            return None
        return result
    except (KeyError, TypeError, IndexError):
        return None


def place_pill(caret, width, height):
    """Return monitor-local coordinates without clipping or covering another app."""
    if not _number(width, 1, 4096) or not _number(height, 1, 256):
        return None
    mx, my, mw, mh = caret.monitor
    wx, wy, ww, wh = caret.window
    left, top = max(mx, wx) + 2, max(my, wy) + 2
    right, bottom = min(mx + mw, wx + ww) - 2, min(my + mh, wy + wh) - 2
    if width > right - left or height > bottom - top:
        return None
    x, y = caret.x + 4, caret.y + (caret.line_height - height) / 2
    if x + width > right:
        x, y = max(left, min(caret.x, right - width)), caret.y + caret.line_height + 4
    if y + height > bottom:
        y = caret.y - height - 4
    y = max(top, y)
    if x < left or x + width > right or y + height > bottom:
        return None
    return round(x - mx), round(y - my)


class Preview:
    def __init__(self):
        # Upstream requires this library before GTK loads libwayland-client.
        # Loading it after gi.repository.Gtk silently disables layer surfaces.
        from ctypes import CDLL
        CDLL("libgtk4-layer-shell.so.0")
        import cairo
        import gi
        gi.require_version("Gtk", "4.0")
        gi.require_version("Gdk", "4.0")
        gi.require_version("Gtk4LayerShell", "1.0")
        from gi.repository import Gdk, GLib, Gtk, Gtk4LayerShell, Pango
        Gtk.init()
        if not Gtk4LayerShell.is_supported() or Gdk.Display.get_default() is None:
            raise RuntimeError("preview_wayland_unavailable")
        self.Gdk, self.GLib, self.Gtk, self.Layer = Gdk, GLib, Gtk, Gtk4LayerShell
        self.cairo, self.Pango = cairo, Pango
        self.timer = None
        self.window = Gtk.Window()
        self.window.set_title("Badi prediction")
        self.window.set_decorated(False)
        self.window.set_resizable(False)
        self.window.set_focusable(False)
        self.window.set_can_target(False)
        self.window.set_default_size(1, 1)
        self.window.add_css_class("badi-writing-preview")
        self.label = Gtk.Label()
        self.label.set_use_markup(False)
        self.label.set_selectable(False)
        self.label.set_single_line_mode(True)
        self.label.set_can_target(False)
        self.label.set_xalign(0)
        self.window.set_child(self.label)
        self.css = Gtk.CssProvider()
        self.css.load_from_string("""
            window.badi-writing-preview {
                background: rgba(39, 42, 48, 0.86);
                border: 1px solid rgba(255, 255, 255, 0.30);
                border-radius: 10px;
                box-shadow: none;
            }
            window.badi-writing-preview label {
                color: rgba(234, 236, 240, 0.97);
                padding: 5px 10px;
                font-weight: 400;
            }
        """)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), self.css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        Gtk4LayerShell.init_for_window(self.window)
        Gtk4LayerShell.set_namespace(self.window, "badi-writing-preview")
        Gtk4LayerShell.set_layer(self.window, Gtk4LayerShell.Layer.OVERLAY)
        Gtk4LayerShell.set_keyboard_mode(self.window, Gtk4LayerShell.KeyboardMode.NONE)
        Gtk4LayerShell.set_exclusive_zone(self.window, -1)
        Gtk4LayerShell.set_anchor(self.window, Gtk4LayerShell.Edge.LEFT, True)
        Gtk4LayerShell.set_anchor(self.window, Gtk4LayerShell.Edge.TOP, True)
        self.window.connect("realize", self._input_transparent)
        self.window.connect("map", self._input_transparent)

    def _input_transparent(self, *_args):
        surface = self.window.get_surface()
        if surface is not None:
            surface.set_input_region(self.cairo.Region())

    def hide(self):
        if self.timer is not None:
            self.GLib.source_remove(self.timer)
            self.timer = None
        self.window.set_visible(False)
        self.label.set_text("")

    def _expire(self):
        self.timer = None
        self.hide()
        return False

    def render(self, focus, text, ttl_ms):
        self.hide()
        if not valid_preview_text(text) or type(ttl_ms) is not int or not 1 <= ttl_ms <= 5000:
            return False
        caret = mapped_caret(focus)
        if caret is None:
            return False
        monitors = self.Gdk.Display.get_default().get_monitors()
        matching = []
        for index in range(monitors.get_n_items()):
            monitor = monitors.get_item(index)
            rect = monitor.get_geometry()
            if (monitor.get_connector() == caret.connector and
                    all(abs(a - b) <= 1 for a, b in zip((rect.x, rect.y, rect.width, rect.height), caret.monitor))):
                matching.append(monitor)
        if len(matching) != 1:
            return False
        self.Layer.set_monitor(self.window, matching[0])
        self.label.set_text(text)
        attributes = self.Pango.AttrList()
        attributes.insert(self.Pango.attr_size_new_absolute(round(max(12, min(20, caret.line_height * .85)) * self.Pango.SCALE)))
        self.label.set_attributes(attributes)
        width = self.window.measure(self.Gtk.Orientation.HORIZONTAL, -1).natural
        height = self.window.measure(self.Gtk.Orientation.VERTICAL, width).natural
        position = place_pill(caret, width, height)
        if position is None:
            self.label.set_text("")
            return False
        self.Layer.set_margin(self.window, self.Layer.Edge.LEFT, position[0])
        self.Layer.set_margin(self.window, self.Layer.Edge.TOP, position[1])
        self.window.set_visible(True)
        self.timer = self.GLib.timeout_add(ttl_ms, self._expire)
        return True
