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
ID = re.compile(r"[A-Za-z0-9_.-]{1,64}\Z")


class Denied(Exception):
    """A fixed, prose-free failure reason safe to return over IPC."""


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
    if metadata["role"] == "password text" or metadata.get("sensitive", False):
        raise Denied("sensitive_field")
    if not all(metadata.get(flag, False) for flag in ("focused", "editable", "showing", "visible", "enabled")):
        raise Denied("ineligible_field")
    if metadata["selection_count"] != 0:
        raise Denied("selection_present")
    if metadata["role"] not in ("entry", "text", "paragraph"):
        raise Denied("unsupported_field")
    tag = metadata.get("tag", "")
    input_type = metadata.get("input_type", "")
    if metadata["browser"]:
        if tag == "input" and input_type != "text":
            raise Denied("unsupported_field")
        if tag not in ("input", "textarea", "div", "p"):
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

    def invalidate(self, reason):
        previous = self.tracked
        self.hide()
        self.disarm()
        self.caret_edge = None
        self.epoch += 1
        self.tracked = None
        self.notify({"schema": SCHEMA, "event": "invalidate", "epoch": self.epoch,
                     "reason": reason, "app_id": previous["app_id"] if previous else ""})

    def describe(self, app_id):
        metadata = self.backend.metadata(app_id)
        eligible(metadata)
        key = {name: metadata[name] for name in ("bus", "path", "process_id", "app_id", "uri")}
        if self.tracked is not None and self.tracked != key:
            self.invalidate("focus_changed")
        self.tracked = key
        binding = {"epoch": self.epoch, **key}
        target_id = hashlib.sha256(f"{key['bus']}\0{key['path']}\0{self.epoch}".encode()).hexdigest()
        if metadata["browser"]:
            target = {"kind": "browser", "app_id": "chromium", "target_id": target_id,
                      "origin": canonical_origin(metadata["uri"])}
        else:
            target = {"kind": "desktop_application", "app_id": app_id, "target_id": target_id}
        if metadata.get("geometry") and self.caret_edge:
            metadata["geometry"]["caret_edge"] = self.caret_edge
        focus = {"binding": binding, "target": target, "purpose": "plain_text",
                 "caret": metadata["caret"], "selection_count": metadata["selection_count"],
                 "geometry": metadata.get("geometry")}
        return metadata, focus

    def request(self, request):
        request_id = request.get("id") if isinstance(request, dict) else None
        try:
            if not isinstance(request, dict) or not isinstance(request_id, str) or not ID.fullmatch(request_id):
                raise Denied("invalid_request")
            if request.get("schema") != SCHEMA:
                raise Denied("invalid_request")
            op = request.get("op")
            fields = {"inspect": {"schema", "id", "op", "app_id"},
                      "snapshot": {"schema", "id", "op", "binding", "policy_target"},
                      "preview": {"schema", "id", "op", "binding", "policy_target", "text", "ttl_ms",
                                  "expected_caret", "expected_total_chars"},
                      "status": {"schema", "id", "op"},
                      "hide": {"schema", "id", "op"}}
            if not isinstance(op, str) or op not in fields or set(request) != fields[op]:
                raise Denied("invalid_request")
            if op == "status":
                return {"schema": SCHEMA, "id": request_id, "ok": True,
                        "status": {"ready": True, "process_id": os.getpid(), "protocol_version": 1}}
            if op == "hide":
                self.hide()
                return {"schema": SCHEMA, "id": request_id, "ok": True, "hidden": True, "epoch": self.epoch}
            if op == "preview":
                value, ttl = request["text"], request["ttl_ms"]
                if not isinstance(value, str) or not 1 <= len(value) <= 160 or not value.strip() or type(ttl) is not int or not 1 <= ttl <= 5000:
                    raise Denied("invalid_preview")
                caret, total = request["expected_caret"], request["expected_total_chars"]
                if type(caret) is not int or type(total) is not int or not 0 <= caret <= total <= 2**31 - 1:
                    raise Denied("invalid_preview")
                for index, char in enumerate(value):
                    if unicodedata.category(char).startswith("C") or char in "\n\r\t":
                        if not (char == "\u200c" and index > 0 and index + 1 < len(value) and value[index-1].isalpha() and value[index+1].isalpha()):
                            raise Denied("invalid_preview")
            if op == "inspect":
                app_id = request["app_id"]
            else:
                binding = request["binding"]
                if not isinstance(binding, dict) or set(binding) != {"epoch", "bus", "path", "process_id", "app_id", "uri"}:
                    raise Denied("invalid_request")
                if type(binding["epoch"]) is not int or binding["epoch"] != self.epoch:
                    raise Denied("stale_binding")
                app_id = binding["app_id"]
            if not isinstance(app_id, str) or not re.fullmatch(r"[a-zA-Z0-9_.-]{1,128}", app_id):
                raise Denied("invalid_request")
            metadata, focus = self.describe(app_id)
            if op in ("snapshot", "preview"):
                if request["binding"] != focus["binding"]:
                    raise Denied("stale_binding")
                if request["policy_target"] != focus["target"]:
                    raise Denied("target_mismatch")
            if op == "preview":
                # Field events are advisory: repeated bounded text can conceal
                # an absolute caret/length change without an epoch change.
                # Reject before rendering, rather than clearing a stale flash
                # only after the caller receives our acknowledgement.
                if request["expected_caret"] != metadata["caret"] or request["expected_total_chars"] != metadata["total_chars"]:
                    raise Denied("stale_binding")
                focus["total_chars"] = metadata["total_chars"]
                focus["rendered"] = bool(self.render(focus, request["text"], request["ttl_ms"]))
                if focus["binding"]["epoch"] != self.epoch:
                    self.hide()
                    raise Denied("stale_binding")
            if op == "snapshot":
                self.authorize(focus["binding"])
                caret = metadata["caret"]
                start, end = max(0, caret - MAX_BEFORE), min(metadata["total_chars"], caret + MAX_AFTER)
                text = self.backend.text(metadata, start, end)
                # Recheck both metadata and the bounded text after acquisition.
                # This is a snapshot contract, never an atomic mutation claim.
                updated, again = self.describe(app_id)
                if again["binding"] != focus["binding"] or any(updated[k] != metadata[k] for k in ("caret", "total_chars", "selection_count")):
                    raise Denied("stale_binding")
                if text != self.backend.text(updated, start, end) or len(text) != end - start:
                    raise Denied("stale_binding")
                if any(ord(c) < 32 and c not in "\n\t\r" for c in text):
                    raise Denied("invalid_text")
                split = caret - start
                before = text[:split]
                line = before.rsplit("\n", 1)[-1]
                bidi = [unicodedata.bidirectional(char) for char in line]
                self.caret_edge = "right" if metadata.get("direction") in ("lr", "ltr") and line and any(c in ("L", "EN") for c in bidi) and not any(c in ("R", "AL", "AN") for c in bidi) else None
                if focus["geometry"] and self.caret_edge:
                    focus["geometry"]["caret_edge"] = self.caret_edge
                focus.update(before=before, after=text[split:], total_chars=metadata["total_chars"])
            return {"schema": SCHEMA, "id": request_id, "ok": True, "focus": focus}
        except Denied as error:
            if str(error) not in ("invalid_request", "invalid_preview"):
                self.hide()
                self.disarm()
                self.caret_edge = None
            return {"schema": SCHEMA, "id": request_id, "ok": False, "error": str(error), "epoch": self.epoch}
