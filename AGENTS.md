# Badi development

Build dependable local prediction and spelling assistance for Linux, with
Omarchy first. Preserve exact editing authority and prove the affected user flow.

## Start here

- Read this file, the affected source, and its runbook. Use
  [future plans.md](<future plans.md>) as the only active backlog,
  [VISION.md](VISION.md) as the durable product contract and
  [docs/decisions](docs/decisions/) for durable architecture decisions. Git
  history keeps everything removed; delete stale material rather than archive it.
- Inspect Git status before editing. This checkout may contain substantial work
  in progress; preserve existing changes, untracked files, and intentional
  deletions. Work on the user's requested outcome without expanding into the
  rest of the backlog.
- For implementation requests, carry the change through code, relevant tests,
  and documentation. Make routine local decisions autonomously and reuse prior
  authorization. Report concrete blockers after finishing independent work.
- [README.md](README.md) owns the coverage summary. Adapter runbooks contain
  tested versions and boundaries; recheck them before making compatibility claims.

## Architecture and runbooks

| Area | Responsibility and starting point |
| --- | --- |
| `broker/` | Rust workspace member: strict Unix-socket protocol, per-target policy, revision-bound suggestion/commit authority, private settings, local writing model, and CLI tools. Start with the affected module and tests. |
| `protocol/`, `broker/schemas/` | Wire and control schemas. Keep validators, producers, consumers, and regression tests consistent when contracts change. |
| `adapters/fcitx5/` | Cooperative C++20 module. Read [native contract, trials, installation, and rollback](adapters/fcitx5/README.md). |
| `adapters/obsidian/`, `adapters/shell/`, `adapters/shared/` | V2 editor clients, editor-owned acquisition/mutation, and native undo. Read [editor integrations](adapters/shared/README.md). |
| `ui/omarchy-plugin/` | Omarchy settings panel and native writing bar. Read [controls and lifecycle checks](ui/omarchy-plugin/README.md). |
| `scripts/badi-desktop.py`, `scripts/install-*.py`, `packaging/` | Persistent desktop broker controls, user-local installers, service units, and launcher integration. Check the affected script and its existing tests. |
| `evaluation/` | Prediction Lab (`evaluation/writing/lab/`). Read the [Lab runbook](evaluation/writing/README.md) before changing qualification behavior. |

## Editing and privacy invariants

- Adapters own document acquisition and mutation. Check exact app/site policy
  before acquiring text. Keep the broker's policy and commit authority central.
- Preserve hard field denial and negotiated capabilities. Sensitive fields,
  foreign composition, unsupported selections, unknown authority, and stale
  context fail closed according to the adapter's explicit protocol path.
- Bind suggestions and acceptance to the exact document/field identity available
  in that path, revision, fingerprint, caret, focus, and expiry. Preserve
  cancellation and one-shot acceptance; never weaken binding to obtain coverage.
- An unknown native widget identity is allowed only through the existing explicit
  manual Fcitx contract. Do not invent stable identity or reuse a grant across
  focus/authority epochs. The Fcitx module yields to foreign preedit/candidates.
- Fcitx `commitString` proves `dispatched-unverified`, never verified `applied`.
  Preserve native undo and explicit user acceptance. Bash suggestions edit the
  Readline buffer without submitting or evaluating generated commands.
- IME-parity (user decision, 2026-09-26,
  [ADR 0003](docs/decisions/0003-ime-parity-append-only.md)) covers
  Chromium-based apps without an editor-owned channel: Chromium, Brave,
  Codex/ChatGPT desktop, VS Code, Cursor and Discord. By user request
  (2026-09-26) it also covers the Zen browser (Gecko, exact `zen` identity
  only); its urlbar has no Url purpose, so the observer alone must deny browser
  UI there. Other Gecko builds stay unavailable. These apps may accept through
  one append-only Fcitx `commitString` that behaves like typed text: undo may
  coalesce with preceding typing, and page script that moves focus or caret
  during `beforeinput` may redirect it like a keystroke. Document that; never
  claim exact undo or verified field authority there. Every other guard stays:
  sensitive/purpose denial, foreign composition yield, exact observer identity
  with snapshot/caret agreement before display and dispatch,
  revision/fingerprint/expiry binding, one-shot acceptance, no retries or
  synthetic keys. Replacement stays editor-owned.
- Spelling replacement requires negotiated `text_replacement`, an exactly bound
  suffix/range, and explicit acceptance. Test stale replacement and undo behavior.
- Do not implement raw keylogging, clipboard replacement, or blind synthetic
  typing as product integrations. Use disposable text and isolated profiles for
  headed tests. Keep typed prose and credentials out of diagnostics by default.
