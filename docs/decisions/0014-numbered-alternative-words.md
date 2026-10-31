# ADR 0014: the alternatives chord may list words to pick by number

- Status: accepted (user request, 2026-10-31)
- Date: 2026-10-31
- Scope: the broker's alternatives generation and binding, the v2 wire,
  and the Fcitx addon's candidate panel and digit keys

## Context

Cotypist shows its alternatives as a numbered list chosen by number
keys ([future plans](../../future%20plans.md), step 1). Badi's
Ctrl+Shift+Space cycled one replacement at a time, because the broker
binds exactly one shown suggestion — revision, fingerprint, caret and
expiry — and acceptance is one-shot. A list of simultaneously grantable
continuations would multiply that binding, and each extra continuation
would cost another model generation inside the one explicit request.

## Decision

1. **The broker still binds one shown suggestion.** A list carries only
   words. Picking a word asks the broker to generate that word's
   continuation, which is then shown and bound like any suggestion.
   Nothing in a list is itself acceptable text.
2. **Capability.** A session that negotiates `alternative_list` gets a
   `suggestion.list` of up to four other first words when the chord
   asks; any other session keeps the one-at-a-time cycle and is never
   sent the new message. The Fcitx addon is the first taker; the editor
   clients keep cycling.
3. **Binding.** The list names the shown suggestion it replaces and
   that suggestion's exact coordinates, and lives for the session's
   suggestion lifetime. A pick is a session `alternatives` control
   carrying one listed `word`; the broker checks it against the live
   list and spends the list on the first pick. A pick for an unlisted
   word, a spent list or a moved context is `stale`. Generation that
   cannot continue the picked word falls back to the last shown
   continuation, as an exhausted cycle does.
4. **Generation.** One list is one provider call. The local model reuses
   the first-token candidates a single continuation already fetches and
   finishes each admissible candidate's first word; a candidate that
   fails is dropped, never retried. A pick is one more call, asked for
   the picked word.
5. **Keys.** The addon shows the words as numbered candidates. While
   Badi's own list is up and no foreign IME UI is, a plain digit picks
   its word and Escape dismisses the list; every other key retires the
   list and types normally. Digits are taken in the pre-input-method
   phase, before a terminal's typed line could record them as text.
   The list never yields Tab or any other key's meaning.

## Consequences

- The wire gains `suggestion.list`, the `alternative_list` capability
  and the optional `word` on a session `alternatives` control; the
  schema and examples grow with them.
- Broker and addon install together and their connection negotiation
  stays exact, as with every capability before this one. The session
  capability is what gates the list: a client that never asks for it
  (today every editor client) keeps cycling and is never sent
  `suggestion.list`.
- Alternatives latency is bounded by two calls (list, then the pick's
  continuation) instead of one per candidate.
