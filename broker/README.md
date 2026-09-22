# Badi broker and native bridge

This crate contains the local policy broker, its control CLI, and the narrow
Chromium native-messaging bridge. It is Linux/Unix-socket only in the current
proof and never drives a keyboard, clipboard, or accessibility API.

## Binaries

- `badi-broker` owns policy, session state, cancellation, the local writing
  provider, content-free metrics, and the private Unix socket. `--provider phrase`
  explicitly selects the deterministic integration fixture.
- `badictl` sends explicit control and health requests to that socket and runs
  offline hardware/model recommendation commands.
- `badi-native-host` translates Chrome native-message frames to validated
  Badi protocol frames on the existing broker socket. It does not start the
  broker or provide suggestions itself.
- `badi-native-manifest` prints one deterministic native-host manifest to
  standard output. It never installs or writes that manifest.

Build all four without installing them:

```sh
cargo build --workspace --bins
```

Inspect hardware and model candidates without starting the broker or using the
network:

```sh
target/debug/badictl hardware --json
target/debug/badictl models writing --json
target/debug/badictl models code --json
```

The recommendation includes a pinned Hugging Face revision, artifact SHA-256,
and non-executing `hf` argument vector. It does not download weights or claim
that the semantic provider is ready.

The opt-in Prediction Lab adds dynamic public Hugging Face discovery and measured
device qualification. The offline `badictl models` commands remain the pinned
catalog advice path. Run the integrated workbench from the repository root:

```sh
npm run writing:lab -- --port 36889
```

