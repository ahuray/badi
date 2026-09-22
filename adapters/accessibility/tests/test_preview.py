from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from preview import Preview, mapped_caret, place_pill, valid_preview_text


def focus():
    return {"binding": {"app_id": "chromium", "process_id": 42}, "caret": 12,
        "geometry": {"coordinate_convention": "chromium151_wayland_window_physical",
            "caret_edge": "right",
            "requested_space": "atspi_screen", "normalized": False, "scale": 2,
            "character_offset": 11,
            "raw_character": {"x": 200, "y": 100, "width": 20, "height": 40},
            "window": {"pid": 42, "xwayland": False, "at": [100, 100], "size": [900, 650]},
            "monitor": {"name": "eDP-1", "x": 0, "y": 0, "width": 2880, "height": 1800,
                        "scale": 2, "transform": 0}}}


class PreviewGeometryTests(unittest.TestCase):
    def test_physical_window_glyphs_map_once_into_global_logical_coordinates(self):
        caret = mapped_caret(focus())
        self.assertEqual((caret.x, caret.y, caret.line_height), (210, 150, 20))
        self.assertEqual(caret.monitor, (0, 0, 1440, 900))
        self.assertEqual(place_pill(caret, 200, 32), (214, 144))

    def test_output_origin_is_removed_once_for_layer_shell_margins(self):
        data = focus()
        data["geometry"]["window"]["at"] = [1540, 100]
        data["geometry"]["monitor"]["x"] = 1440
        caret = mapped_caret(data)
        self.assertEqual(caret.x, 1650)
        self.assertEqual(place_pill(caret, 200, 32), (214, 144))

    def test_unknown_stale_or_inconsistent_coordinate_metadata_fails_closed(self):
        changes = [
            (("binding", "app_id"), "chatgpt"),
            (("binding", "process_id"), 999),
            (("geometry", "coordinate_convention"), "atspi_screen"),
            (("geometry", "caret_edge"), "left"),
            (("geometry", "normalized"), True),
            (("geometry", "character_offset"), 10),
            (("geometry", "scale"), float("nan")),
            (("geometry", "scale"), 1),
            (("geometry", "window", "xwayland"), True),
            (("geometry", "monitor", "transform"), 1),
            (("geometry", "raw_character", "x"), -1),
            (("geometry", "raw_character", "width"), 9000),
            (("geometry", "raw_character", "height"), 0),
            (("geometry", "raw_character", "x"), True),
        ]
        for path, value in changes:
            data = focus()
            destination = data
            for key in path[:-1]:
                destination = destination[key]
            destination[path[-1]] = value
            self.assertIsNone(mapped_caret(data), path)
        for data in [None, {}, {"binding": {}}, {"binding": {}, "geometry": None}]:
            self.assertIsNone(mapped_caret(data))

    def test_edge_placement_stays_inside_the_exact_target_window(self):
        data = focus()
        data["geometry"]["raw_character"]["x"] = 1740
        caret = mapped_caret(data)
        self.assertEqual(place_pill(caret, 200, 32), (798, 174))
        self.assertIsNone(place_pill(caret, 1000, 32))
        self.assertIsNone(place_pill(caret, float("inf"), 32))
        data["geometry"]["raw_character"]["y"] = 1240
        caret = mapped_caret(data)
        self.assertEqual(place_pill(caret, 200, 32), (798, 684))

    def test_text_is_plain_bounded_and_excludes_hidden_controls(self):
        for text in [" the next words", "teh → the", "می‌توانیم", "<b>literal text</b>", "één woord"]:
            self.assertTrue(valid_preview_text(text), text)
        for text in [None, "", " ", "x" * 65, "line\nline", "x\t", "x\u202ey", "می‌ شود", "\ud800"]:
            self.assertFalse(valid_preview_text(text), repr(text))

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
