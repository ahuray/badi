# ADR 0013: a synonym for one short selection rides the revision authority

- Status: accepted (user request, 2026-10-31)
- Date: 2026-10-31
- Scope: the broker's revision modes, prompt and gate, and the Fcitx
  addon's revision chord in the fields of ADR 0012

## Context

Cotypist shows synonyms for a word or short phrase the writer already
wrote, as a numbered list chosen by number keys
([future plans](../../future%20plans.md), step 1). Badi's selection
revision (ADR 0012) already replaces one selected line under exact
binding, with changes marked before anything is applied. A numbered
list would need several simultaneously grantable candidates on the
wire and digit-key capture in the addon, which today deliberately
retires its candidate on any other key.

## Decision

1. **Mode.** `synonyms` is a third revision mode beside `grammar` and
   `refine`. It reuses ADR 0012 whole: the same admission and refusals,
   binding, one model call at a time on the revision runtime, 60-second
   showing, Tab-only application over exactly the revised selection,
   read-back and privacy rules. No wire payload changes; `revision.show`
   carries the one synonym as its `text`.
2. **Scope.** The selected text, surrounding spaces aside, is one to
   three words with a letter each, and no digit, address or sentence
   mark. Anything else is answered `invalid_output`, never an error.
3. **Trigger.** Ctrl+Shift+Space keeps its cycle: grammar, then a
   refined wording, then — for a selection in that scope — a synonym,
   and another synonym on each further press. A long selection cycles
   grammar and refine as before. Escape and any edit start over.
4. **Another one.** The broker remembers the synonyms it showed for
   exactly this selection and its surroundings, in memory only, until
   the showing expires, and names them in the prompt as words not to
   use. One that comes back anyway is not shown (`no_change`). At most
   eight are kept; past that, offers start over.
5. **Gate.** The answer passes ADR 0012's gate with the refine length
   bounds, and a synonym never adds the sentence's final mark. The
   model reads the selection after its sentence's start, which must
   come back unchanged, so only the selected word or phrase at its end
   is replaced.

## Consequences

A synonym is one more marked replacement the writer reads before Tab
applies it, so a weak 1.7B answer is visible, never silently typed.
Offers are one at a time, like alternatives on a suggestion, not a
numbered list; the list remains future work if it is wanted. Synonyms
exist only where revision does — the Fcitx addon — not in the editor
clients, which have no selection revision today.
