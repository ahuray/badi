# ADR 0004: type-through carries text, never authority

- Status: accepted
- Date: 2026-10-06
- Scope: the broker's suggestion admission and every adapter that publishes a
  new context and requests again (Fcitx native and IME-parity fields, Obsidian,
  Bash)

## Context

[VISION](../../VISION.md) promises that you can type through a suggestion, and
Cotypist keeps a suggestion while you type its next characters. Badi instead
dropped the suggestion on every key and asked the model again. Each edit ends
an observed field's binding: the observer starts a new epoch, so Fcitx opens a
new broker session with a new target id. No suggestion, grant or lease can
outlive that edit, and this ADR keeps it that way.

## Decision

The broker keeps the most recently shown continuation as text only:

- its context (`before`, `after`, language);
- its app or site, meaning adapter kind, target kind, app id and origin,
  without the per-epoch target id;
- its absolute deadline.

A later request whose context passed every admission check may be answered
from it without inference when all of these hold:

- the request is in the same app or site;
- `after` and the language are unchanged;
- `before` equals the old `before` plus a typed, non-empty proper prefix of the
  suggestion;
- at least 500 ms of the original deadline remain.

If the adapter's bounded window has dropped leading text since, `before` may
instead keep at least 128 bytes of the old text exactly. An automatic request
whose `before` equals the carry's own, with nothing typed since, is answered
with the whole carried text: a keystroke in Chromium also changes the observed
field's accessibility epoch, so the same context is admitted again under a new
session (live trial, 2026-10-13), and a model call there would replace the
remainder already on screen. An explicit request for that context asks the
model.

The answer is a new suggestion, with a new id, bound to the new session,
revision and fingerprint. Its shape is checked against the new context; a
continuation of the word at the caret is allowed, because the user typed that
word's start. It keeps the original deadline, so typing never extends how long
derived text stays visible. It also keeps the original `Shown` aggregate, so
one suggestion is counted once.

The adapter then shows it only after its usual checks, which for an observed
field are an observer snapshot agreeing with Fcitx's text before display and
again before dispatch. Acceptance stays explicit and one-shot.

The carry ends in any of these cases:

- dismissal;
- acceptance of the whole suggestion;
- an authority epoch, pause or settings change;
- shutdown;
- its deadline;
- a newer suggestion replacing it.

A word acceptance keeps the carry, so the rest of the suggestion can follow the
accepted word. Spelling corrections are never carried. The carry lives only in
broker memory, holding at most one context window and suggestion.

Adapters may shorten their typing pause after a key that types the suggestion's
next characters, or after a word acceptance. The Fcitx addon uses 30 ms rather
than 120 ms.

On every surface Tab accepts the next word and Ctrl+Right all of it over
Badi's own live candidate; otherwise both stay the application's keys
(since 2026-10-16; Fcitx and Neovim previously had the two swapped, because
an observed field takes a moment to show the carried remainder and a second
Tab in that gap would reach the app and move focus). That gap is now closed
without giving the preview any authority: after a word acceptance whose
remainder returns on its own, the Fcitx addon holds at most one Tab for
about 700 ms and, when the remainder is displayed and still begins with the
word that Tab meant, accepts that word through the normal broker grant for
the newly displayed suggestion. Any other key, Escape, a focus change or
the timeout drops the held Tab; it is never replayed to the app.

## Consequences

- No model call follows a correctly typed character, so the remainder
  reappears after the adapter's own checks rather than after inference. Tests:
  `broker/src/engine/type_through.rs`, the engine tests, the Fcitx real-broker
  smoke (`--type-through`) and `adapters/fcitx5/tests/observed-desktop.py`.
- An observed field still hides the preview between the keystroke and the
  re-verified remainder. A seamless overlay would need the preview to outlive
  the binding, which this decision does not allow.
- After an applied commit an editor still has to publish fresh context and ask
  again; the carry answers that request only after a word acceptance.
