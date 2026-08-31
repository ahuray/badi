# GrillMe round: Omarchy fit and suggestion quality

- Audit snapshot: uncommitted implementation candidate
- Base commit: `75abbfad315f026922645fe00f8d75d15f266879`
- Prepared: 2026-08-30
- Freeze authorization: owner-approved on 2026-08-30; exact-SHA CI required
- Review mode: independent source review; one lead-owned serial verification lane

## Executive verdict

**Requires material engineering before a product showing.**

The hostile round materially improved the candidate: control mutations now
fail closed, permission and retention controls say what they actually do, the
supported Chromium display lane owns a literal 600 ms ceiling, enumerated unsafe
suggestion shapes are rejected instead of rewritten, language reaches the
provider, and the phrase lane cannot silently fall back to generic filler. The
result is credible for a private source-architecture critique after an
immutable commit has passed CI. Interaction quality still requires headed
evidence.

It is not an Omarchy-quality local co-writer yet. Production still wires four
exact English integration phrases, not a semantic model
([main](../../broker/src/main.rs#L17),
[rules](../../broker/src/provider.rs#L64)). The aggregate store retains counters,
not writing style. The browser surface is one localhost fixture and renders a
field-width panel below the control rather than caret-inline ghost text
([positioning](../../adapters/chromium/src/content/ghost-view.ts#L22),
[panel](../../adapters/chromium/src/content/ghost-view.ts#L143)). The control
center is a second standalone `ShellRoot` with a private hardcoded palette, not
an Omarchy shell plugin
([historical shell](https://github.com/ahuray/badi/blob/e113d45e43338f30235e0830d6674c520dedb242/ui/quickshell/badi/shell.qml#L8),
[historical theme](https://github.com/ahuray/badi/blob/e113d45e43338f30235e0830d6674c520dedb242/ui/quickshell/badi/BadiTheme.qml#L5)). No current clean-commit,
headed Chromium, or headed Omarchy evidence exists for this tree.

## Findings

### Critical

No Critical finding was established.

### High

#### H1 — Production has no useful general writing intelligence

**Classification:** missing product capability and release blocker, not a
security defect.

**Evidence:** the production binary constructs only
`DeterministicPhraseProvider` ([main.rs:17-20](../../broker/src/main.rs#L17)).
That provider contains four English exact-current-line rules and otherwise
abstains ([provider.rs:64-124](../../broker/src/provider.rs#L64)). The local model
module is evaluation-only and is not wired into production. The UI truthfully
reports that adaptive writing memory is unavailable; stored data consists of
text-free counters.

**Impact:** normal prose produces silence. Badi cannot yet demonstrate the
context-sensitive quality, tone adaptation, or continuity the product vision
asks for.

**Smallest fix:** build one frozen, owner-approved writing corpus and evaluator;
qualify at most one pinned local model through the real adapter-to-visible path;
then wire it behind explicit artifact/runtime attestation. Do not add more model
abstractions or catalog entries first.

**Blocks:** product/Omarchy showing and release; does not block a private
architecture review.

#### H2 — The UI is not an Omarchy-native integration

**Classification:** product-fit defect.

**Evidence:** Badi owns a standalone `ShellRoot` and `FloatingWindow`
([historical shell.qml:8-24](https://github.com/ahuray/badi/blob/e113d45e43338f30235e0830d6674c520dedb242/ui/quickshell/badi/shell.qml#L8)) plus hardcoded color and
spacing tokens ([historical BadiTheme.qml:5-32](https://github.com/ahuray/badi/blob/e113d45e43338f30235e0830d6674c520dedb242/ui/quickshell/badi/BadiTheme.qml#L5)).
Current Omarchy guidance places menus/panels inside its long-running Quickshell
shell as plugins and provides shared theme primitives. See the official
[Omarchy shell plugin contract](https://github.com/omacom/omarchy/blob/quattro/docs/omarchy-shell.md)
and [theming contract](https://github.com/omacom/omarchy/blob/quattro/docs/theming.md).

**Impact:** a second resident shell can look and behave foreign, duplicate
runtime mass, and fail theme/focus/compositor conventions even though isolated
QML loading succeeds.

**Smallest fix:** after the control contract stabilizes, port this panel to one
disabled-by-default Omarchy shell plugin using shared theme tokens, then perform
headed scaling, focus, keyboard, screen-reader, and theme-switch validation.
Do not patch `/usr/share/omarchy` or fabricate plugin compatibility in this
working tree.

**Blocks:** product/Omarchy showing and release; not source architecture review.

#### H3 — Current claims are not attested by an immutable candidate

**Classification:** evidence and handoff defect.

**Evidence:** at audit preparation, `develop` was still at committed base
`75abbfad315f026922645fe00f8d75d15f266879` with a broad modified/untracked tree.
The durable 1,000 insertion, 100 stale-race, 12.6 ms, and 0.6 ms results belong
to an earlier recorded commit, as README explicitly says
([README:68-72](../../README.md#L68)). GitHub Actions cannot execute or attest
uncommitted content at that point.

**Impact:** reviewers cannot reproduce the exact candidate or distinguish live
proof from historical evidence.

**Smallest fix:** only after owner approval, freeze one review commit, run CI at
that SHA, and reproduce headed evidence from an isolated clone/worktree. Keep
historical receipts labeled historical.

**Blocks:** external review handoff and release; private local source review is
still possible.

### Medium

#### M1 — Trusted accept keys can be consumed even when authorization later fails

**Classification:** known interaction risk.

**Evidence:** Tab and Ctrl/Command+Right are synchronously prevented before the
asynchronous native authorization resolves
([field-controller.ts:598-634](../../adapters/chromium/src/content/field-controller.ts#L598)).
The source correctly explains that denial cannot replay native browser behavior
without synthetic input.

**Impact:** a stale/denied commit can swallow Tab navigation or word movement
while inserting nothing.

**Smallest fix:** headed-test the tradeoff. If it is distracting, pre-authorize
the short lease before showing an actionable suggestion or use a non-native
accept gesture; never synthesize the lost key.

**Blocks:** broad browser/product claim; not architecture review.

#### M2 — The evaluation scaffold still exceeds the product it serves

**Classification:** maintainability risk and overengineering.

**Evidence:** the normal binary has no model provider, corpus, evaluator,
supervisor, or qualifying receipt, while `local_model.rs` already defines a
large prompt/runtime/receipt surface. Readiness still consumes caller-supplied
aggregate metrics ([historical metrics contract](https://github.com/ahuray/badi/blob/d9e39ee7f6fb7a0a48d3cca178506569381f9167/broker/src/local_model.rs#L994),
[historical gate](https://github.com/ahuray/badi/blob/d9e39ee7f6fb7a0a48d3cca178506569381f9167/broker/src/local_model.rs#L1371)). The improved evaluator contract
now defines one schedule-to-visible clock and a 600 ms deadline
([historical contract](https://github.com/ahuray/badi/blob/d9e39ee7f6fb7a0a48d3cca178506569381f9167/broker/src/local_model.rs#L55)), but no implementation measures
it. The production portions of `engine.rs` and `local_model.rs` are each well
over one thousand lines and the field controller is also a large monolith,
while the shipped provider still owns only four rules.

**Impact:** polished schemas can be mistaken for runtime proof and impose
maintenance cost before a candidate model has shown useful completions. Large
state-machine surfaces make review and change harder before one real product
lane has earned their full abstraction set.

**Smallest fix:** stop expanding the scaffold. Implement the corpus/evaluator
and owned runtime boundary next; delete any surface the first real lane does not
need. Do not perform a decorative rewrite now—use the first qualified lane to
identify cohesive state boundaries and split only where ownership becomes
clearer.

**Blocks:** model activation and release; not architecture review when labeled
evaluation-only.

#### M3 — Generated output is intentionally not multilingual

**Classification:** explicit scope limitation and future correctness risk.

**Evidence:** both sanitizers reject Unicode format controls, including Persian
ZWNJ and emoji ZWJ
([Rust sanitizer](../../broker/src/segment.rs#L97),
[TypeScript sanitizer](../../adapters/chromium/src/content/context.ts#L195)).
The shared word splitter can handle Unicode graphemes, but that is not evidence
that generated Persian, Arabic, dictionary-script, or emoji output is valid.
The current phrase lane is English-only ([provider.rs:98-111](../../broker/src/provider.rs#L98)).
The boundary gate also deliberately rejects Latin partial-token output such as
`look` + `ing`, does not normalize canonically equivalent Unicode, and cannot
fully identify duplicate suffixes inside unspaced multi-scalar words. Simple
punctuation joins remain language/context ambiguous.

**Impact:** future model output can be rejected despite being linguistically
correct, or pass an insufficient boundary rule for an unsupported script.

**Smallest fix:** keep production English-only until language-aware cross-Rust/
TypeScript fixtures cover ZWNJ/ZWJ policy, word counts, adjacency, overlap, and
punctuation, including explicit partial-word and normalization cases. Prefer
safe abstention to a broad multilingual claim.

**Blocks:** multilingual product/release claim; neither current English
architecture review nor the four-rule probe.

#### M4 — The browser experience is a controlled fixture, not Cotypist-like inline UX

**Classification:** product-scope gap.

**Evidence:** the manifest and controller are intentionally limited to the
localhost fixture. The ghost view anchors a 240–560 px panel below or above the
whole field ([ghost-view.ts:22-32](../../adapters/chromium/src/content/ghost-view.ts#L22))
and includes a separate shortcut hint
([ghost-view.ts:153-193](../../adapters/chromium/src/content/ghost-view.ts#L153)).
Its visibility predicate checks geometry and computed styles, not actual
stacking/occlusion by a hostile page, while a visible suggestion arms its accept
shortcut.

**Impact:** jsdom and static fixture success can coexist with incorrect caret
placement, editor incompatibility, obstruction, and a visibly non-inline
product. On a broader origin, a page overlay could hide the panel while Tab
acceptance remains armed.

**Smallest fix:** perfect one real textarea/editor in headed Chromium with
caret-relative ghost rendering, native undo, zoom/DPI/scroll, hostile CSS, and
framework-controlled value tests before widening origin support.

**Blocks:** product showing and broad browser claim; not trust-boundary review.

#### M5 — Same-UID checks are privacy isolation, not process authentication

**Classification:** accepted residual security risk.

**Evidence:** the private socket and peer UID exclude other OS users, but any
same-UID process may impersonate a client or broker. The evaluation loopback
provider similarly cannot prove which local process owns the port or which
artifact it loaded.

**Impact:** a malicious process already running as the user can access bounded
context or inject a suggestion through local interfaces.

**Smallest fix:** retain the narrow same-UID claim for the current foundation;
before model activation, own/supervise the runtime and bind launch identity,
transport secret, and opened artifact identity. Do not call loopback alone
authenticated.

**Blocks:** release threat model for model activation; neither current private
architecture review nor protection from other OS users.

#### M6 — Per-target revocation depends on aggregate-store repair

**Classification:** privacy-control availability and recovery tradeoff, not an
active privacy leak.

**Evidence:** the UI disables subject mutation while optional aggregate state is
unavailable ([historical BadiClient.qml:121-123](https://github.com/ahuray/badi/blob/e113d45e43338f30235e0830d6674c520dedb242/ui/quickshell/badi/BadiClient.qml#L121)).
The control plane permits a subject-identical global pause but refuses permission
changes until it can reconcile retained aggregates
([control_plane.rs:159-177](../../broker/src/control_plane.rs#L159)). The header
Pause action remains available when runtime state is known.

**Impact:** after aggregate corruption, the user cannot block only one origin
until clearing or repairing Memory. They can stop all context/suggestions when
runtime control is known, and the UI explains the exact available recovery path,
but the advertised per-app control is temporarily unavailable.

**Smallest fix:** keep the current transactional privacy guarantee for this
foundation. Before release, design and test a durable privacy-reducing
tombstone/cleanup state that can publish deny authority without ever presenting
unreconciled retained data as compliant. Do not weaken deletion ordering merely
to keep the button enabled.

**Blocks:** release recovery UX; neither private architecture review nor the
healthy-store control-center path.

### Low

#### L1 — Packaging decisions remain unresolved

**Classification:** handoff risk.

**Evidence:** README discloses the unrelated `badi` CLI collision and uses
`badictl`, but the repository still has no top-level project LICENSE
([README:9-13](../../README.md#L9)).

**Impact:** an external reviewer cannot infer redistribution terms or final
package naming.

**Smallest fix:** choose and add the project license and complete package/name
clearance before merging to a public release branch.

**Blocks:** release; neither private architecture nor UX review.

## Text-suggestion quality verdict

The production lane is now **conservative but not intelligent**. Its complete
semantic repertoire is:

| Exact English current line | Emitted suffix |
| --- | --- |
| `thank you` | ` for your time` |
| `looking forward` | ` to hearing from you` |
| `the next step` | ` is to verify the result` |
| `please` | ` let me know what you think` |

Those four pairs are grammatical integration probes. They do not learn tone,
use earlier window context, infer intent, or establish Cotypist-like usefulness.
The provider requires declared English, an empty suffix after the caret, a
provable current-line boundary, and an exact case-insensitive trigger
([provider.rs:64-126](../../broker/src/provider.rs#L64)). Every other input
abstains, and there is no hidden generic fallback.

The display-safety gate was challenged with concrete false-accept and
false-reject cases. It now rejects ambiguous Unicode/repeated spacing,
non-CJK word concatenation, and boundary-aware duplicate tokens on either side,
while preserving valid cases such as `class` + ` as`, `hello.` +
` Hello again`, and CJK-family adjacency
([segment.rs:19-153](../../broker/src/segment.rs#L19)). This makes the current
probe less distracting; it does not prove model quality. Partial Latin tokens,
punctuation seams, Unicode normalization equivalence, and deep overlap inside
unspaced scripts remain unsupported model-output cells rather than silently
claimed coverage.

The evaluation-only local-model prompt asks for no wrapper quotes or markup
([historical local_model.rs:52](https://github.com/ahuray/badi/blob/d9e39ee7f6fb7a0a48d3cca178506569381f9167/broker/src/local_model.rs#L52)), but its parser does
not yet enforce a complete wrapper grammar beyond reasoning markers, limits,
and the shared sanitizer
([historical local_model.rs:703-748](https://github.com/ahuray/badi/blob/d9e39ee7f6fb7a0a48d3cca178506569381f9167/broker/src/local_model.rs#L703)). Quoted,
backticked, or HTML-like output can therefore survive this early parser when it
otherwise meets the safety contract. This is dormant because no model is wired
to production. Before activation, frozen adversarial outputs and the real
writing corpus must determine the narrow grammar to enforce; globally banning
punctuation without that evidence would damage valid prose.

## Improvements produced by this hostile round

### Suggestion correctness and latency

- Language is derived from the nearest canonical page declaration, fingerprint
  bound, mutation-invalidated, transported, and enforced by the English phrase
  lane.
- The generic configurable phrase fallback was deleted. Only explicit rules can
  produce output ([provider.rs:59-124](../../broker/src/provider.rs#L59)).
- Output is rejected—not trimmed or truncated—when empty, oversized, control-
  bearing, trailing-whitespace, ambiguously spaced, joined across a script that
  requires word separators, or overlapping either neighboring token. Overlap
  checks use token boundaries rather than substring suffixes, so `class` +
  ` as` and sentence-boundary `hello.` + ` Hello again` remain valid
  ([segment.rs](../../broker/src/segment.rs#L19)).
- Malformed language tags with empty subtags are rejected by one shared broker/
  local-model validator, the schema matches that nonempty-subtag syntax,
  language mutation tests assert the changed payload and fingerprint, and the
  phrase lane abstains when a full 512-scalar prefix cannot prove the start of
  the current line. This is bounded syntax, not a full BCP 47 implementation;
  current Chromium additionally canonicalizes with the platform API.
- Chromium owns the 140 ms user-idle debounce; the production broker no longer
  adds another 120 ms ([engine.rs:63-74](../../broker/src/engine.rs#L63)).
- Broker timing is checked before parsing, after the state lock, immediately
  before publication. Chromium owns an independent timer and final pre-show
  fence. Every applied commit retires the old suffix; a fresh context and
  provider result are required instead of republishing a cached remainder. The
  now-unused post-commit rebind coordinates and extra context capture were
  removed from the protocol/adapter rather than retained as speculative surface
  ([engine deadline](../../broker/src/engine.rs#L931),
  [adapter deadline](../../adapters/chromium/src/content/field-controller.ts#L730)).
- A matching terminal `suggestion.clear` was second-source traced and proven to
  settle the native pending promise; it does not wait for the three-second
  transport timeout
  ([native dispatch](../../adapters/chromium/src/background/native-client.ts#L503),
  [regression test](../../adapters/chromium/test/native-client.test.ts#L455)).

### Control-center correctness

- Runtime pause uses explicit `pause on` / `pause off`; it never calls a missing
  command or toggles an unknown state.
- Reads and writes are mutually gated; protocol safe integers use QML `double`;
  stale/unavailable/degraded status is explicit in the fixed header.
- Partial permissions use separate **Block all** and **Allow bundle** actions.
  The 64-subject limit is enforced and described only when actually reached.
- Aggregate collection is independent from persistence. **Memory only** remains
  available as a privacy-reducing action during recorder degradation, while
  bounded retention requires healthy storage. Re-selecting the current option
  does not churn the settings revision.
- Destructive clear has a spatially distinct confirmation and resets whenever
  the window hides.
- Missing control-plane condition can no longer coexist with a green **Active**
  header, and retention choices expose radio/checked semantics to assistive
  technology.

These are real quality gains. They do not turn the standalone panel into an
Omarchy plugin or the aggregate recorder into adaptive writing memory.

## Claim-verification matrix

| Claim | Result | Evidence and caveat |
| --- | --- | --- |
| Production suggestions are semantic and context-aware | **Contradicted** | Four exact English rules are wired; arbitrary prose abstains. |
| The phrase lane has no hidden generic fallback | **Verified by source** | The provider contains only a rules vector and exact match. |
| Results older than 600 ms cannot newly display | **Verified by source and focused tests** | Broker and adapter own absolute fences; applied commits retire cached suffixes instead of restarting a display TTL. No headed timing reproduction exists. |
| Normal terminal clears wait three seconds | **Contradicted** | Pending-loop tracing and a new test show a matching clear resolves to a null suggestion immediately. Truly missing/nonmatching replies remain bounded by transport timeout and the adapter's 600 ms cancellation. |
| The model recommendation proves writing quality | **Contradicted** | Hardware/artifact fit only; `runtime_ready` remains false and no candidate passed the quality gate. |
| The current machine should receive balanced writing/code advice | **Previously reproduced, live-value caveat** | Hardware advice selected Qwen3 1.7B writing and Qwen2.5-Coder 1.5B code; available memory and power state can vary. |
| No model was downloaded or enabled | **Verified locally for this session** | No model artifact was found; production constructs the phrase provider. |
| Adaptive writing memory exists | **Contradicted** | Stored records are text-free origin/provider/day counters and are not provider input. |
| The control center is Omarchy-native | **Contradicted** | It is a standalone repo-local Quickshell shell with private tokens. |
| Local-only means process-authenticated | **Contradicted** | Same-UID and loopback boundaries are deliberately narrower claims. |
| Historical 1,000/100 and 12.6/0.6 results prove this tree | **Contradicted** | They remain hash-linked historical evidence for an earlier commit. |
| At report preparation, this tree was CI-attested | **Contradicted** | It was uncommitted; CI could not attest it. |

## Tests that can pass while the product remains broken

1. All Rust tests can pass while ordinary prose yields no suggestion, because
   the production provider has only four exact triggers.
2. All jsdom tests can pass while caret placement, native undo, real MV3 timing,
   and hostile page CSS fail in headed Chromium.
3. QML lint and an offscreen load can pass while the window looks foreign,
   steals focus, scales badly, or violates Omarchy shell/plugin conventions.
4. Model receipt schema tests can pass using caller-supplied aggregates even
   though no model, corpus, evaluator, or qualifying run exists.

## Likely objections from the head of Omarchy

1. “Why is this a second Quickshell shell instead of a small plugin in the shell
   we already run?”
2. “Show me excellent real inline writing assistance—not a localhost fixture
   and four canned phrases.”
3. “Why is the model gate so elaborate when no model has passed it or entered
   production?”
4. “Where is the headed Omarchy/Hyprland proof for theme, focus, scaling,
   permissions, and recovery?”

## Challenge disagreements and resolution

| Challenge | Resolution |
| --- | --- |
| GrillMe initially said broker emission was not deadline-rechecked after provider completion. | **Resolved by fix and source recheck.** The absolute deadline is carried into `finish_generation`, checked after the state lock and before publication, and tested. |
| A specialist found applied-word continuation could republish an old remainder, then showed that a relative TTL could still outlive the deadline in transit. | **Resolved by a second-source counterexample and simplification.** The continuation path and its now-unused rebind fields were deleted. Applied commits retire all pre-mutation text and require a fresh context/provider cycle. |
| GrillMe said terminal abstentions remained pending for three seconds. | **Contradicted by second-source trace and test.** Matching clears flow past the event callback into `parseSuggestionReply`, resolve null, and clear the timer. |
| UX review found degraded header, retention, capacity, and no-op defects. | **Resolved, then re-reviewed.** A first capacity-label fix introduced an unavailable-state mislabel; a distinct `fixtureCapacityReached` predicate corrected it. |
| Documentation implied an Omarchy-quality control center. | **Resolved as scope language, not integration.** The handoff now calls it standalone and cites the official plugin/theme contracts. Headed plugin proof remains unresolved. |
| “All generation paths share 600 ms” included the evaluation-only HTTP client. | **Narrowed.** The supported Chromium display lane is fenced; the dormant client has a separate bounded request timeout and no production-ready claim. |
| GrillMe rated aggregate-store-coupled revocation as underweighted. | **Resolved by severity review.** It is now a Medium privacy-control availability finding; global pause and honest repair messaging mitigate impact but do not restore per-target control. |
| The report implied CI would establish interaction quality. | **Narrowed.** Exact-SHA CI establishes source reproducibility only; private interaction review still requires headed evidence. |

Unresolved questions remain explicit: the emergency per-origin revoke/tombstone
design, hostile-page stacking/occlusion, language-specific partial-token and
normalization rules, the dormant model-output wrapper grammar, and the exact
module boundaries justified by the first real semantic lane.

### Final-round direct answers

- **Strongest reasons not to approve:** no semantic production provider; no
  Omarchy-native or caret-inline product surface; no immutable current SHA,
  exact-SHA CI, or headed evidence.
- **Claims most likely to be overstated:** “600 ms” beyond the supported browser
  display lane; “unsafe shapes” beyond the enumerated policy cells; and
  “interaction review” based on CI rather than headed use.
- **What the team initially underweighted:** store-health coupling for per-origin
  revoke, page-controlled occlusion, and the amount of state-machine/model
  scaffold relative to four shipped phrases.
- **Least-supported positive conclusion:** CI could make the source reproducible
  for architecture critique, but it cannot make the interaction credible.
- **What fails first on a clean Omarchy laptop:** there is no installed broker,
  package, shell plugin, launcher, or native manifest; after manual staging,
  ordinary writing and non-fixture sites still produce no useful assistance.
- **Elegant in documentation but incomplete in code:** the artifact-bound model
  receipt and strict control-center contract; neither has a qualified production
  model or real Omarchy integration behind it.
- **Historical rather than current proof:** 1,000 insertion trials, 100 stale
  trials, 12.6/0.6 ms measurements, and all headed browser evidence.
- **Personal approval:** GrillMe would permit a frozen private architecture
  critique, not a product or interaction showing.

## Verification report

The lead ran all build/test commands serially; source-only reviewers ran none.
The companion [implementation handoff](2026-08-30-control-center-local-intelligence-handoff.md#verification-for-this-working-tree)
records the same gate in implementation context.

| Check | Exact result |
| --- | --- |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo test --workspace --all-features` | pass: 183 tests, 0 failed |
| `cargo +1.85.0 check --workspace --all-targets --all-features --locked` | pass |
| `npm run check` | pass: TypeScript compile, 112 tests, reproducible build check, live-script syntax, naming, documentation, and historical capability validation |
| `npm audit --audit-level=moderate` | pass: 0 vulnerabilities reported |
| Nine reusable QML files through `qmllint` | pass individually |
| Quickshell 0.3.1 isolated offscreen load | `Configuration Loaded`; intentional five-second timeout returned `124` |
| `git diff --check` | pass |

Focused checks added during the hostile round also passed:

- 72 Chromium tests covering controller and native-client timing/correlation;
- broker generation deadline, commit retirement, phrase abstention, boundary
  shape, language grammar, and quality-gate contract tests;
- nine reusable QML components through `qmllint`; and
- an isolated Quickshell 0.3.1 offscreen load that reached
  `Configuration Loaded` and ran until the intentional five-second timeout.

The offscreen load is parser/process evidence only. `npm ci` was not rerun
because installation remained prohibited. Durable browser evidence was not
rerun in the working checkout. One attempted temporary-load cleanup command was
blocked before execution by command policy; the successful runs left only
isolated `/tmp` XDG/log trees and touched no persistent Badi, browser, Omarchy,
or user configuration.

## Scope truth

### Works today

- private fail-closed settings/control authority;
- exact localhost Chromium textarea/input foundation;
- four explicit English phrase probes with strict silence elsewhere;
- addressed display/commit lifecycle, cancellation, pause, and stale fences;
- text-free optional aggregate counters;
- non-executing hardware/model fit advice; and
- a repo-local operator console loadable with Quickshell.

### Foundation only

- model receipt and local `llama.cpp` evaluation client;
- generic subject schema beyond the one editable fixture identity;
- adaptive-memory permission language;
- Omarchy/Quickshell UI design.

### Unsupported

- semantic production inference and learned writing style;
- arbitrary Chromium sites, contenteditable/framework editors, iframes,
  Obsidian, terminals, and general Linux applications;
- multilingual generated output, including Persian ZWNJ and emoji ZWJ;
- automatic model download, installation, activation, or fallback;
- clipboard/synthetic typing/global input capture; and
- an installed Omarchy plugin, launcher, service, or tray entry.

### Physical/headed proof still required

- caret-relative rendering, scrolling, zoom, DPI, hostile CSS, focus, native
  undo, and asynchronous acceptance in headed Chromium;
- real MV3 suspension/reconnect and native-host lifecycle;
- Omarchy shared-shell plugin loading, theme switching, Hyprland focus/placement,
  keyboard traversal, reduced motion, and screen-reader behavior; and
- local-model cold/warm latency, usefulness, memory pressure, cancellation, and
  power behavior on the named laptop.

## Minimal next actions

### Before private head-of-Omarchy architecture review

1. Owner reviews this diff and scope report.
2. Freeze one immutable review commit and require exact-SHA CI.
3. Present it explicitly as a trust/control foundation, not a product demo.

### Before a product showing

1. Deliver one genuinely useful, qualified local writing provider on a frozen
   corpus through the real schedule-to-visible lane.
2. Replace the field-width browser panel with headed-proven caret-inline UX.
3. Port the stable control surface into the official Omarchy shell/plugin and
   theme contracts, then validate it in a real Omarchy/Hyprland session.

### Before release

1. Add current commit-linked headed evidence and an honest compatibility matrix.
2. Resolve project licensing and package/name clearance.
3. Complete the model runtime ownership/authentication boundary and local
   artifact attestation.
4. Publish multilingual support only per tested output-policy cell.

## Recommended next action

Freeze a 100-case, content-private English inline-completion evaluation corpus
and its exact adapter-schedule-to-visible measurement contract. Do not download
or activate a model yet. This one bounded artifact turns “model quality” from a
schema claim into an executable product gate and tells the team whether the
existing model/runtime scaffold is useful or should be deleted.

## Final GrillMe verdict

> FINAL VERDICT (verbatim): The post-remediation tree is a stronger narrow trust foundation, but it is not ready for a head-of-Omarchy product or interaction showing: production still offers only four exact English probes on one localhost fixture, the control center is a standalone hard-themed Quickshell shell rather than an Omarchy plugin, and all durable performance and headed evidence predates the current uncommitted multi-thousand-line diff. Freeze and CI-attest it for a private architecture critique; require one qualified semantic provider, caret-inline headed Chromium proof, and a real Omarchy plugin before presenting Badi as a product.
