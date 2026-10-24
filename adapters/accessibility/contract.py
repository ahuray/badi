"""Focused accessibility acquisition contract; independent of desktop bindings."""
from __future__ import annotations

import hashlib
import math
import os
import re
import time
import unicodedata
from urllib.parse import urlsplit

import debugtrace as trace

SCHEMA = "badi.accessibility.v1"
MAX_FRAME = 16 * 1024
MAX_BEFORE = 512
MAX_AFTER = 128
# A revision reads one selection of at most this many characters (ADR 0012).
MAX_SELECTED = 600
MAX_PREVIEW_TTL_MS = 15_000
# A preview draws a broker suggestion (at most 64 characters, the broker's and
# the addon's cap) or, for a correction, "→ " and the corrected words.
MAX_PREVIEW_CHARS = 66
MAX_POSITION = 2**31 - 1
ID = re.compile(r"[A-Za-z0-9_.-]{1,64}\Z")
APP_ID = re.compile(r"[a-zA-Z0-9_.-]{1,128}")
REQUEST_FIELDS = {"inspect": {"schema", "id", "op", "app_id"},
                  "snapshot": {"schema", "id", "op", "binding", "policy_target"},
                  "preview": {"schema", "id", "op", "binding", "policy_target", "text", "ttl_ms",
                              "expected_caret", "expected_total_chars"},
                  "status": {"schema", "id", "op"},
                  "hide": {"schema", "id", "op"}}
# A preview may add the caret rectangle the app gave its input method.
# A snapshot may ask once per field for the window's text (ADR 0010).
# Inspect and snapshot may ask for the field's one selection instead of a
# caret, for an explicit revision (ADR 0012).
OPTIONAL_FIELDS = {"preview": {"caret_rect"}, "snapshot": {"window", "selection"}, "inspect": {"selection"}}
CARET_RECT_FIELDS = frozenset({"x", "y", "width", "height", "scale"})
IDENTITY = ("bus", "path", "process_id", "app_id", "uri")
BINDING_FIELDS = frozenset({"epoch", *IDENTITY})
POSITION = ("caret", "total_chars", "selection_count")
# Form controls whose value Chromium sends to Fcitx unchanged.
FORM_CONTROLS = frozenset({"input", "textarea"})
# A malformed request changes no field state; every other denial withdraws it.
MALFORMED = frozenset({"invalid_request", "invalid_preview"})
# Facts that quiet automatic suggestions only; the field stays eligible and an
# explicit request is the caller's decision. Reported by the backend, carried
# on every focus reply, never a denial.
QUIET_REASONS = frozenset({"file_dialog", "narrow_field"})
RIGHT_TO_LEFT = frozenset({"R", "AL", "AN"})


class Denied(Exception):
    """A fixed, prose-free failure reason safe to return over IPC."""

    @property
    def timed_out(self):
        return str(self) == "operation_timeout"


def canonical_origin(uri):
    try:
        parsed = urlsplit(uri)
        if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password:
            raise ValueError
        host = parsed.hostname.encode("idna").decode("ascii").lower()
        port = parsed.port
        if len(host) > 253 or port == 0:
            raise ValueError
        result = {"scheme": parsed.scheme, "host": host}
        if port is not None and port != (443 if parsed.scheme == "https" else 80):
            result["port"] = port
        return result
    except (ValueError, UnicodeError):
        raise Denied("unsupported_origin") from None


# Words that name a secret in a field's label, in Badi's languages. A page's
# "show password" button turns a password into an ordinary text input, and
# card or one-time-code fields are often plain text from the start: their
# label is then all that marks them.
CREDENTIAL_LABEL = re.compile(
    r"\b(?:password|passwort|kennwort|passphrase|passcode|pin|otp|one[- ]time (?:code|password)|"
    r"verification code|security code|authentication code|2fa|cvv|cvc|csc|card number|"
    r"mot de passe|code de v[ée]rification|contrase[ñn]a|clave|c[óo]digo de verificaci[óo]n|"
    r"senha|palavra[- ]passe|c[óo]digo de verifica[çc][ãa]o|wachtwoord|has[łl]o|"
    r"best[äa]tigungscode|sicherheitscode|kartennummer)\b",
    re.IGNORECASE)


