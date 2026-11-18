# Security Policy

Badi processes text you are actively typing on your Linux desktop. Editing
authority, exact field binding and keeping typed prose off logs and networks
are core security boundaries ([VISION.md](VISION.md),
[AGENTS.md#editing-and-privacy-invariants](AGENTS.md#editing-and-privacy-invariants)).

## Supported versions

Badi is pre-release software (`0.2.0` in [Cargo.toml](Cargo.toml)). Security
fixes land on `main`; older commits and retired protocol v1 builds
([ADR 0007](docs/decisions/0007-retire-protocol-v1.md)) are not supported.

| Version | Supported |
| --- | --- |
| `main` (`0.2.x` pre-release) | Yes |
| Earlier commits and `< 0.2.0` | No |

## Reporting a vulnerability

Report security and text-privacy vulnerabilities privately through
**[GitHub Private Vulnerability Reporting](https://github.com/ahuray/badi/security/advisories/new)**
(**Security → Report a vulnerability** on this repository). Do **not** open a
public issue, discussion or pull request for an unpatched vulnerability.

Include in your report:

- the affected component and commit (`badi doctor` names the installed commit;
  delete any `Snoozed:` line, private site origins, app ids or personal paths),
- your Linux distribution, Wayland compositor / desktop and target app version
  when relevant,
- minimal reproduction steps using **disposable text only** (never include real
  prose, credentials or `badi debug on --text` output), and
- which boundary below was crossed and its impact.

Code of Conduct reports submitted through the same private draft form (with
`[Conduct]` in the title) are handled confidentially under
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) and are never published as advisories.

## Response expectations

Badi is maintained on a best-effort basis by an open-source maintainer:

- **Acknowledgement:** within **7 days** of receiving a private report.
- **Triage and plan:** within **14 days**, confirming whether the behavior
  crosses a security or privacy boundary and outlining remediation.
- **Coordinated disclosure:** once a fix and regression test land on `main`, a
  GitHub Security Advisory will be published crediting the reporter (unless you
  prefer to remain anonymous). Please allow up to **90 days** before public
  disclosure while a fix is developed and verified.

## Scope

### In scope

- **Editing authority:** inserting, replacing or revising text without an
  active policy grant and explicit one-shot user acceptance; accepting stale
  suggestions after pause, snooze, sleep, session switch, screen-share hold
  ([ADR 0005](docs/decisions/0005-desktop-signals-withhold-authority.md)) or an
  authority-epoch change; or submitting commands / executing code (for example
  in Bash, Neovim or Obsidian).
- **Field binding and fail-closed gates:** suggestions, commits or typo
  deletions crossing field, note, buffer, window or browser-origin boundaries;
  bypassing revision, fingerprint, caret, focus or expiry checks; reading or
  editing password / sensitive fields; or failing to yield to foreign IME
  preedit and candidates.
- **Privacy of typed text and learned state:** acquiring context from an
  unauthorized app, site or field; writing typed prose, credentials, window
  text or media titles to logs, `stats.json` or normal diagnostics outside an
  explicit, expiring `badi debug on --text` memory trace
  ([ADR 0008](docs/decisions/0008-debug-trace-may-hold-text-on-request.md));
  unencrypted storage, retention bypass or terminal/shell leakage of opt-in
  context words ([ADR 0010](docs/decisions/0010-context-finishes-words.md)) or
  style samples; unexpected network access by Badi or its model runtime; or
  escaping the broker and runtime systemd sandbox
  ([ADR 0001](docs/decisions/0001-same-uid-trust-boundary.md)) via untrusted
  field or page text.
- **Local sockets and IPC:** weak filesystem modes, symlink or inode confusion,
  cross-UID access, framing / memory exhaustion or schema-validation bypasses
  on `$XDG_RUNTIME_DIR/badi/broker.sock` (`0600`),
  `$XDG_RUNTIME_DIR/badi/accessibility.sock` (`0600`), the editor stdio
  bridges, or the desktop focus helpers.
- **Installers and pinned artifacts:** bypassing SHA-256 verification of model
  weights or the pinned `llama.cpp` runtime, downloading license-gated models
  without explicit acceptance, unsafe path handling or file permissions in
  `scripts/install-*.py` and `scripts/badi_install.py`, modifying desktop UI
  state while the session lock is active, or corrupting backup restore state.

### Out of scope (documented design boundaries)

- **Malicious same-UID processes
  ([ADR 0001](docs/decisions/0001-same-uid-trust-boundary.md)):** processes
  already running as the logged-in Linux user are inside the local trust
  boundary. Attacks that require code execution as that same user are out of
  scope unless they cross UIDs, elevate privileges, or escape the broker's
  systemd confinement from untrusted text.
- **Documented IME-parity limits
  ([ADR 0003](docs/decisions/0003-ime-parity-append-only.md),
  [ADR 0011](docs/decisions/0011-checked-corrections-in-append-only-apps.md)):**
  in append-only apps without an editor API, page script moving focus at the
  exact moment of acceptance can redirect an append-only commit like a typed
  keystroke or split a checked typo deletion, as documented.
- **Ordinary model wording quality** that stays within authorized, explicitly
  accepted suggestions and crosses no authority, binding or privacy boundary.