Open the printed HTTP address and expand **Choose and qualify a local model**:
inspect the device, search/inspect metadata, assess fit, verify a download, select
it for a local comparison, then review and measure its results. Opening the raw
HTML file does not run the application. See the
[writing workflow, API and persistence contract](../evaluation/writing/README.md#discover-assess-and-compare-a-model).

The `writing-lab` feature also exposes read-only JSON helpers after building the
Lab worker. These commands inspect resources or assess supplied metadata; they
do not download or launch an inference model:

```sh
target/release/badi-writing-lab --inspect-device
target/release/badi-writing-lab --assess-model < /absolute/path/assessment-input.json
target/release/badi-writing-lab --rank-models < /absolute/path/ranking-input.json
```

`--assess-model` takes `{candidate, settings, evidence}`; `--rank-models` takes an
array of at most 32 such objects. JSON input is limited to 128 KiB. Their strict types and stages are defined in
[the qualification module](src/model_selection/qualification.rs). Optional
`--cache-directory /absolute/path` selects the filesystem whose free capacity is
inspected. The browser sends server-issued IDs instead of these low-level
metadata/settings objects, arbitrary paths or runtime flags.

Qualification separates discovery, estimated fit, verified local exercise,
measured performance, measured prediction quality and recommendation. It reuses
hardware inspection, exact artifact/runtime verification and owned Lab cleanup.
Unknown architecture-specific memory costs block loading; shared GPU/RAM
capacity is never counted twice. This implementation qualifies the pinned CPU
configuration. GPU enumeration and any separate physical Vulkan execution proof
do not qualify another backend automatically. Hard gates require untouched,
fully reviewed confirmation and measured 550 ms complete-word delivery; the
result remains `no_qualified_model` until every gate passes. Expected text never
enters model requests, and model experiments do not establish adapter editing
authority, native undo or application coverage.

Normal startup selects an installed, pinned writing artifact that fits current
available memory after the host reserve and runtime headroom. Hardware advice is
a preference: a battery-state change does not require downloading a different
model when the installed artifact still fits. The advised tier is preferred,
then smaller installed artifacts, then the smallest fitting larger artifact.
Missing or insufficient memory fails closed. Selection never bypasses the model
and runtime size/hash checks, and a corrupt selected artifact is an error.

The broker polls its owned model process every 200 ms. Unexpected exit ends the
broker with `error_code=local_model: runtime_process_exited` after the normal
session cancellation and socket cleanup. The desktop service's existing
`Restart=on-failure` then starts a fresh broker; old requests and grants are
never replayed. This checks the owned child handle, not a reused PID.

The current writing route accepts `en`, `de`, and `fa` language tags and their
subtags. English/German output is limited to Latin script; Persian output uses
Arabic script. This routing is not a multilingual quality qualification. Persian
ZWNJ is preserved only between the exact Arabic-letter ranges shared by
`protocol/orthographic-joiner-fixtures.json`; other invisible and bidi controls
remain forbidden. A suggestion beginning with ZWNJ still abstains, since its
left letter is outside the standalone suggestion. Spelling remains English and
requires negotiated exact replacement authority.

Cold inference uses the current sentence within at most 160 Unicode scalars;
larger cold prompts can consume the inference deadline before the first token.
The broker keeps the original bounded context for revision, fingerprint and edit
authority. When a trailing English word is not recognized by the lexicon, prefix
completion regenerates it from its word boundary under a grammar requiring the
exact typed stem. Recognized whole words use ordinary prefix completion. The streaming reader
strips that stem before validating or displaying the suffix. A mismatched or
incomplete echo abstains. English suffix joins must form a known word in the
embedded [pinned SCOWL lexicon](data/writing-lexicon/README.md); this rejects
invented extensions of names and brands. German/Persian currently suggest whole
words and abstain on alphabetic suffix joins until equivalent lexicons are
reviewed. Known English words and dictionary prefixes bypass spelling inference,
so pausing inside `docum` or `expl` goes directly to completion. If the existing
single-edit spelling constraints yield exactly one dictionary candidate, the
broker proposes that correction without an inference round trip. Ambiguous
candidates use the model and must still target a lexicon word and satisfy the
same conservative edit constraints. Correction remains an explicit, negotiated
replacement of the word immediately before the caret, optionally including the
single ASCII space just typed after it. The exact same space is preserved in
both word and full acceptance. Multiple spaces, punctuation, tabs and
sentence-wide grammar correction are outside that edit contract.
Spelling and continuation share one 550 ms budget; at
its end the reader can return only words whose following
separator actually arrived; it never treats a timeout as a completed word.

Writing inference reuses the common prompt prefix in one active 512-token KV
slot. This transient runtime state is separate from writing-history retention:
the additional RAM cache archive and disk slot saving are disabled. Unrelated
prompts replace nonmatching state; cancellation, policy and document authority
are still checked independently before a suggestion can be used. Cached
generation can vary numerically even with a fixed seed, as documented in the
[pinned llama.cpp HTTP contract](https://github.com/ggml-org/llama.cpp/blob/b10726/tools/server/README.md#post-completion-given-a-prompt-it-returns-the-predicted-completion).
The historical evaluator
keeps its original uncached English-only prompt and runtime contract.
Writing launches also cap both prefill batch sizes at 16 so interrupted prompts
yield sooner between runtime decode operations. A clean development comparison
preserved all 24 outputs/abstentions without a measured short-text penalty;
this does not guarantee immediate cancellation of an in-flight runtime batch.
Historical evaluator batch defaults remain unchanged.

The opt-in development probes exercise the installed model without opening an
editor or reading a document:

```sh
cargo test --release --locked --test writing_runtime -- --ignored --nocapture --test-threads=1
```

Run the real-model tests serially to avoid measuring two competing CPU models.
Their synthetic prompts are development checks, not the held-out writing corpus.

## Native-message boundary

The development host accepts only this caller origin:

```text
chrome-extension://ckkiehcjbclcjckkkajohopoikeejkoa/
```

That ID is derived from the public development key in the Chromium manifest.
The host and generated native manifest both pin it exactly; wildcards and
caller-selected origins are rejected. A future production identity therefore
requires rebuilding the host as well as changing the extension manifest.

Chrome uses a native-endian unsigned 32-bit length followed by UTF-8 JSON.
Chrome's transport permits larger messages, but this bridge applies Badi's
65,536-byte encoded-envelope ceiling in both directions and rejects an
oversized declared input before allocating its body. Every frame is decoded as
a strict protocol envelope before relay. The bridge verifies the broker socket
metadata and peer UID, writes no content to logs, and treats expected EOF or a
closed Chrome output pipe as a clean disconnect.

The host uses `$XDG_RUNTIME_DIR/badi/broker.sock` by default. An absolute
`--socket` override exists for direct development tests; Chrome itself supplies
only the caller origin.

## Broker shutdown and socket cleanup

`badi-broker` registers Linux SIGINT and SIGTERM handlers before exposing
its socket. Either signal returns through the normal server path, drops the
socket guard, and removes the private socket before the process exits
successfully. Cleanup rechecks the socket's device and inode, so it will not
unlink a path that another process replaced.

Phrase-provider process tests send both signals, require a zero exit status and empty
stdout/stderr, and observe the socket disappear within a bounded interval. A
runner should therefore wait for that disappearance rather than force-deleting
the socket.

The [broker service](../packaging/systemd/badi-broker.service) uses
`KillMode=mixed`: systemd sends graceful TERM to the broker, and the broker
signals its owned runtime process group once. Sending TERM to both via
`control-group` makes the runtime receive a second signal during cleanup;
llama.cpp b10726 treats that as an immediate unsuccessful exit. The runtime has
two seconds for an active decode and HTTP readers to unwind, then retains the
bounded process-group KILL/reap fallback. The unit's ten-second stop timeout
still covers the entire cgroup.

Private real-service trials reproduced unsuccessful/forced runtime exits with
the old unit and 200 ms grace. With mixed signalling and a two-second grace,
an idle runtime exited zero in 93 ms and an in-flight synthetic Persian request
exited zero in 1042 ms; every broker/runtime and transient unit was cleaned up.
A regression child that needs 350 ms after TERM verifies that graceful cleanup
is allowed to finish. Receipts and source-controlled diagnostic builds are in
`output/writing/2026-09-07-runtime-cancellation/`.

Two installed-runtime SIGABRT records on 2026-09-07 at 22:59:33 and 23:01:06
CEST coincided with service restarts. Their cores were truncated before complete
ELF notes, so no symbolized crash mechanism was available. The duplicate-signal
and insufficient-grace faults above were reproduced separately; the trials
do not claim an exact SIGABRT stack trace. Task-private follow-up crash probes
set their own core limit to zero and retained stderr plus actual exit status.

## Print-only manifest workflow

The checked-in [example manifest](native-messaging/io.github.ahuray.badi.example.json)
is reproducible with:

```sh
target/debug/badi-native-manifest \
  --host-path /opt/badi/badi-native-host
```

The supplied path must be absolute UTF-8 without `.` or `..` components. The
command prints JSON and makes no profile, user-configuration, or system change.
The isolated live runner creates its own temporary HOME and writes the emitted
manifest only below that disposable directory.

## Verify

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Tests cover fragmented frames, empty/truncated input, a 65,537-byte declaration
rejected from its header alone, strict caller-origin validation, deterministic
manifest output, bidirectional socket relay, EOF, broken output pipes, and
observed SIGINT/SIGTERM socket cleanup.
