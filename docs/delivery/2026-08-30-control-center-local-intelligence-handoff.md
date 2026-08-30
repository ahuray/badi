# Badi control center and local-intelligence handoff

- Audit snapshot: uncommitted implementation candidate
- Base commit: `75abbfad315f026922645fe00f8d75d15f266879`
- Prepared: 2026-08-30
- Freeze authorization: owner-approved on 2026-08-30 for one complete `develop` commit
- Audience: owner, architecture reviewers, and follow-on coding agents

## Executive outcome

This working tree delivers a fail-closed settings/control-plane foundation, a
repo-owned Quickshell control center, stricter Chromium lifecycle authority,
text-free optional interaction aggregates, and an evaluation-only local-model
gate. It is suitable for source review after it is frozen and CI passes at that
exact commit.

It is **not** yet the requested finished local co-writer. The editable policy
identity is one exact Chromium development origin; adaptive writing memory is
not implemented; and the broker still runs the deterministic `phrase_v1`
provider. No semantic model is installed, activated, or represented as ready.
The UI has passed a real isolated Quickshell load but has not had headed visual,
keyboard, or assistive-technology validation on Omarchy/Hyprland.

The distinction matters:

- this tree is a credible privacy and authority foundation;
- it is not a generic per-application allowlist;
- its aggregate counters are not learned writing style;
- its model catalog is advice, not runtime proof; and
- historical Chromium evidence does not prove this uncommitted tree.

## Delivered boundaries

### Settings and policy

