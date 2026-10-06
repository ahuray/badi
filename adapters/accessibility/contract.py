"""Focused accessibility acquisition contract; independent of desktop bindings."""
from __future__ import annotations

import hashlib
import os
import re
import unicodedata
from urllib.parse import urlsplit

SCHEMA = "badi.accessibility.v1"
MAX_FRAME = 16 * 1024
MAX_BEFORE = 512
MAX_AFTER = 128
MAX_POSITION = 2**31 - 1
ID = re.compile(r"[A-Za-z0-9_.-]{1,64}\Z")
APP_ID = re.compile(r"[a-zA-Z0-9_.-]{1,128}")
REQUEST_FIELDS = {"inspect": {"schema", "id", "op", "app_id"},
                  "snapshot": {"schema", "id", "op", "binding", "policy_target"},
                  "preview": {"schema", "id", "op", "binding", "policy_target", "text", "ttl_ms",
                              "expected_caret", "expected_total_chars"},
                  "status": {"schema", "id", "op"},
                  "hide": {"schema", "id", "op"}}
IDENTITY = ("bus", "path", "process_id", "app_id", "uri")
BINDING_FIELDS = frozenset({"epoch", *IDENTITY})
POSITION = ("caret", "total_chars", "selection_count")
# A malformed request changes no field state; every other denial withdraws it.
MALFORMED = frozenset({"invalid_request", "invalid_preview"})
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


