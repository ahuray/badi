# ADR 0010: opt-in context finishes words from the window and learned words

- Status: accepted (user decision, 2026-10-20)
- Date: 2026-10-20
- Scope: settings, broker, accessibility observer, Fcitx module, Omarchy panel

## Context

The [vision](../../VISION.md#context-firewall) allows bounded same-window
context and local learning as opt-ins with global and per-app opt-outs. Two
earlier forms failed in the Prediction Lab. Window text in the prompt drifted
in unrelated cases and copied a fact the writer was changing. A logit bias
toward the window's words changed almost nothing
([Lab runbook](../../evaluation/writing/README.md#window-context)).

Finishing the word being typed with an exact word in view does not ask the
model to guess. On a second independent set of 60 mid-word cases, it gave 23
useful additions against 1, and 23 of its 26 finishes were right. The other 3
were traps: a different word in view starting with the same five letters
("Domini" became "Dominic", "Dominican" meant). Harmful additions therefore
rose from 4 to 7, failing the registered gates. The user accepted that cost.

## Decision

**Off until chosen.** The settings key `context_words` is absent until the user
chooses. While it is absent, the Omarchy panel asks once. Users control it
with these commands:

- `badi context on|off` switches it for every app and site;
- `badi context app ID on|off` and `badi context site ORIGIN on|off` record
  per-subject exceptions in `context_words_off`;
- `badi context forget` deletes the learned words.

Context grants nothing: no acquisition, suggestion or edit. Any pause turns it
off everywhere.

**Window words.** While the policy reply for an observed web field says
`context_words`, the Fcitx module asks the observer once per field visit for
the text displayed near that field. The observer walks the field's own
document, nearest first, for at most 2,000 characters, 400 nodes or 120 ms. It
never reads another field (anything editable), a password, a hidden node or a
nested document. The module sends that text as `window_text` on the visit's
`context.changed` frames. The broker takes its whole words for one request
and keeps none of them. Native apps send no window text.

**Learned words.** The broker learns, from prose fields where context is on,
the finished words the user wrote: letters only, six to 24 long. It counts each
word once per field visit and never learns from a terminal's typed line, which
may hold a passphrase. Storage and expiry:

- Each subject's words are sealed with the keyring key that style learning
  uses; without that key nothing is learned.
- A word is offered after it appeared in two field visits.
- A word unused for 90 days is deleted.
- Turning context off for a subject deletes its words. Turning context off
  everywhere keeps them until they expire or `badi context forget` runs.

Linux apps may learn this way. Style samples still may not.

**Finishing a word.** After five typed letters, Badi offers the rest of the one
longer word in view (window, learned or earlier in the draft) that starts with
them, matched case-sensitively and never normalized
([broker runbook](../../broker/README.md#vocabulary-completion)). When several
words fit, it offers one only if that word begins all the others. English
function words and plain inflections of a finished word abstain. This runs
before spelling and before the model, which answers when it abstains.

The text is processed on the device only and never reaches a prompt, a log or
a remote service. Only an explicit `badi debug on --text` trace records the
finished word ([ADR 0008](0008-debug-trace-may-hold-text-on-request.md)).

## Consequences

- With context on, a word in view that shares five typed letters with the word
  meant shows as a wrong ghost suggestion until the next key. Tab would accept
  it. The Lab measured this at 3 of 13 traps on the second set.
- The observer reads more of a web page than the Lab-only heading and
  paragraph, but only while context is on and only once per field visit.
- Learned words are personal data at rest: sealed, bounded (512 per subject,
  64 subjects), expiring, deletable in one step, and readable only with the
  keyring key.
