# Contributing to Badi

Badi is a local co-writing and spelling assistant for Linux. Contributions that
preserve exact editing authority, local-only privacy and focused component
contracts are welcome, under the [Code of Conduct](CODE_OF_CONDUCT.md) and the
repository's [MIT License](LICENSE).

## Where to start

1. Read [VISION.md](VISION.md) for the product contract and non-goals, and
   [future plans.md](<future plans.md>) for the single backlog and open
   decisions.
2. Read the runbook of the component you plan to touch (indexed in
   [AGENTS.md#architecture](AGENTS.md#architecture)) and the
   [ADRs](docs/decisions/) it links.
3. Use the [issue templates](.github/ISSUE_TEMPLATE/) for reproducible bug
   reports and concrete feature requests,
   [GitHub Discussions](https://github.com/ahuray/badi/discussions) for usage
   questions and open-ended design conversations, and
   [SECURITY.md](SECURITY.md) for private security or text-privacy reports.

Keep each fact in one home ([AGENTS.md#start-here](AGENTS.md#start-here)):
[README.md](README.md) owns what users can do, component `README.md` files own
their contracts and tested-with matrices, [future plans.md](<future plans.md>)
is the only backlog, and `docs/` holds only `docs/architecture/`,
`docs/decisions/` and cited images in `docs/evidence/`. Link instead of
repeating.

## Setup

From a fresh checkout of the repository root:

```sh
npm ci
cargo test --workspace --all-features --locked
```

- **Node.js and npm:** [package.json](package.json) requires Node.js `^22.20.0`,
  `^24.12.0` or `>=25.0.0`. `package-lock.json` at the root is the only
  JavaScript manifest; do not add adapter `package.json` files or mix package
  managers.
- **Rust:** Cargo installs the pinned `1.99.0` toolchain (`clippy` and
  `rustfmt`) from [rust-toolchain.toml](rust-toolchain.toml) on first use.
  [Cargo.toml](Cargo.toml) declares a `1.85` minimum supported Rust version;
  install `1.85.0` with `rustup toolchain install 1.85.0` if you run the MSRV
  check locally.
- **Surface-specific tools:** native and adapter checks need only that
  surface's dependencies — CMake, Ninja, a C++20 compiler and Fcitx 5
  development headers for `adapters/fcitx5/`; Bash builtins and `libreadline`
  headers for `adapters/shell/`; Python 3 with `PyGObject` for
  `adapters/accessibility/` and `adapters/session/`; LibreOffice with `unopkg`
  for `adapters/libreoffice/`; and `quickshell` with `qt6-declarative` for the
  Omarchy panel host checks ([.github/workflows/ci.yml](.github/workflows/ci.yml)
  shows the exact packages per job).

Source checks never touch your home configuration. Only run
`scripts/install-*.py` when deliberately testing an installed build on your own
desktop ([AGENTS.md#live-trials](AGENTS.md#live-trials)).

## Editing and privacy invariants

Adapters own document acquisition and mutation while the broker keeps policy
and one-shot commit authority central, bound to exact field identity, revision,
fingerprint, caret, focus and expiry; sensitive or password fields, foreign IME
composition, unknown authority and stale context always fail closed rather than
guessing. Native undo is preserved — and where append-only
[IME-parity](docs/decisions/0003-ime-parity-append-only.md) cannot guarantee
single-step undo, that limit is stated plainly — raw keylogging, clipboard
replacement, blind synthetic typing and remote inference are never product
integrations, and typed prose and credentials never enter logs, statistics or
diagnostics outside an explicit, expiring `badi debug on --text` trace. Read
the complete invariants in
[AGENTS.md#editing-and-privacy-invariants](AGENTS.md#editing-and-privacy-invariants)
and [VISION.md](VISION.md).

## Checks to run

Pick the local checks for the surface you changed from
[AGENTS.md#verification](AGENTS.md#verification). External contributors cannot
run the maintainer's local `grok-review` tool; GitHub Actions CI
([.github/workflows/ci.yml](.github/workflows/ci.yml)) is the gate for external
pull requests, and running the matching local checks (including `npm run check`
and the Rust gate when touching Rust, schemas or shared contracts) before
pushing is strongly recommended.

Two repository checks catch common mistakes early
([AGENTS.md#rules-and-what-enforces-them](AGENTS.md#rules-and-what-enforces-them)):

- **Behavior commits update [CHANGELOG.md](CHANGELOG.md) in the same commit.**
  Any commit touching product code in `broker/`, `adapters/`, `protocol/`,
  `ui/`, `packaging/` or the installers must add a line under `## Unreleased`
  in that commit (`npm run agent-rules:check` fails if a later commit adds the
  changelog line separately).
- **Keep generated and formal artifacts in sync.** Run
  `python3 scripts/third-party-notices.py` when `Cargo.lock` changes a shipped
  crate version, `npm run docs:check` when editing Markdown or adding files
  ([AGENTS.md#keep-the-workspace-lean](AGENTS.md#keep-the-workspace-lean)), and
  `npm run formal:check` if you touch code covered by a TLA+ model under
  `.cursor/skills/verify-badi-formal-*`.

## How a pull request is judged

Keep each pull request to one coherent change carried through code,
deterministic regression tests and the affected runbook (plus
[README.md](README.md) when user-visible keys, commands or app coverage
change). Never weaken a test, schema, check or safety gate to pass.

Every pull request is judged against
[.github/pull_request_template.md](.github/pull_request_template.md):

- **Proof artifact:** include at least one concrete proof in the PR description
  — a command with its exit code, test output, or (required for desktop
  input-flow changes) a Wayland and Fcitx screenshot or recording captured with
  disposable text in an isolated profile or scratch document.
- **Unverified boundaries:** list under `Not verified:` any app, desktop or
  hardware path you could not exercise yourself, so reviewers know what still
  needs a live trial before or after merge.