def credential_label(*labels):
    """Whether a field's accessible name or description names a secret."""
    return any(isinstance(label, str) and CREDENTIAL_LABEL.search(label) for label in labels)


def eligible(metadata, selection=False):
    """Reject purpose/selection before asking a provider for any prose.

    A suggestion needs a caret without a selection; a revision exactly one
    selection.
    """
    if metadata["role"] == "password text":
        raise Denied("sensitive_field")
    if not all(metadata.get(flag, False) for flag in ("focused", "editable", "showing", "visible", "enabled")):
        raise Denied("ineligible_field")
    if metadata["selection_count"] != (1 if selection else 0):
        raise Denied("selection_absent" if selection else "selection_present")
    # A web contenteditable root without role=textbox is a section: a div (Lexical,
    # Quill) or an editing iframe's body (TinyMCE).
    web_root = metadata["web"] and metadata["role"] == "section" and metadata.get("tag") in ("div", "body")
    if metadata["role"] not in ("entry", "text", "paragraph") and not web_root:
        raise Denied("unsupported_field")
    tag = metadata.get("tag", "")
    input_type = metadata.get("input_type", "")
    # Electron apps and Gecko render web content too: the same HTML purpose
    # gates apply. A field whose tag or input type is not yet exposed (Gecko's
    # first query of a new document can omit them) is ineligible, never guessed.
    if metadata["web"]:
        if tag == "input" and input_type != "text":
            raise Denied("unsupported_field")
        if tag not in ("input", "textarea", "div", "p", "body"):
            raise Denied("unsupported_field")
        if input_type not in ("", "text", "textarea"):
            raise Denied("unsupported_field")
        # An ARIA search box is a plain entry to accessibility, like the
        # search input type above: queries, not prose.
        if "searchbox" in metadata.get("xml_roles", "").split():
            raise Denied("unsupported_field")
    if metadata["caret"] < 0 or metadata["caret"] > metadata["total_chars"]:
        raise Denied("invalid_caret")


def reply_facts(reply, tracked):
    """Content-free facts of an answered observer request, for the debug trace."""
    focus = reply.get("focus") if isinstance(reply.get("focus"), dict) else {}
    facts = {"ok": True, "app": (tracked or {}).get("app_id", "")}
    for key in ("caret", "total_chars", "rendered", "scope", "purpose"):
        if key in focus:
            facts[key] = focus[key]
    for key in ("before", "selected", "after", "window_text"):
        if isinstance(focus.get(key), str):
            facts[f"{key}_chars"] = len(focus[key])
    if isinstance(focus.get("geometry"), dict):
        geometry = focus["geometry"]
        facts["geometry"] = {key: geometry[key] for key in ("caret_edge", "text_follows", "text_ends")
                             if key in geometry}
    return facts


