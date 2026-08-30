# Badi

Badi (`بعدی`, Persian for “next”) is a capability-aware local co-writing layer
for Linux. It offers one
short, revision-bound continuation only where an adapter can read, display,
and edit through a tested target API. Unsupported is a valid result; raw global
input capture and synthetic typing are not part of the architecture.

> **Naming boundary:** Badi is now the selected product name. An unrelated
> [AI workflow CLI](https://github.com/fatihkan/badi) already uses the `badi`
> command, so this project uses `badictl`
> and the owned `io.github.ahuray.badi` native identity. Public package and
> trademark clearance, plus the project license, remain release decisions.

## Current status

The repository contains the research, the M1 trust foundation, and an M2A
isolated Chromium integration slice:

- a strict JSON Schema protocol with positive/negative examples, 64 KiB
  little-endian frames, explicit UTF-16 browser offsets, surrogate-safe bounded
  context, relative TTLs, and a shared multilingual accept-word fixture;
- a Rust 2024 broker and `badictl` with fail-closed policy, secure local
  Unix socket, peer-UID checks, deterministic suggestions, latest-wins
  cancellation, addressed commits, global pause, content-free metrics, and
  graceful SIGINT/SIGTERM cleanup;
- content-free hardware inspection and pinned writing/code model
  recommendations that never download or activate a model automatically;
- a bounded Rust Chrome-native-message bridge plus a deterministic print-only
  manifest generator pinned to one public development extension identity;
- a strict TypeScript Manifest V3 adapter limited to one exact localhost page,
  with pre-acquisition field denial, adapter-owned ghost UI, exact
  type-through, broker-authorized word/all acceptance, dismissal, and pause;
  and
- deterministic Rust, schema, TypeScript, jsdom race, extension-build, and
  evidence-link checks in CI, plus a repeatable isolated system-Chromium run.

The live runner uses disposable HOME/XDG/profile directories and leaves no
extension, native manifest, socket, or process installed. It proves the named
headless Chromium build and exact controlled document, not general browser
support. Headed permission consent, background visibility, the extension
command accelerator, native undo, policy epochs, framework fields, arbitrary
sites, semantic-model quality, Obsidian, and terminal support remain explicit
gaps.

The current M2A receipt records 1,000/1,000 exact insert/caret trials and
100/100 delayed stale races. Nearest-rank p95 was 8.4 ms from trusted accept to
observed insertion and 0.7 ms from invalidation marker to hidden UI, after 50
warmups for each 1,000-sample distribution.

## Start here

- [Vision V2](VISION-V2.md) — the current product and trust contract.
- [V2 landscape](docs/research/vision-v2-landscape.md) — primary-source review
  of Cotypist and sixteen direct or adjacent products/projects.
- [V2 implementation plan](docs/plan/vision-v2-implementation.md) — exact
  milestones, capability cells, agent workflow, tests, rollout, and stop
  conditions.
- [Develop branch roadmap](docs/plan/develop-roadmap.md) — the next coding
  order, architecture rules, first sprint, and promotion gates.
- [Hardware-aware model selection](docs/architecture/model-selection.md) — the
  compact probe, conservative tiers, pinned artifacts, and runtime gates.
- [Chromium foundation receipt](capabilities/chromium-dom-foundation.v1.json) —
  machine-readable evidence class, supported surface, exclusions, versions,
  and checks.
- [Chromium native live receipt](capabilities/chromium-native-live.v2.json) —
  hash-linked real-browser/native-host scenarios, measurements, isolation, and
  honest unsupported surfaces.
- [Capability evidence guide](capabilities/README.md) — receipt classes,
  linkage rules, validation, and attestation limits.
- [Chromium runbook](adapters/chromium/README.md) and
  [broker/native-bridge runbook](broker/README.md) — narrow boundaries,
  commands, isolation, and cleanup behavior.

The original [V1 vision](VISION.md),
[Linux architecture research](docs/research/linux-architecture.md),
[adversarial review](docs/research/adversarial-review.md), and
[two-day plan](docs/plan/two-day-delivery.md) remain useful decision history.
Their conflicting three-target/48-hour gates are superseded by V2.

## Architecture

```text
supported field -> Chromium controller -> MV3/native boundary -> Rust broker
      ^                    |                         |
      |                    v                         v
target-API edit      adapter ghost UI       policy/provider/metrics
```

The adapter alone may read and mutate its target. The broker validates policy
and state, prepares a commit, and never drives a keyboard. Every non-global
action carries session, focus epoch, revision, fingerprint, and suggestion ID.
Only pause is global. See the V2 plan for the full trust boundary.

## Hardware-aware model candidates

```sh
cargo run --quiet --bin badictl -- hardware --json
cargo run --quiet --bin badictl -- models writing --json
cargo run --quiet --bin badictl -- models code --json
```

These commands are content-free and offline. They return pinned Hugging Face
metadata and a non-executing download plan; semantic inference remains disabled
until the candidate passes Badi's quality and latency gates.

## Verify from a clean checkout

CI tests Node.js 22.23.2 and 24.20.0, Rust 1.98.0, and the declared Rust 1.85
minimum. The project default is pinned to Node 24.20.0 in `.nvmrc`; the
capability receipt records the separate local validation environment. From the
repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm ci
npm run check
git diff --check
```

The Chromium build is generated at `adapters/chromium/dist/` and is ignored by
Git. Its timestamp-free `BUILD_MANIFEST.json` records stable SHA-256 hashes;
the root check compares two clean builds byte for byte and validates local
documentation links, receipt schemas, evidence hashes, and cross-document
claims.

To inspect the controlled page only:

```sh
npm run fixture --workspace @badi/chromium
```

Open `http://localhost:4173/chromium.html`. A normal browser still needs an
explicit native-host setup; no command in the ordinary build/test path installs
one.

To reproduce the isolated live lane on Linux with system Chromium at
`/usr/bin/chromium`:

```sh
npm run live:smoke --workspace @badi/chromium
npm run live --workspace @badi/chromium
```

The smoke uses reduced counts and writes only ignored, content-free diagnostic
JSON. The durable command runs at least 1,000 measured interactions after 50
warmups and 100 delayed stale-result trials, then writes the schema-validated
raw evidence document. Both commands build locally, create only disposable
directories, and verify cleanup; neither uses the real Chromium profile.

## Product order

1. Preserve the narrow M2A Chromium receipt while completing the headed
   permission/policy-epoch gates before any origin expansion.
2. Prove the same signature loop through supported Obsidian/CodeMirror APIs.
3. Evaluate a small local semantic suffix provider against the deterministic
   baseline.
4. Run the manual Fcitx5/Ghostty/Codex feasibility cell independently and
   report failure honestly if the native path cannot meet the contract.

There is no fallback to `evdev`, `wtype`, clipboard insertion, `xdotool`, or a
shell-only demonstration.
