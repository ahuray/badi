# ADR 0003: extension-free editing accepts one append-only IME commit

- Status: accepted
- Date: 2026-09-26
- Scope: Fcitx IME-parity apps (Chromium, Brave and its web-app windows, Zen,
  Codex, VS Code, Cursor, Discord, Grok Bot, LibreOffice Writer)

## Context

Without an editor-owned channel, Badi reaches these apps only through the
accessibility observer and Fcitx. Physical Chromium 151 tests on 2026-09-08
showed that this route cannot carry an editor transaction:

- Blink merges an IME commit into the preceding typing group, so one undo
  removes the typed prefix together with the accepted text.
- A commit fires `beforeinput` and then edits whatever target is current: page
  script that moves focus or the caret redirects it, like a keystroke. External
  field and caret snapshots cannot pin the original field across that handler.
- Correction needs a deletion and a commit, which are not atomic: an `input`
  handler moved focus between them, so one field lost `adress ` and another
  received `address `.
- Chromium's Linux accessibility exposes no EditableText, and a real navigation
  key may be cancelled by the page, so neither can close a typing group.

## Decision

These apps accept a suggestion through exactly one append-only
`commitString`, which behaves like typed text and reports
`dispatched-unverified`. Badi documents that undo may coalesce with earlier
typing and that `beforeinput` script may redirect the text; it never claims
exact undo or verified field authority there.

Every other guard stays: sensitive and purpose denial, foreign composition
yield, exact observer identity with snapshot and caret agreement before display
and dispatch, revision, fingerprint and expiry binding, one-shot acceptance,
and no retries or synthetic keys. The native adapter negotiates no
`text_replacement`, so correction stays editor-owned (Obsidian, Bash).

## Extension (2026-10-06)

The user extended the scope to the Chromium-based apps found on this machine,
Grok Bot and Omarchy's web apps (Brave Origin `--app` windows, whose page origin
selects policy), and to LibreOffice Writer. LibreOffice is native, but every
module reports one Fcitx program id, so a manual path would claim Tab in Calc
cells; it therefore takes this same observed, append-only contract, limited by
the observer to Writer's document paragraphs.

## Consequences

Chromium-based apps and Zen get continuations without a browser extension.
Exact undo and replacement need a cooperating editor, or an upstream protocol
that carries field generation and edit revision through Wayland and the
renderer and rejects a stale target.