class Observer:
    """Binds each read to a current object, compositor identity and epoch.

    The trusted caller owns broker policy. A snapshot request must repeat the
    exact target for which that caller holds a current context-read grant.
    """

    def __init__(self, backend):
        self.backend = backend
        self.epoch = 1
        self.tracked = None
        self.notify = lambda _message: None
        self.render = lambda _focus, _text, _ttl: False
        self.hide = lambda: None
        self.authorize = lambda _binding, _node=None: None
        self.disarm = lambda: None
        self.caret_edge = None
        self.text_follows = False
        self.text_ends = False
        self.preview_hold = None
        self.preview_place = None
        self.operations = {"status": self.status, "hide": self.hide_preview, "inspect": self.inspect,
                           "snapshot": self.snapshot, "preview": self.preview}

    def invalidate(self, reason):
        previous = self.tracked
        trace.emit("observer invalidate", {"reason": reason, "app": previous["app_id"] if previous else ""})
        self.withdraw()
        self.epoch += 1
        self.tracked = None
        self.notify({"schema": SCHEMA, "event": "invalidate", "epoch": self.epoch,
                     "reason": reason, "app_id": previous["app_id"] if previous else ""})

    def withdraw(self):
        """Clear the preview, the field's event match and the verified caret line."""
        self.hide()
        self.disarm()
        self.caret_edge = None
        self.text_follows = False
        self.text_ends = False
        self.preview_hold = None

    def request(self, request):
        request_id = request.get("id") if isinstance(request, dict) else None
        name = None
        try:
            name = operation_name(request)
            reply = {"schema": SCHEMA, "id": request_id, "ok": True, **self.operations[name](request)}
            if name != "status":
                trace.emit(f"observer {name}", reply_facts(reply, self.tracked))
            return reply
        except Denied as error:
            if str(error) not in MALFORMED:
                self.withdraw()
            trace.emit(f"observer {name or 'request'}", {"ok": False, "error": str(error),
                                                         "app": (self.tracked or {}).get("app_id", "")})
            return {"schema": SCHEMA, "id": request_id, "ok": False, "error": str(error), "epoch": self.epoch}

    def status(self, _request):
        return {"status": {"ready": True, "process_id": os.getpid(), "protocol_version": 1}}

    def hide_preview(self, _request):
        # A hidden preview is withdrawn: no later scroll may draw it again.
        self.hide()
        self.preview_hold = None
        return {"hidden": True, "epoch": self.epoch}

    def inspect(self, request):
        selection = flag(request, "selection")
        _metadata, focus = self.describe(checked_app_id(request["app_id"]), check_lock=True,
                                         selection=selection)
        return {"focus": focus}

    def snapshot(self, request):
        if request.get("window", True) is not True:
            raise Denied("invalid_request")
        selection = flag(request, "selection")
        if selection and "window" in request:
            raise Denied("invalid_request")
        metadata, focus = self.bound_focus(request, selection=selection)
        self.authorize(focus["binding"], metadata.get("node"))
        if selection:
            return {"focus": self.selected_window(metadata, focus)}
        if metadata.get("sentence_bounded"):
            metadata["sentence_start"] = self.backend.sentence_start(metadata)
        caret = metadata["caret"]
        # Chromium may send a rich editor's text one line break shorter per block.
        margin = len(metadata.get("blocks") or ())
        start = max(0, caret - MAX_BEFORE - margin, metadata.get("sentence_start", 0))
        end = min(metadata["total_chars"], caret + MAX_AFTER + margin)
        scope = "field"
        if margin:
            start, end, scope = self.backend.rich_window(metadata, start, end)
        text = self.stable_text(metadata, focus["binding"]["epoch"], start, end)
        before, after = text[:caret - start], text[caret - start:]
        if margin:
            before, after = self.backend.serialized(metadata, start, before, after)
        before, after = before[-MAX_BEFORE:], after[:MAX_AFTER]
        before, after = surrounding_text(metadata, before), surrounding_text(metadata, after)
        if scope == "block":
            # Only the caret's block serializes exactly: Fcitx's text must end with it.
            focus["scope"] = "block"
        native = not metadata.get("web")
        self.caret_edge = "right" if ltr_caret_line(metadata.get("direction"), before, native) else None
        # Words after the caret on its line: inline text would cover them.
        self.text_follows = bool(after.split("\n", 1)[0].strip())
        # Nothing visible after the caret in a plain field: the lines under
        # it are empty, so a chip there covers no text. A rich editor's
        # window is one block, which proves nothing about the blocks below.
        self.text_ends = not margin and end == metadata["total_chars"] and not after.strip()
        focus.update(before=before, after=after, total_chars=metadata["total_chars"])
        if request.get("window"):
            window = self.backend.vocabulary_text(metadata)
            if window:
                focus["window_text"] = window
        return {"focus": focus}

    def selected_window(self, metadata, focus):
        """The field's one selection on one line, with the text around it.

        Only a plain field: a rich editor's blocks serialize differently for
        Fcitx, so its selection cannot be confirmed exactly.
        """
        if metadata.get("blocks") or metadata.get("sentence_bounded"):
            raise Denied("unsupported_selection")
        start, end = self.backend.selection(metadata)
        total = metadata["total_chars"]
        if not 0 <= start < end <= total or end - start > MAX_SELECTED:
            raise Denied("unsupported_selection")
        first, last = max(0, start - MAX_BEFORE), min(total, end + MAX_AFTER)
        text = self.stable_text(metadata, focus["binding"]["epoch"], first, last)
        if self.backend.selection(metadata) != (start, end):
            raise Denied("stale_binding")
        selected = text[start - first:end - first]
        if "\n" in selected or "\r" in selected or not selected.strip():
            raise Denied("unsupported_selection")
        focus.update(before=surrounding_text(metadata, text[:start - first]),
                     selected=surrounding_text(metadata, selected),
                     after=surrounding_text(metadata, text[end - first:]), total_chars=total)
        return focus

    def preview(self, request):
        if not well_formed_preview(request):
            raise Denied("invalid_preview")
        metadata, focus = self.bound_focus(request, calibrate=True, caret_rect=request.get("caret_rect"))
        # Field events are advisory: repeated bounded text can conceal an
        # absolute caret/length change without an epoch change. Reject before
        # rendering, rather than clearing a stale flash only after the caller
        # receives our acknowledgement.
        if request["expected_caret"] != metadata["caret"] or request["expected_total_chars"] != metadata["total_chars"]:
            raise Denied("stale_binding")
        focus["total_chars"] = metadata["total_chars"]
        focus["rendered"] = bool(self.render(focus, request["text"], request["ttl_ms"]))
        if focus["binding"]["epoch"] != self.epoch:
            self.hide()
            self.preview_hold = None
            raise Denied("stale_binding")
        self.preview_hold = ((time.monotonic() + request["ttl_ms"] / 1000, request)
                             if focus["rendered"] else None)
        self.preview_place = caret_place(focus) if focus["rendered"] else None
        return {"focus": focus}

    def reposition_preview(self):
        """Move a showing preview after the field scrolls, without a new suggestion.

        The text offset must still match the preview. A moved caret or a caret
        that can no longer be drawn ends the suggestion.
        """
        if self.preview_hold is None:
            return
        expires, request = self.preview_hold
        remaining_ms = int((expires - time.monotonic()) * 1000)
        if remaining_ms < 50:
            return
        nudged = dict(request)
        nudged["ttl_ms"] = remaining_ms
        try:
            result = self.preview(nudged)
        except Denied as error:
            if str(error) not in MALFORMED:
                self.invalidate("field_changed")
            return
        if not result["focus"]["rendered"]:
            self.invalidate("field_changed")

    def preview_moved(self):
        """End the suggestion when a showing preview's caret moved unreported.

        Chromium reports no scroll of an inner container (an overflow pane),
        so the daemon checks a showing preview on a timer. Following an
        unreported move could draw over other words; a moved caret, field,
        window or monitor ends the suggestion instead. True when it ended.
        """
        if self.preview_hold is None:
            return False
        expires, request = self.preview_hold
        if expires <= time.monotonic():
            return False
        try:
            _metadata, focus = self.bound_focus(request, calibrate=True, caret_rect=request.get("caret_rect"))
        except Denied:
            focus = None
        if focus is not None and caret_place(focus) == self.preview_place:
            return False
        self.invalidate("field_changed")
        return True

    def bound_focus(self, request, calibrate=False, caret_rect=None, selection=False):
        """The field as it is now, which must still be the request's binding and policy target."""
        binding = request["binding"]
        if not isinstance(binding, dict) or set(binding) != BINDING_FIELDS:
            raise Denied("invalid_request")
        if type(binding["epoch"]) is not int or binding["epoch"] != self.epoch:
            raise Denied("stale_binding")
        metadata, focus = self.describe(checked_app_id(binding["app_id"]), calibrate=calibrate,
                                        caret_rect=caret_rect, selection=selection)
        if request["binding"] != focus["binding"]:
            raise Denied("stale_binding")
        if request["policy_target"] != focus["target"]:
            raise Denied("target_mismatch")
        return metadata, focus

    def stable_text(self, metadata, epoch, start, end):
        """The bounded text, reread with the same field's position to prove nothing changed.

        This is a snapshot contract, never an atomic mutation claim.
        """
        text = self.backend.text(metadata, start, end)
        updated = self.backend.position(metadata)
        if self.epoch != epoch or any(updated[key] != metadata[key] for key in POSITION):
            raise Denied("stale_binding")
        if text != self.backend.text(updated, start, end) or len(text) != end - start:
            raise Denied("stale_binding")
        if any(ord(char) < 32 and char not in "\n\t\r" for char in text):
            raise Denied("invalid_text")
        return text

    def describe(self, app_id, calibrate=False, check_lock=False, caret_rect=None, selection=False):
        # Only a preview draws, so only it pays for per-request calibration.
        # Only inspect issues a binding, so only it checks the session lock;
        # snapshot and preview require that binding's unchanged epoch.
        metadata = self.backend.metadata(app_id, calibrate, check_lock, caret_rect)
        eligible(metadata, selection)
        key = {name: metadata[name] for name in IDENTITY}
        if self.tracked is not None and self.tracked != key:
            self.invalidate("focus_changed")
        self.tracked = key
        binding = {"epoch": self.epoch, **key}
        target = self.policy_target(metadata, app_id)
        if metadata.get("geometry") and self.caret_edge:
            metadata["geometry"].update(caret_edge=self.caret_edge, text_follows=self.text_follows,
                                        text_ends=self.text_ends)
        focus = {"binding": binding, "target": target, "purpose": "plain_text",
                 "caret": metadata["caret"], "selection_count": metadata["selection_count"],
                 "geometry": metadata.get("geometry")}
        if metadata.get("font"):
            focus["font"] = metadata["font"]
        if metadata.get("quiet") in QUIET_REASONS:
            focus["quiet"] = metadata["quiet"]
        return metadata, focus

    def policy_target(self, metadata, app_id):
        """The broker target of this field in this epoch.

        The broker has one browser-origin policy namespace (adapter
        "chromium"): Brave and Zen share Chromium's per-origin site grants.
        """
        target_id = hashlib.sha256(f"{metadata['bus']}\0{metadata['path']}\0{self.epoch}".encode()).hexdigest()
        if metadata["browser"]:
            return {"kind": "browser", "app_id": app_id, "target_id": target_id,
                    "origin": canonical_origin(metadata["uri"])}
        return {"kind": "desktop_application", "app_id": app_id, "target_id": target_id}


