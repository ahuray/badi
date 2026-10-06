# Badi development

Dependable local prediction and spelling assistance for Linux, Omarchy first.
Preserve exact editing authority and prove the affected user flow.

## Start here

- Read this file, the affected source and its runbook. Each fact has one home:

  | Document | Owns |
  | --- | --- |
  | [README.md](README.md) | What users can do: keys, coverage, install, commands |
  | [VISION.md](VISION.md) | The durable product contract |
  | [future plans.md](<future plans.md>) | The only backlog: open work and open decisions |
  | [docs/decisions](docs/decisions/) | Durable architecture decisions (ADRs) |
  | Component `README.md` files | That component's contract, tests and boundaries |

  Link to a fact instead of repeating it. Delete stale material instead of
  archiving it, and never add parallel planning or handoff documents.
- Inspect Git status before editing; preserve existing changes, untracked files
  and deletions. Work on the requested outcome, not the rest of the backlog.
- For implementation requests, carry the change through code, tests and the
  affected runbook. Make routine local decisions yourself and reuse prior
  authorization; report concrete blockers after finishing independent work.

## Architecture

| Area | Responsibility |
| --- | --- |
| `broker/` | Rust: Unix-socket protocol, per-target policy, revision-bound suggestion and commit authority, type-through carry, private settings, the local writing model, `badictl`. [Runbook](broker/README.md). |
| `protocol/`, `broker/schemas/` | Wire and control schemas. Keep validators, producers, consumers and tests consistent when a contract changes. |
| `adapters/fcitx5/` | C++20 cooperative Fcitx module: native fields, IME-parity, keys, display, commit. [Runbook](adapters/fcitx5/README.md), [wire](adapters/fcitx5/WIRE_PROTOCOL.md). |
| `adapters/accessibility/` | Python AT-SPI observer: app identity, lock state, field acquisition, rich-editor serialization, caret calibration, grey preview. [Runbook](adapters/accessibility/README.md). |
| `adapters/obsidian/`, `adapters/shell/`, `adapters/shared/` | Editor-owned clients with exact acquisition, mutation and native undo. [Editors](adapters/shared/README.md), [Bash](adapters/shell/README.md). |
| `ui/omarchy-plugin/` | Omarchy bar mark and settings panel. [Runbook](ui/omarchy-plugin/README.md). |
| `scripts/`, `packaging/` | `badi` desktop controller, user-local installers, service units, [Fcitx frontend backport](packaging/fcitx5-wayland-compat/README.md). |
| `evaluation/` | Prediction Lab and its worker crate `badi-writing-lab`, which uses only the broker's public API. The broker never references the Lab. [Runbook](evaluation/writing/README.md), [model fit](docs/architecture/model-selection.md). |

## Editing and privacy invariants

- Adapters own document acquisition and mutation. Check exact app or site policy
  before acquiring text; the broker keeps policy and commit authority central.
- Fail closed on sensitive fields, foreign composition, unsupported selections,
  unknown authority and stale context, through each adapter's explicit path.
- Bind suggestions and acceptance to the exact field identity available on that
  path, plus revision, fingerprint, caret, focus and expiry. Keep cancellation
  and one-shot acceptance. Never weaken binding to gain coverage.
- An unknown native widget identity is allowed only through the explicit manual
  Fcitx contract. Never invent a stable identity or reuse a grant across focus
  or authority epochs. The Fcitx module yields to foreign preedit and
  candidates.
- Fcitx `commitString` proves `dispatched-unverified`, never `applied`. Keep
  native undo and explicit acceptance. Bash suggestions edit the Readline buffer
  and never submit or evaluate commands.
- **IME-parity** ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md)
  lists the apps): apps without an editor channel accept through one append-only
  `commitString` that behaves like typed text. Undo may merge with earlier
  typing, and page script may redirect it; document that, and never claim exact
  undo there. Every other guard stays: sensitive and purpose denial,
  foreign-composition yield, exact observer identity with snapshot and caret
  agreement before display and dispatch, revision, fingerprint and expiry
  binding, one-shot acceptance, no retries or synthetic keys. Replacement stays
  editor-owned. Only the exact `zen` identity covers Gecko, and the observer
  alone denies its urlbar.
