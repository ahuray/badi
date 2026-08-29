# Two-day delivery plan: Omarchy/Hyprland vertical slice

Status: execution contract for a 48-hour build beginning after user approval.
Times are wall-clock hours from kickoff (`H0`), not estimates of cumulative
person-hours. The plan is grounded in [VISION.md](../../VISION.md), the
[competitive landscape](../research/competitive-landscape.md), and a read-only
inspection of the older sibling `cotype` prototype.

## Outcome first

By `H12`, show one real, instrumented end-to-end loop in Chromium: app-owned
context, a local suggestion, visible ghost UI, accept one word, accept the
remainder, dismissal, stale-result cancellation, and pause.

By `H30`, attempt that loop in all three promised targets on this machine:
Chromium, Obsidian, and a natural-language Codex or Claude-style TUI prompt
inside Ghostty. The terminal route is a manual-only Fcitx5 C++ module/addon; it
is never armed automatically and never replaces the user's selected input
method.

By `H48`, deliver the code, reproducible fixtures, measured results, and an
honest capability matrix. A missing target is reported as a failed target, not
renamed into support for an easier application. In particular, a Bash/Readline
fixture is not evidence that an agent TUI works.

The order of priorities is:

1. Never read or insert in a forbidden field, and never insert a stale result.
2. Complete the interaction reliably in one target in time for the first live
   session.
3. Reach all three targets through one frozen broker protocol.
4. Use a real local generative provider when it passes the latency and quality
   gate; otherwise label and use the deterministic phrase provider honestly.
5. Improve visual polish only after the safety and compatibility gates pass.

## Baseline to recheck at H0

The local audit on 2026-08-30 found the following. These are observations, not
portable assumptions, so `scripts/audit-host` must capture them again at H0.

| Item | Observed state | Planning consequence |
|---|---|---|
| OS/compositor | Arch Linux, Hyprland 0.56.2, native Wayland | This is the only supported compositor for the slice. |
| Targets | Chromium, Obsidian 1.13.7, Ghostty 1.3.1 | Use isolated demo data, not the user's normal browser profile or vault. |
| Fcitx5 | 5.1.21 running; core headers and `pkg-config` metadata present | A user-local C++ module/addon spike is possible without vendoring Fcitx. |
| Fcitx build tools | GCC 16.2.1, Clang 22.1.8, and Ninja present; CMake absent | CMake is an explicit H0 prerequisite for the primary terminal spike. Ask before installing it with `omarchy pkg add cmake`; declining makes that route a no-go. |
| Rust | `cargo` present; rustc 1.98.0 | Use a small Rust broker with an explicit lockfile. |
| JavaScript | Node 26.8.1 and npm 11.19.0 available; pnpm absent | Use one npm workspace for the two TypeScript adapters. Do not add pnpm during the slice. |
| Local model runtime | neither `llama-server` nor Ollama found on `PATH` | Runtime and model download require explicit user approval and an early spike. |
| Auxiliary UI/input tools | GTK4, AT-SPI, gtk4-layer-shell, and `wtype` present | They may support diagnostics or status UI, but none is an accepted terminal context or insertion fallback. |
| Git | `main` began at seed commit `eb946a6`; the research package is prepared in this turn | Require the pushed research checkpoint and a clean status before creating worker branches. |

The old `cotype` prototype usefully demonstrates a Unix-socket control plane,
generation snapshots, first-word splitting, a no-focus layer-shell pill, and a
fast deterministic provider. It also demonstrates why its production default
must not be copied: it reads every physical keyboard through `evdev`, assumes a
US key map, infers safety from window-title denylist text, positions from the
mouse rather than the caret, and injects with `wtype` without an app-owned
revision check. Omatype may reuse the lessons, not that global raw-key design.

## Exact demo contract

### Fixed environment and setup

- Run on this physical Omarchy/Hyprland machine, not only in unit tests.
- Run one broker process as the logged-in user. Its Unix socket is
  `$XDG_RUNTIME_DIR/omatype/broker.sock`; create the parent directory with mode
  `0700` and the socket with mode `0600`.
- Use an isolated Chromium profile with the unpacked extension, an isolated
  Obsidian scratch vault, and a disposable test conversation in a user-selected
  agent TUI inside Ghostty.
- Start the extension without broad host access; grant only the controlled
  fixture origin at runtime through optional host permissions.
