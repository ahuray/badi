"""Window-local logical caret geometry from AT-SPI SCREEN extents.

Every preview calibrates again; nothing is cached. Any inconsistency yields
None, and the addon then uses the Fcitx panel.
"""
from __future__ import annotations

import math

CALIBRATED = "atspi_frame_calibrated"
SCALE_TOLERANCE = 0.01
EDGE_TOLERANCE = 1
# Gecko's glyph extents are device pixels. Only integer scales keep device
# pixels an exact multiple of logical ones; other scales use the Fcitx panel.
GECKO_SCALES = (1, 2)
RECT_KEYS = ("x", "y", "width", "height")
MAX_COORDINATE = 10**7


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

        return _reply(output, caret, local(glyph["x"] + glyph["width"], left), local(glyph["y"], top),
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
        logical = {key: glyph[key] / scale for key in RECT_KEYS}
        if not _inside(document, frame) or not _inside(field, document) or not _inside(logical, field):
            return None
        return _reply(output, caret, round(logical["x"] + logical["width"], 2), round(logical["y"], 2),
                      round(logical["height"], 2), {key: round(field[key], 2) for key in RECT_KEYS}, window)
    except (KeyError, TypeError, ValueError, ZeroDivisionError):
        return None


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


def _output(frame, document, field, glyph, window, monitors, caret):
    """(monitor, scale, window width, window height) when the inputs are usable."""
    if not all(_is_rect(value) for value in (frame, document, field, glyph)) or type(caret) is not int or caret < 1:
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


def _reply(output, caret, x, y, line, field, window):
    """The geometry reply; the caret must lie inside Hyprland's window size."""
    monitor, scale, width, height = output
    if not (0 <= x <= width and 0 <= y and y + line <= height):
        return None
    return {"coordinate_convention": CALIBRATED, "scale": scale, "character_offset": caret - 1,
            "caret": {"x": x, "y": y, "height": line}, "field": field,
            "window": {key: window[key] for key in ("pid", "address", "at", "size", "monitor", "xwayland")},
            "monitor": {key: monitor[key] for key in ("name", "x", "y", "width", "height", "scale", "transform")}}


def _is_rect(value):
    return (isinstance(value, dict) and all(type(value.get(key)) in (int, float) and math.isfinite(value[key])
            and abs(value[key]) <= MAX_COORDINATE for key in RECT_KEYS)
            and value["width"] >= 0 and value["height"] >= 0)


def _inside(inner, outer):
    return (inner["x"] >= outer["x"] - EDGE_TOLERANCE and inner["y"] >= outer["y"] - EDGE_TOLERANCE and
            inner["x"] + inner["width"] <= outer["x"] + outer["width"] + EDGE_TOLERANCE and
            inner["y"] + inner["height"] <= outer["y"] + outer["height"] + EDGE_TOLERANCE)