- **Type-through** ([ADR 0004](docs/decisions/0004-type-through-carries-text-not-authority.md))
  carries suggestion text, never authority, into the next admitted request.
- Spelling replacement needs negotiated `text_replacement`, an exactly bound
  suffix or range and explicit acceptance; test stale replacement and undo.
- No raw keylogging, clipboard replacement or blind synthetic typing as product
  integrations. Keep typed prose and credentials out of diagnostics. Headed
  tests use disposable text and isolated profiles.
- The normal broker uses the hardware-selected local model; `--provider phrase`
  selects the deterministic fixture. Model readiness alone proves no adapter
  input.

## Verification

Run commands from the repository root with the root `package.json` and
`package-lock.json` (`npm ci` when needed); never add adapter manifests or mix
package managers. `Cargo.toml` sets Rust 1.85+ and edition 2024, and
[CI](.github/workflows/ci.yml) pins the exact toolchains. Pick checks by the
changed surface:

| Change | Checks |
| --- | --- |
| Documentation | `npm run docs:check`, verify documented commands and paths, inspect the diff |
| Rust broker, model, policy | Targeted tests, then the Rust gate below; real-broker adapter lanes for wire or authority changes |
| Fcitx module | `npm run fcitx5:check`; `npm run fcitx5:integration` for transport or broker changes |
| Accessibility observer | `npm run accessibility:check`; `npm run accessibility:integration` for bus or event changes |
| Observer and addon together | `PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/observed-desktop.py` |
| Obsidian, Bash, shared clients | `npm run editors:check`, `npm run editors:integration` |
| Desktop controller, installers | `npm run desktop:check` |
| Prediction Lab | `npm run writing:check`, `cargo test --locked -p badi-writing-lab`; a real-model `run.mjs` comparison when inference changes |
| Omarchy panel | `npm run omarchy:check`; the host and lifecycle lanes in its runbook for QML or runtime changes |
| Schemas, dependencies, broad changes | The Rust gate plus `npm run check` and affected integrations |

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo +1.85.0 check --workspace --all-targets --all-features --locked
```

`npm run check` ([run-checks.mjs](scripts/run-checks.mjs)) runs the docs,
editor, desktop, accessibility, Omarchy and writing checks and summarizes;
`node scripts/run-checks.mjs NAME...` runs a subset. No source check proves
native app behavior: claims about a real app need a live trial in the graphical
session, and missing prerequisites must be reported with the exact unverified
boundary.

## Desktop changes and delivery

- Source checks need no installation. Inspect the installed state before
  assuming this workstation matches the checkout.
- `scripts/badi-desktop.py` controls only the persistent broker's private
  socket, and settings writes are compare-and-swap bound to the revision read.
  `badi pause` persists; `badictl pause` is runtime only.
- `scripts/install-desktop.py` installs the broker, addon, observer, commands
  and launcher and restarts user services. `install-editors.py` updates editor
  integrations. `install-omarchy-ui.py` updates the panel only after explicit
  unlocked state. Installers back up what they replace and keep the newest
  three backups.
- Implementation alone does not authorize desktop installation, release,
  publication or Git history changes. When the user asked for one, carry it
  through without asking again.

## Evidence and completion

- Commit only when the task authorizes it. Never relax schema, identity,
  sample-count or cleanup gates to make a result pass.
- Report fixture, real-broker, real-model and live-app evidence separately. A
  smoke result is not a held-out quality score; a tested app version is not
  universal Linux support.
- When behavior changes or a backlog item completes, update its runbook,
  [README.md](README.md) coverage and [future plans.md](<future plans.md>).
- Before finishing, inspect the final diff and status, including untracked
  files, and state what changed, which checks ran and what remains unverified.
