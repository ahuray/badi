"""A non-interactive Wayland preview; the observer owns text/focus authority."""
from __future__ import annotations

from dataclasses import dataclass
import math
import subprocess
from threading import Thread
import time
import unicodedata

from contract import MAX_PREVIEW_TTL_MS, RIGHT_TO_LEFT, suggestion_text
from geometry import CALIBRATED
import debugtrace as trace


def _number(value, minimum=-100000, maximum=100000):
    return type(value) in (int, float) and math.isfinite(value) and minimum <= value <= maximum


# Space between a chip's edge and its text: (horizontal, vertical).
CHIP_PADDING = (6, 2)
# The chip's text starts at the caret: its padding plus one pixel.
CHIP_INSET = CHIP_PADDING[0] + 1
# Space between the caret's line and a chip above or below it.
CHIP_GAP = 2


@dataclass(frozen=True)
class CaretGeometry:
    connector: str
    monitor: tuple[float, float, float, float]
    window: tuple[float, float, float, float]
    x: float
    y: float
    line_height: float
    field_left: float
    field_top: float
    field_right: float
    field_bottom: float
    text_follows: bool
    scale: float = 1.0
    following_x: float | None = None
    text_ends: bool = False


def ltr_text(text):
    return not any(unicodedata.bidirectional(char) in RIGHT_TO_LEFT for char in text)


def mapped_caret(focus):
    """Global logical caret for the observer's per-request calibrated geometry."""
    try:
        binding, geometry = focus["binding"], focus["geometry"]
        window, monitor, local, field = geometry["window"], geometry["monitor"], geometry["caret"], geometry["field"]
        scale = geometry["scale"]
        if (type(binding["process_id"]) is not int or binding["process_id"] <= 0
                or window["pid"] != binding["process_id"] or window["xwayland"] is not False
                or geometry["coordinate_convention"] != CALIBRATED
                or geometry["caret_edge"] != "right"
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
                or not all(_number(local[key]) for key in ("x", "y", "height"))
                or not all(_number(field[key]) for key in ("x", "y", "width", "height"))
                or not 4 <= local["height"] <= 128 or field["width"] < 0
                or not 0 <= local["x"] <= size[0] or local["y"] < 0 or local["y"] + local["height"] > size[1]
                or not field["x"] <= local["x"] <= field["x"] + field["width"] + 1):
            return None
        following = geometry.get("following_x")
        if following is not None and not (_number(following) and local["x"] < following <= size[0]):
            following = None
        result = CaretGeometry(monitor["name"],
            (monitor["x"], monitor["y"], monitor["width"] / scale, monitor["height"] / scale),
            (at[0], at[1], size[0], size[1]),
            at[0] + local["x"], at[1] + local["y"], local["height"],
            at[0] + max(field["x"], 0), at[1] + max(field["y"], 0),
            at[0] + min(field["x"] + field["width"], size[0]),
            at[1] + min(field["y"] + field["height"], size[1]),
            geometry.get("text_follows") is True, float(scale),
            None if following is None else at[0] + following,
            geometry.get("text_ends") is True and geometry.get("text_follows") is not True)
        mx, my, mw, mh = result.monitor
        if not (mx <= result.x <= mx + mw and my <= result.y < my + mh):
            return None
        return result
    except (KeyError, TypeError, IndexError):
        return None


def font_pixels(caret, font=None):
    """The field's reported size when it is usable, otherwise its glyph box."""
    fallback = max(6, min(72, caret.line_height * .86))
    if not isinstance(font, dict):
        return fallback
    pixels = font.get("size_px")
    if isinstance(pixels, (int, float)) and not isinstance(pixels, bool) and math.isfinite(pixels):
        return max(6, min(72, float(pixels)))
    points = font.get("size_pt")
    if isinstance(points, (int, float)) and not isinstance(points, bool) and math.isfinite(points):
        return max(6, min(72, float(points) * 96 / 72))
    return fallback


