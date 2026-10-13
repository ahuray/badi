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
  for the session helper only, `desktop_signals` (v2).
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
  At most 64 versions fit the protocol's 64-character limit.
- **Type-through** ([ADR 0004](../docs/decisions/0004-type-through-carries-text-not-authority.md)).
  The last shown continuation is kept as text only.
  - When a later admitted request in the same app or site keeps `after` and
    the language, and its `before` adds a typed proper prefix of the
    suggestion, the broker answers with the remainder and no model call.
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

- **Selection.** Startup picks the installed pinned model that fits total
  memory after a host reserve, preferring the advised tier. Unknown memory
  fails closed; short available memory waits with backoff up to 30 s.
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

## Suggestion limits

The exact rules live in [src/writing.rs](src/writing.rs).

- **Budgets.** An automatic request gives the model 550 ms, an explicit one
  (Tab, Ctrl-X Tab or a `request` control) 1,200 ms.
- **Context.** At most 512 characters before the caret and 128 after. The
  prompt is the current sentence within 160 characters, extended back a
  sentence at a time to four words, so text typed in order reuses the cache.
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
  typed text, and every request when the caret splits a word.
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
  stay rejected. Ctrl+→ on that last word includes the space, and typing its
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
- **Spelling** is English only and needs negotiated exact replacement of the
  word before the caret at the end of the field; a single dictionary candidate
  is proposed without inference. The
  [LibreOffice Writer extension](../adapters/libreoffice/README.md) (adapter and
  target kind `libreoffice`, app id `libreoffice-writer`, settings adapter
  `libreoffice`) is explicit-only, like Bash, and reports `applied` only after
  reading the edit back. Settings order Linux app rules by adapter name, as
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
