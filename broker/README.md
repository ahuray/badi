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
- **Negotiation.** `hello` negotiates protocol v1 or v2 and the capabilities a
  connection may use: context, suggestion, commit kinds, text replacement,
  policy, control, health, settings, an optional `long_reading_window` and,
  for the session helper only, `desktop_signals` (v2). Every shipped client
  negotiates v2 only; retiring v1, including what keeps it alive and the
  removal order, is decided in
  [ADR 0007](../docs/decisions/0007-retire-protocol-v1.md).
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
  negotiating `long_reading_window` get 15 s to read a suggestion; other v2
  native/editor clients retain 5 s, and v1/browser clients retain 600 ms.
  Native/editor context lasts at least the generation budget plus that reading
  window (up to 16.25 s for an explicit extended request). Growing previews
  and type-through keep the original expiry; there is no periodic renewal.
  Every field, revision, fingerprint, focus, policy and one-shot guard remains.
  Update the broker before clients advertising the new capability; an older
  broker rejects it. Fcitx, Obsidian, Neovim and the LibreOffice Writer
  extension opt in; Bash keeps its existing lease.
- **Growing previews.** Each extension has an immutable id and text. Earlier
  versions remain acceptable only within the same generation, exact field
  revision and original expiry, so a new word crossing Tab on the socket
  cannot swallow the acceptance. Accepting or dismissing any version retires
  them all; cancellation, focus changes and expiry revoke the whole group.
  At most 64 versions fit the protocol's 64-character limit. When the final
  result is rejected (by the display checks, or by the provider's own
  validation, which it reports as a no-suggestion outcome) or arrives late,
  the last shown partial stays if it ends at a word boundary
  (`ends_at_word_boundary`): it
  was already checked before display, so it counts as the shown suggestion,
  not as a no-suggestion miss, and only an edit, a focus change, Escape or
  its expiry withdraws it. Live in Ghostty on 2026-10-17, ten
  provider-rejected finals followed two or three shown partials and none
  produced a `no_suggestion` clear; the next broker clear was `superseded`
  46–280 ms later, and in the row inspected key-by-key the ghost was still
  marked visible when the next printable key arrived 129 ms after rejection.
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
- **Settings** (`badi.settings.v2`) are replaced by compare-and-swap on their
  revision; while a replacement's outcome is unknown, suggestions pause.
  `badi pause` persists in settings; `badictl pause` is runtime only.
- **List modes.** `all_web_origins` (`badi sites blocklist`, or `badi site all
  on`) and `all_linux_apps` (`badi apps blocklist`) let a site or app without
  its own rule read context and suggest, but never learn or retain. Exact rules
  win, field denial and pause still apply, and private windows are not
  distinguished. An app allowed this way gets policy reason `matched_default`,
  so the Fcitx addon opens only observed fields for it. While the memory store
  is unavailable, only strict authority reductions are accepted.
- **Connections.** Policy and desktop-signal connections stay open while
  idle; control and health clients close after 300 s without a request.

## Desktop session signals

([ADR 0005](../docs/decisions/0005-desktop-signals-withhold-authority.md)) The [session helper](../adapters/session/README.md) reports
`desktop.state {session: active|inactive|sleeping, on_battery, power_saver}`
on a `desktop_signals` connection and receives `desktop.status
{observed, session, low_power, automatic}`, the combined view of every
connected helper. Each report belongs to its connection and disappears when
the connection closes.

- **Hold.** While any helper reports an inactive or sleeping session, the
  broker is runtime-paused like `badictl pause`: entering the hold advances
  the authority epoch, revokes every session's context, cancels pending
  inference, withdraws visible suggestions and pending commits, ends the
  type-through carry and fences queued outcome writes before it replies.
  Leaving it advances the epoch again, so adapters reacquire their fields;
  nothing from before the hold can be accepted. `health.status` reports
  `paused: true` during a hold. A report never grants anything.
- **Low power.** Battery power or the power-saver profile from any helper
  makes `low_power` true. That changes no authority and advances no epoch; it
  selects automatic scheduling from `battery_mode` (below).
