# Selection revision

Select one line of text and press Ctrl+Shift+Space: Badi checks its grammar. The same chord again offers a refined wording, then — for a word or short phrase of at most three words, with no digit, address or sentence mark — a synonym, and another synonym on each further press. A longer selection cycles grammar and refine. Fcitx's candidate list shows the change with removed words struck through and new ones bold. Tab applies it as one edit over the selection, Escape keeps the text ([ADR 0012](../../../../docs/decisions/0012-explicit-revision-of-one-selection.md), [ADR 0013](../../../../docs/decisions/0013-synonyms-for-one-short-selection.md)).

## Sub-features

- `revision-grammar` shows a grammar fix of the selection, or "Badi found nothing to change".
- `revision-refine` replaces the shown fix with a refined wording on the next chord.
- `revision-synonym` offers a synonym after refine when the selection is one to three words, and another synonym on each further press. A longer selection returns to grammar instead. Synonyms are one at a time, not a numbered list, and only on the Fcitx path.
- `revision-apply` replaces exactly the selection on Tab and records `revision_confirmed` after the read-back.
- `revision-dismiss` leaves the text and the selection unchanged on Escape. Any other key retires the revision and is not swallowed, so typing replaces the selection as it would without Badi.
- `revision-refused` sends nothing from a password, terminal or denied field, and answers pause, snooze and holds with a notice.

## How to get to it (user POV)

- In an allowed Chromium-based app's plain text field, or a native exact field, select a line of disposable text with an error ("She dont know where is the station") and press Ctrl+Shift+Space. Press it again for a refined wording. On a short selection such as one word, press it again for a synonym, and again for another. A longer line's next press returns to grammar. A selection that is already correct shows "Badi found nothing to change", and the next press continues to the next mode of that same selection. Escape or an edit starts the cycle over. Obsidian, Neovim and Writer do not revise a selection.
- The first request starts the revision model and takes a few seconds. The model stops after ten idle minutes.
- `badi revise` says whether the model is installed; `badi revise install` downloads it.

## Driving it with verify-badi

Preconditions:

- Desktop only. There is no headless driver. The phrase provider installs `FixtureReviser`, which rewrites a few known strings and offers synonyms such as `beautiful` → `lovely`, then `pretty`, but this harness never selects text or presses the chord. The protocol and engine tests in `broker/src/engine/tests/` and `protocol/v2/examples/` are the source contract.
- `badi doctor` names the installed commit and `badi revise` reports the model installed.
- A throwaway browser profile on a disposable local page, and `badi debug on --text` for the trace.

- **Grammar fix and apply.** Select the sentence, press the chord, check the marked change in Fcitx's candidate list, press Tab. The Fcitx trace records `commit` events whose `reason` is `revision_dispatched_unverified`, then `revision_confirmed`, and the field reads the fixed sentence.
- **Refine, synonym, and dismiss.** On a one-to-three-word selection, press the chord three times (grammar, refine, synonym), then Escape. The field and the selection are unchanged. On a longer line, the second press is refine and the next returns to grammar.
- **Nothing to change.** A correct selection shows "Badi found nothing to change" (`no_change`). That answer does not restart the cycle: the next chord asks for the next mode of the same selection.
- **Refusal.** In a password field the trace shows no `revision.request` and no key lines.

## Gotchas

- A rich editor is refused by the observer as `unsupported_selection`: its blocks cannot be matched to Fcitx's text exactly. The notice Fcitx shows for that denial is the one-line selection message, and the addon trace reason is `observer_snapshot_denied`.
- Only one line of 1–600 characters is revised; a longer or multi-line selection shows "Badi revises one line of up to 600 characters — select again".
- Undo is the app's: in Chromium one Ctrl+Z restores the selection's text.
- A shown revision is development-quality model output; judge the gate (language, facts kept, at most half of the words changed by a grammar check), not the wording.
