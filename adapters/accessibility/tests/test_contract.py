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
        self.assertNotIn("font", result["focus"])

    def test_inspect_carries_a_reported_host_font(self):
        self.backend.meta["font"] = {"family": "Iosevka", "size_pt": 13.0, "weight": 400}
        self.assertEqual(self.inspect()["focus"]["font"], {"family": "Iosevka", "size_pt": 13.0, "weight": 400})

    def test_inspect_carries_quiet_facts_without_denying_the_field(self):
        for reason in ("file_dialog", "narrow_field"):
            with self.subTest(reason=reason):
                self.backend.meta["quiet"] = reason
                focus = self.inspect()["focus"]
                self.assertEqual(focus["quiet"], reason)
                # Quiet is a fact for the caller's automatic path; the field
                # itself stays readable for an explicit request.
                self.assertTrue(self.snapshot(focus)["ok"])
        del self.backend.meta["quiet"]
        self.assertNotIn("quiet", self.inspect()["focus"])
        self.backend.meta["quiet"] = "something_else"
        self.assertNotIn("quiet", self.inspect()["focus"], "only the two known facts reach the wire")

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
        self.assertEqual((focus["target"]["kind"], focus["target"]["app_id"]), ("browser", "brave-origin"))
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

    def test_a_snapshot_reads_the_window_only_when_asked_and_only_with_true(self):
        focus = self.inspect()["focus"]
        calls = []
        self.backend.vocabulary_text = lambda metadata: calls.append(metadata["uri"]) or "Re: Venue deposit"
        plain = self.snapshot(focus)
        self.assertTrue(plain["ok"])
        self.assertNotIn("window_text", plain["focus"])
        self.assertEqual(calls, [], "no window read unless asked")
        request = {"schema": SCHEMA, "id": "3", "op": "snapshot", "binding": focus["binding"],
                   "policy_target": focus["target"], "window": True}
        asked = self.observer.request(request)
        self.assertEqual(asked["focus"]["window_text"], "Re: Venue deposit")
        self.assertEqual(calls, ["https://example.test/writing"])
        for value in (False, 1, "yes"):
            with self.subTest(value=value):
                self.assertEqual(self.observer.request({**request, "window": value})["error"], "invalid_request")
        self.backend.vocabulary_text = lambda _metadata: None
        self.assertNotIn("window_text", self.observer.request(request)["focus"])

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
        self.backend.meta["geometry"] = {"caret": {"y": 10}}
        self.observer.reposition_preview()
        self.assertEqual(len(renders), 2)
        self.assertEqual(renders[1][0]["geometry"]["caret"]["y"], 10)
        self.assertLessEqual(renders[1][2], renders[0][2])
        self.backend.meta["caret"] = 8
        self.observer.reposition_preview()
        self.assertEqual(self.events[-1]["reason"], "field_changed")
        self.assertEqual(len(renders), 2, "a moved caret is not drawn again")
        self.backend.meta["path"] = "/other"
        self.assertFalse(self.observer.request(request)["ok"])
        self.assertEqual(len(renders), 2)

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
        self.assertTrue(self.observer.request({**base, "ttl_ms": 15000})["ok"])
        for patch in ({"text": "\n"}, {"text": ""}, {"text": "x"*161}, {"text": "a\u202eb"}, {"ttl_ms": 15001}, {"ttl_ms": True},
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

    def test_snapshot_marks_words_after_the_caret_on_its_line(self):
        for content, caret, follows in (("Please find the report", 15, True), ("Please find the\nreport", 15, False),
                                        ("Please find the  ", 15, False), ("Please find the", 15, False)):
            self.setUp()
            self.backend.content = content
            self.backend.meta.update(caret=caret, total_chars=len(content), direction="lr", geometry={"test": True})
            focus = self.snapshot(self.inspect()["focus"])["focus"]
            self.assertIs(focus["geometry"]["text_follows"], follows, content)
            self.assertIs(self.observer.text_follows, follows)
        self.observer.withdraw()
        self.assertFalse(self.observer.text_follows)

    def test_snapshot_marks_a_caret_with_nothing_visible_after_it(self):
        for content, caret, ends in (("Please find the", 15, True), ("Please find the \n\n", 15, True),
                                     ("Please find the\nreport", 15, False), ("Please find the report", 15, False)):
            self.setUp()
            self.backend.content = content
            self.backend.meta.update(caret=caret, total_chars=len(content), direction="lr", geometry={"test": True})
            focus = self.snapshot(self.inspect()["focus"])["focus"]
            self.assertIs(focus["geometry"]["text_ends"], ends, content)
            self.assertIs(self.observer.text_ends, ends)
        self.observer.withdraw()
        self.assertFalse(self.observer.text_ends)

    def test_origin_normalization_and_credentials_rejection(self):
        self.assertEqual(canonical_origin("https://EXAMPLE.test:443/path?q=1"), {"scheme": "https", "host": "example.test"})
        self.assertEqual(canonical_origin("http://127.0.0.1:4321/path"), {"scheme": "http", "host": "127.0.0.1", "port": 4321})
        for uri in ("file:///note", "about:blank", "https://user:password@example.test", "https://example.test:0", "not-a-uri"):
            with self.assertRaises(Denied):
                canonical_origin(uri)


class CredentialLabelTests(unittest.TestCase):
    def test_labels_that_name_a_secret_are_credentials_and_prose_fields_are_not(self):
        from contract import credential_label
        for label in ("Password", "New password", "Passwort", "Kennwort", "PIN", "Enter PIN code", "OTP",
                      "One-time code", "Verification code", "CVV", "Card number", "Mot de passe",
                      "Contraseña", "Senha", "Palavra-passe", "Wachtwoord", "Hasło", "Bestätigungscode"):
            self.assertTrue(credential_label(label), label)
        for label in ("Message", "Pinned notes", "Spinning class", "Comment", "Subject", "Code review notes",
                      "Search", "", None):
            self.assertFalse(credential_label(label), label)
        self.assertTrue(credential_label("Message", "Your password"), "the description counts too")


if __name__ == "__main__":
    unittest.main()
