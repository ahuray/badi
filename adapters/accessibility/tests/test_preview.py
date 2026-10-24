from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from field import font_facts
from preview import (REST_ALPHA, STRONG_ALPHA, Preview, disable_preview_animation, field_box_ends_before_caret, font_pixels, ltr_text,
                     mapped_caret, place_chip, place_edge_panel, place_inline, place_on_caret_line)


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
    def setUp(self):
        # Unit rendering must never change the user's running compositor.
        rule = patch("preview.disable_preview_animation", return_value=True)
        self.disable_animation = rule.start()
        self.addCleanup(rule.stop)

    def test_window_local_caret_maps_once_into_global_logical_coordinates(self):
        caret = mapped_caret(focus())
        self.assertEqual((caret.x, caret.y, caret.line_height, caret.field_right), (210, 150, 20, 720))
        self.assertEqual((caret.field_left, caret.field_top, caret.field_bottom), (120, 140, 180))
        self.assertFalse(caret.text_follows)
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

    def test_a_chip_starts_at_the_caret_below_its_line_and_stays_inside_the_window(self):
        data = focus()
        data["geometry"]["text_follows"] = True
        data["geometry"]["field"]["height"] = 200
        caret = mapped_caret(data)
        self.assertTrue(caret.text_follows)
        # Inside a tall field: right under the caret's line, text aligned with the caret.
        self.assertEqual(place_chip(caret, 120, 24), (203, 172))
        # Near the field's right edge it shifts left to stay inside the field,
        # off its border.
        self.assertEqual(place_chip(caret, 590, 24), (128, 172))
        # Wider than the field, or below a one-line field: not on the chrome.
        self.assertIsNone(place_chip(caret, 800, 24), "wider than the field")
        self.assertIsNone(place_chip(caret, 901, 24), "wider than the window")
        data = focus()
        self.assertIsNone(place_chip(mapped_caret(data), 120, 24), "a one-line field has no room under the line")
        data["geometry"]["caret"]["y"] = 610
        data["geometry"]["field"]["y"] = 600
        data["geometry"]["field"]["height"] = 20
        self.assertIsNone(place_chip(mapped_caret(data), 120, 24), "below the field")
        end = mapped_caret(focus())
        self.assertFalse(field_box_ends_before_caret(end))
        self.assertIsNone(place_inline(end, 700, 24, 19), "past the field")
        short = focus()
        short["geometry"]["field"]["width"] = 89
        early = mapped_caret(short)
        self.assertTrue(field_box_ends_before_caret(early))
        self.assertEqual(place_on_caret_line(early, 200, 24, 19), (211, 147))
        for size in ((0, 24), (120, 0), (float("nan"), 24)):
            self.assertIsNone(place_chip(caret, *size), size)

    def test_at_the_end_of_the_text_a_chip_goes_only_under_the_caret_line(self):
        data = focus()
        data["geometry"]["text_ends"] = True
        data["geometry"]["field"]["height"] = 200
        caret = mapped_caret(data)
        self.assertTrue(caret.text_ends)
        self.assertEqual(place_chip(caret, 120, 24, below_only=True), (203, 172))
        # The caret's line at the bottom of the field: above would cover the
        # user's own lines, so there is no chip.
        data["geometry"]["caret"]["y"] = 200
        self.assertIsNone(place_chip(mapped_caret(data), 120, 24, below_only=True))
        self.assertEqual(place_chip(mapped_caret(data), 120, 24), (203, 274))
        # Words after the caret are never "the end".
        data = focus()
        data["geometry"].update(text_ends=True, text_follows=True)
        self.assertFalse(mapped_caret(data).text_ends)
        data["geometry"].update(text_ends="yes", text_follows=False)
        self.assertFalse(mapped_caret(data).text_ends)

    def test_field_ascent_anchors_the_baseline_and_fractional_scale_snaps_to_physical_pixels(self):
        caret = mapped_caret(focus())
        # Ghost top = glyph top + field ascent - layout baseline.
        self.assertEqual(place_inline(caret, 200, 24, 19, 14.0), (211, 145))
        self.assertEqual(place_on_caret_line(caret, 200, 24, 19, 14.0), (211, 145))
        # An unusable ascent falls back to the line-height ratio.
        self.assertEqual(place_inline(caret, 200, 24, 19, float("nan")), (211, 147))
        data = focus()
        data["geometry"]["scale"] = 1.5
        data["geometry"]["monitor"]["scale"] = 1.5
        fractional = mapped_caret(data)
        self.assertEqual(fractional.scale, 1.5)
        # 146.83 logical: 146 lands on an exact physical pixel, 147 does not.
        self.assertEqual(place_inline(fractional, 200, 24, 19), (211, 146))

    def test_edge_panel_stays_outside_the_field_and_inside_the_window(self):
        data = focus()
        caret = mapped_caret(data)
        self.assertEqual(place_edge_panel(caret, 120, 24), (203, 182))
        self.assertEqual(place_edge_panel(caret, 800, 24), (200, 182))
        self.assertIsNone(place_edge_panel(caret, 901, 24))
        data["geometry"]["field"]["height"] = 610
        self.assertEqual(place_edge_panel(mapped_caret(data), 120, 24), (203, 114), 'falls above a field at the bottom')
        data["geometry"]["field"] = {"x": 0, "y": 0, "width": 900, "height": 650}
        self.assertIsNone(place_edge_panel(mapped_caret(data), 120, 24), 'no room outside a full-window editor')

    def test_following_glyph_bounds_inline_text_before_the_next_word(self):
        data = focus()
        data["geometry"]["text_follows"] = True
        data["geometry"]["following_x"] = 420
        caret = mapped_caret(data)
        self.assertEqual(caret.following_x, 520)
        # Caret at x=210: 300 wide ends at 511, before the next glyph at 520.
        self.assertEqual(place_inline(caret, 300, 24, 19), (211, 147))
        self.assertIsNone(place_inline(caret, 310, 24, 19), "would cover the following glyph")
        for bad in (110, 901, float("nan"), "420", True):
            data["geometry"]["following_x"] = bad
            self.assertIsNone(mapped_caret(data).following_x, bad)
        # Render: with room before the next word the ghost goes inline, no chip.
        data["geometry"]["following_x"] = 420
        data["geometry"]["field"]["height"] = 200
        preview, calls, _monitor = self.render_fixture((200, 24))
        self.assertTrue(preview.render(data, " next", 1500))
        self.assertNotIn(("class", "chip"), calls)
        self.assertIn(("left", 211), calls)
        # Without room it falls back to the chip, never over the words.
        preview, calls, _monitor = self.render_fixture((400, 24))
        self.assertTrue(preview.render(data, " next", 1500))
        self.assertIn(("class", "chip"), calls)

    def test_snapping_cannot_move_text_past_the_following_glyph_or_field(self):
        data = focus()
        data["geometry"]["scale"] = 1.5
        data["geometry"]["monitor"]["scale"] = 1.5
        data["geometry"]["caret"]["x"] = 110.6
        data["geometry"]["following_x"] = 311.8
        self.assertIsNone(place_inline(mapped_caret(data), 200, 24, 19),
                          "snapping right would cover the following glyph")
        del data["geometry"]["following_x"]
        data["geometry"]["field"]["width"] = 291.8
        self.assertIsNone(place_inline(mapped_caret(data), 200, 24, 19),
                          "snapping right would cross the field edge")
        data["geometry"]["field"] = {"x": 20.4, "y": 40, "width": 600, "height": 200}
        self.assertIsNone(place_chip(mapped_caret(data), 600, 24),
                          "a full-width chip cannot round outside its field")

    def test_font_metrics_anchor_reported_and_fallback_faces(self):
        preview, _calls, _monitor = self.render_fixture()
        description = Mock()
        metrics = SimpleNamespace(get_ascent=lambda: 15 * 1024, get_descent=lambda: 5 * 1024)
        context = Mock()
        context.get_metrics.return_value = metrics
        preview.label.get_pango_context = lambda: context
        preview.Pango.FontDescription = lambda: description
        self.assertEqual(preview._field_ascent({"family": "Iosevka", "weight": 700}, 18, 30), 15)
        description.set_family.assert_called_once_with("Iosevka")
        description.set_absolute_size.assert_called_once_with(18 * 1024)
        self.assertEqual(preview._field_ascent({}, 18, 30), 22.5)
        self.assertIsNone(preview._field_ascent({}, 18, None))

    def test_preview_animation_is_disabled_best_effort_via_hyprctl(self):
        calls = []
        def fake_run(command, **kwargs):
            calls.append(command)
            return SimpleNamespace(returncode=0)
        self.assertTrue(disable_preview_animation(run=fake_run))
        self.assertIn("badi-writing-preview", calls[0][-1])
        self.assertIn("no_anim", calls[0][-1])
        def missing(*_args, **_kwargs):
            raise FileNotFoundError("hyprctl")
        self.assertFalse(disable_preview_animation(run=missing))
        self.assertFalse(disable_preview_animation(run=lambda *_a, **_k: SimpleNamespace(returncode=1)))

    def test_rule_refresh_runs_off_the_render_path_and_is_rate_limited(self):
        preview = object.__new__(Preview)
        with patch("preview.time.monotonic", side_effect=[1, 2, 31, 32]), patch("preview.Thread") as worker:
            preview._refresh_layer_rule(background=False)
            self.disable_animation.assert_called_once_with()
            worker.assert_not_called()
            preview._refresh_layer_rule()
            worker.assert_not_called()
            preview._refresh_layer_rule()
            worker.assert_called_once_with(target=self.disable_animation, daemon=True)
            worker.return_value.start.assert_called_once_with()
            preview._refresh_layer_rule()
            self.assertEqual(worker.call_count, 1)

    def test_font_follows_the_caret_glyph_and_text_is_ltr_only(self):
        self.assertAlmostEqual(font_pixels(mapped_caret(focus())), 17.2)
        data = focus()
        data["geometry"]["caret"]["height"] = 4
        self.assertEqual(font_pixels(mapped_caret(data)), 6)
        data["geometry"]["caret"]["height"] = 128
        self.assertEqual(font_pixels(mapped_caret(data)), 72)
        self.assertEqual(font_facts({"family-name": "Iosevka, monospace", "size": "13pt", "weight": "bold"}),
                         {"family": "Iosevka", "size_pt": 13.0, "weight": 700})
        self.assertEqual(font_facts({"font-family": "Noto Sans", "font-size": "16px", "font-weight": 450}),
                         {"family": "Noto Sans", "size_px": 16.0, "weight": 400})
        self.assertEqual(font_facts({"family-name": "bad\nname", "size": "nope", "weight": "giant"}), {})
        self.assertEqual(font_facts(None), {})
        host = mapped_caret(focus())
        self.assertAlmostEqual(font_pixels(host, {"size_pt": 13}), 13 * 96 / 72)
        self.assertEqual(font_pixels(host, {"size_px": 15}), 15)
        self.assertAlmostEqual(font_pixels(host, {}), 17.2)
        for text in (" the next words", "teh → the", "v2.0 release", "één woord"):
            self.assertTrue(ltr_text(text), text)
        for text in ("می‌توانیم", "שלום", " word ١٢"):
            self.assertFalse(ltr_text(text), text)

    def render_fixture(self, natural=(200, 24), baseline=19):
        preview = object.__new__(Preview)
        preview._refresh_layer_rule = Mock()
        calls = []
        monitor = SimpleNamespace(get_connector=lambda: "eDP-1",
                                  get_geometry=lambda: SimpleNamespace(x=0, y=0, width=1440, height=900))
        monitors = SimpleNamespace(get_n_items=lambda: 1, get_item=lambda _index: monitor)
        preview.Gdk = SimpleNamespace(Display=SimpleNamespace(get_default=lambda: SimpleNamespace(get_monitors=lambda: monitors)))
        preview.GLib = SimpleNamespace(source_remove=Mock(), timeout_add=lambda ttl, _callback: calls.append(("timer", ttl)) or 7)
        preview.Gtk = SimpleNamespace(Orientation=SimpleNamespace(HORIZONTAL="h", VERTICAL="v"))
        preview.Pango = SimpleNamespace(SCALE=1024, AttrList=lambda: SimpleNamespace(insert=lambda attr: calls.append(("attr", attr))),
                                        attr_size_new_absolute=lambda size: ("size", size),
                                        attr_family_new=lambda family: ("family", family),
                                        attr_weight_new=lambda weight: ("weight", weight),
                                        attr_foreground_alpha_new=lambda alpha: SimpleNamespace(alpha=alpha))
        preview.Layer = SimpleNamespace(Edge=SimpleNamespace(LEFT="left", TOP="top"),
                                        set_monitor=lambda _window, chosen: calls.append(("monitor", chosen)),
                                        set_margin=lambda _window, edge, value: calls.append((edge, value)))
        sizes = {"h": natural[0], "v": natural[1]}
        preview.window = SimpleNamespace(set_visible=lambda visible: calls.append(("visible", visible)),
                                         add_css_class=lambda name: calls.append(("class", name)),
                                         remove_css_class=lambda name: calls.append(("unclass", name)),
                                         measure=lambda orientation, _for: SimpleNamespace(natural=sizes[orientation]))
        preview.label = SimpleNamespace(set_text=lambda text: calls.append(("text", text)),
                                        set_margin_start=lambda value: calls.append(("pad_start", value)),
                                        set_margin_end=lambda value: calls.append(("pad_end", value)),
                                        set_margin_top=lambda value: calls.append(("pad_top", value)),
                                        set_margin_bottom=lambda value: calls.append(("pad_bottom", value)),
                                        set_attributes=lambda _attrs: None,
                                        get_layout=lambda: SimpleNamespace(get_baseline=lambda: baseline * 1024))
        preview.timer = None
        return preview, calls, monitor

    def test_render_places_inline_text_after_the_caret_and_arms_expiry(self):
        preview, calls, monitor = self.render_fixture()
        self.assertTrue(preview.render(focus(), " next words", 1500))
        self.assertEqual(calls[:2], [("visible", False), ("text", "")], "a render first hides the old preview")
        self.assertIn(("monitor", monitor), calls)
        self.assertIn(("attr", ("size", round(17.2 * 1024))), calls)
        self.assertIn(("left", 211), calls)
        self.assertIn(("top", 147), calls)
        self.assertEqual(calls[-2:], [("visible", True), ("timer", 1500)])
        self.assertEqual(preview.timer, 7)

    def test_the_word_tab_takes_is_drawn_opaque_and_the_rest_lighter(self):
        def alphas(calls):
            return [(attr.start_index, attr.end_index, attr.alpha) for kind, attr in calls
                    if kind == "attr" and isinstance(attr, SimpleNamespace)]
        preview, calls, _monitor = self.render_fixture()
        self.assertTrue(preview.render(focus(), " next wörds", 1500, strong=5))
        self.assertEqual(alphas(calls), [(0, 5, STRONG_ALPHA), (5, 12, REST_ALPHA)], "byte offsets")
        self.assertLess(REST_ALPHA, STRONG_ALPHA)

        data = focus()
        data["geometry"]["text_follows"] = True
        data["geometry"]["field"]["height"] = 200
        preview, calls, _monitor = self.render_fixture((120, 24))
        self.assertTrue(preview.render(data, " final words", 1500, strong=6))
        self.assertIn(("text", "final words"), calls)
        self.assertEqual(alphas(calls), [(0, 5, STRONG_ALPHA), (5, 11, REST_ALPHA)],
                         "a chip drops the leading space")

        for strong in (None, 11):
            preview, calls, _monitor = self.render_fixture()
            self.assertTrue(preview.render(focus(), " next words", 1500, strong=strong))
            self.assertEqual(alphas(calls), [], "no split without a rest")

    def test_render_traces_its_placement_and_hide_cause_or_why_it_declined(self):
        preview, _calls, _monitor = self.render_fixture()
        with patch("preview.trace.emit") as emit:
            self.assertTrue(preview.render(focus(), " next words", 1500))
            preview._expire()
            self.assertFalse(preview.render({"caret": 1}, " next", 1500))
        drawn, hidden, declined = emit.call_args_list
        self.assertEqual(drawn.args[0], "preview drawn")
        self.assertEqual((drawn.args[1]["placement"], drawn.args[1]["x"], drawn.args[1]["y"]),
                         ("inline", 211, 147))
        self.assertEqual(drawn.args[2], {"text": " next words"}, "text is offered; the run decides")
        self.assertEqual((hidden.args[0], hidden.args[1]["cause"]), ("preview hidden", "expired"))
        self.assertEqual(declined.args, ("preview declined", {"reason": "uncalibrated_caret"}))

    def test_render_uses_a_reported_host_font(self):
        preview, calls, _monitor = self.render_fixture()
        data = focus()
        data["font"] = {"family": "Iosevka", "size_pt": 13, "weight": 700}
        self.assertTrue(preview.render(data, " next words", 1500))
        self.assertIn(("attr", ("family", "Iosevka")), calls)
        self.assertIn(("attr", ("weight", 700)), calls)
        self.assertIn(("attr", ("size", round(13 * 96 / 72 * 1024))), calls)

    def test_render_shows_a_chip_when_words_follow_and_keeps_overflow_inside_the_field(self):
        data = focus()
        data["geometry"]["text_follows"] = True
        data["geometry"]["field"]["height"] = 200
        preview, calls, _monitor = self.render_fixture((120, 24))
        self.assertTrue(preview.render(data, " final", 1500))
        self.assertIn(("class", "chip"), calls)
        self.assertNotIn(("text", " final"), calls, "a chip never draws inline over the following words")
        self.assertIn(("text", "final"), calls)
        preview, calls, _monitor = self.render_fixture((120, 24))
        self.assertTrue(preview.render(data, "final ", 1500))
        self.assertIn(("text", "final "), calls, "the chip keeps the space that separates the next word")
        self.assertIn(("left", 203), calls)
        self.assertIn(("top", 172), calls)
        preview, calls, _monitor = self.render_fixture((700, 24))
        self.assertTrue(preview.render(focus(), " next", 1500), "overflow uses a quiet panel outside the field")
        self.assertIn(("class", "chip"), calls)
        self.assertIn(("top", 182), calls)
        short = focus()
        short["geometry"]["field"]["width"] = 89
        preview, calls, _monitor = self.render_fixture((200, 24))
        self.assertTrue(preview.render(short, " next", 1500), "a box that ends before the caret")
        self.assertNotIn(("class", "chip"), calls)
        self.assertIn(("left", 211), calls)
        self.assertIn(("top", 147), calls)

    def test_render_keeps_end_of_text_overflow_under_the_caret_inside_the_field(self):
        data = focus()
        data["geometry"]["text_ends"] = True
        data["geometry"]["field"]["height"] = 200
        # 515 wide from the caret at x=211 passes the field's right edge (720).
        preview, calls, _monitor = self.render_fixture((515, 24))
        self.assertTrue(preview.render(data, " next", 1500))
        self.assertIn(("class", "chip"), calls)
        self.assertIn(("text", "next"), calls)
        self.assertIn(("left", 203), calls)
        self.assertIn(("top", 172), calls, "under the caret line, not outside the field")
        # A one-line field has no room under the line: the outside panel.
        one_line = focus()
        one_line["geometry"]["text_ends"] = True
        preview, calls, _monitor = self.render_fixture((515, 24))
        self.assertTrue(preview.render(one_line, " next", 1500))
        self.assertIn(("top", 182), calls)

    def test_chip_padding_is_measured_with_the_text_not_left_to_a_restyle(self):
        # A CSS class toggled right before measuring is not restyled yet; the
        # live chip measured 12 px narrow and crossed the field's edge.
        data = focus()
        data["geometry"]["text_follows"] = True
        data["geometry"]["field"]["height"] = 200
        preview, calls, _monitor = self.render_fixture((120, 24))
        self.assertTrue(preview.render(data, " final", 1500))
        chip = calls.index(("class", "chip"))
        self.assertEqual(calls[chip + 1:chip + 5],
                         [("pad_start", 6), ("pad_end", 6), ("pad_top", 2), ("pad_bottom", 2)])
        preview, calls, _monitor = self.render_fixture()
        self.assertTrue(preview.render(focus(), " next words", 1500))
        self.assertIn(("pad_start", 0), calls)
        self.assertNotIn(("pad_start", 6), calls)

    def test_render_refuses_rtl_unmapped_or_unfitting_text(self):
        for text, data, natural in ((" متن", focus(), (200, 24)), (" next", {**focus(), "geometry": None}, (200, 24)),
                                    (" next", focus(), (1000, 24))):
            preview, calls, _monitor = self.render_fixture(natural)
            self.assertFalse(preview.render(data, text, 1500))
            self.assertNotIn(("visible", True), calls)
            self.assertFalse(any(call[0] == "timer" for call in calls))

    def test_long_reading_window_keeps_a_bounded_local_expiry(self):
        preview, calls, _monitor = self.render_fixture()
        self.assertTrue(preview.render(focus(), " next words", 15000))
        self.assertIn(("timer", 15000), calls)
        for ttl in (15001, 0, True):
            calls.clear()
            self.assertFalse(preview.render(focus(), " next words", ttl))
            self.assertNotIn(("visible", True), calls)

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