def _snap(value, scale):
    """Nearest integer logical coordinate on the physical pixel grid.

    Layer margins are logical integers; at a fractional scale most of them
    land between physical pixels and the ghost renders shifted or blurred
    against the field's own text. Prefer the integer whose physical
    position is exact, breaking ties toward the true float position.
    """
    if not _number(scale, .5, 4) or scale == 1:
        return round(value)
    candidates = {math.floor(value), math.ceil(value), round(value)}
    return min(candidates, key=lambda c: (abs(c * scale - round(c * scale)), abs(c - value)))


def _baseline_y(caret, height, baseline, field_ascent):
    """Ghost top so its baseline lands on the field's baseline.

    With the field font's real ascent, the field baseline sits that far
    below the glyph box top. Without it, the fallback scales the layout's own
    baseline ratio onto the glyph box.
    """
    if field_ascent is not None and _number(field_ascent, 1, 256):
        return caret.y + field_ascent - baseline
    return caret.y + caret.line_height * baseline / height - baseline


def place_inline(caret, width, height, baseline, field_ascent=None):
    """Monitor-local origin for text right after the caret on its baseline.

    The text must end inside the field, window and monitor. A field box that
    ends early is handled by `place_on_caret_line`.
    """
    if not _number(width, 1, 4096) or not _number(height, 1, 256) or not _number(baseline, 1, height):
        return None
    mx, my, mw, mh = caret.monitor
    wx, wy, ww, wh = caret.window
    # Check the actual margins after snapping, including the next glyph bound.
    x = mx + _snap(caret.x + 1 - mx, caret.scale)
    y = my + _snap(_baseline_y(caret, height, baseline, field_ascent) - my, caret.scale)
    right = min(caret.field_right, wx + ww, mx + mw)
    if caret.following_x is not None:
        right = min(right, caret.following_x)
    if (x < max(mx, wx) or x + width > right
            or y < max(my, wy) or y + height > min(my + mh, wy + wh)):
        return None
    return round(x - mx), round(y - my)


def field_box_ends_before_caret(caret):
    """The reported field ends before the caret, so it is not the visible line.

    A box that still contains the caret is a real edge: text that passes it
    must not be drawn over the controls beside the field.
    """
    return caret.x + 1 > caret.field_right


def place_on_caret_line(caret, width, height, baseline, field_ascent=None):
    """Ghost text on the caret's baseline, inside the window.

    Only for a field box that ends before the caret. A chip under that box
    lands on the chrome beneath the input and stays there.
    """
    if not _number(width, 1, 4096) or not _number(height, 1, 256) or not _number(baseline, 1, height):
        return None
    mx, my, mw, mh = caret.monitor
    wx, wy, ww, wh = caret.window
    x = mx + _snap(caret.x + 1 - mx, caret.scale)
    y = my + _snap(_baseline_y(caret, height, baseline, field_ascent) - my, caret.scale)
    left, right = max(mx, wx), min(mx + mw, wx + ww)
    top, bottom = max(my, wy), min(my + mh, wy + wh)
    if x < left or x + width > right or y < top or y + height > bottom:
        return None
    return round(x - mx), round(y - my)


def place_chip(caret, width, height, below_only=False):
    """Monitor-local origin for a compact chip under the caret, inside the field.

    Inline text would cover the words after the caret, or would not fit. The
    chip stays in the field: below the line when it fits, above it when it
    does not and `below_only` is unset. It does not step outside the field
    onto the window chrome.
    """
    if not _number(width, 1, 4096) or not _number(height, 1, 256):
        return None
    mx, my, mw, mh = caret.monitor
    wx, wy, ww, wh = caret.window
    left, right = max(mx, wx), min(mx + mw, wx + ww)
    top, bottom = max(my, wy), min(my + mh, wy + wh)
    # The reported box includes the field's border; keep the chip off it.
    field_left, field_right = max(left, caret.field_left + CHIP_GAP), min(right, caret.field_right - CHIP_GAP)
    field_top, field_bottom = max(top, caret.field_top), min(bottom, caret.field_bottom)
    if width > field_right - field_left:
        return None
    x = min(max(caret.x - CHIP_INSET, field_left), field_right - width)
    x = mx + _snap(x - mx, caret.scale)
    if x < field_left or x + width > field_right:
        return None
    below, above = caret.y + caret.line_height + CHIP_GAP, caret.y - CHIP_GAP - height
    for y in (below,) if below_only else (below, above):
        y = my + _snap(y - my, caret.scale)
        if field_top <= y and y + height <= field_bottom:
            return round(x - mx), round(y - my)
    return None


