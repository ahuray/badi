# Badi development

Build dependable local prediction and spelling assistance for Linux, with
Omarchy first. Preserve exact editing authority and prove the affected user flow.

## Start here

- Read this file, the affected source, and its runbook. Use
  [future plans.md](<future plans.md>) as the only active backlog;
  [what-have-been.md](what-have-been.md) is compact history. Research documents
  and historical capability receipts are references to consult when relevant.
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
| `adapters/chromium/` | Separate localhost fixture, Dillinger/Monaco product, and opt-in general-web extensions. Read [Chromium development](adapters/chromium/README.md) and [editor integrations](adapters/shared/README.md) for the web build. |
| `adapters/fcitx5/` | Cooperative C++20 module. Read [native contract, trials, installation, and rollback](adapters/fcitx5/README.md). |
| `adapters/obsidian/`, `adapters/shell/`, `adapters/shared/` | V2 editor clients, editor-owned acquisition/mutation, and native undo. Read [editor integrations](adapters/shared/README.md). |
| `ui/omarchy-plugin/` | Omarchy settings panel and native writing bar. Read [controls and lifecycle checks](ui/omarchy-plugin/README.md). |
| `scripts/badi-desktop.py`, `scripts/install-*.py`, `packaging/` | Persistent desktop broker, exact registered trial sockets, user-local installers, service units, and launcher integration. Check the affected script and its existing tests. |
| `capabilities/`, `evaluation/` | Historical qualification, evidence validation, and evaluator tooling. Read the applicable schema/runner before changing evidence or qualification behavior. |

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
- Keep fixture, product, and general-web manifests and permissions separate.
  Browser host permission and exact broker-origin policy are distinct gates.
- Spelling replacement requires negotiated `text_replacement`, an exactly bound
  suffix/range, and explicit acceptance. Test stale replacement and undo behavior.
- Do not implement raw keylogging, clipboard replacement, or blind synthetic
  typing as product integrations. Use disposable text and isolated profiles for
  headed tests. Keep typed prose and credentials out of diagnostics by default.
- The normal broker uses the hardware-selected local writing model.
  `--provider phrase` explicitly selects the deterministic integration fixture.
  `local-model-eval` gates the historical evaluator; its receipts do not qualify
  the current writing path. Model readiness alone does not prove adapter input.

## Development and verification

Run commands from the repository root. Use the root npm workspace and
`package-lock.json`; do not add adapter lockfiles or mix package managers.
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
| Chromium | `npm run typecheck`, `npm test`, `npm run build:verify`, `npm run live:check`; use the appropriate headed lane below for changed browser behavior. |
| Fcitx5 | `npm run fcitx5:check`; `npm run fcitx5:integration` for transport or broker changes. Native UI claims additionally need real application trials. |
| Obsidian/Bash/shared clients | `npm run editors:check` and `npm run editors:integration`; real editor checks for acceptance, focus, rendering, or undo changes. |
| Desktop controller/installers | `npm run desktop:check`; include Omarchy/editor checks when those boundaries change. |
| Omarchy UI | `npm run omarchy:check`; host/lifecycle checks below for QML or runtime changes. |
| Shared schemas, dependencies, or broad changes | Relevant Rust checks plus `npm run check` and affected integrations. |

Rust verification:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo +1.85.0 check --workspace --all-targets --all-features --locked
```

`npm run check` aggregates type, test, reproducible build, live-runner validation,
documentation, editor, desktop, Omarchy, and historical evidence gates. It does
not run every headed test or prove native application behavior. Avoid rerunning
checks already covered by an aggregate unless new changes or failures justify it.

Additional integration lanes; read their runbooks before use:

```sh
npm run fcitx5:integration
npm run editors:integration
npm run live:smoke --workspace @badi/chromium
npm run live:product --workspace @badi/chromium
npm run live:web --workspace @badi/chromium -- --local
BADI_OMARCHY_REQUIRE_HOST_CHECKS=1 bash ui/omarchy-plugin/tests/check-source.sh
bash ui/omarchy-plugin/tests/run-client-lifecycle.sh
BADI_SUMMON_CYCLES=100 bash ui/omarchy-plugin/tests/run-isolated.sh healthy
```

`fcitx5:integration` requires CMake, Ninja, Fcitx5Core, nlohmann-json, a C++20
compiler, Python 3, and Cargo. It exercises the C++ transport against a disposable
real broker. Browser live runners use isolated profiles and ignored diagnostics
under `output/playwright/`; consult the runner for local-model prerequisites.
Real desktop tests require the actual graphical session and disposable documents.
Report missing prerequisites and the precise unverified boundary.

## Desktop changes and delivery

- Source checks run without installation. Inspect current installed state before
  assuming this workstation matches the checkout.
- `scripts/badi-desktop.py` mutations must bind to the exact session selected by
  the overview. Never substitute a guessed socket or a different running broker.
- `badi pause` persists across restarts; `badictl pause` is runtime control.
  Preserve this distinction in the UI, CLI, tests, and documentation.
- `scripts/install-desktop.py` installs the broker, addon, commands, and launcher,
  and restarts user services. `scripts/install-editors.py` updates selected editor
  integrations. Use them when the task authorizes installation; inspect backups,
  targets, and service/editor recovery before changing the running session.
- `scripts/install-omarchy-ui.py` updates the installed panel only after explicit
  unlocked state. Preserve the lock gate and existing user customizations.
  Validate source first and verify health after an authorized install/reload.
- Implementation and source verification alone do not authorize desktop
  installation, release, publication, or Git history changes. When the user has
  requested the relevant action, carry it through without redundant confirmation.

## Evidence and completion

- Historical capability receipts are immutable. Ordinary `npm run check`
  validates their historical contracts; it does not requalify changed code.
- V3 qualification requires a clean implementation commit and append-only
  evidence afterward. Commit only when authorized by the task. Do not claim
  qualification if its clean-commit prerequisite is unmet.
- Use an actual ancestor for `CAPABILITY_BASE_SHA`. Never relax schema, identity,
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