def operation_name(request):
    """The op of a well-formed request envelope, else invalid_request."""
    request_id = request.get("id") if isinstance(request, dict) else None
    if not isinstance(request, dict) or not isinstance(request_id, str) or not ID.fullmatch(request_id):
        raise Denied("invalid_request")
    if request.get("schema") != SCHEMA:
        raise Denied("invalid_request")
    op = request.get("op")
    if (not isinstance(op, str) or op not in REQUEST_FIELDS
            or not REQUEST_FIELDS[op] <= set(request) <= REQUEST_FIELDS[op] | OPTIONAL_FIELDS.get(op, set())):
        raise Denied("invalid_request")
    return op


def flag(request, name):
    """An optional boolean request field: absent is false, anything but a boolean is malformed."""
    value = request.get(name, False)
    if type(value) is not bool:
        raise Denied("invalid_request")
    return value


def checked_app_id(app_id):
    if not isinstance(app_id, str) or not APP_ID.fullmatch(app_id):
        raise Denied("invalid_request")
    return app_id


def well_formed_preview(request):
    """Valid suggestion text, lifetime and last-snapshot position."""
    ttl, caret, total = request["ttl_ms"], request["expected_caret"], request["expected_total_chars"]
    return (suggestion_text(request["text"]) and type(ttl) is int and 1 <= ttl <= MAX_PREVIEW_TTL_MS
            and type(caret) is int and type(total) is int and 0 <= caret <= total <= MAX_POSITION
            and ("caret_rect" not in request or caret_rectangle(request["caret_rect"])))


