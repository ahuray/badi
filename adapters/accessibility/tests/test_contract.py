import copy
import os
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from contract import Observer, SCHEMA, canonical_origin, Denied


class FakeBackend:
    def __init__(self):
        self.content = "Same text"
        self.reads = []
        self.geometry_requests = []
        self.caret_rects = []
        self.lock_checks = []
        self.positions = 0
        self.change = None
        self.meta = {"bus": ":1.25", "path": "/org/a11y/atspi/accessible/2", "process_id": 42,
                     "app_id": "chromium", "uri": "https://example.test/writing", "browser": True, "web": True,
                     "role": "entry", "tag": "textarea", "input_type": "textarea", "focused": True,
                     "editable": True, "showing": True, "visible": True, "enabled": True,
                     "caret": 9, "total_chars": 9, "selection_count": 0}

    def metadata(self, app_id, geometry=False, check_lock=False, caret_rect=None):
        self.caret_rects.append(caret_rect)
        self.geometry_requests.append(geometry)
        self.lock_checks.append(check_lock)
        if app_id != self.meta["app_id"]:
            raise Denied("app_mismatch")
        return copy.deepcopy(self.meta)

    def position(self, metadata):
        self.positions += 1
        return {**metadata, **{key: self.meta[key] for key in ("caret", "total_chars", "selection_count")}}

    def text(self, metadata, start, end):
        self.reads.append((start, end))
        result = self.content[start:end]
        if self.change:
            self.change(self)
            self.change = None
        return result