`badi.settings.v1` is the normative user-policy document. It is strict,
revisioned, compare-and-swap protected, and deny-by-default. The current schema
supports up to 64 canonical subjects and keeps the independent permissions
`context_read`, `suggest`, `display`, and `learn`, plus bounded retention
([settings types](../../broker/src/settings.rs#L23),
[formal schema](../../broker/schemas/badi.settings.v1.schema.json)).

The broker, not QML, owns validation and persistence. Settings are written to
private XDG storage with lifetime interprocess locks, atomic replacement,
directory synchronization, strict cleanup of Badi-owned complete-document
temporaries, and fail-closed handling when a write or cleanup commit cannot be
proven. Unsafe temp lookalikes are preserved and rejected. An unknown settings
commit moves the live engine to a
restart-required condition instead of restoring possibly stale authority
([storage boundary](../../broker/src/settings.rs#L532),
[engine condition](../../broker/src/engine.rs#L201),
[mutation handling](../../broker/src/engine.rs#L1518)).

Revoking policy invalidates live sessions and reconciles retained aggregate
data before the new authority is exposed. Permission changes are preflighted;
a failed destructive reconciliation does not publish the new settings. A
subject-identical pause-only change remains available when the optional
aggregate store is recoverably unavailable
([control-plane transaction](../../broker/src/control_plane.rs#L117)).

### Quickshell control center

The control center is a repo-owned Quickshell 0.3.1 configuration under
[`ui/quickshell/badi`](../../ui/quickshell/badi/README.md). It neither installs
itself nor edits Omarchy-owned configuration.

It is currently a standalone `FloatingWindow` controlled through Quickshell
IPC, not an installed tray item, Omarchy menu patch, desktop entry, or service.
Status refreshes on launch, show, explicit refresh, and after mutations; there
is no live subscription or periodic polling
([shell boundary](../../ui/quickshell/badi/shell.qml#L8),
[refresh boundary](../../ui/quickshell/badi/BadiClient.qml#L305)).

This is not yet an Omarchy-native shell integration. Current Omarchy guidance
places menus and panels inside its one long-running Quickshell shell as plugins
and exposes shared theme tokens rather than inviting a second shell with a
private palette. See the official [shell plugin
contract](https://github.com/omacom/omarchy/blob/quattro/docs/omarchy-shell.md)
and [theme contract](https://github.com/omacom/omarchy/blob/quattro/docs/theming.md).
The standalone surface is appropriate for isolated source review, but a real
plugin, shared theme adoption, and headed compositor proof remain required
before calling the UI Omarchy-quality.

The UI shows:

- broker reachability, pause state, sessions, socket/frame facts, authority
  epoch, settings revision, and degraded state;
- the exact supported Chromium document and its suggestion/context/display
  policy;
- optional aggregate permission, retention, integrity, record/byte counts,
  dropped signals, and write failures;
- writing-model recommendation, installed/configured/readiness truth, hardware
  tier, artifact identity, and caveats; and
- explicit unsupported adapters and evidence class.

All reads use one versioned `badictl overview --json` document. All mutations
use fixed process argument arrays—never a shell—and replace a complete locally
validated settings document with a revision precondition. Reads and mutations
are mutually serialized and have a five-second deadline followed by bounded
termination
([QML process boundary](../../ui/quickshell/badi/BadiClient.qml#L305),
[overview assembly](../../broker/src/bin/badictl.rs#L242),
[overview schema](../../broker/schemas/badi.overview.v1.schema.json)).

Missing, malformed, incoherent, or degraded state disables mutation rather
than guessing. Emergency pause has a narrower fallback: if persisted settings
cannot be safely changed while a broker is active, the UI may issue a
process-local `pause on`. It never toggles blindly, and it clears only that
runtime pause with explicit `pause off` when control-plane authority is healthy;
persisted or degraded authority may still keep the effective state paused
([mutation gates](../../ui/quickshell/badi/BadiClient.qml#L52),
[pause behavior](../../ui/quickshell/badi/BadiClient.qml#L358)).

The only editable subject today is exactly:

```text
adapter: chromium
origin:  http://localhost:4173
page:    /chromium.html (manifest/content-script scope)
```

The settings identity is origin-scoped while the tracked extension remains
narrower and injects only the named page
([manifest](../../adapters/chromium/manifest.json#L17),
[document predicate](../../adapters/chromium/src/shared/fixture-document.ts#L1)).
This must not be described as a general site or Linux-application allowlist.

### Chromium authority

The browser connection now carries the settings policy and authority epoch
through the native/runtime boundary. Bootstrap, reconnect, service-worker
replacement, navigation, pause, visibility, focus, and stale native-generation
paths retire or fence authority. A page cannot supply its own caller identity;
the background derives the exact document identity from extension-owned sender
metadata before mapping it to the broker protocol
([service-worker boundary](../../adapters/chromium/src/background/service-worker.ts),
[protocol mapping](../../adapters/chromium/src/background/protocol-mapper.ts),
[content bootstrap](../../adapters/chromium/src/content/content-script.ts)).

This is still an exact controlled-page slice. Static and jsdom coverage do not
prove arbitrary editors, sites, MV3 lifecycle timing, overlay visibility, or
native undo in a headed browser.

### Local private aggregates

Optional personalization storage is deliberately text-free. A record contains
only a stable origin identity, provider class, UTC day, and daily counters for
broker-emitted suggestions and requested outcomes
([record contract](../../broker/src/personalization.rs#L43),
[formal schema](../../broker/schemas/badi.personalization.v1.schema.json)).

It never stores document text, suggestions, accepted prose, phrases, style
features, clipboard data, screenshots, or fine-grained timestamps. `shown`
means the broker emitted a suggestion; acceptance means a commit was requested.
Neither counter independently proves browser display or applied insertion.

Learning requires an explicit exact-origin grant, an unpaused policy, a prior
show, and bounded retention. Retention `none` is memory-only and never becomes
durable; changing from `none` to bounded retention scrubs pre-consent ephemeral
history. Revocation and expiry remove records, and a 60-second idle sweep
enforces expiry even without new interactions. A corrupt or ambiguously
persisted aggregate file is preserved and reported unavailable until the user
explicitly clears it
([recording rules](../../broker/src/personalization.rs#L335),
[control-plane reconciliation](../../broker/src/control_plane.rs#L253),
[outcome worker](../../broker/src/engine.rs#L269)).

The store is bounded to 512 records and 256 KiB. Its maximum valid document is
serialized and schema-tested. Ephemeral subjects cannot evict durable records
([bounds](../../broker/src/personalization.rs#L16),
[maximum-shape test](../../broker/src/personalization.rs#L914)).

This feature is operational telemetry with user-controlled retention. It is
not adaptive writing memory and is not consumed by the suggestion provider.

### Model selection and evaluation gate

`badictl hardware --json` performs content-free local inspection. Model advice
uses a fixed six-artifact official Qwen catalog with exact repository,
revision, filename, size, digest, quantization, license, and reviewed
`llama.cpp` baseline. Recommendations are deterministic and non-executing;
download plans are printed as data and never run
([selection implementation](../../broker/src/model_selection.rs),
[catalog and policy](../architecture/model-selection.md)).

Every recommendation keeps `runtime_ready: false`. Hardware fit is only a
memory/CPU/power suitability screen; it is not quality or latency evidence.
The production broker constructs `DeterministicPhraseProvider`, and no runtime
configuration silently switches it to a model
([production provider](../../broker/src/main.rs#L6),
[readiness contract](../../broker/src/model_selection.rs#L259)).

That deterministic provider is intentionally only a four-rule integration
baseline. It requires a declared English language, an empty suffix after the
caret, and an exact case-insensitive match for the current line after leading
indentation. Trailing whitespace, suffix matches inside larger prose,
non-English or missing language, and nonempty text after the caret all abstain.
The Chromium adapter derives the nearest bounded canonical `lang` declaration,
binds it into the context fingerprint, and transports it to the broker. The
provider emits nothing when no explicit rule matches; the former generic
default fallback was removed rather than presenting context-insensitive filler
as a useful suggestion
([phrase provider](../../broker/src/provider.rs#L65)). This is safer behavior,
not Cotypist-like semantic quality.

Provider output is rejected rather than rewritten when it is empty, contains
forbidden controls or Unicode format controls, exceeds 64 scalars or eight
words, has trailing whitespace, produces an invalid cursor boundary, or
duplicates adjacent text. The current English output lane permits only single
ASCII spaces and explicitly allows unspaced CJK-family adjacency; other scripts
remain unsupported until language-specific fixtures exist. Every applied
commit retires the old suffix and requires fresh context plus fresh generation,
so no cached remainder can inherit document authority or restart a relative
display TTL. The unused post-commit rebind coordinates were removed from the
wire contract instead of preserving a decorative future path. Broker generation
age is capped at 600 ms including
any configured broker debounce and is rechecked after provider completion,
after state-lock acquisition, and immediately before event publication. The
production broker adds no second debounce to Chromium's 140 ms user-idle
debounce. The Chromium controller includes its adapter debounce in the same 600
ms ceiling, owns an independent cancellation timer, and rechecks immediately
before display. The future model gate defines its warm end-to-end clock across
that entire schedule-to-visible path. These gates reduce distracting
suggestions; they do not establish semantic usefulness.

The cursor-boundary gate is intentionally not a general tokenizer. Latin
partial-word completions (`look` + `ing`), punctuation seams, canonically
equivalent Unicode spellings, and multi-scalar overlap in unspaced scripts are
unsupported until the frozen evaluation corpus defines expected behavior. The
shared Rust/schema language validator enforces bounded nonempty ASCII subtags,
not full BCP 47; the current Chromium path additionally requires
`Intl.getCanonicalLocales` canonicalization.

An optional `local-model-eval` feature contains strict artifact verification,
bounded loopback completion parsing, prompt contracts, cancellation, a
versioned aggregate-only receipt, and a deterministic quality gate
([feature declaration](../../broker/Cargo.toml#L15),
[evaluation module](../../broker/src/local_model.rs)). It is deliberately not
compiled into the normal provider path.

The evaluation gate is not yet runnable evidence. The repository has no owned
runtime supervisor, authenticated local transport, launch-manifest producer,
frozen evaluation corpus, evaluator implementation, or raw qualifying run.
The receipt constructor accepts caller-supplied identities and aggregate
metrics, so current validation proves structural/internal consistency—not that
the declared evaluation occurred. A real producer must derive those identities
from inspected artifacts and hash-link immutable raw results.
Plain loopback cannot prove which process owns the port or which artifact it
loaded, and verifying a replaceable model path before use leaves a race. Those
are release blockers, not optional polish.

The evaluation module is also large—more than 2,000 Rust lines plus its receipt
schema—relative to an unimplemented production provider. Keep it isolated; do
not expand it until an owned runtime and real evaluation lane justify the
abstraction.

### Current-machine advice snapshot

The final offline probe reported x86-64, 20 logical CPUs, AVX2, 15,663 MiB
total RAM, approximately 10.1 GiB available RAM, Intel integrated graphics with
no backend-validated dedicated memory, and unknown battery/AC state. Available
memory is a live value and will vary between invocations.

| Use case | Advice | Artifact | Readiness |
| --- | --- | --- | --- |
| Writing | `balanced` | `Qwen/Qwen3-1.7B-GGUF`, `Qwen3-1.7B-Q8_0.gguf` | `runtime_ready: false` |
| Code | `balanced` | `Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF`, `qwen2.5-coder-1.5b-instruct-q4_k_m.gguf` | `runtime_ready: false` |

These are fit recommendations only. Neither artifact exists locally, and the
writing control center intentionally shows the writing recommendation rather
than implying that Badi is a code-completion product.

## Data flow and trust model

```text
exact Chromium field
  -> extension-owned eligibility and bounded context acquisition
  -> native messaging bridge
  -> private same-UID Unix socket
  -> broker policy/state/provider
  -> addressed suggestion
  -> adapter-owned ghost UI
  -> broker-authorized adapter commit

Quickshell UI
  -> fixed-argv badictl JSON contract
  -> private same-UID Unix socket
  -> broker-owned settings and aggregate storage
```

The browser adapter alone reads and mutates its supported field. The broker
never types globally, writes the clipboard, captures the screen, or falls back
to synthetic input. Context is capped at 512 Unicode scalars before and 128
after the caret. Broker IPC is capped at 64 KiB, lives under the user's runtime
directory, uses mode `0600`, and verifies socket metadata and peer UID
([IPC checks](../../broker/src/ipc.rs#L83),
[socket boundary](../../broker/src/server.rs#L36)).

Same-UID verification excludes other operating-system users; it does not
authenticate one process from another process owned by the same user. That
accepted residual risk is recorded separately
([ADR 0001](../decisions/0001-same-uid-trust-boundary.md)).

## Verification for this working tree

The following checks were run serially from the repository root after the last
source change:

| Check | Exact result |
| --- | --- |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo test --workspace --all-features` | pass: 183 tests, 0 failed |
| `cargo +1.85.0 check --workspace --all-targets --all-features --locked` | pass |
| `npm run check` | pass: TypeScript compile, 112 tests, two identical three-file builds, syntax/naming/docs checks, and historical capability validation |
| `npm audit --audit-level=moderate` | pass: 0 vulnerabilities reported |
| Quickshell 0.3.1 isolated offscreen launch with `target/debug/badictl` on `PATH` | configuration loaded and ran until the intentional five-second timeout (`124`); no QML or process error |
| nine reusable QML component files through system `qmllint` | pass individually |
| `git diff --check` | pass |

The isolated Quickshell launch used disposable XDG config, data, cache, state,
and runtime roots. It initially exposed an incomplete `qmldir` registration;
after every local component was registered, the same real loader passed. This
is parser/process-boundary evidence, not visual or accessibility evidence.

`npm ci` was not rerun because the implementation session prohibited software
installation; checks used the existing locked dependency tree. Durable live
browser evidence was intentionally not rerun in this working checkout because
that evidence is immutable/historical and must be produced from a clean,
isolated commit. At handoff preparation, GitHub Actions could not attest the
uncommitted tree; the commit containing this document requires exact-SHA CI.

No model artifact (`*.gguf` or `*.safetensors`) exists in the repository, no
model was downloaded, and no native manifest, extension, Quickshell config, or
Omarchy file was installed. The user's default Badi config/data paths were
verified absent after testing.

## Residual risks and unsupported scope

### Blocks release and a product-readiness claim

1. **No production semantic provider.** The default remains `phrase_v1`; the
   local-model code is evaluation-only and lacks an authenticated, supervised,
   artifact-bound runtime.
2. **No adaptive writing memory.** Stored counters contain no prose or style
   representation and are not provider input.
3. **No general app/site policy identity.** Only one exact Chromium development
   origin is editable, and only `/chromium.html` is injected. Obsidian,
   terminals, arbitrary sites, and native Linux applications are unsupported.
4. **No current live evidence.** The changed Chromium/native/broker path needs a
   new clean-commit receipt plus headed Chromium and Omarchy/Hyprland testing.
5. **No product license/package clearance.** Repository licensing and the
   unrelated `badi` command-name collision remain release decisions.

### Material engineering risks

- The optional aggregate worker rewrites and synchronizes the whole bounded
  file per recorded signal. Its queue is capped at 256. Slow or failing storage
  can drop optional signals and can make the ordered pause fence exceed a
  client's response deadline even though pause authority takes effect first
  ([queue and fence](../../broker/src/engine.rs#L38)). Add a pressure benchmark
  before enabling learning by default; batching is justified only by measured
  failure/latency data.
- `protocol/v1` gained required status/control fields while retaining its
  version. The repository is pre-release and warns that cross-commit strict
  clients are not supported. Freeze the contract or negotiate a new version
  before separately shipping adapters.
- The evaluation-only local-model surface is more code than the current product
  earns. Its isolation is sound, but future work should first deliver the
  missing owned runtime/evaluator boundary rather than add abstractions.
- The current output contract deliberately rejects all Unicode format controls,
  including Persian ZWNJ and emoji ZWJ. Accept-word segmentation is Unicode
  aware, but that does not make generated output multilingual. Persian/Arabic
  shaping, dictionary-script word counts, punctuation adjacency, and emoji
  sequences need language-aware cross-runtime fixtures before any semantic
  provider can claim those output classes.
- Same-UID processes can impersonate a client or broker. This is an accepted
  local-user trust decision for the foundation, not authentication.
- Offscreen loading does not validate visual hierarchy, scaling, focus order,
  compositor behavior, keyboard shortcuts, or screen-reader semantics on a
  real Omarchy laptop.
- Tab and Ctrl/Command+Right commit authorization is asynchronous. The adapter
  must consume the trusted key before broker authorization returns; a later
  denial leaves the field unchanged but cannot replay the browser's native key
  action without synthetic input. This is an honest interaction tradeoff, not
  a tested-good headed-browser experience.
- The QML tree has no automated UI behavior or image-regression suite. The
  control center is snapshot-based and exposes writing advice only; code-model
  advice remains available through `badictl models code --json`.

### Explicitly unsupported today

- background learning from arbitrary windows or applications;
- storage or reuse of the user's prose, tone, vocabulary, or writing history;
- automatic model download, installation, activation, or fallback;
- network inference or remote telemetry;
- clipboard injection, synthetic typing, global key capture, or screen capture;
- arbitrary Chromium origins, framework editors, iframes, Obsidian, terminals,
  and all-app Wayland support; and
- release compatibility for independently versioned `protocol/v1` clients.

## Operator and reviewer entry points

Build locally without installing:

```sh
cargo build --workspace --all-features
```

Run the broker in one terminal and the repo-local control center in another:

```sh
target/debug/badi-broker
PATH="$PWD/target/debug:$PATH" qs --path "$PWD/ui/quickshell/badi/shell.qml"
```

Inspect the machine-readable contracts directly:

```sh
target/debug/badictl overview --json
target/debug/badictl settings show --json
target/debug/badictl hardware --json
target/debug/badictl models writing --json
```

Before changing policy code, read these in order:

1. [`badi.settings.v1` schema](../../broker/schemas/badi.settings.v1.schema.json)
2. [`settings.rs`](../../broker/src/settings.rs)
3. [`control_plane.rs`](../../broker/src/control_plane.rs)
4. [`engine.rs`](../../broker/src/engine.rs)
5. [`BadiClient.qml`](../../ui/quickshell/badi/BadiClient.qml)
6. [control-center contract](../../ui/quickshell/badi/README.md)

The schemas are the public machine contracts; Rust is the normative authority;
QML is a strict consumer. Keep that dependency direction. Do not teach the UI
to edit JSON files, infer permissions, inspect models, or bypass the broker.

## Freeze authorization and boundary

This handoff originally withheld repository mutation. On 2026-08-30, after the
complete diff and residual risks above were reviewed, the owner explicitly
authorized freezing and pushing the complete candidate as one commit on
`develop`. That authorization does not extend to `main`, installation, model
download, or system/Omarchy/browser configuration. Require CI success at the
exact immutable commit. A passing freeze makes the tree ready for focused
source/architecture review—not for release or a claim that the requested
adaptive local co-writer is complete.