def caret_rectangle(value):
    """Window-relative physical pixels and the app's device pixel ratio."""
    if not isinstance(value, dict) or set(value) != CARET_RECT_FIELDS:
        return False
    if not all(type(value[key]) is int and 0 <= value[key] <= 65536 for key in ("x", "y", "width", "height")):
        return False
    scale = value["scale"]
    return type(scale) in (int, float) and math.isfinite(scale) and .5 <= scale <= 4


# The Fcitx addon's isolated-space rule (sanitizeSuggestion): single ASCII
# spaces between words, one leading or trailing space at most.
_SUGGESTION_SPACING = re.compile(r"^ ?\S(?: ?\S)* ?$")
ZERO_WIDTH_NON_JOINER = "\u200c"


def caret_place(focus):
    """Where a preview is anchored on screen: the calibrated caret, its field,
    window and monitor, or None for an uncalibrated field."""
    geometry = focus.get("geometry")
    if not isinstance(geometry, dict):
        return None
    return {key: geometry.get(key) for key in ("caret", "field", "window", "monitor", "scale")}


def surrounding_text(metadata, value):
    """`value` as Chromium's text-input-v3 surrounding text gives it to Fcitx.

    A contenteditable's no-break spaces, which editors insert so that typed
    spaces stay visible, arrive as plain spaces, one character for one, so
    offsets are unchanged; a form control's value arrives as it is. Gecko is
    unverified and keeps its text.
    """
    if metadata.get("web") and not metadata.get("gecko") and metadata.get("tag") not in FORM_CONTROLS:
        return value.replace("\u00a0", " ")
    return value


