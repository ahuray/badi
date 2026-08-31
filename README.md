# Badi

Badi (`بعدی`, Persian for “next”) is a capability-aware, local-first writing
assistant for Linux. It offers one short, revision-bound continuation only when
an adapter can read, display, and edit through a tested target API. Unsupported
is a valid result; raw global input capture, clipboard insertion, and synthetic
typing are outside the architecture.

> **Naming boundary:** an unrelated
> [AI workflow CLI](https://github.com/fatihkan/badi) already uses the `badi`
> command, so this project uses `badictl` and the owned
> `io.github.ahuray.badi` native identity. Public package and trademark
> clearance remains a release decision.

## Current state

Badi is pre-release. This tree contains one narrow product path and keeps its
remaining claims deliberately smaller than its code:

- **Dillinger product slice:** an MV3 extension requests only the optional
  `https://dillinger.io:443/*` host permission, registers its content script
  only after consent, reads and edits the one exact Dillinger Monaco document,
  renders caret-relative ghost text, and accepts with `Ctrl+Shift+Y` as one
  target-native undoable transaction.
- **Local semantic evaluation:** a feature-gated evaluator can supervise one
  pinned `llama.cpp` child and one pinned Qwen3 1.7B GGUF artifact through a
  private, fresh-bearer-gated loopback boundary. It is development evidence only.
  The normal broker still uses the explicit four-rule `phrase_v1` fixture;
  no model is downloaded or activated automatically.
- **Omarchy integration artifact:** `ui/omarchy-plugin` is a small
  disabled-by-default panel for Omarchy's existing shell. It uses shared host
  primitives and a bounded `badictl` process lifecycle. It is validated from
  an isolated copy and is not installed or enabled on this machine.
- **Evidence V3:** strict run and product-cell schemas bind observations,
  artifacts, role attestations, cleanup, and an exact implementation commit.
  No V3 receipt exists yet because the required headed, visual, accessibility,
  and final semantic qualification have not all happened.

The former standalone Quickshell control center was removed after the Omarchy
plugin passed its isolated lifecycle gates. This leaves one control surface and
avoids a second resident shell.

## Trust boundary

```text
exact target API <-> Chromium adapter <-> native host <-> Rust broker
      |                    |                    |            |
 Monaco edit        optional consent      bounded IPC   policy/provider
 caret ghost        document fences       exact origin  private settings

evaluation only: verified model + verified runtime bundle -> owned llama.cpp
control only:    Omarchy panel -> fixed badictl argv -> broker contract
```

The adapter alone may read or mutate target text. Every non-global action is
bound to session, focus epoch, revision, fingerprint, and suggestion ID. The
broker owns policy, cancellation, commit authorization, and content-free
metrics. Same-UID IPC is process isolation, not authentication against a
malicious process running as the same user.

## Try the current product slice

Requirements: Linux, system Chromium, Node.js 22.23+ or 24.20+, Rust 1.85+,
and a graphical session. From the repository root:

```sh
npm ci
npm run live:product --workspace @badi/chromium -- --interactive
```

The runner builds the product extension and Rust bridge, creates a disposable
HOME/XDG/profile/socket tree, and opens Chromium. Approve the exact Dillinger
permission, focus its editor, type the fixed `thank you` integration trigger,
wait for the ghost suggestion, and accept with `Ctrl+Shift+Y`. Return to the
terminal and press Enter; the runner revokes permission, stops its processes,
removes the profile and socket, and fails if cleanup cannot be proven. Its
diagnostics are content-free and are not release evidence.

The automated product transaction can also be exercised without the
interactive hold:

```sh
npm run live:product --workspace @badi/chromium
```

Chromium still requires a real permission decision. Window-manager focus
transfer may require one manual click. The disposable run proves one exact
browser/editor cell, not arbitrary websites or all Chromium versions.

## Evaluate the local-model boundary

The evaluator is deliberately feature-gated and never changes normal broker
composition:

```sh
cargo run -p badi-broker --features local-model-eval --bin badi-evaluator -- \
  fixture-self-test

cargo run -p badi-broker --features local-model-eval --bin badi-evaluator -- \
  pinned-development MODEL.gguf LLAMA_SERVER RELEASE_ARCHIVE.tar.gz
```

The pinned-development path accepts only the declared artifact names, sizes,
digests, and exact extracted runtime-bundle manifest. It re-verifies them
around child launch, uses a fresh bearer secret on private loopback, owns the
process group, and emits content-free development observations. Passing this
small development run does **not** qualify production use; the owner-approved
100-case blinded corpus and final visible-path gates remain open.

Hardware advice is offline and content-free:

```sh
cargo run --quiet --bin badictl -- hardware --json
cargo run --quiet --bin badictl -- models writing --json
cargo run --quiet --bin badictl -- models code --json
```

Recommendations contain pinned metadata and a non-executing download plan.
They never download, start, or claim readiness for a model.

## Validate the Omarchy artifact

No command below changes live Omarchy configuration:

```sh
npm run omarchy:check
omarchy plugin validate ui/omarchy-plugin
BADI_SUMMON_CYCLES=100 bash ui/omarchy-plugin/tests/run-isolated.sh healthy
```

The portable gate runs without Omarchy. On the pinned local Omarchy cell it
also checks the official validator, QML, host hashes, lifecycle teardown, and a
TERM-ignoring child. Visual theme, focus, scaling, screen-reader,
multi-monitor, packaging, and installed-`badictl` checks remain manual gates.

## Verify the repository

CI runs Rust 1.98, the declared Rust 1.85 MSRV, and Node.js 22.23.2/24.20.0.
The local equivalent is:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo +1.85.0 check --workspace --all-targets --all-features --locked
npm ci
npm run check
git diff --check
```

`npm run check` covers strict schemas, TypeScript, jsdom races, reproducible
extension builds, product-manifest policy, scenario descriptions, Omarchy
lifecycle gates, evidence immutability, and capability linkage. Historical V1
and V2 receipts remain verified against their recorded commits. Use an explicit
receipt when requiring current-source linkage:

```sh
npm run capabilities:check:current -- --receipt-id <id>
```

Adding V3 evidence requires a clean implementation commit and chronological
approval; the CI diff gate rejects a receipt added with the implementation it
claims to attest.

## Start here

- [GrillMe product-proof plan](docs/plan/grillme-product-proof.md) — vertical
  slices, stop conditions, ownership, and the final evidence contract.
- [Omarchy review dossier](docs/delivery/2026-08-31-omarchy-review-dossier.md) —
  the pre-implementation product critique and decision context.
- [Vision V2](VISION-V2.md) — current product and trust contract.
- [Hardware-aware model selection](docs/architecture/model-selection.md) —
  candidate, runtime, evaluator, and activation boundaries.
- [Chromium runbook](adapters/chromium/README.md) — historical fixture lane and
  current exact-Dillinger product runner.
- [Omarchy artifact](ui/omarchy-plugin/README.md) — host contract, isolation,
  lifecycle proof, and current limits.
- [Capability evidence guide](capabilities/README.md) — immutable V1/V2 history
  and the V3 approval workflow.
- [Independent adversarial audit](docs/delivery/2026-08-30-independent-adversarial-audit.md)
  and [GrillMe review](docs/delivery/2026-08-30-grillme-omarchy-quality-round.md)
  — historical findings retained as immutable review context.

The original [V1 vision](VISION.md),
[Linux architecture research](docs/research/linux-architecture.md), and
[two-day plan](docs/plan/two-day-delivery.md) remain decision history. Their
broader target and timing claims are superseded by Vision V2 and the GrillMe
plan.

## Explicit non-goals for this milestone

- No arbitrary-site, Obsidian, terminal, Fcitx5, or multilingual product claim.
- No network model provider, automatic model download, personalization, or
  prose retention.
- No `evdev`, `wtype`, clipboard, `xdotool`, synthetic-key, or global-input
  fallback.
- No production semantic activation before final commit-linked qualification.

## License

Badi-owned source code and documentation are available under the
[MIT License](LICENSE), the same license used by Omarchy. Model weights,
tokenizers, datasets, dependencies, generated artifacts, names, and trademarks
retain their independent terms; see
[ADR 0002](docs/decisions/0002-mit-source-license.md).
