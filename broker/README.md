# Badi broker

The local policy broker and its control CLI. Adapters acquire and edit the
document; the broker owns target policy, session state, suggestion and commit
authority, the local writing model and content-free metrics. It is
Linux/Unix-socket only and never drives a keyboard, clipboard or accessibility
API.

- `badi-broker` serves the private socket. `--provider phrase` explicitly
  selects the deterministic integration fixture instead of the writing model.
- `badictl` sends control, health, settings and probe requests to that socket
  and runs offline hardware and model advice.

The Prediction Lab is a separate workspace crate,
[evaluation/writing/lab-worker](../evaluation/writing/lab-worker/), built on
this crate's public API. This crate never depends on it.

```sh
cargo build -p badi-broker --bins
```

`build.rs` embeds the Git commit of the Rust inputs (`broker/`, `Cargo.toml`,
`Cargo.lock`) and whether they differed from it; both binaries print it with
`--version`, for example `badictl 0.1.0 commit=<40 hex> dirty=false`. Without
usable Git metadata both fields are `unknown`; packagers may set both
`BADI_BUILD_COMMIT` and `BADI_BUILD_DIRTY`.

## Protocol and authority

- The socket is `$XDG_RUNTIME_DIR/badi/broker.sock`, mode `0600`, and accepts
  only same-UID peers ([decision](../docs/decisions/0001-same-uid-trust-boundary.md)).
  Frames are at most 64 KiB, and a larger declaration is rejected from its
  header. Wire schemas are in [protocol/](../protocol/) and control documents
  in [schemas/](schemas/).
- `hello` negotiates protocol v1 or v2 and the capabilities a connection may
  use: context, suggestion, commit kinds, text replacement, policy, control,
  health and settings.
- A suggestion is bound to its session, focus epoch, revision, fingerprint and
  expiry, and acceptance is one-shot. An adapter must report a granted commit
  within 1.5 s (a lease capped at 5 s); retained context is revoked after 3 s
  of silence; protocol v2 native and editor suggestions stay readable for 5 s.
  Stale, foreign or unknown state fails closed.
- Type-through ([decision](../docs/decisions/0004-type-through-carries-text-not-authority.md)):
  the last shown continuation is kept as text only.
  - When a later admitted request in the same app or site has the same
    `after` and language, and a `before` that adds a typed proper prefix of
    the suggestion, the broker shows the remainder without a model call.
  - The remainder is a new suggestion bound to that request, with the
    original deadline.
  - Escape, a whole acceptance, pause, an authority change or the deadline end
    the carry; a word acceptance keeps it, so the rest can follow.
- Settings (`badi.settings.v2`) are replaced by compare-and-swap on their
  revision. While a replacement's outcome is unknown, suggestions stay paused.
  `badi pause` persists in settings; `badictl pause` is runtime only.
- `"all_web_origins": true` (`badi site all on`) lets an http(s) browser origin
  without its own rule read context and suggest, but never learn or retain. An
  exact origin rule still wins, field denial and pause still apply, and private
  windows are not distinguished. Protocol v1 settings clients neither see nor
  erase the flag. While the memory store is unavailable only a strict authority
  reduction is accepted: clearing the flag is one; setting it, or removing an
  origin block while it is set, is a grant.
- `"all_linux_apps": true` (`badi apps blocklist`) is the same default for a
  Linux app without its own rule: prediction only, never learning or
  retention, exact app rules win, and it carries the same authority rules. Its
  policy reply says `matched_default` instead of `matched_rule`, so the Fcitx
  addon opens only an observed field with it and keeps the manual Tab-request
  path for exact rules. Browser defaults keep `matched_rule`, which protocol v1
  clients know.
- Policy connections stay open while idle; control and health clients close
  after 300 s without a request. A transient `accept` failure is logged once
  and retried after 100 ms.

## badictl

`badictl --help` lists every command. Broker commands print JSON; any failure
exits 1 with `error_code=CODE` on stderr.

```sh
badictl status            # content-free health, metrics and no-suggestion classes
badictl overview          # broker, settings, privacy and model readiness
badictl settings show
badictl settings replace --if-revision N --json DOCUMENT
badictl memory clear      # text-free origin/day aggregates
badictl pause [on|off|toggle]
badictl request | accept-word | accept-all | dismiss   # the sole active session
badictl probe [--language TAG] [--after TEXT] [--replace] [--explicit] [--] TEXT|-
badictl hardware --json   # offline
badictl models writing --json
```

`status` counts each provider call that showed nothing once under
`no_suggestion` (`request_abstained`, `budget_prefill`, `budget_stream`,
`model_abstained`, `output_rejected`, `stale`, `timeout` or `provider_error`)
and names the latest as `last`. Timeouts, runtime failures and the broker's own
shape or replacement-authority rejections also count in `provider_errors`.
Protocol v1 keeps its frozen counter set.

