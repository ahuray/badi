# Badi

Badi (`بعدی`, Persian for “next”) is a capability-aware local co-writing layer
for Linux. It offers one short, revision-bound continuation only where an
adapter can read, display,
and edit through a tested target API. Unsupported is a valid result; raw global
input capture and synthetic typing are not part of the architecture.

> **Naming boundary:** Badi is now the selected product name. An unrelated
> [AI workflow CLI](https://github.com/fatihkan/badi) already uses the `badi`
> command, so this project uses `badictl`
> and the owned `io.github.ahuray.badi` native identity. Public package and
> trademark clearance remains a release decision. Badi-owned source and
> documentation are licensed under MIT.

## Current status

> **Review state:** this tree contains the post-audit remediation of baseline
> `b8d6786`. GitHub Actions for the commit containing these changes is the
> authoritative source-verification result; the durable Chromium receipt and
> its performance figures still belong to the baseline. Treat this as an
> incomplete M2 architecture surface, not a product demonstration, until it
> has headed Chromium/Omarchy validation; see the
> [current implementation handoff](docs/delivery/2026-08-30-control-center-local-intelligence-handoff.md).

The repository contains the research, the M1 trust foundation, an M2A isolated
Chromium integration slice, and a fail-closed control-plane foundation:

- a strict JSON Schema protocol with positive/negative examples, 64 KiB
  little-endian frames, explicit UTF-16 browser offsets, surrogate-safe bounded
  context, relative TTLs, and a shared multilingual accept-word fixture;
- a Rust 2024 broker and `badictl` with fail-closed policy, secure local
  Unix socket, peer-UID checks, deterministic suggestions, latest-wins
  cancellation, addressed commits, global pause, content-free metrics, and
  graceful SIGINT/SIGTERM cleanup;
- content-free hardware inspection and pinned writing/code model
  recommendations that never download or activate a model automatically;
- strict, revisioned `badi.settings.v1` origin permissions in private XDG
  storage, broker-enforced revocation, structured compare-and-swap errors, and
  optional text-free origin/day interaction aggregates;
- a repo-owned Quickshell control center that reads one versioned
  `badictl overview` contract, persists pause and the exact localhost origin
  rule, exposes recorder/model truth, and never patches Omarchy configuration;
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

Adaptive writing memory is not implemented. The optional local aggregate store
contains origin, provider, UTC day, and interaction counts—never prose—but its
“shown” and “accepted” counters currently mean broker-emitted and
commit-requested, not independently confirmed display/application. The
localhost LLM client and receipt gate are evaluation-only: production model
wiring remains disabled until an owned local runtime can be authenticated and
bound to the verified model artifact.

The latest historical M2A receipt records 1,000/1,000 exact insert/caret trials
and 100/100 delayed stale races. Nearest-rank p95 was 12.6 ms from trusted
accept to observed insertion and 0.6 ms from invalidation marker to hidden UI,
after 50 warmups for each 1,000-sample distribution. The receipt is linked to
its recorded source commit; it is not a reproduction against every later tree.

## Start here

- [Omarchy review dossier](docs/delivery/2026-08-31-omarchy-review-dossier.md)
  — the current architecture, evidence, GrillMe delta, readiness path, and
  reviewer questions in one compact handoff.
- [GrillMe product-proof plan](docs/plan/grillme-product-proof.md) — the
  implementation slices, agent ownership, quality gates, evidence workflow,
  and hard stop conditions for the next build.
- [Vision V2](VISION-V2.md) — the current product and trust contract.
- [V2 landscape](docs/research/vision-v2-landscape.md) — primary-source review
  of Cotypist and sixteen direct or adjacent products/projects.
- [V2 implementation plan](docs/plan/vision-v2-implementation.md) — exact
  milestones, capability cells, agent workflow, tests, rollout, and stop
  conditions.
- [Develop branch roadmap](docs/plan/develop-roadmap.md) — the next coding
  order, architecture rules, first sprint, and promotion gates.
- [Independent adversarial audit](docs/delivery/2026-08-30-independent-adversarial-audit.md)
  — the immutable review of commit `b8d6786`, including findings, claim checks,
  evidence limits, and the GrillMe verdict.
- [Post-audit remediation handoff](docs/delivery/2026-08-30-remediation-handoff.md)
  — the remediation fixes, pre-freeze verification results, residual risks, and
  reviewer decision boundary.
- [Control center and local-intelligence handoff](docs/delivery/2026-08-30-control-center-local-intelligence-handoff.md)
  — the current settings/UI/model implementation, exact verification, privacy
  contract, and unsupported product scope.
- [GrillMe Omarchy and suggestion-quality round](docs/delivery/2026-08-30-grillme-omarchy-quality-round.md)
  — the hostile post-implementation verdict, remediations, remaining product
  blockers, and tests that can pass while the real experience remains broken.
- [Same-UID trust decision](docs/decisions/0001-same-uid-trust-boundary.md) — why
  local UID verification is a process boundary rather than authentication.
- [MIT source-license decision](docs/decisions/0002-mit-source-license.md) — the
  covered material, independent artifact licenses, and remaining release gates.
- [Hardware-aware model selection](docs/architecture/model-selection.md) — the
  compact probe, conservative tiers, pinned artifacts, and runtime gates.
- [Quickshell control center](ui/quickshell/badi/README.md) — the versioned
  status/settings contract, exact supported policy scope, and non-installing
  repo-local launch path.
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

The repository is still pre-release (`0.1.0`). `protocol/v1` is a strict
current-tree contract, but the develop branch has expanded required v1 status
fields during remediation; cross-commit compatibility with older strict v1
clients is not promised. Freeze or negotiate a new version before separately
shipped adapters or third-party clients depend on it.

## Hardware-aware model candidates

```sh
cargo run --quiet --bin badictl -- hardware --json
cargo run --quiet --bin badictl -- models writing --json
cargo run --quiet --bin badictl -- models code --json
```

These commands are content-free and offline. When an artifact fits the
conservative host-memory budget, they return pinned Hugging Face metadata and a
non-executing download plan; otherwise they return an explicit `no_fit` result.
Semantic inference remains disabled until a candidate passes Badi's quality and
latency gates.

## Control center and private settings

With the broker and `badictl` available, inspect the machine-readable contract:

```sh
badictl overview --json
badictl settings show --json
```

Run the UI directly from the repository without installing it or changing
Omarchy:

```sh
qs --path "$PWD/ui/quickshell/badi/shell.qml"
```

Today the editable identity is exactly the Chromium development origin
`http://localhost:4173`; the tracked manifest remains narrower still and
injects only `/chromium.html`. This is not yet a generic application/site
allowlist. Unknown identities deny by default, and the settings file is written
privately under XDG paths through the broker rather than by QML.

## Verify from a clean checkout

CI tests Node.js 22.23.2 and 24.20.0, Rust 1.98.0, and the declared Rust 1.85
minimum. The project default is pinned to Node 24.20.0 in `.nvmrc`; the
capability receipt records the separate local validation environment. From the
repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo +1.85.0 check --workspace --all-targets --all-features --locked
npm ci
npm run check
git diff --check
```

The Chromium build is generated at `adapters/chromium/dist/` and is ignored by
Git. Its timestamp-free `BUILD_MANIFEST.json` records stable SHA-256 hashes;
the root check compares two clean builds byte for byte, validates local
documentation links and receipt internals, and verifies the V2 evidence against
its recorded Git commit. It deliberately labels that result historical. Use
`npm run capabilities:check:current` when current sources and generated
artifacts must match a receipt; source changes are expected to fail that stricter
gate until a new immutable evidence identity is created.

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
npm run live --workspace @badi/chromium -- \
  --evidence-id chromium-native-live-run.2026-08-30-review1.v1
```

The smoke uses reduced counts and writes only ignored, content-free diagnostic
JSON and may be used while iterating. Run the durable command only from a clean,
isolated clone or worktree with a unique `--evidence-id`; the runner refuses a
dirty tree and opens the output with no-overwrite semantics. It runs at least
1,000 measured interactions after 50 warmups and 100 delayed stale-result
trials, then writes the schema-validated raw evidence document. A separate new
receipt must hash-link that raw file. Both commands build locally, create only
disposable directories, and verify cleanup; neither uses the real Chromium
profile.

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

## License

Badi-owned source code and documentation are available under the
[MIT License](LICENSE), the same license used by Omarchy. Model weights,
tokenizers, datasets, dependencies, generated artifacts, names, and trademarks
retain their independent terms; see [ADR 0002](docs/decisions/0002-mit-source-license.md).