def suggestion_text(text):
    """The one rule for preview text: 1–MAX_PREVIEW_CHARS characters, none
    a control, format, private-use or unassigned character except a
    zero-width non-joiner between two Arabic letters, as the addon allows.
    One trailing ASCII space is the mid-sentence separator; repeated spaces
    and any other whitespace are not."""
    if (not isinstance(text, str) or not 1 <= len(text) <= MAX_PREVIEW_CHARS
            or _SUGGESTION_SPACING.fullmatch(text) is None):
        return False
    return all(not unicodedata.category(char).startswith("C") or arabic_joiner(text, index)
               for index, char in enumerate(text))


def arabic_letter(char):
    """The Fcitx addon's Arabic-script letter ranges (sanitizer.cpp)."""
    return any(low <= ord(char) <= high for low, high in (
        (0x0620, 0x063F), (0x0641, 0x064A), (0x066E, 0x066F),
        (0x0671, 0x06D3), (0x06D5, 0x06D5), (0x06E5, 0x06E6),
        (0x06EE, 0x06EF), (0x06FA, 0x06FC), (0x06FF, 0x06FF)))


def arabic_joiner(text, index):
    """Whether `text[index]` is a zero-width non-joiner between two Arabic letters."""
    return (text[index] == ZERO_WIDTH_NON_JOINER and 0 < index < len(text) - 1
            and arabic_letter(text[index - 1]) and arabic_letter(text[index + 1]))


def ltr_caret_line(direction, before, native=False):
    """Whether the caret's line up to the caret is verified left-to-right text.

    Qt reports neither a direction attribute nor glyph boxes. Its paragraphs
    take their direction from their first strong character (Unicode rule P2),
    so a native field without a direction needs a strong left-to-right letter.
    """
    line = before.rsplit("\n", 1)[-1]
    classes = {unicodedata.bidirectional(char) for char in line}
    if not line or classes & RIGHT_TO_LEFT:
        return False
    if direction in ("lr", "ltr"):
        return bool(classes & {"L", "EN"})
    return native and direction == "" and "L" in classes