`probe` runs TEXT (before the caret) through the live provider and the same
display checks as a suggestion, within the normal deadline and admission limit.
It opens no session, grants no commit authority, changes no counters and is
neither logged nor stored. `TAG` defaults to `en`; `--replace` grants exact
replacement and `--explicit` the explicit-request budget. Arguments stay in
shell history and the process list, so `-` reads TEXT from standard input
(one trailing newline removed).

`models` prints pinned catalog advice with a Hugging Face revision, SHA-256 and
a non-executing `hf` argument vector. It downloads nothing and never claims the
runtime is ready; the Lab owns measured qualification
([device fit](../docs/architecture/model-selection.md)).

## Writing model runtime

- **Selection.** Startup picks an installed pinned artifact that fits total
  memory after the host reserve and runtime headroom, a hard floor; unknown
  total memory fails closed. The advised tier wins, then smaller installed
  artifacts, then the smallest larger one that fits. If available memory is
  short, startup waits (2 s, doubling to 30 s) and journals one `waiting for
  available memory` and one `memory available after waiting` line.
- **Verification.** Startup hashes the model, runtime binary, bundle and
  archive once; a size or hash mismatch is an error. Checkpoints before spawn,
  after spawn and after readiness confirm the same files by device, inode,
  size and modification and change times.
- **Warm-up.** Before binding the socket, the broker sends one fixed two-token
  completion bounded at 2 s and discards the reply. The journal records
  `writing runtime warm-up completed elapsed_ms=N` or `failed class=C
  elapsed_ms=N`; a failure only leaves the first request cold.
- **Containment.** The runtime starts through the broker binary's parent-death
  helper, so it cannot outlive the broker, and the broker waits on its pidfd.
  If it exits, the broker ends its sessions, removes the socket and exits 1
  with `error_code=local_model: runtime_process_exited`. The
  [service unit](../packaging/systemd/badi-broker.service) restarts the broker
  after 2 s, backing off to 60 s without giving up, except after exit status
  78: a missing, unsupported or too-large model stays stopped for `badi doctor`.
- **Transport.** `semantic::wire` speaks HTTP/1.1 to loopback only, with the
  runtime's bearer token and `Connection: close`: never proxied, redirected,
  retried or pooled. Heads are limited to 8 KiB and bodies per endpoint.
  Abandoning a request closes its connection, which is how the runtime learns
  it was cancelled.
- **Launch.** One 512-token slot reuses the common prompt prefix; the RAM cache
  archive and slot saving are off. Prefill batches are 16 tokens so a cancelled
  prompt yields sooner, and inference uses up to four threads. Cached
  generation can vary numerically even with a fixed seed
  ([llama.cpp contract](https://github.com/ggml-org/llama.cpp/blob/b10726/tools/server/README.md#post-completion-given-a-prompt-it-returns-the-predicted-completion)).

## Suggestion limits

The exact rules live in [src/writing.rs](src/writing.rs).

- An automatic request gives the provider 550 ms and the broker 600 ms; an
  explicit one (Tab, Ctrl-X Tab or a `request` control) 1,200 ms and
  1,250 ms. Only time differs. Adapter RPC deadlines exceed both: 2 s in the
  editor client and 3 s in Bash.
- Context is at most 512 characters before the caret and 128 after. The prompt
  is the current sentence within 160 scalars, extended back a sentence at a
  time until it has four words, so text typed in order reuses the cached
  prefix. Edit authority still binds the full context.
- A suggestion is at most four space-separated words and 64 characters.
  `en`, `de` and `fa` tags are routed; English and German output must be Latin
  script and Persian Arabic script, with ZWNJ only between the letters in
  [the joiner fixtures](../protocol/orthographic-joiner-fixtures.json).
- An English suffix that continues the typed word must form a word in the
  pinned [SCOWL lexicon](data/writing-lexicon/README.md); an unrecognized
  trailing English word is regenerated from its boundary under a grammar that
  requires the typed stem. German and Persian suggest whole words only.
- Languages in `TRAILING_SPACE_HEALING` (English and Persian) prompt without
  the trailing spaces and require them back; a prompt left empty abstains.
- A continuation abstains when it contains a digit run absent from the typed
  text, or ends a number at the `.` stop.
- Spelling is English only and needs negotiated exact replacement of the word
  before the caret, optionally with the one space typed after it. A single
  dictionary candidate is proposed without inference.

## Shutdown

SIGINT and SIGTERM handlers are registered before the socket exists. Either one
returns through the normal server path and removes the socket after checking
its device and inode, then exits 0; a runner should wait for the socket to
disappear rather than delete it. The service unit uses `KillMode=mixed`: the
broker signals its runtime once and gives it 2 s before the process-group kill.

## Tests

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo +1.85.0 check --workspace --all-targets --all-features --locked
```

The tests cover framing, policy, revision and commit binding, badictl output
against a phrase broker, signal shutdown and the owned runtime. That runtime
test, `tests/owned_runtime.rs`, has no libtest harness: it launches itself as a
fake llama-server (`tests/support/fake_llama_server.rs`).

Development probes of the installed model run serially and use synthetic
prompts, not the held-out writing corpus:

```sh
cargo test --release --locked --test writing_runtime -- --ignored --nocapture --test-threads=1
```