- v2 `health.status` (and `badictl status`) includes `desktop`; protocol v1
  health omits it.

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
out of the broker (allowlisted fields; settings, style and memory frames by
type only; health polls not at all), each `model start` and `model done`
with latency and outcome, and each `carry` decision to the
[debug trace](../README.md#debug-trace). When the local runtime reports
them, `model done` also carries content-free `ttft_ms`, `prompt_tokens`
(llama.cpp `prompt_n`, newly evaluated), `cache_tokens` (`cache_n`, reused)
and `generated_tokens`; a cancelled, shutdown or timed-out call records
`cancelled_before_first_token`. Malformed optional timing statistics stay
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
badictl pause [on|off|toggle]
badictl request | accept-word | accept-all | dismiss   # the sole active session
badictl probe [--language TAG] [--after TEXT] [--replace] [--explicit] [--] TEXT|-
badictl hardware --json   # offline
badictl models writing --json
```

`status` counts each call that showed nothing once under `no_suggestion`
(`request_abstained`, `budget_prefill`, `budget_stream`, `model_abstained`,
`output_rejected`, `stale`, `timeout`, `provider_error`) and names the latest.
Its `type_through` counts requests answered from the carry (`carried`) and,
for admitted requests that reached the provider although a carry existed,
the gate that declined it (`scope`, `after_changed`, `language_changed`,
`deadline_tight`, `prefix_mismatch`, `shape_rejected`, latest in `last_miss`);
no counter moving while `provider_calls` rises means no carry existed.
Its optional v2 `word_savings` counts space-delimited word parts after a valid
commit result, separately for `applied` and `dispatched_unverified`. Spelling
replacements, failed, stale and expired results add nothing. Partial words
count; later undo is unknown. All health counters reset at broker restart.
The frozen v1 health response omits the new counters.

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
  namespace address.
  Requests are never proxied, retried or pooled. Dropping a request closes
  its connection, which cancels it.
- **Launch.** One 512-token slot reuses the common prompt prefix (KV cache).
  A 128 MiB host cache keeps the previous sentence when the prompt anchor
  moves, and a matching run of at least 16 tokens is shifted instead of
  recomputed. A second slot would split that 512-token context in half, so
  the server stays at one slot. Prefill batches are 16 tokens so cancellation
  lands quickly; up to four threads. The GPU is not used: Vulkan on the Iris
  Xe was slower for these short requests.
- **Measured and not adopted** (2026-10-12, this machine, five disposable
  prefixes): six threads produced the same greedy text and were not faster.
  Bartowski's Q4_0 of Qwen3-1.7B
  (`c470091d31c4ada174ee5c2547daa020e930593cbca5ca8ca385ce8ff59a2fdf`, not a
  re-quant of the pinned file) was slightly faster in four of six paired
  8-token decodes, slower in one, and changed greedy text on four prefixes
  ("Thank" → " youngster"); production stays on Q4_K_M. N-gram speculation
  was no faster, and a Qwen3-0.6B Q4_K_M draft
  (`9acfc1e001311f34b4252001b626f2e466d592a42065f66571bff3790d4e1b14`, two
  tokens ahead) took 0.5–1.8 s to the first word against 0.15–0.47 s without
  it, so no draft model is loaded.
- **Speed sample.** Build `5faafcf` (2026-10-12; the same five prefixes four
  times each, predictions on): first grey word in Omawrite 0.5.0 20/20 at
  209–250 ms, middle 220–223 ms, 19th of 20 at 244 ms; in a Chromium
  textarea 20/20 at 227–371 ms, middle 246–249 ms, 19th at 314 ms. A known
  field starts the model on the key, so the 120 ms typing pause runs inside
  the generation wait ([Fcitx runbook](../adapters/fcitx5/README.md)). One
  machine and these prefixes, not a universal latency.
- **Memory.** CPU weights are copied instead of mapped, allowing repacking to
  release the original allocation. A frozen 260-case UD r2.16 confirmation
  comparison on 2026-10-13 produced identical raw and displayed output for
  both loading modes, with no errors or deadline misses. The installed
  broker plus runtime used 1.43 GB RSS (1.33 GiB) after twenty warm Chromium
  trials, versus 2.10 GB before. This is RSS, not cgroup memory or a claim
  about other model sizes and hardware. The model and prompt are unchanged.

Field language is selected only after context admission, from the last 512
Unicode scalars before the caret. `whatlang` 0.18 considers its full language
inventory; Badi uses only confident (≥0.9), at least 32-scalar English,
German or Persian detections. Otherwise the adapter hint stays. An explicit
per-app language always wins. The text, fingerprint, caret and edit authority
are unchanged, and no sample is retained. French and Spanish generation
still requires independent qualification.

## Context word bias

`SemanticClient::with_window_context(text)` lets a caller hand the client the
bounded text around a field. Its distinctive English words (letters only, at
least five scalars, not common function words, not among the last six words
typed, at most 24) raise the first token of each by a logit bias of 2.0
(`writing::CONTEXT_BIAS`). The tokens come from one `/tokenize` call with
pieces, capped at 60 ms and cached per client; a first token of one or two
letters is skipped, and any failure sends the request unbiased. The prompt
never grows, so prefill and the 550 ms budget are unchanged. Only the
Prediction Lab supplies window text today, so production requests carry no
bias; the measurements are in [future plans](<../future plans.md>), step 4.

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
  llama.cpp's prefix cache. Each step re-reads the whole window once. Measured
  2026-10-17 (installed build, real model, a 380-character run-on sentence
  typed word by word into Ghostty): in the performance profile at 250 ms per
  word, 40 of 47 requests showed a suggestion, and each step at inputs 228,
  292 and 359 cost one request cancelled before its first token while it
  re-read about 30 tokens; the next request was warm again (`cache_n` 27-28),
  because the runtime keeps what a cancelled request read. No request missed
  its budget. Reading the next window ahead of time is not done: with one
  runtime slot it would evict the window in use. In the power-saver profile
  (450 ms per word), `quiet` battery scheduling, not the step, set the rate:
  15 of 62 requests showed one, most cancelled by the next word after
  generation had started.
- **Shape.** At most four words and 64 characters. `en`, `de` and `fa` are
  routed; English and German must be Latin script, Persian Arabic script, with
  ZWNJ only between the letters in
  [the joiner fixtures](../protocol/orthographic-joiner-fixtures.json).
- **Partial words.** An English suffix that continues the typed word must form
  a word in the pinned [SCOWL lexicon](data/writing-lexicon/README.md); German
  and Persian suggest whole words only. English healing echoes the last
  unfinished word together with the spaces around it, so the prompt does not
  end on a bare space. English and Persian prompts also echo trailing spaces
  and require them back (`TRAILING_SPACE_HEALING`). Persian uses that same
  token count. A 2.6× Persian budget missed 550 ms on the development cases.
- **Abstention.** A continuation abstains when it adds a number absent from the
  typed text, and every request when the caret splits a word. A German
  request that ends in a space abstains before any model call, because German
  is not healed: after a bare space a model must start a word without its
  usual leading-space token, and neither Qwen3 1.7B (0 useful of 20) nor
  Gemma 3 1B (0 of 26, markup or fragments such as `uralt.`) continued one
  usefully. Healing German on Gemma is an
  [open decision](<../future plans.md>).
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
  `badi mid-line on` asks the model. A 2026-10-12 Lab comparison of the 41
  English development cases did not change this rejoin: today's rejoin showed
  two suggestions and one matched the reference; the framed prompt missed 550 ms
  on every cold case; the probability check showed only the punctuation
  completion. Naming the line after the caret in a lab prompt
  (`following_text`) showed one suggestion and missed 550 ms on 34 of 41,
  because that line changes every case and is not cached. The prompt is still
  the text before the caret, and `mid_line_insertion()` keeps only what fits:
  the words before the point where the continuation reproduces the line's
  first word or mark, a phrase the model closes with a clause mark before the
  line's own mark, or else the rest of the word being typed. Anything else is
  `output_rejected`. If the caret already follows whitespace and the next
  character is a letter or digit, the insertion may end in one ASCII space so
  it does not join that word. End of text, punctuation, and a repeated space
  stay rejected. Word acceptance on that last word includes the space, and
  typing its
  first letters carries the space with the remainder.
- **Language overrides.** Optional `language_overrides` settings map up to 64
  canonical app ids to `en`, `de` or `fa`. `badi language APP TAG` sets one;
  `reset` removes it. The broker applies the override only after context
  validation and policy admission, before generation. It never grants text
  access or changes editing authority. Settings changes retire live sessions.
  Chromium's `chromium-browser` alias also follows a `chromium` override;
  Brave and Zen retain their own app ids while sharing origin permissions.
  Without an override, confident field detection takes precedence over the
  adapter's existing language hint.
  An admitted v2 `policy.status` includes the optional `language_override`
  so Fcitx manual Tab eligibility also uses this preference. Protocol v1
  responses omit it; broker-side context handling still applies it.
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
- **Spelling** needs negotiated exact replacement of the word before the caret
  at the end of the field, in a language of `spelling_languages` (default
  `["en"]`). `spelling.rs` picks the language: a confident detection decides,
  so text in a language that is off is never corrected; otherwise the field's
  hint. English uses the embedded lexicon (`writing.rs`); the other languages
  their Hunspell dictionary from `$XDG_DATA_HOME/hunspell`, `/usr/share/hunspell`
  or `/usr/share/myspell/dicts`, loaded off the runtime when enabled. A word
  unknown to it gets at most three candidates of letters, with the same first
  letter and at most two edits; one is proposed without inference, several go
  to the model, which must answer with one of them. A word without its typed
  space is checked only on an explicit request, and a capitalized word
  mid-sentence only in German. Replacements are 2-32 cased letters plus the
  optional space, in every validator. The
  [LibreOffice Writer extension](../adapters/libreoffice/README.md) (adapter and
  target kind `libreoffice`, app id `libreoffice-writer`, settings adapter
  `libreoffice`) gets corrections only, automatic or explicit, under its own
  `RequestTrigger::Spelling`: no keystroke waits for it, so the model has
  2.5 s to choose between candidates (2.75 s generation limit). It reports
  `applied` only after reading the edit back. Settings order Linux app rules by adapter name, as
  the desktop CLI and panel sort them.

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
