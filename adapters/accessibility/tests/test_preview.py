from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from preview import Preview, font_pixels, ltr_text, mapped_caret, place_inline, valid_preview_text


def focus():
    return {"binding": {"app_id": "chromium", "process_id": 42}, "caret": 12,
        "geometry": {"coordinate_convention": "atspi_frame_calibrated", "caret_edge": "right",
            "scale": 2, "character_offset": 11,
            "caret": {"x": 110, "y": 50, "height": 20},
            "field": {"x": 20, "y": 40, "width": 600, "height": 40},
            "window": {"pid": 42, "xwayland": False, "at": [100, 100], "size": [900, 650]},
            "monitor": {"name": "eDP-1", "x": 0, "y": 0, "width": 2880, "height": 1800,
                        "scale": 2, "transform": 0}}}


class PreviewGeometryTests(unittest.TestCase):
    def test_window_local_caret_maps_once_into_global_logical_coordinates(self):
        caret = mapped_caret(focus())
        self.assertEqual((caret.x, caret.y, caret.line_height, caret.field_right), (210, 150, 20, 720))
        self.assertEqual(caret.monitor, (0, 0, 1440, 900))
        # Right after the caret; the text box's baseline lands on the glyph's.
        self.assertEqual(place_inline(caret, 200, 24, 19), (211, 147))

    def test_output_origin_is_removed_once_for_layer_shell_margins(self):
        data = focus()
        data["geometry"]["window"]["at"] = [1540, 100]
        data["geometry"]["monitor"]["x"] = 1440
        caret = mapped_caret(data)
        self.assertEqual(caret.x, 1650)
        self.assertEqual(place_inline(caret, 200, 24, 19), (211, 147))

    def test_any_verified_application_uses_the_same_calibrated_convention(self):
        data = focus()
        data["binding"]["app_id"] = "code"
        self.assertIsNotNone(mapped_caret(data))

    def test_unknown_stale_or_inconsistent_coordinate_metadata_fails_closed(self):
        changes = [
            (("binding", "process_id"), 999),
            (("geometry", "coordinate_convention"), "chromium151_wayland_window_physical"),
            (("geometry", "caret_edge"), "left"),
            (("geometry", "character_offset"), 10),
            (("geometry", "scale"), float("nan")),
            (("geometry", "scale"), 1),
            (("geometry", "window", "xwayland"), True),
            (("geometry", "monitor", "transform"), 1),
            (("geometry", "caret", "x"), -1),
            (("geometry", "caret", "x"), 901),
            (("geometry", "caret", "x"), True),
            (("geometry", "caret", "y"), 640),
            (("geometry", "caret", "height"), 0),
            (("geometry", "caret", "height"), 200),
            (("geometry", "field", "x"), 111),
            (("geometry", "field", "width"), 80),
            (("geometry", "field", "width"), float("inf")),
        ]
        for path, value in changes:
            data = focus()
            destination = data
            for key in path[:-1]:
                destination = destination[key]
            destination[path[-1]] = value
            self.assertIsNone(mapped_caret(data), path)
        data = focus()
        del data["geometry"]["field"]
        self.assertIsNone(mapped_caret(data))
        for data in [None, {}, {"binding": {}}, {"binding": {}, "geometry": None}]:
            self.assertIsNone(mapped_caret(data))

    def test_inline_text_must_end_inside_the_field_window_and_monitor(self):
        data = focus()
        data["geometry"]["caret"]["x"] = 600
        caret = mapped_caret(data)
        self.assertEqual(place_inline(caret, 18, 24, 19), (701, 147))
        self.assertIsNone(place_inline(caret, 20, 24, 19), "past the field's right edge")
        data["geometry"]["field"]["width"] = 5000
        self.assertIsNone(place_inline(mapped_caret(data), 300, 24, 19), "past the window's right edge")
        data = focus()
        data["geometry"]["caret"]["y"] = 628
        self.assertIsNone(place_inline(mapped_caret(data), 100, 36, 24), "below the window")
        data = focus()
        data["geometry"]["window"]["at"] = [100, 0]
        data["geometry"]["caret"]["y"] = 0
        self.assertIsNone(place_inline(mapped_caret(data), 100, 40, 36), "above the window")
        caret = mapped_caret(focus())
        for size in ((0, 24, 19), (200, 24, 0), (200, 24, 30), (float("inf"), 24, 19), (200, float("nan"), 19)):
            self.assertIsNone(place_inline(caret, *size), size)

    def test_font_follows_the_caret_glyph_and_text_is_ltr_only(self):
        self.assertAlmostEqual(font_pixels(mapped_caret(focus())), 17.2)
        data = focus()
        data["geometry"]["caret"]["height"] = 4
        self.assertEqual(font_pixels(mapped_caret(data)), 6)
        data["geometry"]["caret"]["height"] = 128
        self.assertEqual(font_pixels(mapped_caret(data)), 72)
        for text in (" the next words", "teh → the", "v2.0 release", "één woord"):
            self.assertTrue(ltr_text(text), text)
        for text in ("می‌توانیم", "שלום", " word ١٢"):
            self.assertFalse(ltr_text(text), text)

    def test_text_is_plain_bounded_and_excludes_hidden_controls(self):
        for text in [" the next words", "teh → the", "می‌توانیم", "<b>literal text</b>", "één woord"]:
            self.assertTrue(valid_preview_text(text), text)
        for text in [None, "", " ", "x" * 65, "line\nline", "x\t", "x\u202ey", "می‌ شود", "\ud800"]:
            self.assertFalse(valid_preview_text(text), repr(text))

    def render_fixture(self, natural=(200, 24), baseline=19):
        preview = object.__new__(Preview)
        calls = []
        monitor = SimpleNamespace(get_connector=lambda: "eDP-1",
                                  get_geometry=lambda: SimpleNamespace(x=0, y=0, width=1440, height=900))
        monitors = SimpleNamespace(get_n_items=lambda: 1, get_item=lambda _index: monitor)
        preview.Gdk = SimpleNamespace(Display=SimpleNamespace(get_default=lambda: SimpleNamespace(get_monitors=lambda: monitors)))
        preview.GLib = SimpleNamespace(source_remove=Mock(), timeout_add=lambda ttl, _callback: calls.append(("timer", ttl)) or 7)
        preview.Gtk = SimpleNamespace(Orientation=SimpleNamespace(HORIZONTAL="h", VERTICAL="v"))
        preview.Pango = SimpleNamespace(SCALE=1024, AttrList=lambda: SimpleNamespace(insert=lambda attr: calls.append(("size", attr))),
                                        attr_size_new_absolute=lambda size: size)
        preview.Layer = SimpleNamespace(Edge=SimpleNamespace(LEFT="left", TOP="top"),
                                        set_monitor=lambda _window, chosen: calls.append(("monitor", chosen)),
                                        set_margin=lambda _window, edge, value: calls.append((edge, value)))
        sizes = {"h": natural[0], "v": natural[1]}
        preview.window = SimpleNamespace(set_visible=lambda visible: calls.append(("visible", visible)),
                                         measure=lambda orientation, _for: SimpleNamespace(natural=sizes[orientation]))
        preview.label = SimpleNamespace(set_text=lambda text: calls.append(("text", text)),
                                        set_attributes=lambda _attrs: None,
                                        get_layout=lambda: SimpleNamespace(get_baseline=lambda: baseline * 1024))
        preview.timer = None
        return preview, calls, monitor

    def test_render_places_inline_text_after_the_caret_and_arms_expiry(self):
        preview, calls, monitor = self.render_fixture()
        self.assertTrue(preview.render(focus(), " next words", 1500))
        self.assertEqual(calls[:2], [("visible", False), ("text", "")], "a render first hides the old preview")
        self.assertIn(("monitor", monitor), calls)
        self.assertIn(("size", round(17.2 * 1024)), calls)
        self.assertIn(("left", 211), calls)
        self.assertIn(("top", 147), calls)
        self.assertEqual(calls[-2:], [("visible", True), ("timer", 1500)])
        self.assertEqual(preview.timer, 7)

    def test_render_refuses_rtl_unmapped_or_unfitting_text(self):
        for text, data, natural in ((" متن", focus(), (200, 24)), (" next", {**focus(), "geometry": None}, (200, 24)),
                                    (" next", focus(), (700, 24))):
            preview, calls, _monitor = self.render_fixture(natural)
            self.assertFalse(preview.render(data, text, 1500))
            self.assertNotIn(("visible", True), calls)
            self.assertFalse(any(call[0] == "timer" for call in calls))

    def test_warming_lays_out_text_without_showing_it(self):
        preview, calls, _monitor = self.render_fixture()
        preview.warm()
        self.assertEqual(calls, [("text", "Badi"), ("text", "")])
        self.assertIsNone(preview.timer)

    def test_hide_and_expiry_remove_text_and_old_timer_authority(self):
        preview = object.__new__(Preview)
        preview.GLib = SimpleNamespace(source_remove=Mock())
        preview.window = SimpleNamespace(set_visible=Mock())
        preview.label = SimpleNamespace(set_text=Mock())
        preview.timer = 12
        preview.hide()
        preview.GLib.source_remove.assert_called_once_with(12)
        preview.window.set_visible.assert_called_once_with(False)
        preview.label.set_text.assert_called_once_with("")
        self.assertIsNone(preview.timer)
        preview.GLib.source_remove.reset_mock()
        preview.timer = 13
        self.assertFalse(preview._expire())
        preview.GLib.source_remove.assert_not_called()
        self.assertIsNone(preview.timer)


if __name__ == "__main__":
    unittest.main()