- The normal broker uses the hardware-selected local writing model.
  `--provider phrase` explicitly selects the deterministic integration fixture.
  Model readiness alone does not prove adapter input.

## Development and verification

Run commands from the repository root. Use the root `package.json` and
`package-lock.json`; do not add adapter manifests or lockfiles or mix package
managers.
`Cargo.toml` declares Rust 1.85+, edition 2024, and workspace lint policy.
[package.json](package.json) declares supported Node versions and check scripts;
[CI](.github/workflows/ci.yml) pins the exact Rust, Node, and Arch environments.

Install JavaScript dependencies with `npm ci` when needed. Choose checks by the
affected surface; these commands are a verification map, not a requirement to run
every integration for every edit.

| Change | Relevant checks |
| --- | --- |
| Documentation/instructions | `npm run docs:check`, verify documented commands/paths, and inspect the scoped diff for whitespace. |
| Rust broker/model/policy | Targeted regression tests, then the Rust checks below. Run affected real-broker adapter integrations for changed wire/authority behavior. |
| Fcitx5 | `npm run fcitx5:check`; `npm run fcitx5:integration` for transport or broker changes. Native UI claims additionally need real application trials. |
| Obsidian/Bash/shared clients | `npm run editors:check` and `npm run editors:integration`; real editor checks for acceptance, focus, rendering, or undo changes. |
| Desktop controller/installers | `npm run desktop:check`; include Omarchy/editor checks when those boundaries change. |
| Omarchy UI | `npm run omarchy:check`; host/lifecycle checks below for QML or runtime changes. |
| Shared schemas, dependencies, or broad changes | Relevant Rust checks plus `npm run check` and affected integrations. |

Rust verification:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo +1.85.0 check --workspace --all-targets --all-features --locked
```

`npm run check` ([scripts/run-checks.mjs](scripts/run-checks.mjs)) runs the
documentation, editor, desktop, accessibility, Omarchy, and writing checks,
continues past failures, and ends with a summary; `node scripts/run-checks.mjs
NAME...` runs a subset. It does not run headed tests or prove native application
behavior. Avoid rerunning checks already covered by an aggregate unless new
changes or failures justify it.

Additional integration lanes; read their runbooks before use:

```sh
npm run fcitx5:integration
npm run editors:integration
node adapters/obsidian/live.mjs --headless --local
BADI_OMARCHY_REQUIRE_HOST_CHECKS=1 bash ui/omarchy-plugin/tests/check-source.sh
bash ui/omarchy-plugin/tests/run-client-lifecycle.sh
```

`fcitx5:integration` requires CMake, Ninja, Fcitx5Core, nlohmann-json, a C++20
compiler, Python 3, and Cargo. It exercises the C++ transport against a disposable
real broker. The Obsidian live runner uses an isolated vault/profile and ignored
diagnostics under `output/`; consult it for local-model prerequisites.
Real desktop tests require the actual graphical session and disposable documents.
Report missing prerequisites and the precise unverified boundary.

## Desktop changes and delivery

- Source checks run without installation. Inspect current installed state before
  assuming this workstation matches the checkout.
- `scripts/badi-desktop.py` controls only the persistent broker's private
  socket; settings mutations stay compare-and-swap bound to the revision the
  overview read. Never substitute a guessed socket or a different running broker.
- `badi pause` persists across restarts; `badictl pause` is runtime control.
  Preserve this distinction in the UI, CLI, tests, and documentation.
- `scripts/install-desktop.py` installs the broker, addon, commands, and launcher,
  removes retired components with a backup, and restarts user services.
  `scripts/install-editors.py` updates selected editor integrations. Use them
  when the task authorizes installation; inspect backups, targets, and
  service/editor recovery before changing the running session.
- `scripts/install-omarchy-ui.py` updates the installed panel only after explicit
  unlocked state. Preserve the lock gate and existing user customizations.
  Validate source first and verify health after an authorized install/reload.
- Implementation and source verification alone do not authorize desktop
  installation, release, publication, or Git history changes. When the user has
  requested the relevant action, carry it through without redundant confirmation.

## Evidence and completion

- Commit only when authorized by the task. Never relax schema, identity,
  sample-count, or cleanup gates to make a result pass.
- Report fixture, real-broker, real-model, and physical application evidence
  accurately. A smoke result is not a held-out quality score, and a tested
  application/version is not universal Linux support.
- Update the relevant runbook and existing backlog/history when the task changes
  behavior or completes a backlog item. Keep durable facts concise and source
  bound; avoid new parallel planning or handoff documents.
- Before finishing, inspect the final diff and status, including untracked files.
  State the behavior delivered, checks actually run, and remaining limitations.
  Preserve unrelated work and keep commits/publication within user authorization.
