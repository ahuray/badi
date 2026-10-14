"""Text guards shared with adapters/shared/text-safety.mjs, for Writer context.

Context and replacements are checked here before they reach the broker or the
document. Nothing in this module logs or stores text.
"""

import re
import unicodedata

MAX_BEFORE = 512
MAX_AFTER = 128
_ARABIC_LETTER = re.compile(
    "[ؠ-ؿف-يٮ-ٯٱ-ۓەۥ-ۦ"
    "ۮ-ۯۺ-ۼۿ]")
_UNSAFE_CATEGORIES = {"Cc", "Cf", "Zl", "Zp", "Cs"}
_CORRECTION_WORD = re.compile(r"[a-z]{3,24} ?")
_LANGUAGE_TAG = re.compile(r"[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*")
# U+FFFC stands in for an anchored object or other non-text content; such a
# paragraph's string is not the text the user sees.
OBJECT_REPLACEMENT = "￼"


def _arabic_letter(character):
    return character is not None and _ARABIC_LETTER.fullmatch(character) is not None


def has_unsafe_text(value, allow_context_whitespace=False):
    """Controls, format and separator characters, and lone surrogates.

    A Persian ZWNJ is allowed only between two Arabic-block letters, as in
    protocol/orthographic-joiner-fixtures.json.
    """
    for index, character in enumerate(value):
        if character == "‌":
            previous = value[index - 1] if index else None
            following = value[index + 1] if index + 1 < len(value) else None
            if not (_arabic_letter(previous) and _arabic_letter(following)):
                return True
            continue
        if allow_context_whitespace and character in "\n\t":
            continue
        if unicodedata.category(character) in _UNSAFE_CATEGORIES:
            return True
    return False


def valid_correction(before, original, replacement):
    """A correction owns one exact lowercase English word before the caret,
    with at most its own typed space, and never a wider document range."""
    if not all(isinstance(value, str) for value in (before, original, replacement)):
        return False
    if (not _CORRECTION_WORD.fullmatch(original) or not _CORRECTION_WORD.fullmatch(replacement)
            or original == replacement or original.endswith(" ") != replacement.endswith(" ")
            or not before.endswith(original)):
        return False
    start = len(before) - len(original)
    return start == 0 or before[start - 1].isspace()


_CLOSING = ".,;:!?…)]}\"'»”’"


def correction_span(before, after):
    """Where Alt+F7 corrects, as (target, tail): `target` is the paragraph up
    to the end of the word at or before the caret, with that word's own typed
    space, and `tail` the rest, which the correction leaves untouched. The
    word may hold the caret, end at it, or sit before closing punctuation.
    None without such a word."""
    if not isinstance(before, str) or not isinstance(after, str):
        return None
    if before[-1:].isalpha():
        run = next((index for index, character in enumerate(after) if not character.isalpha()), len(after))
        return before + after[:run], after[run:]
    if before.endswith(" ") and before[-2:-1].isalpha():
        return before, after
    end = len(before.rstrip(" ").rstrip(_CLOSING))
    if end and before[end - 1].isalpha() and end < len(before):
        return before[:end], before[end:] + after
    return None


def valid_language_tag(tag):
    """The broker's language-tag shape: 2-35 ASCII alphanumerics in subtags."""
    return isinstance(tag, str) and 2 <= len(tag) <= 35 and _LANGUAGE_TAG.fullmatch(tag) is not None


def language_tag(language, country=""):
    """A BCP 47-shaped tag from a Writer character locale, or None."""
    if not isinstance(language, str) or not language or language.lower() in ("zxx", "qlt"):
        return None
    tag = language if not country else f"{language}-{country}"
    return tag if valid_language_tag(tag) else None


def bounded_context(before, after):
    """The broker's window: the last 512 scalars before and 128 after the caret,
    or None when the text is not plain, safe prose."""
    if not isinstance(before, str) or not isinstance(after, str):
        return None
    if OBJECT_REPLACEMENT in before or OBJECT_REPLACEMENT in after:
        return None
    before = before[-MAX_BEFORE:]
    after = after[:MAX_AFTER]
    if not before.strip() or has_unsafe_text(before, True) or has_unsafe_text(after, True):
        return None
    return before, after
