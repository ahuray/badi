# ADR 0011: append-only apps take checked corrections

- Status: accepted (user decision, 2026-10-23)
- Date: 2026-10-23
- Scope: broker spelling authority, Fcitx module and accessibility observer
  for the IME-parity apps of [ADR 0003](0003-ime-parity-append-only.md)
  except LibreOffice

## Context

[ADR 0003](0003-ime-parity-append-only.md) kept Chromium, Brave Origin, Zen,
Codex, the Claude app, VS Code, Cursor, Discord and Grok Bot append-only. A
correction there is a delete and then a commit, and the app applies them as
two edits. In 2026-09, page script that moved focus between them split one
correction across two fields. Because of that, Badi let every typo in those
apps go by, and these are where the user writes most. Spelling itself
worked: the live broker turned `sned` into `send`, `qick` into `quick`,
`Dokumnet` into `Dokument` and `mesage` into `message`.

## Decision

Corrections reach these apps by default (user decision, 2026-10-23), each
checked before and after it is applied:

1. **Offer.** The broker grants spelling replacement to an observed field of
   any IME-parity app but LibreOffice, whose Writer extension keeps its own
   corrections.
2. **Show.** The grey preview draws the fix as `→ address` after the typed
   typo, and the Fcitx panel fallback shows `adress → address`.
3. **Check before.** On Tab, the observer re-reads the exact field and must
   agree with Fcitx's text and caret, as every observed dispatch does.
4. **Apply.** Fcitx asks the app to delete exactly the typed typo, then
   commits the fix. The result stays `dispatched-unverified`.
5. **Read back.** For up to a second, the addon waits for the app's own text
   to end as the correction left it. It records `correction_confirmed`, or
   `correction_unconfirmed` with the notice "Badi could not confirm that
   correction — check the text". An update between the two edits (typo gone,
   fix not yet typed) gives no verdict.

The observer also stops treating `TextAttributesChanged` as a field change.
Chromium's spellchecker underlines a misspelled word about 150 ms after it is
typed. That changes neither text nor caret, but it used to end every
suggestion shown for a typo.

## Consequences

Live in Chromium 155 at installed `2b90805` (2026-10-23, an isolated profile
and disposable text), `…to the adress ` showed `→ address`. Tab corrected it
and the read-back confirmed it, and one Ctrl+Z restored the typo.

A hostile page that moves focus to another field on the deletion still
splits the correction. Measured there, the first field lost `adress `, the
second received `address `, and the notice named the unconfirmed correction.
The check finds the split but cannot prevent it; the user accepted that risk.
Exact undo stays the app's: in Chromium one step undoes both edits, and other
apps may differ. Everything else ADR 0003 requires still applies.

The trial driver, fixture and hostile-page capture are in
`output/native-trials/2026-10-23-typo-fixes/`.
