# Badi broker

The local policy broker and its control CLI. Adapters acquire and edit the
document; the broker owns target policy, session state, suggestion and commit
authority, the local writing model and content-free metrics. It speaks only on
a Unix socket and never drives a keyboard, clipboard or accessibility API.

- `badi-broker` serves the socket. `--provider phrase` selects the
  deterministic test fixture instead of the writing model. Where a session
  negotiated replacement, the fixture also offers the model provider's
  inference-free dictionary correction (one unambiguous single edit, such as
  `adress` → `address`), so editors can test exact replacement.
- `badictl` sends control, health, settings and probe requests and prints
  offline hardware and model advice.

The Prediction Lab is a separate crate built on this crate's public API; this
crate never depends on it. `build.rs` embeds the Git commit of the Rust inputs
and whether they were dirty (`--version` prints both; packagers may set
`BADI_BUILD_COMMIT` and `BADI_BUILD_DIRTY`).

## Protocol and authority

- **Socket.** `$XDG_RUNTIME_DIR/badi/broker.sock`, mode `0600`, same-UID peers
  only ([ADR 0001](../docs/decisions/0001-same-uid-trust-boundary.md)). Frames
  are at most 64 KiB. Wire schemas live in [protocol/](../protocol/), control
  documents in [schemas/](schemas/).
- **Negotiation.** `hello` names protocol v2 (`min_v` and `max_v` 2) and the
  capabilities a connection may use: context, suggestion, commit kinds, text
  replacement, policy, control, health, settings, an optional
  `long_reading_window` and, for the session helper only, `desktop_signals`.
  Protocol v1 is retired
  ([ADR 0007](../docs/decisions/0007-retire-protocol-v1.md)): a v1 frame or a
  hello asking for v1 is answered with an `unsupported_version` error and
  closed before any session, grant or settings access exists. Every other
  refused hello also gets an `error` naming its reason, and leaves a
  content-free journal line; a peer of another user gets no reply.
- **Binding.** A suggestion is bound to its session, focus epoch, revision,
  fingerprint and expiry, and acceptance is one-shot.
- **Authority acknowledgement.** A policy-capable connection must answer
  `authority.changed` with `authority.ack` for the current epoch before the
  broker opens a session, admits context or serves a suggestion request on
  that connection. Until then those messages fail closed with a stale error
  and the connection stays open so the client can still acknowledge. Acking
  an older epoch never admits; acking a future epoch is rejected. Every epoch
  advance (pause, settings install or control-plane change) re-blocks the
  data plane until the new epoch is acknowledged.
- **Leases.** An adapter must report a granted commit within 1.5 s. Retained
  context normally expires after 3 s of silence. V2 native/editor clients
  negotiating `long_reading_window` get 15 s to read a suggestion; other
  native/editor clients retain 5 s, and other clients 600 ms.
  Native/editor context lasts at least the generation budget plus that reading
  window (up to 16.25 s for an explicit extended request). Type-through
  keeps the original expiry; there is no periodic renewal.
  Every field, revision, fingerprint, focus, policy and one-shot guard remains.
  Update the broker before clients advertising the new capability; an older
  broker rejects it. Fcitx, Obsidian, Neovim and the LibreOffice Writer
  extension opt in; Bash keeps its existing lease.