def place_edge_panel(caret, width, height):
    """A quiet overflow panel outside the field, bounded by window and monitor.

    Keep the whole surface below or above the field so it cannot cover the
    field's neighboring button or be mistaken for inserted inline text.
    """
    if not _number(width, 1, 4096) or not _number(height, 1, 256):
        return None
    mx, my, mw, mh = caret.monitor
    wx, wy, ww, wh = caret.window
    left, right = max(mx, wx), min(mx + mw, wx + ww)
    top, bottom = max(my, wy), min(my + mh, wy + wh)
    if width > right - left:
        return None
    x = min(max(caret.x - CHIP_INSET, left), right - width)
    x = mx + _snap(x - mx, caret.scale)
    if x < left or x + width > right:
        return None
    for proposed in (caret.field_bottom + CHIP_GAP, caret.field_top - CHIP_GAP - height):
        y = my + _snap(proposed - my, caret.scale)
        outside = y >= caret.field_bottom + CHIP_GAP or y + height <= caret.field_top - CHIP_GAP
        if outside and top <= y and y + height <= bottom:
            return round(x - mx), round(y - my)
    return None


LAYER_NAMESPACE = "badi-writing-preview"
# Hyprland (Omarchy) animates layer surfaces in by default; a ghost that
# fades/slides reads as hovering and arrives late. The rule is best-effort:
# other compositors simply have no hyprctl. Re-applied at most this often,
# since a compositor config reload drops runtime rules.
RULE_REFRESH_SECONDS = 30.0