- Default provider access is local only. `llama.cpp` is a suitable first spike
  because it runs GGUF models locally and exposes a local completion server;
  those capabilities are documented by the
  [llama.cpp project](https://github.com/ggml-org/llama.cpp#llama-server).
- Cap context at 512 Unicode scalar values before the caret and 128 after it.
  Do not collect clipboard content, screen pixels, other fields, or full files.
- Cap a displayed suggestion at 64 characters, eight word-parts, and one line.
  Strip all control characters; a provider response containing CR, LF, NUL, or
  an invalid UTF-8 sequence is suppressed.

### Stable user actions

| Action | Demo binding | Required behavior |
|---|---|---|
| Manual invoke/arm | `Alt+\` | Required in Ghostty; optional one-shot request in the other adapters. |
| Accept next word-part | `Alt+]` | Insert only the first broker-provided part; never insert Enter. |
| Accept remainder | `Alt+Shift+]` | Insert the cached remaining parts only if the revision still matches. |
| Dismiss | `Escape` | Consume Escape only while a suggestion is visible; otherwise pass it through. |
| Global pause/resume | `omatypectl pause` / `omatypectl resume` | Hide all UI and stop requests while paused. A Hyprland shortcut is optional and user-approved. |

Bindings are configuration, not hidden constants. At H0 the user either
accepts these or chooses alternatives before the protocol and adapter keymaps
freeze. Tab remains untouched.

### Per-target contract

| Target | Context and identity | Policy in the demo | Rendering and insertion | Passing interaction |
|---|---|---|---|---|
| Chromium | A Manifest V3 content script reads only the focused fixture `<textarea>` after a local field-purpose gate. The service worker conveys a verified extension origin and domain through a native-messaging host. | `Always` only for the loopback fixture origin; `Never` for `<input type=password>`; unknown domains are `Manual`. | An app-owned DOM ghost is anchored to the textarea caret. Insert with `setRangeText`, dispatch an input event, and re-read selection/value before every acceptance. A field-corner status pill may explain a placement failure, but does not pass the anchored-ghost gate. | Type, wait, see one suggestion, accept one part, accept the rest, type through a stale response, dismiss, pause, and prove password silence. |
| Obsidian | A desktop plugin reads the active CodeMirror editor state in the scratch vault; no vault-wide scan. Obsidian permits Node APIs only on desktop, which supports a direct user-owned socket connection ([Obsidian plugin requirements](https://docs.obsidian.md/community-directory/submission-requirements-for-plugins#nodejs-and-electron-apis-are-only-allowed-on-desktop)). | `Always` in the scratch vault; other vaults remain `Manual` until approved. | A CodeMirror 6 widget decoration follows the caret and insertion is one editor transaction. Widget decorations are the supported way to insert visual DOM at a document position ([CodeMirror decoration guide](https://codemirror.net/examples/decoration/)). A status-bar notice may explain a widget failure, but does not pass the ghost-rendering gate. | The same word/rest/dismiss/stale/pause loop passes without changing another note or breaking undo. |
| Ghostty + agent TUI | An Fcitx5 C++ module/addon remains disarmed and pass-through inside the existing Fcitx process. Only `Alt+\` arms a one-shot session. It uses valid Fcitx surrounding text when available, otherwise retains only printable keys forwarded after that explicit arm. Focus-out, dismissal, acceptance, or 15 seconds of inactivity erases the buffer and disarms. | Always `Manual`; password/sensitive capability flags are `Never`; no learning. It never changes the selected input method and never arms itself. | Fcitx input panel/candidate UI and `InputContext::commitString`, with control characters rejected before dispatch. Fcitx exposes input contexts, surrounding text, input panels, and commit APIs in its public addon/input-method interfaces; the official [input-method tutorial](https://fcitx-im.org/wiki/Develop_an_simple_input_method/en) is the implementation starting point. | In a real Codex or Claude-style TUI prompt, arm before composing, type prose, receive a candidate, accept one/rest, and show that ordinary unarmed typing produces zero context or provider calls. |

Chrome native messaging is the primary browser transport, not a web page's
direct loopback request. Chrome launches a registered host and uses
length-prefixed JSON on stdin/stdout; Linux host paths must be absolute and the
manifest pins allowed extension origins
([Chrome native-messaging documentation](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)).
The host still validates every message because Chrome explicitly warns that
content-script messages should be treated as untrusted
([Chrome messaging security guidance](https://developer.chrome.com/docs/extensions/develop/concepts/messaging#security-considerations)).

The Fcitx route is deliberately a spike rather than an assumption. Wayland
input paths vary among GTK, Qt, Chromium/Electron, compositor protocols, and
XWayland, and Wayland popup placement is constrained; the
[Fcitx Wayland guide](https://fcitx-im.org/wiki/Using_Fcitx_5_on_Wayland/en)
documents those differences. An IME is therefore not the browser or Obsidian
default in this slice. Likewise, current Wayland preedit cannot reliably give
typed and suggested text distinct styling in the IBus route
([IBus Typing Booster documentation](https://mike-fabian.github.io/ibus-typing-booster/docs/user/#use-inline-completion)),
which is why app-owned rendering remains the quality path.

### Demo sequence

1. Start the local provider and broker; `omatypectl status --json` must name the
   provider, model or phrase engine, socket permissions, pause state, and
   `remote_network=false`.
2. Open the controlled Chromium fixture. Type the fixture seed, accept one
   part, then the remainder. Trigger a delayed response, type another character,
   and show that the delayed generation never appears or inserts.
3. Focus the fixture password input and type 20 characters. Provider-call count
   and serialized-context count must not change. The adapter may show a local
   `Never: password` receipt that contains no field value.
4. Pause globally, type in the allowed textarea for 10 seconds, and prove zero
   new requests; resume and dismiss one visible suggestion with Escape.
5. Repeat the full loop in a scratch Obsidian note and undo each acceptance in
   one normal editor undo.
6. In Ghostty, enter the chosen agent TUI. Show that unarmed typing is silent,
   explicitly arm, compose the fixture prompt, accept a word-part and the
   remainder, and verify acceptance never submits the prompt.
7. Show the generated compatibility matrix and latency report beside the live
   behavior. Do not substitute a recording for a failing physical-app test.

### Quantitative delivery gates

- Zero forbidden-field context payloads and zero provider calls across 20
  password-field edits.
- Zero stale displays and zero stale insertions across 100 induced races per
  adapter: rapid typing, caret move, focus change, dismissal, and pause.
- Ten exact word-part and ten exact remainder insertions in every passing
  target, including leading-space and punctuation fixtures.
- Across the 20 Ghostty trials, every Fcitx candidate must stay on the same
  monitor/workspace, place its nearest edge within 96 logical pixels of the
  advertised cursor rectangle, avoid focus theft or line coverage, and clear
  on focus loss.
- Deterministic visible-result latency after debounce: p95 at most 50 ms.
- Warm local-model edit-to-visible latency over 30 balanced requests: p50 at
  most 250 ms and p95 at most 500 ms. Any result older than 600 ms is
  suppressed. Report the actual distribution even when it fails; use the
  deterministic provider if this gate fails.
- The selected generative provider becomes the demo default only if the user
  marks the next word useful on at least 8 of 12 blind fixture prompts and the
  latency gate passes. Otherwise use `phrase-v1`, label it as deterministic,
  and report generative completion as not ready.
- Broker and adapters open no `/dev/input/event*` file and never invoke
  `wtype`, a virtual keyboard, clipboard insertion, or synthetic typing.
- No raw context, suggestion text, or accepted text appears in normal logs.
  Evidence records IDs, lengths, reason codes, timings, and aggregate counts.

### Non-goals for these 48 hours

- Literal “works everywhere,” other compositors, X11, Flatpak packaging, mobile
  Obsidian, Firefox, arbitrary Chromium sites, contenteditable parity, or
  multilingual IME coexistence claims.
- Ambient completion in terminals, ordinary shell-command completion,
  automatic Codex/Claude prompt detection, or selecting Omatype as the login
  default input method.
- Treating a Bash/Readline hook, terminal echo fixture, or GTK demo window as
  proof of Ghostty agent-TUI support. A Readline hook may remain as a protocol
  and insertion fixture only; GNU Bash exposes `READLINE_LINE` and
  `READLINE_POINT` to `bind -x` handlers
  ([Bash manual](https://www.gnu.org/software/bash/manual/html_node/Bash-Variables.html)),
  but that does not reach a TUI's private input buffer.
- Global raw-device capture, global clipboard capture, screenshots/OCR,
  window-title-only safety policy, or unconditional virtual-keyboard injection.
- Remote providers, accounts, sync, personalization, history, telemetry,
  rewriting, mid-line generation, or model fine-tuning.
- A polished settings UI, installer, package, auto-updater, public launch, or
  final public name. `omatype` remains a repository codename.

## Architecture and the contract freeze

```text
Chromium content script -> service worker -> native host --\
Obsidian CodeMirror plugin ------------------------------+--> Rust broker --> policy
Fcitx5 manual C++ module/addon --------------------------/        |             |
                                                                |             +--> metrics only
                                                                +--> local provider
