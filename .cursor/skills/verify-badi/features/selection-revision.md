# Selection revision

Select one line of text and press Ctrl+Shift+Space: Badi checks its grammar, and the same chord again offers a refined wording. The panel shows the change with removed words struck through and new ones bold. Tab applies it as one edit over the selection, Escape keeps the text ([ADR 0012](../../../../docs/decisions/0012-explicit-revision-of-one-selection.md)).

## Sub-features

- `revision-grammar` shows a grammar fix of the selection, or "Badi found nothing to change".
- `revision-refine` replaces the shown fix with a refined wording on the second chord, and back.
- `revision-apply` replaces exactly the selection on Tab and records `revision_confirmed` after the read-back.
- `revision-dismiss` leaves the text and the selection unchanged on Escape or any other key.
- `revision-refused` sends nothing from a password, terminal or denied field, and answers pause, snooze and holds with a notice.

## How to get to it (user POV)

- In an allowed Chromium-based app's plain text field, or a native exact field, select a line of disposable text with an error ("She dont know where is the station") and press Ctrl+Shift+Space.
- The first request starts the revision model and takes a few seconds; later ones take about a second for grammar and two for a refinement.
- `badi revise` says whether the model is installed; `badi revise install` downloads it.

## Driving it with verify-badi

Preconditions:

- Desktop only. There is no headless driver: the phrase fixture has no revision model, and the protocol and engine tests in `broker/src/engine/tests/` and `protocol/v2/examples/` are the source contract.
- `badi doctor` names the installed commit and `badi revise` reports the model installed.
- A throwaway browser profile on a disposable local page, and `badi debug on --text` for the trace.

- **Grammar fix and apply.** Select the sentence, press the chord, check the panel's marked change, press Tab. The trace shows `revision_dispatched_unverified` then `revision_confirmed`, and the field reads the fixed sentence.
- **Refine and dismiss.** Press the chord twice, then Escape. The field and the selection are unchanged.
- **Nothing to change.** A correct selection shows "Badi found nothing to change" (`no_change`).
- **Refusal.** In a password field the trace shows no `revision.request` and no key lines.

## Gotchas

- Rich editors are refused (`unsupported_selection`): their blocks cannot be matched to Fcitx's text exactly.
- Only one line of 1–600 characters is revised; a longer or multi-line selection shows "Badi revises one line of up to 600 characters — select again".
- Undo is the app's: in Chromium one Ctrl+Z restores the selection's text.
- A shown revision is development-quality model output; judge the gate (language, facts kept, at most half of the words changed by a grammar check), not the wording.