- **One version.** Each request is answered by at most one
  `suggestion.show`, with an immutable id and text, after the local model's
  [confidence gate](#suggestion-limits) judged the whole suggestion. Accepting
  or dismissing it, or its `suggestion.clear`, which names that id, retires
  it.
- **Type-through** ([ADR 0004](../docs/decisions/0004-type-through-carries-text-not-authority.md)).
  The last shown continuation is kept as text only.
  - When a later admitted request in the same app or site keeps `after` and
    the language, and its `before` adds a typed proper prefix of the
    suggestion, the broker answers with the remainder and no model call.
  - An automatic request for exactly the carry's context, which Fcitx sends
    when the typed key also changes the observed field's epoch, gets the
    same text again; an explicit request for it asks the model.
  - The remainder is a new suggestion that keeps the original deadline.
  - Escape, a whole acceptance, pause or an authority change ends the carry; a
    word acceptance keeps it. Editors send `dismiss` for Escape. Closing the
    session, which Fcitx does on every edit, does not end the carry. A carried
    remainder is not another shown suggestion and does not record a second
    outcome on the original aggregate.
- **Settings** (`badi.settings.v2`, the only settings schema on the wire)
  are replaced by compare-and-swap on their revision; while a replacement's
  outcome is unknown, suggestions pause. A `badi.settings.v1` file on disk
  still migrates to v2 on load.
  `badi pause` persists in settings; `badictl pause` is runtime only.
- **List modes.** `all_web_origins` (`badi sites blocklist`, or `badi site all
  on`) and `all_linux_apps` (`badi apps blocklist`) let a site or app without
  its own rule read context and suggest, but never learn or retain. Exact rules
  win, field denial and pause still apply, and private windows are not
  distinguished. An app or site allowed this way gets policy reason
  `matched_default`, so the Fcitx addon opens only observed fields for it. While the memory store
  is unavailable, only strict authority reductions are accepted.
- **Connections.** Policy and desktop-signal connections stay open while
  idle; control and health clients close after 300 s without a request.

## Desktop session signals

([ADR 0005](../docs/decisions/0005-desktop-signals-withhold-authority.md)) The [session helper](../adapters/session/README.md) reports
`desktop.state {session: active|inactive|sleeping, on_battery, power_saver,
screen_shared}` on a `desktop_signals` connection and receives
`desktop.status {observed, session, low_power, automatic, screen_share}`, the
combined view of every connected helper. Each report belongs to its
connection and disappears when the connection closes, except a live screen
share (below).

- **Hold.** While any helper reports an inactive or sleeping session, or a
  shared screen while settings `screen_share` is `hold` (the default), the
  broker is runtime-paused like `badictl pause`: entering the hold advances
  the authority epoch, revokes every session's context, cancels pending
  inference, withdraws visible suggestions (`suggestion.clear` reason
  `paused`) and pending commits, ends the type-through carry and fences
  queued outcome writes before it replies. Leaving it advances the epoch
  again, so adapters reacquire their fields; nothing from before the hold
  can be accepted. `health.status` reports `paused: true` during a hold. A
  report never grants anything.
- **Screen share.** `screen_share` is `none` unless a helper reports a live
  share; then `held`, or `suggesting` after `badi screen-share suggest`
  (settings `screen_share: suggest`). A helper that disconnects while its
  last report said shared leaves the share held for `screen_share_grace`
  (`SCREEN_SHARE_GRACE`, 10 s) or until any helper reports again, so a
  helper crash or restart never releases it early; `observed` is then false
  while `screen_share` stays `held`. A share is not a session hold:
  `session` stays `active`. Changing the setting is a settings write, which
  already advances the epoch, so a share starts holding or stops at once.
  The debug trace records each `desktop.state` and `desktop.status` whole;
  neither carries text.
- **Low power.** Battery power or the power-saver profile from any helper
  makes `low_power` true. That changes no authority and advances no epoch; it
  selects automatic scheduling from `battery_mode` (below).
- `health.status` (and `badictl status`) includes `desktop`.

## Editor focus

An editor with its own Badi integration that runs inside a terminal (Neovim,
capability `editor_focus`, which only the `neovim` adapter kind may negotiate)
sends `editor.focus {focused}` when it gains or loses its terminal's focus and
receives `editor.status {native_withheld}`. While any connection holds focus,
every `desktop_application` target resolves to `editor_focused`: no such
session opens, open ones lose their context, and the input method's
typed-line path in that terminal stays quiet, so Tab and Ctrl+Shift+Space
reach the editor ([ADR 0009](../docs/decisions/0009-terminals-use-the-typed-line.md)).
Starting or ending the hold advances the authority epoch. The report belongs
to its connection and ends with it; it withholds only and never grants.

## Snooze

`badi snooze [MINUTES]` (the session helper's `snooze` shortcut, 10 minutes
by default, at most 1440) sends the global control `snooze {minutes}`. The
broker snoozes the settings identity of the one focused field it follows: the
exact app id, or the site origin, which every browser shares as it shares
site rules. It refuses with `no_context` when no session has focused context,
`ambiguous_session` when focused fields belong to different apps or sites,
and `paused` while every grant is already withheld; `badictl` prints these as
`error_code=rejected:REASON`.

- **Withhold only** ([ADR 0005](../docs/decisions/0005-desktop-signals-withhold-authority.md)).
  Starting a snooze advances the authority epoch and revokes the context,
  pending inference, visible suggestion and pending commit of every session
  on that identity, and ends the type-through carry. Policy for it resolves
  to `snoozed` (not `paused`): no session opens, open ones lose their data
  access, and every other app and site keeps its policy.
- **Runtime only.** Snoozes live in the broker's memory, never in settings,
  and end with a broker restart. When one runs out, a timer advances the
  epoch again so adapters resolve their policy afresh. `snooze_end` (`badi
  snooze end`, `badi resume` and a resuming `badi pause toggle`) ends every
  snooze at once.
- `health.status`, `badictl status` and `badictl overview` (under
  `broker`) list `snoozes`: each `subject` identity and its `remaining_ms`.

## Style learning (opt-in)

Off by default (`style_learning` omitted). With `badi style on` and a site
whose rule allows learning with bounded retention (`badi style site ORIGIN
on [DAYS]`), each accepted suggestion that the adapter reports `applied` or
`dispatched_unverified` keeps the most recent complete sentence the user
wrote before it ([src/style.rs](src/style.rs)): at most 240 characters,
earlier sentences of the same line added only while it is shorter than 24,
and nothing with a digit, an at sign or a web address. The suggestion itself
is never kept.

- **Sealed.** `style.json` in the private data directory holds each record's
  subject and day in the clear and its text sealed with XChaCha20-Poly1305.
  The associated data binds the text to its key id, subject and day. The
  32-byte key lives in the desktop keyring (Secret Service); the session
  helper or `badi style on` delivers it (`style.key`, capability
  `style_key`), and the broker holds it in memory only. Without a key,
  samples are dropped, never kept in the clear.
- **Bounded.** 16 samples per site and 64 in all, oldest first out;
  duplicates are skipped. A complete export fits one frame.
- **Purged without the key.** A settings change that revokes a site's
  learning or retention purges its samples before the settings commit, as
  for the personalization aggregates; expiry runs at startup and with the
  periodic sweep. Turning `style_learning` off stops collection and keeps
  existing samples until they expire or are forgotten.
- **Explicit reads only.** `style.query` (content-free counts), `style.export`
  (decrypted samples) and `style.forget` (delete the file and the in-memory
  key) need the CLI settings capability; the session helper cannot read
  samples. A store sealed under another key reports `key: "mismatch"` and
  stays unreadable until `forget`.
- **Not used by prompts.** No production prompt reads these samples. They
  exist so the Lab can measure, on the user's own text, whether style examples
  help without the `style_fact_conflict` harm before any prompt uses them.

This protects stored text at rest; a process running as the same user can
still ask the keyring or the broker ([ADR 0001](../docs/decisions/0001-same-uid-trust-boundary.md)).

## Debug trace

While `badi debug on` runs, `debug_trace.rs` appends each wire frame in and
out of the broker (allowlisted fields; settings, style, memory and
statistics frames by type only; health and statistics polls not at all), each `model start` and `model done`
with latency and outcome, and each `carry` decision to the
[debug trace](../README.md#debug-trace). When the local runtime reports
them, `model done` also carries content-free `ttft_ms`, `prompt_tokens`
(llama.cpp `prompt_n`, newly evaluated), `cache_tokens` (`cache_n`, reused)
and `generated_tokens`; a cancelled, shutdown or timed-out call records
`cancelled_before_first_token`. Each located misspelling adds a `spelling`
line: its language, the number of dictionary candidates, whether the model
was asked and whether a correction resulted, and with `--text` the word, the
candidates and the model's answer. Malformed optional timing statistics stay
unknown rather than invalidating the completion, and no new text is recorded
([ADR 0008](../docs/decisions/0008-debug-trace-may-hold-text-on-request.md)).
Unit tests never write to a live trace.

## badictl

`badictl --help` lists every command. Broker commands print JSON; a failure
exits 1 with `error_code=CODE` on stderr.

```sh
badictl status            # health, metrics and no-suggestion classes
badictl overview          # broker, settings, privacy and model readiness
badictl settings show
badictl settings replace --if-revision N --json -   # read the document from stdin
badictl memory clear      # text-free origin and day aggregates
badictl style status|forget|export   # style learning; style key - reads the key from stdin
badictl stats status|forget   # usage counts per day and per app or site
badictl pause [on|off|toggle]
badictl snooze [MINUTES] | snooze end   # the focused app or site, runtime only
badictl request | accept-word | accept-all | dismiss   # the sole active session
badictl probe [--language TAG] [--after TEXT] [--replace] [--explicit] [--] TEXT|-
badictl hardware --json   # offline
badictl models writing --json
```

`status` counts each call that showed nothing once under `no_suggestion`
(`request_abstained`, `budget_prefill`, `budget_stream`, `model_abstained`,
`output_rejected`, `stale`, `timeout`, `provider_error`, `low_confidence`,
`repeated`) and
names the latest.
Its `type_through` counts requests answered from the carry (`carried`) and,
for admitted requests that reached the provider although a carry existed,
the gate that declined it (`scope`, `after_changed`, `language_changed`,
`deadline_tight`, `prefix_mismatch`, `shape_rejected`, latest in `last_miss`);
no counter moving while `provider_calls` rises means no carry existed.
Its `word_savings` counts space-delimited word parts after a valid
commit result, separately for `applied` and `dispatched_unverified`. Spelling
replacements, failed, stale and expired results add nothing. Partial words
count; later undo is unknown. All health counters reset at broker restart;
the per-day counts that persist are under [Usage statistics](#usage-statistics).

Settings replacements accept a bounded UTF-8 document on stdin with `--json -`;
the desktop helper and panel use this path so settings stay out of process
arguments. Schema validation and revision compare-and-swap are identical to
the legacy `--json DOCUMENT` form, which remains available for compatibility.
`probe` runs text through the live provider and the same display checks as a
suggestion, without a session, commit authority, counters or logs; `-` reads
the text from standard input, which keeps it out of shell history. `models`
prints pinned download advice and downloads nothing
([model fit](../docs/architecture/model-selection.md)).

## Writing model runtime

- **Release.** llama.cpp b11840 (the Ubuntu x64 CPU build), pinned by
  `src/semantic/pinned_runtime.rs`: the archive, `llama-server` and the
  unpacked bundle's manifest. It is past b11393, whose fix for a double free
  through a client-supplied `chat_parser` on `/completion`
  ([llama.cpp#29942](https://github.com/ggml-org/llama.cpp/pull/29942)) Badi
  needs only as hardening (the broker never sends that field), and outside
  b11555–b11634, which slowed quantized kernels.
- **Selection.** Startup reads the settings' `writing_model` (the user's
  `badi model use` choice) and loads that installed catalog model when it
  fits total memory after a host reserve; otherwise the installed model the
  recommendation prefers: the advised tier first, then the higher screen
  score, then the smaller file. Unknown memory fails closed; short available
  memory waits with backoff up to 30 s. A new choice takes effect at the
  next start, which `badi model use` triggers.
- **Verification.** Startup hashes the model and runtime once. Checkpoints
  before spawn, after spawn and after readiness confirm the same files by
  device, inode, size and times.
- **Warm-up.** Before binding the socket, one fixed two-token completion warms
  the model; a failure only leaves the first request cold.
- **Containment.** The runtime (llama.cpp `llama-server`) starts through a
  parent-death helper and cannot outlive the broker. If it exits, the broker
  exits 1; the [service unit](../packaging/systemd/badi-broker.service) restarts
  it with backoff, except after exit 78 (missing, unsupported or too-large
  model), which waits for `badi doctor`.
- **Transport.** HTTP/1.1 over a Unix socket in the `0700` runtime directory
  (`$XDG_RUNTIME_DIR/badi/llama-<pid>.sock`, mode `0600`), with a bearer token
  that does not cross TCP. The pid in the name keeps another process from
  replacing the running service's socket. The service unit has no network
  namespace address, and the runtime starts with its slots endpoint off
  (`LLAMA_ARG_ENDPOINT_SLOTS=0`), which would describe the prompt it is
  processing; metrics and properties stay off by default.
  Requests are never proxied, retried or pooled. Dropping a request closes
  its connection, which cancels it.
- **Launch.** One 512-token slot reuses the common prompt prefix (KV cache).
  A 128 MiB host cache keeps the previous sentence when the prompt anchor
  moves, and a matching run of at least 16 tokens is shifted instead of
  recomputed. Sliding-window attention layers keep a full-size cache
  (`LLAMA_ARG_SWA_FULL`), so a sliding-window model such as Gemma 3 can drop
  any suffix of its prompt and reuse the rest; other models ignore it. A
  second slot would split that 512-token context in half, so the server
  stays at one slot. Prefill batches are 16 tokens so cancellation lands
  quickly; up to four threads. The GPU is not used: typing waits mostly on
  generation, which the Iris Xe ran slower than the CPU.
- **Memory.** CPU weights are copied instead of mapped, allowing repacking to
  release the original allocation; the output is the same either way.

### Runtime: tested with

One machine (Iris Xe graphics); these are its numbers, not a universal
latency. Rows after the first ran on the earlier pin, b10726, unless they
name another build.

| Change or setting | Setup | Result |
| --- | --- | --- |
| b11840 against b10726 | Paired Lab runs, seed 42, 15 W, `output/writing/2026-10-24-runtime-b11840/` | European and Portuguese production-boundary suites: same exact, first-word and keystroke scores and p50. Revision suite: 53 of 75 errors fixed (50 before), 19 of 20 correct inputs left alone; changed cases are near-tie token choices |
| Full sliding-window cache, Lab | Gemma 3 1B, disposable English and German, four alternating runs, 720 typing requests per arm ([bench](../evaluation/llm-bench/README.md)) | New tokens per request 2.5 against 7.9; first token p50 68 against 158 ms; after a rejected suggestion p90 249 against 4,095 ms; decode speed unchanged; peak RSS 1.02 against 1.08–1.10 GB; cached text matched an uncached run 100 of 120 (the rest near-ties) |
| Full sliding-window cache, real broker | 504 `badictl probe` steps per arm, installed build against a same-build control | Appended words shown at p50 245 against 327 ms, 3 against 31 deadline misses; sentence starts p50 441 against 397 ms (36 per arm, unexplained); 423 against 435 probes showed a suggestion |
| Full sliding-window cache, live | Chromium 154, `5e141ed` against `3184097`, the same 1,109 `wtype` keys | New tokens per call 1.5 against 6.4; preview p50 241 against 288 ms (p90 345 against 396) after the key; 116 against 115 proposals |
| Threads | `llama-bench`, 48-token prompts in 16-token batches, 32 generated tokens, battery, 28 W | Gemma 3 1B 37.6, 41.4 and 33.9 tok/s at four, six and twenty threads; Gemma 4 E2B 21.3, 22.3 and 19.2. Five prefixes earlier: six threads wrote the same greedy text, no faster |
| Iris Xe (Vulkan, b11719, all layers) | Same bench | Gemma 3 1B prompt 172 against 106 tok/s, generation 28.8 against 37.7; Gemma 4 E2B 77 against 61 and 17.1 against 21.3. Not adopted |
| Bartowski Q4_0 of Qwen3-1.7B (`c470091d31c4ada174ee5c2547daa020e930593cbca5ca8ca385ce8ff59a2fdf`) | Six paired 8-token decodes | Faster in four, slower in one; changed greedy text on four prefixes ("Thank" → " youngster"). Not adopted; production stays on Q4_K_M |
| N-gram speculation; Qwen3-0.6B Q4_K_M draft (`9acfc1e001311f34b4252001b626f2e466d592a42065f66571bff3790d4e1b14`, two tokens ahead) | Five disposable prefixes | N-gram no faster; the draft took 0.5–1.8 s to the first word against 0.15–0.47 s. No draft model is loaded |
| Copied weights | Frozen 260-case UD r2.16 confirmation, both loading modes; installed broker plus runtime after twenty warm Chromium trials | Identical raw and displayed output, no errors or deadline misses; 1.43 GB RSS (1.33 GiB) against 2.10 GB mapped (RSS, not cgroup memory) |
| First grey word | Build `5faafcf`, five prefixes four times each | Omawrite 0.5.0: 20/20 at 209–250 ms, middle 220–223 ms. Chromium textarea: 20/20 at 227–371 ms, middle 246–249 ms. A known field starts the model on the key, so the 120 ms typing pause runs inside the wait ([Fcitx runbook](../adapters/fcitx5/README.md)) |

Field language is selected only after context admission, from the last 512
Unicode scalars before the caret. `whatlang` 0.18 considers its full language
inventory; Badi uses only confident (≥0.9), at least 32-scalar English,
German, Persian, French, Spanish or Portuguese detections. Otherwise the
adapter hint stays. An explicit per-app language always wins. The text,
fingerprint, caret and edit authority are unchanged, and no sample is
retained.

## Selection revision

An explicit grammar check or refinement of one selection
([ADR 0012](../docs/decisions/0012-explicit-revision-of-one-selection.md)),
in `src/revision.rs` (prompts and gate), `src/revision/` (the runtime and its
fixture) and `src/engine/revision.rs` (authority).

- **Admission.** `revision.request` needs the negotiated `selection_revision`
  capability (the Fcitx addon) and is a context snapshot of its own: it
  revokes the session's context, suggestion and earlier revision first. Policy
  is that of an explicit request, plus no terminal field; a refusal (pause,
  snooze, hold, setting, sensitive or composing field) is answered with
  `revision.clear`, never an error. The language is the app's override, else
  detected from the selection and the text before it, else the request's,
  else the first writing language; outside en, de, fr, es and pt the answer
  is `unsupported_language`.
- **Model.** `LocalReviser` runs the catalog's Qwen3 1.7B (Apache-2.0) in a
  second pinned llama.cpp process with its own socket
  (`llama-<pid>-revision.sock`) and token, launch contract
  `badi-revision-qwen3-v1`: a 1,024-token context, 64-token batches, the
  model copied into memory. It starts on the first request (hashing the model
  once per broker run, then confirming the same files) from one thread that
  lives as long as the broker, since a contained runtime dies with the
  thread that started it, and stops after ten idle minutes. One revision
  runs at a time. Its exit, a missing model, short available memory or too
  little room under the service's own memory limit (which the writing
  runtime shares; its file cache does not count) answers
  `revision_unavailable` and never stops suggestions. The prompt is
  `ChatML` with an empty reasoning block, a system line that names the field's
  language, and one worked example in that language: examples in another
  language made answers switch to it. A mid-sentence selection is sent after
  its sentence's start (up to 160 characters, from a word), so a clause keeps
  its grammar; the answer must begin with that start unchanged, which is then
  removed (`context` otherwise). Greedy decoding, at most two thirds of a
  token per selected character plus 48 (64–512), a 40-second bound for the
  call including a cold start. `--provider phrase` uses `FixtureReviser`,
  which fixes a few known mistakes, for integration lanes.
- **Gate.** `accept` keeps an answer only when it is one line with no control
  or format character, keeps the selection's surrounding spaces, keeps a
  mid-sentence lowercase start, adds no final period when words or marks
  follow on the line, is in
  the same language (all Latin letters and a confident detection that agrees),
  keeps every number, address and handle and every capitalized name after the
  first word (not in German, which capitalizes nouns), stays within the mode's
  length bounds (grammar 0.5–1.6, refine 0.4–1.8, ±12 characters), and, for a
  grammar check of more than three words, changes at most half of the words
  ignoring case and punctuation. An unchanged answer is `no_change`; any
  other failure `invalid_output`.
- **Show and apply.** `revision.show` lasts 60 s within a context lease of
  the call bound plus that window. `control.request` `accept_all` with the
  revision id grants one `commit.prepare` with `replace_selection` and the
  revised text; `dismiss` retires it. A revision's commit saves no words, adds
  no statistics and no style sample. Any context change, focus change, pause,
  hold, authority change or session close retires it with the matching
  reason.
- **Trace.** `revision start` and `revision done` record the mode, language,
  length, time, whether the runtime started cold and the outcome (`shown` or
  the gate's class); only a `--text` run adds the selection and its revision.

## Vocabulary completion

[src/vocabulary.rs](src/vocabulary.rs) finishes the word at the caret with a
word already in front of the writer. A request's `vocabulary` carries the
window's words and the writer's earlier words in the app; the draft before
the caret is the third source. When the caret ends a partial word of at least
five letters after a separator, and exactly one longer whole word in those
sources starts with it (case-sensitive, never normalized), its rest is
offered before spelling or the model runs. Several fitting words abstain,
unless one begins all the others (`almond`, `almonds`). An English function
word (`about`, `their`) or a finished English word plus an ending (`crash`,
`crashes`) abstains; a capital mid-sentence still marks a name. Words joined
to others by a hyphen, underscore, apostrophe, digit or dot are no evidence,
and a request carries at most 256 window and 1,024 earlier words. Each such
request adds a `vocabulary` trace line: word counts, outcome and sources, and
the word only in a `--text` run.

A request carries a vocabulary only while context is on for its subject
(`context_words`, [ADR 0010](../docs/decisions/0010-context-finishes-words.md)).
Without it, the draft is not used either, and nothing changes. Two
independent sets measured a large gain and more wrong words in traps
([Lab runbook](../evaluation/writing/README.md#vocabulary-completion)); the user
accepted that cost.

## Context: window words and learned words (opt-in)

Settings v2 holds `context_words` and `context_words_off`:

- `context_words` is absent until the user chooses, true when context is on
  and false when it is off.
- `context_words_off` lists the apps and sites, in canonical order, where
  context stays off.

`SettingsV2::context_words_for` combines them and any pause. Where context
is on and the subject may suggest, the broker does four things:

- **Says so.** `policy.status` adds `context_words: true` (never with
  suggestions denied). An observed field may then send up to 2,048 scalars of
  `window_text` with `context.changed`. Text from an unobserved, sensitive or
  denied field is rejected.
- **Uses it.** Each admitted request's provider call carries the window's
  words (at most 256) and the subject's learned words (at most 1,024). The
  generation task reads the learned words after the state lock is released,
  from a decrypted copy in memory that the broker replaces after every change
  to the settings, the key or the file, so a suggestion never waits for a
  write.
- **Learns.** The finished words of a `normal`-purpose field's text are
  counted toward that subject's learned words: letters only, six to 24 long,
  without the word still being typed, and without the first word when the
  draft fills the 512-scalar cap, where it may be clipped. Each word counts once per field
  visit, keyed by target; an observed field reopens its session as it changes,
  but its target id stays for the visit. A terminal field never learns. The
  words go through the outcome worker, like style samples.
- **Keeps them sealed.** `learned-words.json` in the private data directory
  ([src/vocabulary/learned.rs](src/vocabulary/learned.rs)) holds one record
  per subject. The subject and its last day are in the clear; the word list
  is sealed with the style-learning key. A word is offered after two field
  visits and dropped after 90 days unused. A subject holds at most 512 words
  and the file at most 64 subjects; the rarest, then the oldest, make room.
  Turning context off for a subject purges its record without the key, at
  the settings change and at startup. Records sealed under another key give
  way to the new key's.

`context.query` reports content-free counts and `context.forget` deletes
every learned word but keeps the key; both need the CLI settings capability
(`badictl context status|forget`, or `badi context`).

## Usage statistics

[src/stats.rs](src/stats.rs) keeps `stats.json` (`badi.stats.v1`) in the
private data directory, mode 0600 and written atomically: per local calendar
day and per exact app id or site origin, the suggestions shown, the
acceptances the adapter reported `applied` or `dispatched_unverified` and the
word parts they inserted (as `word_savings` counts them). No text,
fingerprint, field or window detail is kept.

- **What counts.** A show counts where `suggestions_shown` does, so a
  type-through remainder is the same suggestion. Every landed
  acceptance counts, so a suggestion taken word by word counts once per word;
  a failed, stale or expired one counts nothing.
- **What never counts.** The outcome worker keeps a count only under the
  settings revision it was admitted under, and only while those settings let
  that app or site suggest and do not pause Badi; a runtime pause or desktop
  hold admits nothing to count. Turning an app off keeps its earlier counts
  until they expire or are forgotten.
- **Bounded.** 90 days, today included, plus tomorrow, so moving west across
  a time zone keeps the day just counted; 64 apps and sites a day, further
  ones adding to that day's total. The day is the system time zone's
  ([jiff](https://github.com/BurntSushi/jiff); UTC when it is unknown).
- **Written.** Counts collect in memory and reach the disk with the worker's
  one-minute sweep, before a pause is acknowledged and at shutdown, so a
  restart keeps them and a crash loses at most a minute. Expired days go at
  startup and with each sweep. An unreadable file is preserved, counts
  nothing and reports `available: false` until `stats.forget` replaces it.
- **Read.** `stats.query` returns each kept day's totals, exact totals for
  today, the last 7 days and every kept day, and the apps and sites among the
  16 that lead any of those windows; `stats.forget` deletes everything. Both
  need the CLI settings capability (`badictl stats status|forget`, or `badi
  stats`).

## Suggestion limits

The exact rules live in [src/writing.rs](src/writing.rs).

- **Budgets.** An automatic request gives the model 550 ms, an explicit one
  (Tab, Ctrl-X Tab or a `request` control) 1,200 ms.
- **Context.** At most 512 characters before the caret and 128 after. The
  prompt is the current sentence from a 160-Unicode-scalar window; once
  the text is longer, its start advances only in 64-scalar
  steps at a word boundary (so the window is under 224 scalars before
  sentence narrowing), then extends back a sentence at a time to four words.
  Consecutive requests therefore share a prompt prefix while a long sentence
  grows, instead of changing its first bytes on every keystroke and losing
  llama.cpp's prefix cache. Each step re-reads the whole window once, and the
  runtime keeps what a cancelled request read. The next window is not read
  ahead: with one runtime slot that would evict the window in use. The
  prompt is the current sentence, not the whole window, which read no
  better; long Tab-only chains drift either way (a run of Tabs turned a
  sentence into a maths exercise), so the model, not the window, is the
  limit.
- **Shape.** At most four words (two with `short`, below) and 64
  characters. `en`, `de`, `fa`, `fr`,
  `es` and `pt` are routed; every language but Persian must be Latin
  script, Persian Arabic script, with
  ZWNJ only between the letters in
  [the joiner fixtures](../protocol/orthographic-joiner-fixtures.json).
- **Length.** `suggestion_length` in settings (`badi length short|normal`
  or the panel; omitted when `normal`, the default) shows one or two words
  or up to four, with the same token budget; a short stream ends as
  soon as its second word is complete. Settings changes retire live
  sessions, so a type-through carry never outlives a change of length.
  Corrections, vocabulary completions and the phrase fixture ignore it.
  Short cut harmful additions; up to eight words doubled them and was
  removed ([tested with](#suggestion-limits-tested-with)).
- **Partial words.** An English suffix that continues the typed word must form
  a word in the pinned [SCOWL lexicon](data/writing-lexicon/README.md); German
  and Persian suggest whole words only. English healing echoes the last
  unfinished word together with the spaces around it, so the prompt does not
  end on a bare space. Prompts in a healed language (below) also echo
  trailing spaces and require them back. Every language gets the same token
  count, 8, or 12 after a healed English word: a larger Persian budget missed
  550 ms.
- **Typed lexicon words.** An English word at the caret that is itself a
  lexicon word but begins a longer one (`rep`, `fin`, `the`) is healed too,
  because unhealed the model took it as finished (`can sh` became `uck off
  the background`). The request forbids the lone space token, which let the
  model meet the echo without writing the word (` ` then `finger` for
  ` fin`). The answer stands only when its first token is the whole echo or
  extends it (` report`, ` the`); a shorter one spelled out something else
  from pieces (`and` → `andash`), so the stream is dropped at that token and
  the unhealed prompt is sent within the same budget
  (`CompletionPlan::finished_word`). The confidence gate skips the token
  that writes the word: its probability is how likely the word was before
  any letter was typed.
- **Abstention.** A continuation abstains when it adds a number absent from the
  typed text, and every request when the caret splits a word.
- **Token confidence gate.** Each continuation request asks the runtime for
  `n_probs: 1` (pre-sampling), and a continuation is shown only when every
  token of its shown text has a log-probability of at least −2.75
  (`writing::CONFIDENCE_FLOOR`, user decision). Tokens wholly
  inside the healed echo and a withheld stop token do not count; a token
  that straddles the echo does. Missing or misaligned probabilities fail
  closed. A withheld suggestion is `low_confidence`, and `model done` in the
  trace carries `min_logprob`. The whole suggestion is judged before
  display. Spelling choices are not gated. Cutting before the first
  low-confidence token instead removed almost no harm, because invented
  facts are fluent ([tested with](#suggestion-limits-tested-with)).
- **Languages and healing.** Settings `writing_languages` (default `en` and
  `de`; `fr`, `es`, `pt` and, from the terminal only, `fa` opt-in) decide
  where Badi suggests, and `languages_chosen` records that the user picked
  them (`badi languages` or the panel's first-run question); a request in
  any other language abstains before the model, and detection names French,
  Spanish and Portuguese so that they are not written as English. Each catalog model's
  `healed_languages` lists the languages whose trailing space it heals, by
  its own blind review; the engine hands settings `trailing_space_healing`
  choices to the provider (`set_language_preferences`), which applies them
  per language over that default; a model outside the catalog heals English
  and Persian (`TRAILING_SPACE_HEALING`). An unhealed request that ends in a
  space abstains before any model call: after a bare space a model must
  start a word without its usual leading-space token, and no reviewed model
  continued one usefully. Gemma 3 1B heals English, German (by user
  decision), Persian, French, Spanish and Portuguese.
- **Generation grammar.** Every continuation request carries a grammar that
  excludes what could never be shown: `<`, `>`, backticks and line breaks,
  and, when the typed text has no digit, digits in any script. Without it
  Gemma 3 1B wrote markup such as `<strong>Bezug</strong>` or an invented
  `town of 1000`, and the whole suggestion was rejected; with it the model
  writes its next best word. A healed echo keeps its literal ahead of the
  same class.
- **Mid-line.** Off by default (`badi mid-line off`; omit `mid_line` in
  settings). A word after the caret then abstains before any model call.
  Punctuation after the caret can still complete the word being typed.
  `badi mid-line on` asks the model. The prompt is the text before the caret
  (a framed prompt or the line after the caret missed 550 ms; [tested
  with](#suggestion-limits-tested-with)), and `mid_line_insertion()` keeps
  only what fits:
  the words before the point where the continuation reproduces the line's
  first word or mark, a phrase the model closes with a clause mark before the
  line's own mark, or else the rest of the word being typed. Anything else is
  `output_rejected`. If the caret already follows whitespace and the next
  character is a letter or digit, the insertion may end in one ASCII space so
  it does not join that word. End of text, punctuation, and a repeated space
  stay rejected. Word acceptance on that last word includes the space, and
  typing its
  first letters carries the space with the remainder. Letters typed at a
  word start in front of a word (`the |report`, then `fin`) are a new word
  while the text after the caret stays the same and only letters are typed
  (the engine's `WordStart`, kept per app or site like a carry because an
  observed field reopens its session on each edit; it grants nothing): the
  provider is asked as if a space followed
  them, and the suggestion ends in that space (`al ` → `the final report`).
  A caret moved into a word, or anything but letters typed there, still
  abstains. A next word of three letters or fewer rejoins only together
  with the word after it: ` the right place` does not rejoin ` the
  figures`, which on the 41 mid-sentence development cases removed the one
  insertion `main` showed (`I am not sure |the figures` → `if this is `).
  A finished word that repeats the next one (`ha` + `d` before `had`) does
  not fit either.
- **Language overrides.** Optional `language_overrides` settings map up to 64
  canonical app ids to `en`, `de` or `fa`. `badi language APP TAG` sets one;
  `reset` removes it. The broker applies the override only after context
  validation and policy admission, before generation. It never grants text
  access or changes editing authority. Settings changes retire live sessions.
  Chromium's `chromium-browser` alias also follows a `chromium` override;
  Brave and Zen retain their own app ids while sharing origin permissions.
  Without an override, confident field detection takes precedence over the
  adapter's existing language hint.
  An admitted `policy.status` includes the optional `language_override` so
  Fcitx manual Tab eligibility also uses this preference.
- **Battery mode.** `battery_mode` in settings (`badi battery
  quiet|normal|explicit`; omitted when `quiet`, the default) applies only
  while a session helper reports low power. `quiet` delays the model's start for an automatic
  request by 300 ms, cancelled by any newer request or edit, and extends that
  request's deadline and context lease by the same 300 ms so the writing
  budget is unchanged. `explicit` abstains automatic requests before the model
  (`request_abstained`). Explicit requests and type-through remainders are
  never delayed or refused, and the model stays resident. With no helper
  connected, every mode behaves like `normal`. The default and the 300 ms
  wait come from the paced trial in the
  [session runbook](../adapters/session/README.md#battery).
- **Escape quiet.** Off by default (`badi escape-quiet off`; omit
  `escape_quiet_seconds` in settings), so a dismissal lasts only until the
  text changes, as the adapters decline it. With `badi escape-quiet SECONDS`
  (1–3600) or the panel presets, a dismissal arms a broker-side quiet bound
  to its field visit: automatic generation abstains across edits
  with no provider call, counted as `request_abstained`, until the time runs
  out, the focus epoch ends, or an explicit request asks again, which ends
  the quiet at once. Fcitx observed fields keep a random broker target id for
  that exact field visit across observer rebinds. The broker retains only the
  latest such dismissal (target and deadline); a newly authorized session may
  inherit its denial, never its context or editing grants. Unknown native
  fields and other adapters keep session-and-focus-bound quieting. Every
  context and request already carries `explicit`, so no wire change is needed. During the
  quiet, adapters still inspect and publish context under their existing
  grants; the model is never called.
- **Turned-down words.** Escape on a continuation, or typing other letters
  at a shown one, turns its next word down at that spot: the last three words
  before it, with the suggestion's leading space, in that app or site (the
  carry's scope). One Escape, or other letters typed at two shown
  suggestions, withholds that next word there from later automatic requests
  (`repeated`). Words typed in full were taken, so only the word being typed
  over is turned down. An explicit request still shows it, and taking it
  there, a word or all, forgets the turn-down. Neovim's dismissal on
  leaving Insert mode turns nothing down. Only keyed
  hashes are kept, in memory, for 256 spots; a restart forgets them
  (`engine/rejections.rs`).
- **Alternatives.** `control.request` `alternatives` on a shown
  continuation (Fcitx's Ctrl+Shift+Space) is an explicit request for the
  same context. The runtime returns its eight most likely first tokens
  (one generated token, `n_probs`, before sampling); the best that starts
  with the healed echo, adds letters, passes the generation class and is no
  word shown yet is continued greedily within the length setting, with up
  to three tries. Each context (fingerprint) gets up to three alternatives;
  after that, or once the model has none, presses cycle through those shown
  with no model call, and the one shown again is not counted twice.
  Corrections and mid-line continuations have none, and the editor
  integrations do not ask yet (`engine/alternatives.rs`).
- **Quiet apps.** Each app or site's last seven days of usage counts set
  its quiet level, read again at most every ten minutes: with at least 50
  suggestions shown and fewer than 8% accepted, an automatic suggestion
  needs every token at −2.25 or better; under 4%, −1.75. A level loosens only
  at 15% and 8%, and with fewer than 50 shown it stays as it was. It only
  tightens `CONFIDENCE_FLOOR`; explicit requests, corrections and
  completions the model did not score keep their own gates, and `badi stats
  forget` resets it. A withheld suggestion counts as `low_confidence`, and
  the trace's `suggestion withheld` names `quiet_app` and its floor
  (`engine/quietness.rs`).
- **Spelling** needs negotiated exact replacement of the word before the caret
  at the end of the field, in a language of `spelling_languages` (default
  `["en"]`). `spelling.rs` picks the language: a confident detection decides,
  so text in a language that is off is never corrected; otherwise the field's
  hint. English uses the embedded lexicon (`writing.rs`); the other languages
  their Hunspell dictionary from `$XDG_DATA_HOME/hunspell`, `/usr/share/hunspell`
  or `/usr/share/myspell/dicts`, loaded off the runtime when enabled. A word
  unknown to it gets at most three candidates of letters, with the same first
  letter and at most two edits; one is proposed without inference, several go
  to the model, which must answer with one of them. The choice prompt ends
  inside an opened quote (`The intended word is "`) and its raw answer is read
  up to the closing quote; echoing the typo is how the model abstains. After
  a bare `Answer:`, base models such as Gemma 3 1B start a new line and never
  answer. A word without its typed
  space is checked only on an explicit request, and a capitalized word
  mid-sentence only in German. Replacements are 2-32 characters with case
  (Unicode Lowercase or Uppercase) plus the optional space, in every
  validator, each tested against
  [the shared fixture](../protocol/spelling-replacement-fixtures.json). The
  [LibreOffice Writer extension](../adapters/libreoffice/README.md) (adapter and
  target kind `libreoffice`, app id `libreoffice-writer`, settings adapter
  `libreoffice`) gets corrections only, automatic or explicit, under its own
  `RequestTrigger::Spelling`: no keystroke waits for it, so the model has
  2.5 s to choose between candidates (2.75 s generation limit). It reports
  `applied` only after reading the edit back. Settings order Linux app rules by adapter name, as
  the desktop CLI and panel sort them.

### Suggestion limits: tested with

Lab rows are development or held-out comparisons on this machine, not
universal scores.

| Rule | Setup | Result |
| --- | --- | --- |
| Window steps | Installed build, real model, a 380-character run-on sentence typed word by word into Ghostty | Performance profile, 250 ms per word: 40 of 47 requests showed a suggestion; each step (inputs 228, 292 and 359) cost one request cancelled before its first token while it re-read about 30 tokens, and the next was warm again (`cache_n` 27–28); none missed its budget. Power-saver, 450 ms per word: `quiet` scheduling set the rate, 15 of 62 showed one |
| Per-keystroke sliding start (replaced by steps) | Once one sentence passed 160 scalars | Prefix cache reuse 0 of about 30 prompt tokens; first token 380–510 ms |
| Whole window as the prompt (not kept) | 19 real-text Lab cases, cold; a live paragraph in Ghostty | 10 against 15 suggestions, no more matches; read no better live |
| `short` length | Paired Lab, 395 requests in six languages, two blind rounds, 15 W, `output/writing/2026-10-20-suggestion-length/` | Harmful additions 16/20 → 8/9 with more useful full additions; p50 201 against 259 ms. Up to eight words (`f9515e0`) doubled them to 32/39, rising in five languages |
| Persian token budget | Lab `production_boundary`, the 13 Persian development cases, Qwen3 1.7B | 21 tokens (2.6×): all 13 missed 550 ms and four returned no word. 8 tokens: two misses, a word for every case 21 had answered (one shorter: `نظر من` became `نظر`); neither matched a reference first word |
| Typed lexicon words | Paired Lab, 134 English stems from the development references (34 mid-sentence) and 29 prefixes ending on a finished word, seed 42, 15 W, `output/writing/2026-10-23-word-stems/` | Intended word finished 75 times against 0 (23 of the mid-sentence ones against 0); text running on after the stem 9 against 40; finished words kept their 23 suggestions; shown p90 392 against 485 ms. Forbidding the lone space token: 124 of 134 first tokens stood against 91 |
| Confidence floor, calibration | Gemma 3 1B's exact production payloads replayed for 301 reviewed development suggestions, three blind label rounds each; every replay reproduced its text | The lowest token log-probability separates useful from unhelpful or harmful at an AUC of 0.70–0.77 overall, strongly in German and not at all in English; the first word's confidence alone was much weaker |
| Confidence floor, held out | Confirmation outputs (224 shown), rule frozen before the test | Harmful additions 15 → 10, unhelpful 18 → 10, harm falling or holding in every language; lost 7 of 51 useful full additions (2 English, 3 Persian); 19% fewer suggestions |
| Confidence floor, live | Chromium 154, 15 W, 115 paced word pauses of the same disposable text | Preview p50 294 against 218 ms (p90 395 against 265; 4 against 0 over 550 ms) after the last key; 92 against 112 pauses showed one, 17 withheld as `low_confidence` |
| Unhealed prompt ending in a space | German boundary requests | Qwen3 1.7B 0 useful of 20; Gemma 3 1B 0 of 26 (markup, or with markup excluded, fragments such as `uralt.`) |
| Gemma 3 1B healing | Blind review per language against the unhealed prompt | German 15 useful against 7 of 67 development and boundary requests, 3 harmful against 2 (user decision); French and Spanish 12 and 14 useful of 20 boundary requests ending in a space, none harmful; Portuguese 11 of 20, none harmful |
| Fallback healing set | Sealed and development sets before the catalog's per-model review | German unlisted: the sealed set rose from 1 to 9 harmful. English kept by user decision at +1 harmful for +19 useful in 40 |
| Mid-line prompts | The 41 English mid-sentence development cases | The rejoin showed two suggestions, one matching the reference; a framed prompt missed 550 ms on every cold case; a probability check showed only the punctuation completion; naming the line after the caret (`following_text`) showed one and missed 550 ms on 34 of 41, uncached |
| Spelling choice prompt | 26 disposable development cases (German, Spanish, French), 28 W | The quoted prompt chose the intended word in 18 with Gemma 3 1B (2 wrong, 6 abstained), 15 with Llama 3.2 1B (4 wrong) and 22 with Qwen3 1.7B (3 wrong), 0.4–0.6 s per choice; a bare `Answer:` chose 1, 4 and 20 |

## Shutdown and tests

SIGINT and SIGTERM remove the socket (after checking its device and inode) and
exit 0; wait for the socket to disappear rather than deleting it. The service
uses `KillMode=mixed` and gives the runtime 2 s.

Run the Rust gate from [AGENTS.md](../AGENTS.md#verification). The tests cover
framing, policy, binding, type-through, badictl output against a phrase broker,
signal shutdown and the owned runtime (`tests/owned_runtime.rs` launches itself
as a fake `llama-server`). Probes of the installed model run serially on
synthetic prompts:

```sh
cargo test --release --locked --test writing_runtime -- --ignored --nocapture --test-threads=1
```