```

Adapters own context acquisition, rendering, current-revision checks, and
insertion. The Rust broker owns policy, provider lifecycle, cancellation,
word-part segmentation, pause state, receipts, and content-free metrics. The
provider never talks directly to an adapter.

### Transport and trust boundaries

- Primary transport is a four-byte little-endian length followed by UTF-8 JSON
  over `$XDG_RUNTIME_DIR/omatype/broker.sock`, with a 64 KiB frame limit in
  both directions. The broker rejects a peer whose `SO_PEERCRED` UID differs
  from its own.
- The Chromium native host uses the browser's native byte-order length framing
  externally and relays the same versioned JSON body to the Unix socket. It
  validates the registered extension origin and contains no policy or
  prediction logic. It never listens on TCP or WebSocket.
- The broker derives adapter provenance from the connection handshake and the
  registered native-host origin. It does not trust an `app_id` string supplied
  by a content script.
- The owned local model sidecar binds a random `127.0.0.1` port. Non-loopback
  provider hosts are rejected in the slice, and the receipt describes this as
  `loopback`, not “no network.”

### Protocol v1 minimum

`protocol/v1/schema.json` and its golden examples are normative. Human prose,
Rust types, TypeScript types, and C++ structs are subordinate. Freeze the base
envelope from the [architecture recommendation](../research/linux-architecture.md)
by H2:

```json
{
  "v": 1,
  "id": "optional-request-id",
  "type": "context.changed",
  "session_id": "128-bit-random-id",
  "focus_epoch": 12,
  "revision": 17,
  "mono_ms": 81234567,
  "payload": {}
}
```

Each connection starts with `hello` and negotiates one version plus declared
capabilities. Required message families are `session.open/close`,
`context.changed`, `suggest.request/cancel`, `suggestion.show/clear`,
`control.request`, `commit.prepare/result`, and content-free `health/status`.
Every suggestion and commit message carries the same `session_id`,
`focus_epoch`, `revision`, and context fingerprint as the current target.

Required invariants:

- The broker owns Unicode accept-word segmentation; adapters never tokenize
  independently.
- A session has at most one live generation. A newer revision, focus-out,
  pause, dismissal, timeout, or policy change cancels the prior generation.
- An adapter renders or accepts only when the live field's identity, revision,
  caret/selection, and context tail still match its cached request.
- Sensitive or `Never` targets emit zero text bytes and never reach a provider.
  Unknown identity plus ambient mode resolves to `manual_required`.
- `commit.result` reports `applied`, `dispatched-unverified`, `stale`,
  `blocked`, or `failed`; it never includes inserted text. Only app-owned
  adapters may report `applied`; Fcitx reports `dispatched-unverified`.
- Unknown protocol versions, unknown enum values, oversized frames, missing
  capability fields, and invalid Unicode fail closed with a reason code.
- Protocol changes after H2 require an integrator-owned schema commit, updated
  positive and negative golden fixtures, and a conformance run in Rust,
  TypeScript, and C++ before any adapter may consume them.

### Policy matrix

| Condition | Decision | Adapter behavior | Broker behavior |
|---|---|---|---|
| Verified allowed Chromium fixture or approved Obsidian vault | `Always` | Debounced ambient requests permitted | Provider permitted while unpaused |
| Ghostty/Fcitx manual session or unknown app/domain with explicit request | `Manual` | Request only after the invoke action | One generation, then terminal session disarms |
| Password/sensitive capability, password DOM type, lock screen, paused state | `Never` | Do not serialize field content; hide UI | No provider call and no learning/log content |
| Identity, field purpose, or revision unavailable | `manual_required` or suppress | Show reason locally if useful | Never inherit `Always` |

## Repository and worktree layout

Proposed repository layout after implementation:

```text
omatype/
├── package.json
├── package-lock.json
├── protocol/
│   └── v1/
│       ├── schema.json
│       ├── protocol.md
│       └── examples/{valid,invalid,sequences}/
├── broker/
│   ├── Cargo.toml
│   ├── Cargo.lock
│   └── src/{main,ipc,policy,session,segment,metrics}.rs
├── providers/
│   ├── phrase-v1.json
│   └── prompts/completion-v1.txt
├── adapters/
│   ├── chromium/
│   │   ├── extension/{package.json,manifest,content,worker,ghost,field-policy}.ts
│   │   ├── native-host/{Cargo.toml,Cargo.lock,src/main.rs}
│   │   └── tests/
│   ├── obsidian/
│   │   ├── {manifest.json,package.json,tsconfig.json}
│   │   ├── src/{main,broker,ghost,policy}.ts
│   │   └── tests/
│   ├── fcitx5/
│   │   ├── {CMakeLists.txt,cmake/}
│   │   ├── src/{engine,broker_client,session,candidates}.{cc,h}
│   │   ├── data/{omatype-addon.conf.in,omatype.conf.in}
│   │   └── tests/
│   └── bash-fixture/omatype-readline.bash
├── fixtures/
│   ├── completion/prompts.jsonl
│   ├── traces/{stale,focus,pause,password}.jsonl
│   ├── web/{index.html,fixture.js}
│   ├── obsidian-vault/Demo.md
│   ├── terminal/{fake-provider.jsonl,readline-cases.json}
│   └── compatibility/targets.yaml
├── tests/{contract,e2e,safety}/
├── scripts/{audit-host,bootstrap-demo,run-demo,bench,collect-evidence,rollback-demo}
├── evidence/<run-id>/{environment,results,compatibility,checksums}.json
└── docs/{research,plan}/
```

The root `package.json` declares npm workspaces only for the Chromium extension
and Obsidian plugin; their single root `package-lock.json` is owned by the app
worker, who handles those adapters sequentially. Native-host and broker Cargo
lockfiles remain independent. Do not add pnpm or make another agent resolve the
npm lock. The protocol fixtures and `targets.yaml` are read-only to workers;
proposed changes go to the integration lead.

### Worktrees and branches

The research package must be committed and pushed on `main` before
implementation. After the user reviews it and authorizes implementation, the
integration lead verifies that exact checkpoint and creates the integration
branch:

```bash
cd /path/to/omatype
git status --short
git pull --ff-only
git switch -c integration/two-day-slice
git rev-parse HEAD
```

`git status --short` must print nothing before the branch is created. Record
the research checkpoint SHA in the H0 evidence.

Immediately after the H0 documentation checkpoint, create three sibling
worktrees from `integration/two-day-slice` so the risk probes can run in
parallel:

```bash
git worktree add ../omatype-wt-broker -b agent/broker integration/two-day-slice
git worktree add ../omatype-wt-apps -b agent/app-adapters integration/two-day-slice
git worktree add ../omatype-wt-terminal -b agent/fcitx-terminal integration/two-day-slice
```

H0-H2 worker changes are feasibility probes only and may not define production
wire types. At H2 the integration lead commits protocol v1; every worker
cherry-picks that exact SHA before product implementation. The main checkout
remains the integration worktree. Do not place worktrees inside the repository,
and do not remove them until their commits are integrated and the user agrees
they are no longer needed.

## Bounded agent roles and file ownership

At most four agents run concurrently: the integration lead plus three workers.

| Role | Owns | Must deliver | Must not edit |
|---|---|---|---|
| Integration/contract lead | `protocol/`, `fixtures/`, root `tests/`, `scripts/`, `evidence/`, `docs/` | Protocol freeze, fixture server, conformance runner, integration, physical-app evidence, user sessions | Worker modules except minimal reviewed integration fixes |
| Rust broker/provider worker | `broker/`, `providers/`, broker-local tests | UDS broker, policy, cancellation, segmentation, phrase provider, llama.cpp client, metrics | Protocol, adapters, root fixtures, user config |
| App-adapter worker | root `package.json`/`package-lock.json`, `adapters/chromium/`, then `adapters/obsidian/` | Chromium slice by H12; Obsidian plugin after browser checkpoint; one npm workspace build | Broker, terminal, protocol, shared fixtures |
| Terminal worker | `adapters/fcitx5/`, `adapters/bash-fixture/` | H0 terminal probe; manual-only Fcitx module/addon; fail-closed compatibility evidence | Broker policy, Chromium/Obsidian, protocol, existing Fcitx config without approval |

The app worker handles Chromium and Obsidian sequentially so their shared
TypeScript experience accelerates delivery without two agents changing common
tooling. The terminal worker begins with feasibility, not a broad implementation.
No agent invents a private wire contract; all three implementations must consume
the same golden frames before feature work.

## Risk-first spikes and route deadlines

Each spike ends with a tiny executable, captured output, and one decision. It
must not quietly grow into a second architecture.

| Deadline | Spike and proof | Go | Fallback / stop |
|---|---|---|---|
| H1 | Fcitx prerequisites: compile an official-style minimal C++ module/addon against installed 5.1.21 headers; install under a disposable user prefix; confirm safe reload/restore steps. | Addon loads without root, leaves the selected input method unchanged, and remains pass-through while disarmed. | If CMake installation is not approved or the addon cannot load by H3, mark the terminal target failed for this slice. |
| H3 | Ghostty agent-TUI path: leave the user's input method selected, arm the addon manually, observe only post-arm keys or valid surrounding text, draw an Fcitx candidate, and dispatch a suffix without newline through `commitString`. Test in the actual user-selected Codex/Claude-style TUI, not Bash. | Candidate position and commit behavior both pass; unarmed counters remain zero. | If context, candidate placement, coexistence, or safe commit fails, mark terminal unsupported. A status notification may explain why, but no alternate insertion mechanism counts. |
| H3 | Chromium native host: one fixture textarea request and response through content script -> worker -> registered host -> fake broker. | Verified origin and revision survive round trip. | If native messaging cannot be made reliable by H4, mark the browser transport failed; do not open a loopback listener as a shortcut. |
| H4 | Chromium ghost/insertion: leading-space suggestion at textarea caret, typing invalidates it, password value never crosses worker boundary. | App-owned renderer and exact insertion pass. | A field-corner status pill may expose the failure, but the browser target remains partial until anchored rendering passes. |
| H4 | Rust broker: schema validation, one live generation per surface, cancellation, pause, and deterministic provider pass golden sequences. | Freeze broker API. | Cut optional metrics/streaming; do not weaken lifecycle or policy. |
| H6 | Local model: benchmark at most two user-approved GGUF models of at most 1.5B parameters, with 12 completion prompts and 30 warm requests. | Quality and latency gates pass; pin model ID, quantization, license, checksum, and prompt. | Ship `phrase-v1` as the labeled live provider. No third-model bake-off inside the 48 hours. |
| H14 | Obsidian socket + CodeMirror widget spike in scratch vault. | Widget follows caret and one transaction inserts/undoes. | A status-bar notice may expose the failure, but the Obsidian target remains partial until widget rendering passes. |

Terminal installation and configuration are user-state changes. CMake is a
hard H0 prerequisite for the Fcitx spike: install it only after approval with
`omarchy pkg add cmake`; if approval is declined, record the terminal target
as no-go for this slice. Use a user-local prefix, never `/usr/share/omarchy`
or `/usr/share/fcitx5`; capture the current Fcitx addon and input-method
state; back up the exact user configuration before any approved change; and
restore it on rollback.

## Integration order

1. **Protocol, fake provider, and conformance runner.** Nothing else merges
   until Rust, TypeScript, and C++ accept every valid golden frame and reject
   every invalid one.
2. **Broker policy/lifecycle.** Merge pause, `Always/Manual/Never`, single-live-
   generation cancellation, segmentation, and content-free metrics before any
   real provider.
3. **Chromium vertical slice.** Merge native host/transport, then local password
   gate, then textarea renderer/insertion. This is the H12 live result.
4. **Local generative provider.** Merge only behind the provider trait after the
   deterministic provider proves the UI; it cannot block the first live loop.
5. **Terminal adapter.** The feasibility spike starts at H0, but merge only the
   Fcitx route if it passes manual arming, agent-TUI context,
   candidate-position, commit, coexistence, and unarmed-silence gates. If it
   fails, publish the unsupported evidence and stop that lane.
6. **Obsidian adapter.** Reuse protocol semantics, not Chromium DOM code; merge
   direct socket, editor revision, CodeMirror widget, and transaction insertion.
7. **Cross-target safety and evidence.** No new features after all passing
   targets enter the 100-race harness.

This order guarantees a useful Chromium slice if terminal integration fails,
and it guarantees a useful broker/fixture package if a second app adapter fails.
It does not let one target's shortcut around the contract become the next
target's foundation.

## Milestone schedule, H0-H48

| Time | Integration lead | Broker worker | App worker | Terminal worker | Exit condition |
|---|---|---|---|---|---|
| H0-H1 | User kickoff; re-audit host; approve bindings, model budget/license, CMake, temporary Fcitx change, and target TUI; checkpoint docs. | Read fixtures and prepare Rust crate locally. | Launch isolated Chromium and map textarea/password events. | Run Fcitx prerequisite audit and minimal-addon compile. | Decisions recorded; no hidden install/config mutation. |
| H1-H2 | Write schema, valid/invalid frames, stale sequence, and ownership map; commit protocol and send its SHA. | Review, then cherry-pick the protocol SHA; no incompatible API. | Prove native-host framing without defining product messages; then cherry-pick protocol SHA. | Prove addon load/pass-through and rollback without defining product messages; then cherry-pick protocol SHA. | Protocol v1 frozen and present identically in every worktree. |
| H2-H4 | Build conformance runner and fixture web server. | Implement UDS, validation, policy, session cancellation, phrase provider. | Implement field gate, request revision, ghost spike, exact insertion. | Test manual arm/candidate/commit in the real Ghostty agent TUI. | H4 route decisions made once. |
| H4-H6 | Integrate broker, run schema and safety gates. | Add segmentation, receipts, pause, metrics, fake delayed provider. | Complete Chromium actions and native-host installer/rollback. | Harden the passing Fcitx route or publish failed-target evidence and stop. | Broker + Chromium candidate ready; model and terminal go/no-go evidence. |
| H6-H10 | Run physical Chromium races; prepare live script. | Integrate and benchmark at most two approved local models; pin one or fall back. | Fix only Chromium P0 defects; add browser tests. | Finish passing terminal route and its test driver, or publish failed-target evidence. | Chromium has ten clean loops and safety gate green. |
| H10-H12 | **Live session 1** with user; record latency, visual comfort, bindings, and two defects maximum. | Observe metrics; fix only release-blocking broker defects. | Drive Chromium demo and apply bounded fixes. | Demonstrate terminal spike separately if already green; it cannot delay Chromium. | A live Chromium result exists by H12, with evidence and user verdict. |
| H12-H16 | Update matrix and integrate model decision. | Provider cancellation/timeout soak. | Start Obsidian direct-socket and CodeMirror revision/widget implementation. | Integrate terminal commits or stop target after H16 if neither route is safe. | Provider labeled correctly; terminal status final enough to schedule. |
| H16-H22 | Run contract checks after each cherry-pick. | Add only defects found by adapters; no features. | Complete Obsidian word/rest/dismiss/pause/undo path. | Add terminal race tests, uninstall/restore script, and capability receipt. | Two app-owned targets pass full loop; terminal either passing or explicitly failed. |
| H22-H26 | Physical Obsidian tests and first combined run. | Latency and memory benchmark. | Fix Obsidian P0 defects and freeze adapter features. | Physical Ghostty agent-TUI tests; optional Bash fixture only for protocol coverage. | Per-target 10-loop and 30-latency samples captured. |
| H26-H30 | Integrate all green paths and produce matrix draft. | Support integration only. | Cross-adapter whitespace and stale fixtures. | Support integration only. | Three-target demo ready, or failed rows contain evidence and no false claim. |
| H30-H36 | Run 100-race harness per target, password tests, log-content scan, socket-permission check, and open-FD check. | Fix safety/revision defects only. | Fix safety/revision defects only. | Fix safety/revision defects only; any unsafe dispatch fails the terminal target. | Safety gate fully green for every target still marked passing. |
| H36-H38 | **Live session 2**: user tries 12 blind prompts across their chosen writing style and agent TUI. | Capture provider and quiet-score metrics. | Observe without steering; record acceptance/dismissal. | Verify manual semantics and no accidental submit. | User approves/rejects model, timing, visuals, policies, bindings, terminal capability. |
| H38-H42 | Apply only session blockers; rerun affected and contract suites. | Release-blocker fixes. | Release-blocker fixes. | Release-blocker fixes. | No unresolved P0; every P1 is documented. |
| H42-H46 | Feature freeze; fresh-profile/vault install, rollback rehearsal, full demo, compatibility JSON, checksums. | Clean build from lockfile. | Clean adapter builds. | Clean user-local install and exact profile restore. | Reproduction succeeds from instructions and rollback leaves prior user state. |
| H46-H48 | Final live run, optional capture, artifact index, branch review, delivery note. No push without user request. | Stand by for blockers only. | Stand by for blockers only. | Stand by for blockers only. | Delivery manifest complete; scope claims match evidence. |

If the calendar's “tomorrow” session occurs before H12, move live session 1 to
the last completed green Chromium milestone and show the deterministic provider.
Do not rush an unreviewed model or unproven terminal route into that session.

## Fixtures, benchmarks, and evidence gates

### Canonical fixtures

- `protocol/v1/examples/valid/`: one request, show response, suppress response,
  acceptance event, pause broadcast, and error for each adapter capability set.
- `protocol/v1/examples/invalid/`: wrong version, missing revision, mismatched
  parts, oversized frame/context, unknown policy, invalid field purpose, control
  characters, and context included in a `Never` request.
- `protocol/v1/examples/sequences/`: old response after new edit, focus-out before
  response, caret move before acceptance, pause during generation, dismissal
  during generation, provider timeout, and reconnect with an old generation.
- `fixtures/web/index.html`: one textarea, one password input, an explicitly
  unsupported contenteditable, mid-line/caret controls, focus-switch button,
  and an event ledger that never prints password content.
- `fixtures/obsidian-vault/Demo.md`: fixed prose seeds, long note, Unicode,
  punctuation, undo checks, and two open panes for focus races.
- `fixtures/completion/prompts.jsonl`: 12 blind prompts split among email, notes,
  and agent instructions. Store constraints and human ratings separately from
  exact model wording.
- `fixtures/traces/`: replayable event streams with fake provider delays of 0,
  100, 700, 1,300, and 3,000 ms.
- `fixtures/terminal/`: a PTY/TUI fixture for automated Fcitx mechanics plus a
  Bash/Readline harness for protocol/whitespace tests. Neither replaces the
  physical Ghostty agent-TUI gate.
- `fixtures/compatibility/targets.yaml`: target version, launch mode, transport,
  context source, field-purpose certainty, render mode, insertion mode, policy,
  test counts, latency, and pass/partial/fail reason.

### Gates

| Gate | Automated evidence | Physical evidence | Failure effect |
|---|---|---|---|
| G0 Host/reproducibility | Audit JSON, dependency versions, lockfile clean builds | Apps launch in isolated state | Stop affected route before coding past its spike. |
| G1 Protocol | Rust/TS/C++ conformance on all golden frames | One message trace per target with content redacted | No adapter merge. |
| G2 Policy/privacy | Password and unknown-identity counters; log canary scan; socket `0600`; non-loopback rejection | Chromium password field and Fcitx password/sensitive capability if available | Stop delivery; no ambient demo. |
| G3 Lifecycle | 100 delayed races per adapter with zero stale show/insert | Focus and caret races in each real app | Stop target until zero; no waiver. |
| G4 Insertion | Exact Unicode/space/punctuation outputs; no CR/LF/control | 10 word + 10 rest actions and undo behavior per app | Mark target partial or failed; renderer-only status does not pass. |
| G5 Provider | 30 warm latencies, timeout/cancel, output constraints, model checksum/license | User rates 12 blind next words | Fall back to labeled phrase provider. |
| G6 Terminal manual safety | Unarmed request/context/provider counters all zero; 15-second wipe; focus-out wipe | Actual agent TUI, no accidental submit, prior Fcitx setup restored | Terminal target fails; Bash cannot rescue the claim. |
| G7 Delivery | Fresh isolated setup, rollback, artifact checksums, matrix schema | Final scripted demo | Do not call the slice delivered. |

Latency instrumentation uses the adapter's edit/arm timestamp and its
post-render timestamp, not merely broker service time. Report median, p95,
maximum, timeout count, cancellation count, and provider separately. Store only
fixture text in debug traces; user-provided live text is represented by lengths
and IDs.

The evidence directory for each run contains machine-readable JSON and a short
README/index. If the user approves a visual record, use the supported Omarchy
capture flow (`omarchy screenrecord --fullscreen`, then
`omarchy screenrecord --stop-recording`) and record only disposable fixtures.
No recording is required for correctness, and no personal vault, browser
profile, terminal history, or notifications should appear.

## Branch, commit, and integration protocol

1. The integration lead alone changes protocol, canonical fixtures, plan,
   evidence schema, and shared scripts. Workers request a contract change by
   sending the failing golden case and proposed field semantics; they do not
   edit the schema on their branch.
2. Every worker commit is atomic, stays inside its ownership paths, and uses a
   scoped subject such as `feat(broker): cancel superseded generations` or
   `test(fcitx): prove unarmed passthrough is silent`.
3. Stage named paths, run `git diff --cached --check`, and include the exact test
   command and result in the handoff. Never use `git add .` in the integration
   repository.
4. The integration lead reviews one commit at a time and cherry-picks with `-x`
   in dependency order. A worker never merges `main`, rebases the integration
   branch, resolves another worker's conflicts, pushes, or force-pushes.
5. If a safety contract must change after H2, the integration lead commits the
   schema plus fixtures first and sends that SHA. Each worker cherry-picks that
   exact commit before implementing it. Cosmetic or convenience changes wait
   until after H48.
6. Run G1 and broker safety tests after every cherry-pick; run the affected
   adapter suite before the next integration commit. Tag local green points
   `slice-h12`, `slice-h30`, and `slice-h48-candidate`; create no public release
   or push unless the user asks.
7. Never commit downloaded models, browser profiles, vault state, sockets,
   Fcitx backups, captures, raw live text, or credentials. Evidence includes
   checksums and setup metadata, not model weights.

## Stop, go, and rollback rules

### Scope status

- **Go / full slice:** all G1-G7 gates pass for Chromium, Obsidian, and the
  manual Ghostty agent-TUI route. The provider may be generative or explicitly
  labeled deterministic.
- **Go / two-adapter prototype:** Chromium and Obsidian pass, while Ghostty is
  marked unsupported with its failed Fcitx evidence. This is useful progress,
  but it is not completion of the three-target slice.
- **Conditional delivery:** one app-owned target plus broker and safety fixtures
  pass by H48. Deliver as an interaction/architecture proof, not the VISION's
  three-app proof.
- **Stop ambient demo:** any password payload, non-loopback call, raw context in
  logs, stale insertion, focus-mismatched insertion, or accidental terminal
  submit occurs. Manual display-only inspection may continue while fixing it.
- **Stop terminal target:** the Fcitx context, candidate placement,
  coexistence, or commit route fails by H16, or any race dispatches once after
  revalidation should have blocked it. Record the failed capability and
  redirect time to evidence and the two safe adapters.
- **Stop provider expansion:** two approved models fail H6. Freeze the provider
  interface and ship the phrase engine; do not spend the adapter window on a
  third model.

### Route-specific rollback

- **Broker:** stop the process, remove only its socket/runtime directory, and
  leave fixtures and user data untouched. A stale socket is replaced only after
  verifying its path is exactly under `$XDG_RUNTIME_DIR/omatype/`.
- **Chromium:** close the isolated profile and delete that disposable profile
  only after validating its explicit path; remove the user-level native-host
  manifest created for the demo. Never edit or clean the normal profile.
- **Obsidian:** disable/remove the plugin only from the scratch vault. Never
  install into or scan another vault without live approval.
- **Fcitx:** record the selected input method and checksum the existing user
  configuration; make a timestamped backup; install only under
  `$HOME/.local`; and on rollback restore the exact configuration,
  remove only Omatype's addon/library files, restart/reload Fcitx, and verify
  the previously selected input method remains active. No recursive deletion
  and no edit under `/usr/share`.
- **Hyprland shortcut, if approved:** first inspect
  `omarchy menu keybindings --print`. If the chord exists, disclose it and use
  `hl.unbind(...)` before the new `o.bind(...)`; back up
  `~/.config/hypr/bindings.lua`; then validate with `hyprctl reload` followed by
  `hyprctl configerrors`. Restore the exact backup if validation is not clean.
  The demo remains operable through `omatypectl` if the user declines a global
  shortcut.
- **Model:** stop the local server; keep or delete a downloaded model only by
  explicit user choice. Report its path, size, license, and checksum first.

## User validation required during live sessions

### H0 decisions, before mutation

- Approve or replace the four demo bindings, especially any existing Fcitx or
  Hyprland collision.
- Choose the actual terminal agent TUI to test. Success in any other prompt does
  not satisfy the terminal contract.
- Approve or decline CMake installation and a temporary, backed-up Fcitx addon
  installation/reload.
- Approve a model download budget (up to two models, each at most 1.5B
  parameters), storage location, and each model license. Declining still permits
  the phrase-provider slice.
- Choose whether a Hyprland pause shortcut and screen recording are wanted.
- Confirm that all testing starts with fixtures, scratch vault, isolated browser
  profile, and disposable agent conversation rather than personal prose.

### Live session 1, by H12

The user, not the implementing agent, tries Chromium and answers:

- Is the ghost visible without pulling attention away from typing?
- Is the pause threshold too early, comfortable, or too late?
- Are `Alt+]`, `Alt+Shift+]`, Escape, and manual invoke conflict-free?
- Does one-word acceptance preserve authorship better than full acceptance?
- Does the capability receipt clearly say context source, policy, provider, and
  whether loopback networking occurred?

Only two user-ranked defects enter the H12-H16 fix window: any safety defect,
then the highest-friction interaction defect. Everything else becomes a
post-slice issue.

### Live session 2, H36-H38

The user runs 12 blind prompts and rates only the next word as `useful`,
`neutral`, or `distracting`; chooses the acceptable debounce/TTL; validates the
Obsidian widget or an explicitly failed rendering receipt; and personally
exercises manual arm, dismissal, word/rest acceptance, and non-submission in
the chosen Ghostty agent TUI. They also approve the final compatibility wording
for every partial or failed row.

The user must explicitly confirm that their selected Fcitx input method and
configuration, plus any Hyprland bindings, are restored after the final run.
That restoration is part of delivery, not cleanup deferred to later.

## H48 delivery manifest

Delivery is complete only when the integration branch contains or points to:

- protocol v1 schema, positive/negative/sequence fixtures, and passing Rust,
  TypeScript, and C++ conformance results;
- reproducible broker and adapter builds from pinned lockfiles/tool versions;
- the scripted Chromium and Obsidian setup/rollback flows and the user-approved
  Fcitx install/restore flow;
- per-target policy/capability receipts and compatibility rows with exact app,
  version, Wayland/native mode, context, render, insertion, and limitation;
- provider model ID/license/checksum or an explicit `phrase-v1` decision;
- raw benchmark JSON plus a concise latency/quality summary;
- G0-G7 results, zero-stale and password-field evidence, content-free log scan,
  and any failed-route evidence;
- the final scripted demo and user validation notes; and
- a short next-step list headed by the highest-risk failed capability, not a
  premature roadmap.

The slice is credible when claims are no broader than this evidence. Three
passing real targets establish the VISION's first proof; fewer targets still
produce useful architecture and research, but must be delivered under the
narrower status above.
