"""The style-learning key in the desktop keyring (Secret Service, libsecret).

The secret is two random hex strings, a key id and a 256-bit key. Its label
and lookup attribute name only its purpose: no prose, account or site ever
reaches the keyring. The key never touches a file; it travels to the broker
over the private socket only.
"""
from __future__ import annotations

import re
import secrets

import gi

gi.require_version("Secret", "1")
from gi.repository import GLib, Secret  # noqa: E402

SCHEMA_NAME = "io.github.ahuray.badi.StyleKey"
ATTRIBUTES = {"purpose": "style-learning"}
LABEL = "Badi style learning key"
VALUE = re.compile(r"([0-9a-f]{32}) ([0-9a-f]{64})")


class KeyringError(Exception):
    """A fixed, content-free reason the key is unavailable."""


def schema():
    return Secret.Schema.new(SCHEMA_NAME, Secret.SchemaFlags.NONE,
                             {"purpose": Secret.SchemaAttributeType.STRING})


def parse(value):
    match = VALUE.fullmatch(value or "")
    if match is None:
        raise KeyringError("keyring_value_invalid")
    return match.group(1), match.group(2)


def lookup():
    """The stored key as (key_id, key), or None when there is none. Never
    prompts: a locked keyring raises ``keyring_locked`` so a background
    caller can retry after the user unlocks it."""
    try:
        service = Secret.Service.get_sync(Secret.ServiceFlags.NONE, None)
        items = service.search_sync(schema(), ATTRIBUTES, Secret.SearchFlags.NONE, None)
    except GLib.Error:
        raise KeyringError("keyring_unavailable") from None
    if not items:
        return None
    item = items[0]
    if item.get_locked():
        raise KeyringError("keyring_locked")
    try:
        item.load_secret_sync(None)
    except GLib.Error:
        raise KeyringError("keyring_unavailable") from None
    secret = item.get_secret()
    return parse(secret.get_text() if secret is not None else None)


def ensure():
    """The stored key, creating one first if needed. Interactive: the
    keyring may ask the user to unlock it."""
    try:
        value = Secret.password_lookup_sync(schema(), ATTRIBUTES, None)
    except GLib.Error:
        raise KeyringError("keyring_unavailable") from None
    if value is not None:
        return parse(value)
    key = (secrets.token_hex(16), secrets.token_hex(32))
    try:
        stored = Secret.password_store_sync(schema(), ATTRIBUTES, Secret.COLLECTION_DEFAULT, LABEL,
                                            " ".join(key), None)
    except GLib.Error:
        raise KeyringError("keyring_unavailable") from None
    if not stored:
        raise KeyringError("keyring_store_failed")
    return key


def delete():
    """Removes the stored key; returns whether one existed."""
    try:
        return bool(Secret.password_clear_sync(schema(), ATTRIBUTES, None))
    except GLib.Error:
        raise KeyringError("keyring_unavailable") from None
