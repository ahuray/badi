# Badi Vision V2

Status: research-backed product contract, 2026-08-30

> Badi (`بعدی`, Persian for “next”) is the selected product name. Because an
> unrelated [AI workflow CLI](https://github.com/fatihkan/badi) already uses the
> `badi` command, this project uses `badictl` and owned technical namespaces.
> Badi-owned source and documentation are MIT-licensed. Public package and
> trademark clearance remain release gates.

## The product

Badi is the capability-aware co-writing layer Linux is missing.

It helps the user write their own next words inside the app they already use.
One short continuation appears only when the current target can be understood
and edited safely. The user accepts one useful word, accepts the remaining
suffix, types through it, or ignores it without leaving their flow.

The model is replaceable. The durable product is the combination of:

- one calm partial-acceptance interaction across verified adapters;
- exact focus, revision, cancellation, and insertion behavior;
- fast deterministic and local semantic prediction;
- policy that separates context, inference, learning, and retention; and
- visible, auditable evidence of what the system used and why it acted or
  stayed silent.

Badi is not a generic AI writing assistant, a chatbot floating over the
desktop, or a promise that one Linux hook works everywhere.

## Product character: precise, light, native

Badi follows a McLaren-like engineering rule: performance comes from removing
avoidable mass while refining every surface that remains. The product should
feel fast, deliberate, and mechanically honest—not feature-heavy.

- One signature interaction is polished before a second interaction is added.
- Shared contracts stay small; target-specific behavior stays in adapters.
- Hardware and policy decisions are inspectable instead of “smart” black boxes.
- Visual language follows the Linux desktop and Omarchy's calm, direct feel;
  it does not imitate a macOS overlay or introduce a separate design universe.
- A feature that cannot meet latency, safety, and usefulness gates remains off.

## The job to be done

Linux users already write in browsers, notes, chats, editors, and terminal
agents. Their current choices are fragmented:

- app-specific completion can be polished but does not travel;
- input methods travel further but cannot provide equivalent context, styling,
  identity, and edit semantics in every Wayland/toolkit path;
- rewrite tools require a second, deliberate workflow;
- text expanders are fast and reliable but cannot infer the next thought; and
- cloud assistants often ask for more context and trust than the small writing
  decision requires.

Badi's job is to preserve one co-writing habit across the Linux apps that
can prove support, while yielding cleanly everywhere else.

The first user is a Linux knowledge worker who moves among Chromium, Obsidian,
and supported browser/editor AI-agent prompt surfaces, values local control,
and will trade theoretical reach for observable safety and consistent
behavior. Ghostty/Codex remains a separate manual feasibility experiment.

## The signature loop

1. The user types normally in an eligible, focused field.
2. After a short adaptive pause, Badi may show one short suffix at the caret.
3. The next accepted word is visually stronger than the remaining suffix.
4. One configurable action accepts the next word; another accepts the rest.
5. Exact type-through reconciliation removes matching characters without
   flicker. Any contradiction, selection change, composition, focus change, or
   late result clears the suggestion.
6. Escape dismisses only while Badi is visible. Normal application shortcuts
   remain normal otherwise.
7. A capability receipt can explain the adapter, context source, activation
   mode, provider locality, learning state, retention state, and any reason for
   suppression.

Tab is not globally stolen. An adapter may offer Tab only when its visible
current candidate and app profile make that choice explicit; the universal
defaults use non-conflicting configurable actions.

## The authorship contract

The ambient V2 product appends a bounded suffix. It does not silently rewrite
existing prose, answer the user's sentence, change several locations, execute
an action, or submit text.

The user remains the author because:

- one-word acceptance is primary;
- whole-suffix acceptance is optional; undo/reversibility is claimed only for
  target cells that verify it live;
- the system abstains when confidence or target state is weak;
- acceptance is one adapter-supported target-API transaction when the surface
  supports it;
- no adapter retries through a second insertion mechanism; and
- Badi never synthesizes a submit action. A target's normal input event can
  still trigger autosave, search, validation, or application logic, so each
  compatibility cell must test and disclose those side effects.

Selected-text rewriting, snippets, voice, richer context, and next-edit
prediction are separate modes with separate grants and later milestones.

## Quietness is model quality

A grammatical suggestion can still be a bad product event. It may arrive late,
appear in the wrong voice, overlap what the user already typed, or demand more
attention than typing the words would have taken.

Badi optimizes retained useful text per interruption—not tokens generated or
suggestions displayed. Quiet intelligence includes:

- confidence-based abstention and tail trimming;
- one displayable generation per target and latest-wins cancellation;
- an adaptive debounce and a hard late-result TTL;
- per-app quieting based on immediate dismissals, contradictions, and erasure
  after acceptance;
- bounded output of at most eight words and 64 Unicode scalars in the first
  proof; and
- deterministic validation for spacing, duplication, controls, and suffix
  shape before anything appears.

The product must earn the right to appear more often.

## Two-speed prediction

The fastest correct source wins, not the largest model.

### Deterministic lane

An instant local lane handles user-authored phrases, accepted vocabulary,
common continuations, spelling, emoji, and high-confidence snippets. Every
result is inspectable and reproducible. User-authored data can be imported,
exported, and deleted.

### Semantic lane

A small local suffix model runs speculatively behind the same provider
contract. It must meet the interaction latency and usefulness gates on named
hardware. Streaming chunks remain provisional until they pass output,
revision, and confidence checks.

Before any model is downloaded, Badi may inspect content-free CPU, memory, GPU,
and battery facts and recommend a conservative `compact`, `balanced`, or
`quality` candidate for writing or code. Recommendations pin repository
revision, artifact, license, and checksum. They do not activate inference;
download and runtime approval remain explicit, separate steps.

A larger local or remote provider may become an explicit later option. Remote
inference requires both a provider grant and a target grant, remains visually
distinct, and can never be enabled merely by approving an app.

If the semantic lane is slow, poor, or unavailable, Badi degrades to honest
deterministic/manual behavior—not a hidden cloud call.

## The context firewall

“Enabled” is not one permission. Policy has five independent axes:

1. **Activation** — `Always`, `Manual`, or `Never`.
2. **Context** — bounded active-field text and, only in later separately
   granted modes, clipboard, focused-window text, or another named source.
3. **Inference** — deterministic, local model, or explicitly configured remote
   provider.
4. **Learning** — off, user-authored/imported, accepted-only, or a later
   explicitly enabled source.
5. **Retention** — ephemeral by default, with separately visible and deletable
   stores when a future feature needs persistence.

Deny wins. Hard-sensitive gates run before acquisition or serialization, not
merely before generation. Targets positively identified by supported metadata
as password, PIN, OTP, payment-secret, lock-screen, active foreign composition,
ambiguous, non-editable, or multiply focused produce zero context bytes.

Metadata can be missing or wrong: an ordinary text field may still contain a
secret that no adapter can infer safely from its prose. The first proof claims
zero bytes only for its recognized types and autocomplete purposes and records
this residual risk. Unknown identity never inherits `Always`; without a truly
explicit path that blocks acquisition until activation, it is unsupported.

Policy is enforced in two places. An adapter-local metadata gate runs before
any text access. The broker then makes the authoritative inference decision
from validated session state, declared field metadata, activation, and global
pause. The first foundation has no persisted origin-policy store and must not
pretend that a decorative wire counter solves that absence. When independent
origin grants or policies are added, every request, suggestion, and commit is
bound to a monotonically increasing broker policy epoch; changes broadcast,
cancel, clear, and require adapter acknowledgement before acquisition resumes.

The M1 pause linearizes provider work and new commit authorization inside the
broker, while adapters that receive the authoritative state also clear and
stop locally. An adapter not reached by the browser-owned broadcast—including
an existing controller when pause comes from a separate CLI connection, or a
controller that starts or restarts afterward—may continue to acquire and send
requests that the broker rejects before any provider call; there is no M1
cross-connection acquisition barrier. Nor does M1 prove that an already
transmitted commit preparation cannot mutate after a pause acknowledgment on
another connection. M2 requires policy epochs, broadcast/ack, and a revocation
barrier before making either stronger claim.

Origin grants and policy preferences are privacy-relevant retained data even
when prose is ephemeral. They need visible scope, revocation, export/delete,
and migration behavior of their own.

Normal diagnostics contain ephemeral connection/message identifiers, byte
counts, timing buckets, policy reasons, and error codes—not prose, origins,
paths, fingerprints, stable target identifiers, suggestions, or accepted text.
Local inference reduces a network boundary; it does not excuse broad capture,
logging, weak IPC, model-license opacity, swap, or crash remnants.

## Linux-native means capability-native

Linux is not one text API. Badi uses this integration order:

1. **Target-API adapter** — a browser extension through supported DOM APIs, an
   app-owned Obsidian/CodeMirror plugin, or another explicit editor API owns
   context, adapter UI, revalidation, and the final edit. Extension-owned UI is
   never described as app-owned.
2. **Existing input-method framework** — Fcitx5 and later IBus provide
   conditional breadth when the real compositor/toolkit/app tuple exposes the
   required capabilities and Badi can coexist with the user's IME.
3. **AT-SPI observation** — optional context or geometry assistance only after
   target-specific tests; never generic edit authority.
4. **Manual/status surface** — explains state where inline presentation is not
   possible, but does not manufacture a safe insertion path.
5. **Unsupported** — the correct result when target identity, sensitive state,
   revision, placement, or native insertion cannot be proved.

There is no production rung for raw `evdev`, screenshots or clipboard scraping
by default, `wtype`, virtual keyboards, `xdotool`, arbitrary Electron preload
injection, or title-only security policy.

## Architecture as a product promise

One local Rust broker owns protocol validation, policy, cancellation, provider
lifecycle, segmentation, pause state, receipts, and content-free metrics.

Small target-native adapters own the text surface:

- a strict TypeScript Manifest V3 adapter for Chromium;
- a strict TypeScript CodeMirror adapter for Obsidian;
- a manual-only C++ Fcitx5 experiment for Ghostty/Codex; and
- later adapters only when a target exposes a stable, testable contract.

The broker cannot mutate another app. It can only authorize a revision-bound
commit for the one adapter that owns the still-focused, unchanged target. The
adapter awaits that authorization, re-reads target identity, field purpose,
selection, revision, and fingerprint, then attempts one target-API mutation.
It reports `applied` only when the target can verify state and undo semantics;
otherwise it reports `dispatched-unverified`.

Every non-global action is addressed by session, focus epoch, revision,
fingerprint, and suggestion ID. Only pause is global. “Active” additionally
means the exact active element in a visible document inside the active tab and
focused browser window. A CLI may receive a content-free locator only when
exactly one candidate exists, and the broker atomically revalidates it before
acting.

Wire framing is bounded by 64 KiB of encoded bytes. Browser selection offsets
are full-target JavaScript-safe counters explicitly tagged
`utf16_code_units`; the broker treats them as opaque state unless a named
adapter conversion says otherwise. Transmitted context is independently
capped at 512 Unicode scalars before and 128 after. Ill-formed UTF-16 in a
candidate context slice fails closed before transport while valid astral
scalars remain intact. Output is capped at 64 scalars/eight words using shared
multilingual segmentation fixtures. Monotonic timestamps are sender-local and
never compared across processes; suggestion lifetime is a relative TTL
interpreted on receipt.

Fingerprints are fresh, session-scoped, opaque race detectors. They are never
persisted or logged and do not authenticate an adapter. The local trust model
still admits same-UID impersonation; peer credentials and socket permissions
reduce exposure but do not make receipts cryptographic proof.

Every adapter publishes a capability manifest and auditable test receipt.
Within the declared trusted-local-component boundary, the receipt records
evidence; it is not proof of all machine behavior. Compatibility is a shipped
artifact indexed by app, version, surface, display route, toolkit, sandbox,
context source, rendering mode, insertion mode, policy certainty, evidence
class, and failure reason. Local suppression receipts cover events that
correctly never reach the broker.

## Terminal behavior

Terminals combine valuable natural-language agent prompts with shell
completion, raw-mode TUIs, secret prompts, and commands that can execute. They
do not receive ambient V2 completion.

The first Ghostty/Codex work is an independent feasibility experiment:

- the user explicitly arms a short-lived, local, ephemeral Fcitx session before
  composing a one-line agent prompt;
- unarmed typing produces zero context and provider calls;
- foreign IME preedit/candidates always win;
- control characters, Tab, Escape, CR/LF, and Enter are rejected from output;
- one Fcitx `commitString` dispatch is reported as
  `dispatched-unverified`, never transactionally `applied`; and
- failure in context, candidate positioning, coexistence, or commit marks the
  tuple unsupported.

A Bash hook does not prove support inside a Codex TUI. A layer-shell pill or
synthetic typer does not rescue a failed terminal target. Future ambient agent
support requires a supported terminal or agent API, not process/title/prompt
heuristics.

## Personalization without surveillance

Personalization begins with evidence the user authored or approved:

1. explicit vocabulary, names, phrases, snippets, and app instructions;
2. imported writing chosen by the user;
3. accepted completions and, later, sessions in which acceptance occurred; and
4. only after measured value and explicit consent, any broader source.

Raw cross-app typing is not the default training set. Personalization must be
local, inspectable, exportable, reversible, and deletable. Badi should show
which source influenced a suggestion without exposing the source text in
normal telemetry.

## Product ladder

### P0 — Trust substrate

Ship the versioned protocol, focus/revision/fingerprint state machine, hard
field deny, five-axis policy, pause, deterministic provider, metadata-only
health metrics, and capability receipts.

### P1 — Signature loop

Prove adapter-owned ghost text, target-API context and mutation, and exact
partial/whole acceptance in Chromium ordinary fields, then Obsidian Markdown.
Add one evaluated small local suffix provider without making the UI depend on
it.

### P2 — Quiet intelligence

Add confidence abstention, adaptive timing, deterministic phrase/spell/emoji
ranking, accepted-only vocabulary, explicit app instructions, and
import/export/delete. Dogfood for one week before widening.

### P3 — Linux breadth

Run the manual Fcitx/Ghostty/Codex gate independently, then test exact GTK, Qt,
XWayland, Firefox, and later IBus tuples. Publish failures with reason codes.

### P4 — Deliberate tools

Add selected-text rewrite as one explicit undoable transaction, a snippet
library, and alternatives on demand. These modes never start ambiently.

### P5 — Optional richer context

Only after separate consent, add named context sources, remote providers,
fill-in-the-middle, voice adapters, or sync. Each source has independent grant,
revocation, audit, retention, and deletion tests.

### P6 — Advanced prediction

Consider next-edit prediction, broader adapter SDKs, and organization policy
only after suffix completion shows durable weekly value and acceptable adapter
maintenance cost.

## The first proofs

### Browser-adapter proof ladder

The M1 implementation exercises the trust substrate and signature loop first
in a simulated controlled-DOM ordinary field: a strict bounded protocol,
deny-before-acquisition fixtures, deterministic local suggestions,
adapter-owned display and target-API actions, induced late/focus/revision
races, and reproducible Rust/TypeScript checks.

The foundation's only positive surface is a connected, visible, enabled,
editable, stable-identity, top-level `textarea` or `input type="text"` with a
collapsed selection, no active composition, and no constraint that can alter
the attempted insertion. It explicitly excludes other input types,
non-collapsed selection, readonly/disabled fields, contenteditable,
framework-controlled fields, iframes, shadow-DOM targets, detached/replaced
nodes, arbitrary origins, and unverified `maxlength`, validation, undo, or
side-effect behavior.

Its browser output gate rejects NUL, C0/C1 controls, DEL, CR/LF/Tab, unpaired
UTF-16 surrogates, and dangerous invisible/bidirectional override controls. It
renders text nodes only, never model-provided markup. Multiline, bidi, and
language-specific invisible controls require later named capability profiles.

The page remains untrusted. It can detect or remove an extension-owned host,
mutate a field during event dispatch, intercept the emitted input event, or
perform network/application side effects. A closed shadow root is isolation,
not secrecy. Hostile-page observation and sabotage remain acceptance tests
before any arbitrary origin is considered.

The foundation receipt contains deterministic cases for edit,
selection, composition, blur, visibility, DOM removal/replacement,
policy/pause, expiry, disconnect, and accept-versus-edit, plus at least 100
permuted late-result schedules. Stale display and stale insertion are counted
separately. Its machine-readable record remains explicitly `simulated-dom`.

The M2A integration record then proves a narrower live target end to end. A
real named Chromium build loads an unpacked extension from a disposable HOME,
XDG tree, and profile; Chrome launches the shipped Rust native-messaging host;
the host validates Chrome framing and the exact development origin, then
relays strict envelopes to the private peer-UID-checked broker socket. The
temporary host manifests, profile, socket, and processes are removed after the
run, while the user's real profile and configuration remain untouched.

M2A's positive evidence is deliberately specific:

- only the exact `http://localhost:4173/chromium.html` document is matched, the
  top frame is required, `nativeMessaging` is the only API permission, and
  incognito operation is declaratively disabled and independently denied;
- Chromium 132 is the minimum supported browser for this cell because the
  fail-closed sender boundary requires an explicit non-frozen tab state;
- the service worker requires an active nonempty document identity plus exact
  extension, origin, URL, tab, and frame identity before binding a route;
- trusted real-browser dismiss, accept-word, accept-all, and authoritative
  pause/resume gestures plus navigation, policy mutation, geometry, and native
  disconnect paths run against the Rust host and broker; composition lifecycle
  uses synthetic `CompositionEvent`s in the real browser and is not an IME
  claim;
- recognized password and one-time-code fields create zero broker context,
  provider-call, or provider-input-byte deltas;
- repeated exact insert/caret trials cover end and mid-line positions, astral
  text, Greek text, and a combining boundary; and
- delayed stale-response injection is reported separately as a live-browser
  fault-host test, never as production-host evidence.

A V2 raw-run record and capability receipt bind versions, scenario counts,
nearest-rank p95 measurements, isolation claims, and artifact hashes. The
receipt is auditable evidence inside the declared trusted-local-component
boundary, not a signature or proof that an unrelated binary was not substituted
at runtime.

The retained historical M2A cell passed 1,000/1,000 exact insert/caret trials
and 100/100 delayed stale races. With 50 warmups excluded from each
1,000-sample distribution, nearest-rank p95 was 12.6 ms from trusted accept to
observed input and 0.6 ms from invalidation marker to hidden UI. These are
source-linked controlled local interaction measurements, not a reproduction
against every later tree and not semantic-model or end-to-end generation
latency.

M2A is not full Chromium support. Headless Chromium did not prove a visible
runtime-origin consent prompt, background-tab visibility, the extension command
accelerator, browser-native undo grouping, headed compositor/accessibility
rendering, active-window arbitration, permission revocation, MV3 restart, or a
cross-connection policy epoch. Framework-controlled fields, `contenteditable`,
iframes, shadow targets, arbitrary sites, production packaging, and semantic
model quality also remain unsupported. Those limits are product truth, not
items inferred away by a successful controlled run.

### Closed-alpha product proof

The product proof requires both target-API adapters:

- unpackaged Chromium ordinary `input`/`textarea` on a controlled origin; and
- Obsidian desktop Markdown through supported CodeMirror APIs.

The Chromium cell begins with no broad web access. It requires an exact
runtime-granted scheme/host/port, a top-level controlled frame, active tab and
focused window checks, service-worker sender validation, and a native-host
manifest allowlisting only the development extension ID. Restricted schemes,
file access, incognito, arbitrary frames, and `<all_urls>` are excluded.
Navigation or permission revocation cancels work and clears UI before any next
read or commit.

The manual terminal cell is reported separately. Its failure does not erase a
valid two-adapter product result, and it cannot be relabeled through a shell
fixture or synthetic fallback.

## Provisional closed-alpha gates

These are explicit starting hypotheses, not claims already achieved.

### Safety and correctness

- Positively recognized hard-denied fixture text crossing adapter IPC,
  provider boundaries, storage, or normal/error logs: **0 bytes**.
- Stale display or insertion after any focus, revision, selection, composition,
  policy, or target change: **0**.
- Exact target-API insertion and caret result: **100/100** real controlled
  trials per supported adapter.
- Foreign IME events swallowed, reordered, or cleared: **0**.
- Advertised compatibility cells failing their published suite: **0**.

### Responsiveness

These gates remain unachieved until live tests run. Latency distributions use
at least 1,000 measured interactions after 50 warmups on an otherwise idle
named machine, with receiver-local monotonic endpoints and nearest-rank p95.

- Deterministic provider work after debounce: p95 **≤15 ms**; visible result
  p95 **≤50 ms**.
- Warm local-model edit-to-visible result on the named i7-12700H/16 GB machine:
  p50 **≤250 ms**, p95 **≤500 ms**.
- Adapter accept-to-verified-insert: p95 **≤30 ms**.
- Invalidating event to hidden UI: p95 **≤32 ms**.
- Result older than the 600 ms generation TTL becoming visible: **0**.

### Utility and quietness

- Blind next-word usefulness on the frozen mixed set: at least **8/12**.
- Accepted graphemes retained for ten seconds: at least **90%**.
- Median net keystroke savings over one week: at least **10%** after correction
  cost.
- Immediate bad appearances after tuning: at most **15%**.
- Accepted-only vocabulary and explicit instructions must improve blind or
  retained acceptance by at least **5 percentage points** before becoming a
  default feature.

Ten-second retention measurement, if enabled, observes the accepted range only
in volatile local memory, persists aggregate counters only, and is separately
visible and disableable. It is still a context use and receives a policy
receipt.

### Trust

- Eligible and locally suppressed decisions with complete adapter/context/
  policy/provider/learning/retention evidence: **100%**.
- Remote calls without both target and provider grants plus visible state:
  **0**; remote is absent from the first proof.
- Personalization records remaining after verified delete-all and restart:
  **0**.

## Non-goals for V2 proof

- literal support for every Linux app, compositor, toolkit, sandbox, or
  distribution;
- raw global input capture or synthetic typing as the architecture;
- ambient completion in ordinary shells or secret/raw terminal prompts;
- arbitrary rewrite, chat, desktop-agent actions, MCP tools, or code execution
  in the typing boundary;
- clipboard/screen context, remote inference, raw-history learning, accounts,
  sync, telemetry, teams, billing, or a plugin marketplace by default;
- voice, next-edit prediction, or polished settings before the suffix loop
  proves retained value;
- public packages, domains, or launch before Badi's package/trademark and
  technical-namespace review is complete; or
- importing third-party source without license-compatibility review or without
  recording all code/model/tokenizer/data obligations.

## What success feels like

The user stops thinking about Badi as an application. They write in a
supported target, occasionally accept exactly the useful connective word, and
never wonder whether a late suggestion will land somewhere dangerous. When the
system is quiet, blocked, manual, local, or unsupported, the reason is visible
without exposing the prose itself.

The product succeeds when it preserves authorship and flow while asking for
less trust than the alternatives—not when it generates the most text.

## North-star sentence

**Help me write my own next words across the Linux apps that can support it—
quietly, locally, and with an auditable account of what the system saw and
did.**