def disable_preview_animation(run=subprocess.run):
    """Ask Hyprland not to animate the preview layer; True when it agreed."""
    try:
        result = run(
            ["hyprctl", "eval",
             'hl.layer_rule({ name = "badi-preview-noanim", '
             f'match = {{ namespace = "^{LAYER_NAMESPACE}$" }}, no_anim = true }})'],
            capture_output=True, timeout=2, check=False)
        return result.returncode == 0
    except (OSError, subprocess.SubprocessError):
        return False


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
        if Gdk.Display.get_default() is None or not Gtk4LayerShell.is_supported():
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
        # Inline ghost text: mid grey stays readable on light and dark fields.
        # A chip carries its own dark background, since it may lie over text.
        # Nothing here changes a size: a class toggled just before measuring
        # is not restyled yet, so the chip's padding is a label margin.
        self.css.load_from_string("""
            window.badi-writing-preview {
                background: none;
                background-color: transparent;
                border: none;
                box-shadow: none;
            }
            window.badi-writing-preview label {
                color: rgba(128, 128, 128, 0.85);
                background: none;
                padding: 0;
                margin: 0;
                font-weight: 400;
            }
            window.badi-writing-preview.chip {
                background-color: rgba(28, 30, 35, 0.94);
                border-radius: 3px;
            }
            window.badi-writing-preview.chip label {
                color: rgba(176, 176, 176, 0.96);
            }
        """)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), self.css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        Gtk4LayerShell.init_for_window(self.window)
        Gtk4LayerShell.set_namespace(self.window, LAYER_NAMESPACE)
        self._rule_applied_at = None
        self._refresh_layer_rule(background=False)
        Gtk4LayerShell.set_layer(self.window, Gtk4LayerShell.Layer.OVERLAY)
        Gtk4LayerShell.set_keyboard_mode(self.window, Gtk4LayerShell.KeyboardMode.NONE)
        Gtk4LayerShell.set_exclusive_zone(self.window, -1)
        Gtk4LayerShell.set_anchor(self.window, Gtk4LayerShell.Edge.LEFT, True)
        Gtk4LayerShell.set_anchor(self.window, Gtk4LayerShell.Edge.TOP, True)
        self.window.connect("realize", self._input_transparent)
        self.window.connect("map", self._input_transparent)
        self.window.connect("map", self._mapped)
        self.shown = None

    def _refresh_layer_rule(self, *, background=True):
        now = time.monotonic()
        previous = getattr(self, "_rule_applied_at", None)
        if previous is not None and now - previous < RULE_REFRESH_SECONDS:
            return
        self._rule_applied_at = now
        if background:
            # Compositor IPC must not block preview expiry or field invalidation.
            # The helper's two-second timeout is shorter than the refresh interval.
            Thread(target=disable_preview_animation, daemon=True).start()
        else:
            disable_preview_animation()

    def _field_ascent(self, font, pixels, line_height=None):
        """Ascent anchoring the field's baseline at the ghost size, or None.

        With a reported family this is that font's real ascent. Without one
        (Qt fields report no font attributes), scale the overlay face's own
        ascent fraction of its ascent+descent onto the field's line height:
        the caret glyph box is one em line, so its baseline sits at that
        fraction — closer than the layout height ratio, which leading skews.
        """
        try:
            context = self.label.get_pango_context()
            if isinstance(font, dict) and isinstance(font.get("family"), str):
                description = self.Pango.FontDescription()
                description.set_family(font["family"])
                weight = font.get("weight")
                if isinstance(weight, int) and not isinstance(weight, bool):
                    description.set_weight(weight)
            elif _number(line_height, 4, 128):
                description = context.get_font_description().copy()
            else:
                return None
            description.set_absolute_size(round(pixels * self.Pango.SCALE))
            metrics = context.get_metrics(description, None)
            ascent = metrics.get_ascent() / self.Pango.SCALE
            if not _number(ascent, 1, 256):
                return None
            if isinstance(font, dict) and isinstance(font.get("family"), str):
                return ascent
            descent = metrics.get_descent() / self.Pango.SCALE
            if not _number(descent, 0, 256) or ascent + descent <= 0:
                return None
            scaled = ascent * line_height / (ascent + descent)
            return scaled if _number(scaled, 1, 256) else None
        except (AttributeError, TypeError, ValueError):
            return None

    def _input_transparent(self, *_args):
        surface = self.window.get_surface()
        if surface is not None:
            surface.set_input_region(self.cairo.Region())

    def _layout(self, text, chip):
        """(width, height, baseline) of `text` drawn inline or as a chip."""
        if chip:
            self.window.add_css_class("chip")
        else:
            self.window.remove_css_class("chip")
        across, down = CHIP_PADDING if chip else (0, 0)
        self.label.set_margin_start(across)
        self.label.set_margin_end(across)
        self.label.set_margin_top(down)
        self.label.set_margin_bottom(down)
        self.label.set_text(text)
        width = self.window.measure(self.Gtk.Orientation.HORIZONTAL, -1).natural
        height = self.window.measure(self.Gtk.Orientation.VERTICAL, width).natural
        return width, height, self.label.get_layout().get_baseline() / self.Pango.SCALE

    def warm(self):
        """Load fonts and the layout path once, before any request's deadline."""
        self.label.set_text("Badi")
        self.window.measure(self.Gtk.Orientation.HORIZONTAL, -1)
        self.label.get_layout().get_baseline()
        self.label.set_text("")

    def hide(self, cause="withdrawn"):
        if self.timer is not None:
            self.GLib.source_remove(self.timer)
            self.timer = None
        self.window.set_visible(False)
        self.label.set_text("")
        if getattr(self, "shown", None) is not None:
            trace.emit("preview hidden", {**self.shown, "cause": cause})
            self.shown = None

    def _expire(self):
        self.timer = None
        self.hide("expired")
        return False

    def _mapped(self, *_args):
        """GTK mapped the layer surface. Mapping happens inside render(), before
        its trace line, so it is reported once the main loop is idle again; the
        compositor's own box is the `badi debug watch` line."""
        self.GLib.idle_add(lambda: trace.emit("preview mapped", {"mapped": self.window.get_mapped()}) and False)

    def render(self, focus, text, ttl_ms):
        """Draws `text` at the caret; false when the field's geometry declines it."""
        self.hide("replaced")
        details = self._render(focus, text, ttl_ms)
        if "reason" in details:
            trace.emit("preview declined", details)
            return False
        self.shown = details
        trace.emit("preview drawn", details, {"text": text})
        return True

    def _render(self, focus, text, ttl_ms):
        """The drawn placement and box, or the `reason` it was declined."""
        # The observer checked the request already; the overlay keeps its own
        # bound on what it draws and for how long, with the same rule.
        if not suggestion_text(text) or type(ttl_ms) is not int or not 1 <= ttl_ms <= MAX_PREVIEW_TTL_MS:
            return {"reason": "invalid_request"}
        caret = mapped_caret(focus)
        if caret is None:
            return {"reason": "uncalibrated_caret"}
        if not ltr_text(text):
            return {"reason": "right_to_left_text"}
        monitor = self._caret_monitor(caret)
        if monitor is None:
            return {"reason": "monitor_unmatched"}
        self.Layer.set_monitor(self.window, monitor)
        font = focus.get("font") if isinstance(focus, dict) else None
        self.label.set_attributes(self._font_attributes(caret, font))
        self._refresh_layer_rule()
        position, placement, width, height = self._place(caret, text, font)
        if position is None:
            self.label.set_text("")
            return {"reason": "no_room"}
        self.Layer.set_margin(self.window, self.Layer.Edge.LEFT, position[0])
        self.Layer.set_margin(self.window, self.Layer.Edge.TOP, position[1])
        self.window.set_visible(True)
        self.timer = self.GLib.timeout_add(ttl_ms, self._expire)
        # Monitor-local logical pixels, as `hyprctl layers` reports them.
        return {"placement": placement, "monitor": caret.connector, "x": position[0], "y": position[1],
                "width": width, "height": height, "ttl_ms": ttl_ms}

    def _caret_monitor(self, caret):
        """The one monitor with the caret's connector and geometry, or None."""
        monitors = self.Gdk.Display.get_default().get_monitors()
        matching = []
        for index in range(monitors.get_n_items()):
            monitor = monitors.get_item(index)
            rect = monitor.get_geometry()
            if (monitor.get_connector() == caret.connector and
                    all(abs(a - b) <= 1 for a, b in zip((rect.x, rect.y, rect.width, rect.height), caret.monitor))):
                matching.append(monitor)
        return matching[0] if len(matching) == 1 else None

    def _font_attributes(self, caret, font):
        """The field's size, and its family and weight when it reports them."""
        attributes = self.Pango.AttrList()
        attributes.insert(self.Pango.attr_size_new_absolute(round(font_pixels(caret, font) * self.Pango.SCALE)))
        if isinstance(font, dict):
            family = font.get("family")
            if isinstance(family, str):
                attributes.insert(self.Pango.attr_family_new(family))
            weight = font.get("weight")
            if isinstance(weight, int) and not isinstance(weight, bool):
                attributes.insert(self.Pango.attr_weight_new(weight))
        return attributes

    def _place(self, caret, text, font):
        """(position, placement, width, height); position is None without room."""
        position = None
        if not caret.text_follows or caret.following_x is not None:
            width, height, baseline = self._layout(text, chip=False)
            ascent = self._field_ascent(font, font_pixels(caret, font), caret.line_height)
            position, placement = place_inline(caret, width, height, baseline, ascent), "inline"
            if position is None and not caret.text_follows and field_box_ends_before_caret(caret):
                position = place_on_caret_line(caret, width, height, baseline, ascent)
                placement = "caret_line"
        if position is None and (caret.text_follows or caret.text_ends):
            # A leading space would draw over the following words. The trailing
            # space is the separator that acceptance inserts, so the chip keeps it.
            # At the end of the text only the empty lines below are free; the
            # lines above hold the user's own words.
            width, height, _baseline = self._layout(text.lstrip(" "), chip=True)
            position = place_chip(caret, width, height, below_only=caret.text_ends)
            placement = "chip"
        if position is None:
            width, height, _baseline = self._layout(text.lstrip(" "), chip=True)
            position, placement = place_edge_panel(caret, width, height), "edge_panel"
        return position, placement, width, height
