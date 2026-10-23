# ADR 0012: an explicit request may revise one selection

- Status: accepted (user request, 2026-10-23)
- Date: 2026-10-23
- Scope: the broker's revision authority and its on-demand revision
  runtime, the Fcitx addon in native exact and observed IME-parity fields,
  and the observer's selection snapshot

## Context

Badi appended suffixes and replaced one misspelled word
([ADR 0011](0011-checked-corrections-in-append-only-apps.md)). Its vision
listed "arbitrary rewrite" as a non-goal. The user asked for a hotkey that
checks the grammar of the selected text, or refines it, and applies the
result. Two things make that different from a suggestion: the text is
already written, so a revision replaces it, and the base model that writes
continuations cannot follow an instruction such as "fix the grammar".

## Decision

1. **Trigger.** Ctrl+Shift+Space while text is selected asks for a grammar
   check of that selection, and pressing it again while a revision shows
   asks for a refined wording instead (and back). It is always explicit;
   nothing is ever revised on its own. Without a selection the chord keeps
   its meaning (an explicit suggestion).
2. **Scope.** One selection on one line, 1–600 characters, in a field of
   an app or site whose rule allows suggestions. Sensitive, purpose-typed,
   terminal and denied fields, foreign composition, pause, snooze, desktop
   holds and editor focus refuse it, as they refuse a suggestion, in an
   answer the adapter can show rather than an error. The request binds the
   field identity, fingerprint, revision and the exact selection offsets and
   text. In an observed field the observer reads the selection only after
   policy allows the app, and only in a plain field: a rich editor's blocks
   cannot be matched to Fcitx's text exactly.
3. **Model.** An instruction-tuned model from the catalog runs in its own
   runtime, started on the first request and stopped after ten idle
   minutes, with its own socket and token. It is never the continuation
   runtime, and its failure or exit never stops suggestions. The model is
   told the field's language and given one worked example in that
   language, and reads a mid-sentence selection after the start of its
   sentence, which must come back unchanged. The first model is Qwen3 1.7B (Apache-2.0), already in the
   catalog.
4. **Gate.** The broker keeps a revision only when it is one line in the
   same language, keeps every number, address, handle and name of the
   selection, stays within the mode's length bounds, and (for a grammar
   check of more than three words) changes no more than half of the words,
   ignoring case and punctuation. An unchanged result is "nothing
   to fix". A selection that starts mid-sentence keeps its first letter's
   case, and one that is followed by more text on its line gains no final
   mark.
5. **Show.** The adapter shows the revision with its changes marked and
   applies nothing until the user accepts it with Tab; Escape or any other
   key retires it, and it expires after 60 seconds.
6. **Apply.** Just before applying, the adapter checks that the selection
   is still exactly the one revised (and, in an observed field, that the
   observer agrees). One `commitString` then replaces the selection, as
   typing over a selection does, and the result is `dispatched-unverified`.
   Within a second the adapter reads the field back; the text around the
   caret must read as the revision left it, or a notice names an
   unconfirmed revision. There are no retries, synthetic keys or clipboard
   use.
7. **Privacy.** The debug trace records modes, lengths, timings and
   outcomes; only a `--text` run records the selection and its revision
   ([ADR 0008](0008-debug-trace-may-hold-text-on-request.md)). Revisions
   are never learned from and never carried forward
   ([ADR 0004](0004-type-through-carries-text-not-authority.md)).

## Consequences

A revision replaces text the user wrote, so it is shown in full before it
is applied and one Ctrl+Z restores the selection where the app merges the
edit into one undo step. Page script can still intercept the commit, as
with ADR 0011; the read-back reports it but cannot prevent it. The
revision model costs about 1.6 GB of memory while it runs and starts in a
few seconds on first use. A 1.7B model is weaker in Spanish and Portuguese
than in English, German and French; the diff makes a weak revision
visible rather than silently applied.