class ContractTests(unittest.TestCase):
    def setUp(self):
        self.backend = FakeBackend()
        self.observer = Observer(self.backend)
        self.events = []
        self.observer.notify = self.events.append

    def inspect(self):
        return self.observer.request({"schema": SCHEMA, "id": "1", "op": "inspect", "app_id": "chromium"})

    def snapshot(self, focus):
        return self.observer.request({"schema": SCHEMA, "id": "2", "op": "snapshot", "binding": focus["binding"], "policy_target": focus["target"]})

    def preview_request(self, focus):
        return {"schema": SCHEMA, "id": "preview", "op": "preview", "binding": focus["binding"],
                "policy_target": focus["target"], "text": " suggestion", "ttl_ms": 2000,
                "expected_caret": focus["caret"], "expected_total_chars": focus["total_chars"]}

    def test_inspect_never_reads_prose(self):
        result = self.inspect()
        self.assertTrue(result["ok"])
        self.assertEqual(self.backend.reads, [])
        self.assertNotIn("before", result["focus"])
        self.assertEqual(result["focus"]["target"]["origin"], {"scheme": "https", "host": "example.test"})

    def test_status_preserves_authority_and_never_acquires_or_changes_ui(self):
        self.observer.tracked = {"test": "unchanged"}
        def forbidden(*_args):
            self.fail("Status must not acquire metadata/text, hide, render or disarm")
        self.backend.metadata = self.backend.text = forbidden
        self.observer.hide = self.observer.render = self.observer.disarm = forbidden
        result = self.observer.request({"schema": SCHEMA, "id": "health", "op": "status"})
        self.assertEqual(result, {"schema": SCHEMA, "id": "health", "ok": True,
                                 "status": {"ready": True, "process_id": os.getpid(), "protocol_version": 1}})
        self.assertEqual(self.observer.tracked, {"test": "unchanged"})
        self.assertEqual(self.observer.epoch, 1)
        self.assertEqual(self.events, [])
        self.assertEqual(self.observer.request({"schema": SCHEMA, "id": "health", "op": "status", "app_id": "chromium"})["error"], "invalid_request")

    def test_sensitive_selected_and_unknown_fields_deny_before_read(self):
        focus = self.inspect()["focus"]
        for change in ({"role": "password text"}, {"selection_count": 1},
                       {"editable": False}, {"focused": False}, {"showing": False}, {"enabled": False},
                       {"tag": "input", "input_type": "email"}, {"tag": "unknown"}):
            before = copy.deepcopy(self.backend.meta)
            self.backend.meta.update(change)
            self.assertFalse(self.snapshot(focus)["ok"], change)
            self.assertEqual(self.backend.reads, [], change)
            self.backend.meta = before

    def test_electron_web_content_keeps_html_purpose_gates(self):
        self.backend.meta.update(app_id="code", browser=False, web=True, uri="")
        for change in ({"tag": "input", "input_type": "email"}, {"tag": "input", "input_type": "search"},
                       {"tag": "input", "input_type": "text", "xml_roles": "searchbox"},
                       {"tag": "div", "xml_roles": "searchbox"},
                       {"tag": "unknown"}, {"role": "password text"}):
            with self.subTest(change=change):
                before = copy.deepcopy(self.backend.meta)
                self.backend.meta.update(change)
                result = self.observer.request({"schema": SCHEMA, "id": "1", "op": "inspect", "app_id": "code"})
                self.assertIn(result["error"], ("unsupported_field", "sensitive_field"))
                self.backend.meta = before
        focus = self.observer.request({"schema": SCHEMA, "id": "1", "op": "inspect", "app_id": "code"})["focus"]
        self.assertEqual(focus["target"], {"kind": "desktop_application", "app_id": "code",
                                           "target_id": focus["target"]["target_id"]})
        # Native toolkits (Telegram, Omawrite) expose no HTML tag or input type.
        self.backend.meta.update(app_id="telegram", web=False, tag="", input_type="")
        self.assertTrue(self.observer.request({"schema": SCHEMA, "id": "1", "op": "inspect", "app_id": "telegram"})["ok"])

    def test_brave_shares_chromium_site_policy_but_keeps_its_binding(self):
        self.backend.meta.update(app_id="brave-origin")
        focus = self.observer.request({"schema": SCHEMA, "id": "1", "op": "inspect", "app_id": "brave-origin"})["focus"]
        self.assertEqual((focus["target"]["kind"], focus["target"]["app_id"]), ("browser", "chromium"))
        self.assertEqual(focus["target"]["origin"], {"scheme": "https", "host": "example.test"})
        self.assertEqual(focus["binding"]["app_id"], "brave-origin")

    def test_only_inspect_checks_the_session_lock_and_later_reads_keep_its_epoch(self):
        focus = self.inspect()["focus"]
        focus = self.snapshot(focus)["focus"]
        self.observer.render = lambda *_args: True
        self.assertTrue(self.observer.request(self.preview_request(focus))["ok"])
        self.assertEqual(self.backend.lock_checks, [True, False, False])

        def locked(*_args):
            raise Denied("desktop_locked")
        self.backend.metadata = locked
        self.assertEqual(self.inspect()["error"], "desktop_locked")
        self.backend.metadata = FakeBackend.metadata.__get__(self.backend)
        self.observer.invalidate("focus_changed")
        self.assertEqual(self.snapshot(focus)["error"], "stale_binding", "a binding never outlives its epoch")

    def test_snapshot_rechecks_only_the_same_fields_position(self):
        focus = self.inspect()["focus"]
        self.backend.lock_checks.clear()
        self.assertTrue(self.snapshot(focus)["ok"])
        self.assertEqual((self.backend.lock_checks, self.backend.positions), ([False], 1))

    def test_sentence_bounded_snapshot_starts_at_the_carets_sentence(self):
        # LibreOffice Writer gives Fcitx only the caret's sentence; the snapshot must equal it.
        self.backend.content = "First. Same text"
        self.backend.meta.update(caret=16, total_chars=16, sentence_start=7)
        snapshot = self.snapshot(self.inspect()["focus"])["focus"]
        self.assertEqual((snapshot["before"], snapshot["after"]), ("Same text", ""))
        self.assertTrue(all(start == 7 for start, _end in self.backend.reads))

    def test_invalidation_during_the_read_rejects(self):
        focus = self.inspect()["focus"]
        self.backend.change = lambda _backend: self.observer.invalidate("field_changed")
        self.assertEqual(self.snapshot(focus)["error"], "stale_binding")

    def test_exact_same_text_different_field_rejects(self):
        focus = self.inspect()["focus"]
        self.backend.meta["path"] = "/org/a11y/atspi/accessible/3"
        self.assertEqual(self.snapshot(focus)["error"], "stale_binding")
        self.assertEqual(self.backend.reads, [])
        self.assertEqual(self.events[-1]["app_id"], "chromium")

    def test_bus_restart_process_change_and_same_origin_navigation_reject(self):
        for key, value in [("bus", ":1.26"), ("process_id", 43), ("uri", "https://example.test/other")]:
            self.setUp()
            focus = self.inspect()["focus"]
            self.backend.meta[key] = value
            self.assertEqual(self.snapshot(focus)["error"], "stale_binding")
            self.assertEqual(self.backend.reads, [])

    def test_policy_target_mismatch_rejects_before_read(self):
        focus = self.inspect()["focus"]
        focus["target"]["origin"]["host"] = "different.test"
        self.assertEqual(self.snapshot(focus)["error"], "target_mismatch")
        self.assertEqual(self.backend.reads, [])

    def test_event_invalidates_prior_binding(self):
        focus = self.inspect()["focus"]
        self.observer.invalidate("field_changed")
        self.assertEqual(self.snapshot(focus)["error"], "stale_binding")
        self.assertEqual(self.backend.reads, [])

    def test_unicode_bounds_preserve_scalar_offsets(self):
        self.backend.content = "آ" * 600 + "🦊" * 200
        self.backend.meta.update(caret=600, total_chars=800)
        result = self.snapshot(self.inspect()["focus"])
        self.assertTrue(result["ok"])
        self.assertEqual(result["focus"]["before"], "آ" * 512)
        self.assertEqual(result["focus"]["after"], "🦊" * 128)
        self.assertEqual(self.backend.reads, [(88, 728), (88, 728)])

    def test_mutation_during_read_rejects(self):
        focus = self.inspect()["focus"]
        self.backend.change = lambda b: b.meta.update(caret=8)
        self.assertEqual(self.snapshot(focus)["error"], "stale_binding")
        self.backend.meta.update(caret=9)
        focus = self.inspect()["focus"]
        self.backend.change = lambda b: setattr(b, "content", "Othertext")
        self.assertEqual(self.snapshot(focus)["error"], "stale_binding")

    def test_malformed_frames_and_unknown_fields(self):
        for request in (None, [], {}, {"schema": SCHEMA, "id": "1", "op": "inspect", "app_id": "chromium", "allow": True},
                        {"schema": SCHEMA, "id": "x\nsecret", "op": "inspect", "app_id": "chromium"}):
            self.assertFalse(self.observer.request(request)["ok"])
        self.assertEqual(self.backend.reads, [])

    def test_preview_revalidates_without_text_acquisition(self):
        focus = self.snapshot(self.inspect()["focus"])["focus"]
        reads = list(self.backend.reads)
        renders = []
        self.observer.render = lambda f, t, ttl: renders.append((f, t, ttl)) or True
        request = self.preview_request(focus)
        result = self.observer.request(request)
        self.assertTrue(result["focus"]["rendered"])
        self.assertEqual(result["focus"]["total_chars"], focus["total_chars"])
        self.assertEqual(self.backend.reads, reads)
        self.assertEqual(len(renders), 1)
        self.assertEqual(self.backend.geometry_requests, [False, False, True], "only preview calibrates")
        self.backend.meta["path"] = "/other"
        self.assertFalse(self.observer.request(request)["ok"])
        self.assertEqual(len(renders), 1)

    def test_preview_denies_missed_caret_and_length_changes_before_render(self):
        for change in ({"caret": 700}, {"total_chars": 1201}):
            with self.subTest(change=change):
                self.setUp()
                self.backend.content = "x" * 1200
                self.backend.meta.update(caret=600, total_chars=1200)
                focus = self.snapshot(self.inspect()["focus"])["focus"]
                reads = list(self.backend.reads)
                renders, hidden = [], []
                self.observer.render = lambda *_args: renders.append(True) or True
                self.observer.hide = lambda: hidden.append(True)
                self.backend.meta.update(change)
                self.backend.content = "x" * self.backend.meta["total_chars"]
                # Both changed positions still have the same 512/128 context;
                # deliberately omit the advisory accessibility event.
                caret = self.backend.meta["caret"]
                self.assertEqual(self.backend.content[caret-512:caret], focus["before"])
                self.assertEqual(self.backend.content[caret:caret+128], focus["after"])
                request = self.preview_request(focus)
                self.assertEqual(self.observer.request(request)["error"], "stale_binding")
                self.assertEqual(renders, [])
                self.assertEqual(hidden, [True])
                self.assertEqual(self.backend.reads, reads)
                self.assertEqual(self.observer.epoch, focus["binding"]["epoch"])
                self.assertEqual(self.events, [])

    def test_preview_passes_a_valid_input_method_caret_rectangle_to_calibration(self):
        request = self.preview_request(self.snapshot(self.inspect()["focus"])["focus"])
        rect = {"x": 1496, "y": 1636, "width": 2, "height": 36, "scale": 2.0}
        self.assertTrue(self.observer.request({**request, "caret_rect": rect})["ok"])
        self.assertEqual(self.backend.caret_rects[-1], rect)
        self.assertTrue(self.observer.request(request)["ok"])
        self.assertIsNone(self.backend.caret_rects[-1], "without one, calibration gets none")
        for bad in ({**rect, "scale": 9}, {**rect, "x": -1}, {**rect, "y": 1.5}, {**rect, "extra": 1},
                    {key: value for key, value in rect.items() if key != "scale"}, [1, 2, 3, 4], None):
            self.assertEqual(self.observer.request({**request, "caret_rect": bad})["error"], "invalid_preview", bad)
        self.assertEqual(self.observer.request({**request, "caret": rect})["error"], "invalid_request")

    def test_preview_requires_both_snapshot_position_fields(self):
        request = self.preview_request(self.snapshot(self.inspect()["focus"])["focus"])
        def forbidden(*_args):
            self.fail("An unbound preview must not acquire metadata/text or render")
        self.backend.metadata = self.backend.text = self.observer.render = forbidden
        for fields in (("expected_caret",), ("expected_total_chars",),
                       ("expected_caret", "expected_total_chars")):
            incomplete = {key: value for key, value in request.items() if key not in fields}
            self.assertEqual(self.observer.request(incomplete)["error"], "invalid_request")

    def test_invalid_preview_and_hide_lifecycle(self):
        focus = self.snapshot(self.inspect()["focus"])["focus"]
        hidden = []
        self.observer.hide = lambda: hidden.append(True)
        base = self.preview_request(focus)
        for patch in ({"text": "\n"}, {"text": ""}, {"text": "x"*161}, {"text": "a\u202eb"}, {"ttl_ms": 5001}, {"ttl_ms": True},
                      {"expected_caret": -1}, {"expected_caret": True}, {"expected_caret": 9.0}, {"expected_caret": "9"},
                      {"expected_total_chars": 8}, {"expected_total_chars": True}, {"expected_total_chars": 2**31}):
            self.assertEqual(self.observer.request({**base, **patch})["error"], "invalid_preview")
        self.assertTrue(self.observer.request({"schema": SCHEMA, "id": "hide", "op": "hide"})["hidden"])
        self.observer.invalidate("focus_changed")
        self.assertEqual(len(hidden), 2)

    def test_snapshot_marks_only_verified_ltr_caret_edges(self):
        for text, direction, web, expected in [
                ("Hello world", "lr", True, "right"), ("متن فارسی", "rl", True, None),
                ("Hello فارسی", "lr", True, None), ("Hello", "", True, None),
                # Qt reports no direction: its paragraph follows the first strong letter.
                ("Hello", "", False, "right"), ("2026", "", False, None), ("Hello فارسی", "", False, None),
                ("Hello", "rl", False, None)]:
            self.setUp()
            self.backend.content = text
            self.backend.meta.update(caret=len(text), total_chars=len(text), direction=direction, web=web,
                                     geometry={"test": True})
            focus = self.snapshot(self.inspect()["focus"])["focus"]
            self.assertEqual(focus["geometry"].get("caret_edge"), expected)

    def test_origin_normalization_and_credentials_rejection(self):
        self.assertEqual(canonical_origin("https://EXAMPLE.test:443/path?q=1"), {"scheme": "https", "host": "example.test"})
        self.assertEqual(canonical_origin("http://127.0.0.1:4321/path"), {"scheme": "http", "host": "127.0.0.1", "port": 4321})
        for uri in ("file:///note", "about:blank", "https://user:password@example.test", "https://example.test:0", "not-a-uri"):
            with self.assertRaises(Denied):
                canonical_origin(uri)


if __name__ == "__main__":
    unittest.main()
