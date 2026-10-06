# Badi broker

The local policy broker and its control CLI. Adapters acquire and edit the
document; the broker owns target policy, session state, suggestion and commit
authority, the local writing model and content-free metrics. It speaks only on
a Unix socket and never drives a keyboard, clipboard or accessibility API.

- `badi-broker` serves the socket. `--provider phrase` selects the
  deterministic test fixture instead of the writing model.
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
  policy, control, health and settings.
- **Binding.** A suggestion is bound to its session, focus epoch, revision,
  fingerprint and expiry, and acceptance is one-shot.
- **Leases.** An adapter must report a granted commit within 1.5 s; retained
  context is revoked after 3 s of silence; v2 native and editor suggestions
  stay readable for 5 s. Anything stale, foreign or unknown fails closed.
- **Type-through** ([ADR 0004](../docs/decisions/0004-type-through-carries-text-not-authority.md)).
  The last shown continuation is kept as text only.
  - When a later admitted request in the same app or site keeps `after` and
    the language, and its `before` adds a typed proper prefix of the
    suggestion, the broker answers with the remainder and no model call.
  - The remainder is a new suggestion that keeps the original deadline.
  - Escape, a whole acceptance, pause or an authority change ends the carry; a
    word acceptance keeps it.
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
- **Connections.** Policy connections stay open while idle; control and health
  clients close after 300 s without a request.

## badictl

`badictl --help` lists every command. Broker commands print JSON; a failure
exits 1 with `error_code=CODE` on stderr.

```sh
badictl status            # health, metrics and no-suggestion classes
badictl overview          # broker, settings, privacy and model readiness
badictl settings show
badictl settings replace --if-revision N --json DOCUMENT
badictl memory clear      # text-free origin and day aggregates
badictl pause [on|off|toggle]
badictl request | accept-word | accept-all | dismiss   # the sole active session
badictl probe [--language TAG] [--after TEXT] [--replace] [--explicit] [--] TEXT|-
badictl hardware --json   # offline
badictl models writing --json
```

`status` counts each call that showed nothing once under `no_suggestion`
(`request_abstained`, `budget_prefill`, `budget_stream`, `model_abstained`,
`output_rejected`, `stale`, `timeout`, `provider_error`) and names the latest.
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
- **Transport.** HTTP/1.1 to loopback only, with a bearer token, never proxied,
  retried or pooled. Dropping a request closes its connection, which cancels it.
- **Launch.** One 512-token slot reuses the common prompt prefix (KV cache);
  prefill batches are 16 tokens so cancellation lands quickly; up to four
  threads. The GPU is not used: Vulkan on the Iris Xe was slower for these
  short requests.

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
  and Persian suggest whole words only. English and Persian prompts drop
  trailing spaces and require them back (`TRAILING_SPACE_HEALING`).
- **Abstention.** A continuation abstains when it adds a number absent from the
  typed text.
- **Spelling** is English only and needs negotiated exact replacement of the
  word before the caret; a single dictionary candidate is proposed without
  inference.

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