def eligible(metadata):
    """Reject purpose/selection before asking a provider for any prose."""
    if metadata["role"] == "password text":
        raise Denied("sensitive_field")
    if not all(metadata.get(flag, False) for flag in ("focused", "editable", "showing", "visible", "enabled")):
        raise Denied("ineligible_field")
    if metadata["selection_count"] != 0:
        raise Denied("selection_present")
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
    if metadata["caret"] < 0 or metadata["caret"] > metadata["total_chars"]:
        raise Denied("invalid_caret")


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
        self.authorize = lambda _binding: None
        self.disarm = lambda: None
        self.caret_edge = None
        self.operations = {"status": self.status, "hide": self.hide_preview, "inspect": self.inspect,
                           "snapshot": self.snapshot, "preview": self.preview}

    def invalidate(self, reason):
        previous = self.tracked
        self.withdraw()
        self.epoch += 1
        self.tracked = None
        self.notify({"schema": SCHEMA, "event": "invalidate", "epoch": self.epoch,
                     "reason": reason, "app_id": previous["app_id"] if previous else ""})

    def withdraw(self):
        """Clear the preview, the field's event match and the verified caret edge."""
        self.hide()
        self.disarm()
        self.caret_edge = None

    def request(self, request):
        request_id = request.get("id") if isinstance(request, dict) else None
        try:
            operation = self.operations[operation_name(request)]
            return {"schema": SCHEMA, "id": request_id, "ok": True, **operation(request)}
        except Denied as error:
            if str(error) not in MALFORMED:
                self.withdraw()
            return {"schema": SCHEMA, "id": request_id, "ok": False, "error": str(error), "epoch": self.epoch}

    def status(self, _request):
        return {"status": {"ready": True, "process_id": os.getpid(), "protocol_version": 1}}

    def hide_preview(self, _request):
        self.hide()
        return {"hidden": True, "epoch": self.epoch}

    def inspect(self, request):
        _metadata, focus = self.describe(checked_app_id(request["app_id"]), check_lock=True)
        return {"focus": focus}

    def snapshot(self, request):
        metadata, focus = self.bound_focus(request)
        self.authorize(focus["binding"])
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
        if scope == "block":
            # Only the caret's block serializes exactly: Fcitx's text must end with it.
            focus["scope"] = "block"
        self.caret_edge = "right" if ltr_caret_line(metadata.get("direction"), before) else None
        if focus["geometry"] and self.caret_edge:
            focus["geometry"]["caret_edge"] = self.caret_edge
        focus.update(before=before, after=after, total_chars=metadata["total_chars"])
        return {"focus": focus}

    def preview(self, request):
        if not well_formed_preview(request):
            raise Denied("invalid_preview")
        metadata, focus = self.bound_focus(request, calibrate=True)
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
            raise Denied("stale_binding")
        return {"focus": focus}

    def bound_focus(self, request, calibrate=False):
        """The field as it is now, which must still be the request's binding and policy target."""
        binding = request["binding"]
        if not isinstance(binding, dict) or set(binding) != BINDING_FIELDS:
            raise Denied("invalid_request")
        if type(binding["epoch"]) is not int or binding["epoch"] != self.epoch:
            raise Denied("stale_binding")
        metadata, focus = self.describe(checked_app_id(binding["app_id"]), calibrate=calibrate)
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

    def describe(self, app_id, calibrate=False, check_lock=False):
        # Only a preview draws, so only it pays for per-request calibration.
        # Only inspect issues a binding, so only it checks the session lock;
        # snapshot and preview require that binding's unchanged epoch.
        metadata = self.backend.metadata(app_id, calibrate, check_lock)
        eligible(metadata)
        key = {name: metadata[name] for name in IDENTITY}
        if self.tracked is not None and self.tracked != key:
            self.invalidate("focus_changed")
        self.tracked = key
        binding = {"epoch": self.epoch, **key}
        target = self.policy_target(metadata, app_id)
        if metadata.get("geometry") and self.caret_edge:
            metadata["geometry"]["caret_edge"] = self.caret_edge
        focus = {"binding": binding, "target": target, "purpose": "plain_text",
                 "caret": metadata["caret"], "selection_count": metadata["selection_count"],
                 "geometry": metadata.get("geometry")}
        return metadata, focus

    def policy_target(self, metadata, app_id):
        """The broker target of this field in this epoch.

        The broker has one browser-origin policy namespace (adapter
        "chromium"): Brave and Zen share Chromium's per-origin site grants.
        """
        target_id = hashlib.sha256(f"{metadata['bus']}\0{metadata['path']}\0{self.epoch}".encode()).hexdigest()
        if metadata["browser"]:
            return {"kind": "browser", "app_id": "chromium", "target_id": target_id,
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
    if not isinstance(op, str) or op not in REQUEST_FIELDS or set(request) != REQUEST_FIELDS[op]:
        raise Denied("invalid_request")
    return op


def checked_app_id(app_id):
    if not isinstance(app_id, str) or not APP_ID.fullmatch(app_id):
        raise Denied("invalid_request")
    return app_id


def well_formed_preview(request):
    """Valid suggestion text, lifetime and last-snapshot position."""
    ttl, caret, total = request["ttl_ms"], request["expected_caret"], request["expected_total_chars"]
    return (suggestion_text(request["text"]) and type(ttl) is int and 1 <= ttl <= 5000
            and type(caret) is int and type(total) is int and 0 <= caret <= total <= MAX_POSITION)


def suggestion_text(text):
    """1–160 characters, none a control or format character except a
    zero-width non-joiner between two letters."""
    if not isinstance(text, str) or not 1 <= len(text) <= 160 or not text.strip():
        return False
    return all(not unicodedata.category(char).startswith("C") or joins_letters(text, index)
               for index, char in enumerate(text))


def joins_letters(text, index):
    return (text[index] == "‌" and 0 < index < len(text) - 1
            and text[index - 1].isalpha() and text[index + 1].isalpha())


def ltr_caret_line(direction, before):
    """Whether the caret's line up to the caret is verified left-to-right text."""
    line = before.rsplit("\n", 1)[-1]
    classes = {unicodedata.bidirectional(char) for char in line}
    return direction in ("lr", "ltr") and bool(line) and bool(classes & {"L", "EN"}) and not classes & RIGHT_TO_LEFT
