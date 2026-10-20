# Badi development

Dependable local prediction and spelling assistance for Linux, Omarchy first.
Preserve exact editing authority and prove the affected user flow.

## Start here

1. Read this file, then "Where Badi stands", the open decisions and "Next
   steps" in [future plans.md](<future plans.md>). Its step 1 is the current
   priority unless the user asks for something else.
2. Read the runbook of the area you will touch (see [Architecture](#architecture))
   and the ADRs it links. The top of [CHANGELOG.md](CHANGELOG.md) shows what
   changed last.
3. Run `git status` and `git log --oneline -5`. Preserve existing changes,
   untracked files and deletions. `output/` holds ignored local evaluation
   evidence; leave it alone.
4. Set up once: `npm ci`. Cargo installs the pinned toolchain from
   [rust-toolchain.toml](rust-toolchain.toml) on first use.
5. Before reasoning about live behavior, check what is installed: `badi doctor`
   names the installed commit, `badi status` the model and counters.

Each fact has one home:

| Document | Owns |
| --- | --- |
| [README.md](README.md) | What users can do: keys, coverage, install, commands |
| [VISION.md](VISION.md) | The durable product contract |
| [future plans.md](<future plans.md>) | The only backlog: where Badi stands, open decisions, the ordered next steps and later work |
| [CHANGELOG.md](CHANGELOG.md) | What shipped, by merged pull request |
| [docs/decisions](docs/decisions/) | Durable architecture decisions (ADRs) |
| Component `README.md` files | That component's contract, tests and boundaries |

Link to a fact instead of repeating it. Delete stale material instead of
archiving it, and never add parallel planning, handoff or report documents.

## How to work

1. Branch from an up-to-date `main`: `git switch -c <agent>/<topic>` (for
   example `cursor/tab-word`). Work on the requested outcome, not the rest of
   the backlog.
2. Carry a change through code, focused tests and the affected runbook in one
   branch. Make routine local decisions yourself; report concrete blockers
   after finishing independent work.
3. Run the checks for the changed surface ([Verification](#verification)).
   Never weaken a test, gate or hook to pass.
4. Prove user-visible behavior live ([Live trials](#live-trials)) when the
   user authorized installation; otherwise name the unverified boundary.
5. Finish the records: delete a completed backlog item from
   [future plans.md](<future plans.md>), update the runbook and
   [README.md](README.md) keys or coverage, and add one
   [CHANGELOG.md](CHANGELOG.md) line under the newest heading.
6. Commit, push, open or merge a pull request only when the user asked. Stage
   exact paths and inspect the staged diff. A pull request says what changed,
   which checks ran, the live evidence and what remains unverified.

## Architecture

| Area | Responsibility |
| --- | --- |
| `broker/` | Rust: Unix-socket protocol, per-target policy, revision-bound suggestion and commit authority, type-through carry, private settings, the local writing model, `badictl`. [Runbook](broker/README.md). |
| `protocol/`, `broker/schemas/` | Wire and control schemas. Keep validators, producers, consumers and tests consistent when a contract changes. |
| `adapters/fcitx5/` | C++20 cooperative Fcitx module: native fields, IME-parity, keys, display, commit. [Runbook](adapters/fcitx5/README.md), [wire](adapters/fcitx5/WIRE_PROTOCOL.md). |
| `adapters/accessibility/` | Python AT-SPI observer: app identity, lock state, field acquisition, rich-editor serialization, caret calibration, grey preview. [Runbook](adapters/accessibility/README.md). |
| `adapters/session/` | Python session helper: logind sleep and session state, UPower and power-profile state, GlobalShortcuts portal, and delivery of the keyring's style-learning key; reports to the broker and only withholds authority. [Runbook](adapters/session/README.md). |
| `adapters/obsidian/`, `adapters/shell/`, `adapters/neovim/`, `adapters/shared/` | Editor-owned clients with exact acquisition, mutation and native undo. [Editors](adapters/shared/README.md), [Bash](adapters/shell/README.md), [Neovim](adapters/neovim/README.md). |
| `adapters/libreoffice/` | Python UNO Writer extension: explicit English spelling replacement in one undo context, verified by reading back. [Runbook](adapters/libreoffice/README.md). |
| `ui/omarchy-plugin/` | Omarchy bar mark and settings panel. [Runbook](ui/omarchy-plugin/README.md). |
| `scripts/`, `packaging/` | `badi` desktop controller, user-local installers, service units, the [Arch package candidate](packaging/aur/README.md). |
| `evaluation/` | Prediction Lab and its worker crate `badi-writing-lab`, which uses only the broker's public API. The broker never references the Lab. [Runbook](evaluation/writing/README.md), [model fit](docs/architecture/model-selection.md). `evaluation/power/` holds the paced battery trial; `evaluation/llm-bench/` the [local LLM bench](evaluation/llm-bench/README.md) for runtime and model speed. |

Where code lives inside the larger components:

- **Broker** (`broker/src/`): `server.rs` decodes wire frames and runs one
  handler per message; `engine/` owns sessions, authority epochs, generation,
  acceptance, type-through and desktop holds, with its tests in
  `engine/tests/` by topic; `settings/` is the private settings document
  (identity, permissions, policy, paths, private files, the compare-and-swap
  store); `writing.rs` and `semantic/` run the local model; `style.rs`,
  `personalization.rs`, `model_selection.rs` and `control_plane.rs` are what
  their names say; `bin/badictl/` is the control CLI. A module's unit tests
  live beside it in `<module>/tests.rs`.
- **Fcitx addon** (`adapters/fcitx5/src/`): `addon.h` declares the addon;
  `addon.cpp` (lifecycle and input-context events), `addon_keys.cpp`,
  `addon_observer.cpp`, `addon_broker.cpp` (broker events and display) and
  `addon_session.cpp` implement it over the tested core in `state`,
  `observation`, `transport`, `accessibility`, `sanitizer` and `debug`.
- **Observer** (`adapters/accessibility/`): `daemon.py` (the private D-Bus
  service and AT-SPI events), `desktop.py` (active window, Omarchy lock and
  process identity), `field.py` (acquisition, serialization and Lab-only
  window context), `contract.py` (the acquisition contract), `geometry.py`
  and `preview.py` (caret calibration and the overlay), `health.py`
  (readiness probe).

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
- **Desktop signals** ([ADR 0005](docs/decisions/0005-desktop-signals-withhold-authority.md))
  from the session helper may hold or quiet suggestions; they never grant.
- Spelling replacement needs negotiated `text_replacement`, an exactly bound
  suffix or range and explicit acceptance; test stale replacement and undo.
- No raw keylogging, clipboard replacement or blind synthetic typing as product
  integrations; an allowed terminal's typed line, and the DELs that erase a
  tracked typo when Tab accepts its correction, are the one bounded exception
  ([ADR 0009](docs/decisions/0009-terminals-use-the-typed-line.md)). Keep typed prose and credentials out of diagnostics; only the
  explicit, expiring `badi debug on --text` trace records text
  ([ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md)).
  Headed tests use disposable text and isolated profiles.
- The normal broker uses the hardware-selected local model; `--provider phrase`
  selects the deterministic fixture. Model readiness alone proves no adapter
  input.

## Verification

Run commands from the repository root with the root `package.json` and
`package-lock.json` (`npm ci` when needed); never add adapter manifests or mix
package managers. `Cargo.toml` sets the 1.85 MSRV and edition 2024.
[rust-toolchain.toml](rust-toolchain.toml) pins the active toolchain at 1.99.0,
and [CI](.github/workflows/ci.yml) uses that pin for the Rust and Fcitx jobs.
The MSRV job stays on `cargo +1.85.0` because the toolchain file would
otherwise override it. Pick checks by the
changed surface:

| Change | Checks |
| --- | --- |
| Documentation | `npm run docs:check`, verify documented commands and paths, inspect the diff |
| Rust broker, model, policy | Targeted tests, then the Rust gate below; real-broker adapter lanes for wire or authority changes |
| Fcitx module | `npm run fcitx5:check`; `npm run fcitx5:integration` for transport or broker changes |
| Accessibility observer | `npm run accessibility:check`; `npm run accessibility:integration` for bus or event changes |
| Session helper | `npm run session:check`; `npm run session:integration` for broker, bus or portal changes |
| Observer and addon together | `PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/observed-desktop.py` |
| Obsidian, Bash, Neovim, shared clients | `npm run editors:check`, `npm run editors:integration` |
| LibreOffice Writer extension | `npm run libreoffice:check`, `npm run libreoffice:integration` |
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
editor, LibreOffice, desktop, accessibility, Omarchy and writing checks and summarizes;
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

## Live trials

- Install the branch: `python3 scripts/install-desktop.py`, plus
  `scripts/install-editors.py` or `scripts/install-omarchy-ui.py` when those
  changed; `badi doctor` must then name your commit.
- Trace the flow: `badi debug on --text`, `badi debug watch` in a second
  terminal, then `badi debug log --json` for evidence and `badi debug off`.
  The trace shows requests, model outcomes, keys with `ghost_visible`, and
  every ghost show and hide with its cause.
- Type only disposable text, in isolated browser profiles and scratch
  documents. Name the app, its version and the installed commit in results.

## Evidence and completion

- Never relax schema, identity, sample-count or cleanup gates to make a result
  pass.
- A Prediction Lab claim pairs runs of the same suites, settings and seed
  before and after the change; a single after-run is not a comparison.
- Report fixture, real-broker, real-model and live-app evidence separately. A
  smoke result is not a held-out quality score; a tested app version is not
  universal Linux support.
- Before finishing, inspect the final diff and status, including untracked
  files, and state what changed, which checks ran and what remains unverified.
