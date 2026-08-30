# Badi post-audit remediation handoff

> **Historical boundary:** this handoff describes the earlier remediation
> candidate before the later settings/control-center/personalization work in
> the current working tree. Its command results and “no remaining defect”
> statements must not be reused as verification of later changes. Use the
> current tree's control-center contract and fresh verification output instead.

Status: pre-freeze verification and residual-risk record
Audit baseline: `develop` at `b8d6786a451defed5e186c3ba1fdf90a29b7099e`
Promotion boundary: foundation-only merge after exact-SHA CI; not a product or release approval
Prepared: 2026-08-30
Audience: owner and private architecture reviewers

This document is the delta after the
[independent adversarial audit](2026-08-30-independent-adversarial-audit.md).
The audit remains an immutable account of `b8d6786`; none of the changes below
is represented as having existed in that commit or in its historical browser
receipt.

## Executive outcome

**The remediation is suitable for one immutable review commit and, after
exact-SHA CI, a foundation-only merge to `main`. It is also suitable, with
explicit scope disclaimers, for a private source-level architecture critique.
It is not ready to be sent to the head of Omarchy as an approval,
demonstration, or release candidate.**

The source is materially stronger than the audited baseline. Continuous
document checks, session retirement and authority leases, bounded connection
and provider work, resource-safe model advice, shared text fixtures, and
honest evidence modes close the most important confirmed defects. Independent
Rust and browser re-reviews found no remaining source-level correctness or
security defect that blocks a narrow private architecture discussion. Those
reviews are a contemporaneous process attestation summarized in the challenge
log, not an independently signed review artifact.

At handoff capture, the evidence boundary had not moved with the code: the
candidate was uncommitted, had no GitHub Actions run at its own SHA, and had not
been tested in a real Manifest V3 lifecycle or a headed Omarchy/Hyprland
session. The strict current-evidence gate correctly rejected it. Freezing the
tree and passing exact-SHA CI closes only that source-verification gate. A
service-worker restart can still lose inbound UI-control routes; a locally
paused controller may then have no outbound command with which to recover and
can remain stranded until reload or another re-registration trigger. Arbitrary
page occlusion also remains outside the proof. Those facts rule out a stronger
readiness claim.

This is also a large cross-cutting remediation, not “minor cleanup.” The owner
requested one immutable review snapshot; this handoff partitions that snapshot
by browser authority, broker lifecycle, model contract, and evidence integrity.
Later changes should return to smaller boundary-scoped commits.

## What changed

### Browser authority and lifecycle

- One predicate owns the exact top-level document contract: origin, path, and
  complete URL. It is checked before field adoption and again at request,
  display, keyboard, type-through, and commit boundaries
  ([`fixture-document.ts`](../../adapters/chromium/src/shared/fixture-document.ts#L1),
  [`field-controller.ts`](../../adapters/chromium/src/content/field-controller.ts#L318),
  [request](../../adapters/chromium/src/content/field-controller.ts#L677),
  [display](../../adapters/chromium/src/content/field-controller.ts#L751),
  [keyboard](../../adapters/chromium/src/content/field-controller.ts#L570),
  [type-through](../../adapters/chromium/src/content/field-controller.ts#L1145),
  [commit](../../adapters/chromium/src/content/field-controller.ts#L919)).
- Field eligibility is decided before text or selection acquisition. Hidden,
  inert, identity-ambiguous, offscreen, filtered, clip/clip-path hidden,
  masked, transformed, paint-contained, rectangular-overflow-clipped, and
  policy-denied fields fail closed;
  an eligible focused field can be safely re-adopted after a policy or CSS
  restoration
  ([`field-policy.ts`](../../adapters/chromium/src/content/field-policy.ts#L114),
  [`field-controller.ts`](../../adapters/chromium/src/content/field-controller.ts#L365),
  [`field-controller.ts`](../../adapters/chromium/src/content/field-controller.ts#L513)).
- A document-scoped session is registered without field content. One session
  may own a tab/frame route; a new document displaces and closes the prior
  session. Trusted outbound commands can reconstruct the route after an MV3
  worker restart
  ([`content-script.ts`](../../adapters/chromium/src/content/content-script.ts#L6),
  [`session-routes.ts`](../../adapters/chromium/src/background/session-routes.ts#L41),
  [`service-worker.ts`](../../adapters/chromium/src/background/service-worker.ts#L164)).
- Pause state is learned during the strict hello exchange before any field
  context is sent. Bootstrap has bounded event-driven recovery, and lifecycle,
  visibility, navigation, focus, policy, and transport invalidations advance or
  retire local authority
  ([`content-script.ts`](../../adapters/chromium/src/content/content-script.ts#L17),
  [`content-script.ts`](../../adapters/chromium/src/content/content-script.ts#L91),
  [pause](../../adapters/chromium/src/content/field-controller.ts#L196),
  [transport](../../adapters/chromium/src/content/field-controller.ts#L239),
  [visibility/focus](../../adapters/chromium/src/content/field-controller.ts#L475),
  [pagehide/navigation](../../adapters/chromium/src/content/field-controller.ts#L501),
  [policy mutation](../../adapters/chromium/src/content/field-controller.ts#L513)).
- `Shift+Tab` remains native navigation. Plain `Tab` and Ctrl/Command+Right are
  considered only for a visible broker-bound suggestion; final insertion still
  rechecks document, policy, focus, value, selection, revision, fingerprint,
  expiry, ghost visibility, and broker authorization
  ([`field-controller.ts`](../../adapters/chromium/src/content/field-controller.ts#L570),
  [authorization](../../adapters/chromium/src/content/field-controller.ts#L919),
  [final visibility](../../adapters/chromium/src/content/field-controller.ts#L1080)).
- The ghost now rejects common computed hiding mechanisms, host overflow/paint
  containment, panel collapse, and off-viewport geometry. This is a logical
  visibility guard, not a proof against arbitrary overlay or top-layer
  occlusion
  ([`ghost-view.ts`](../../adapters/chromium/src/content/ghost-view.ts#L39)).

### Native client and broker lifecycle

- Suggestion, commit-authorization, and global-control waits have a common
  three-second deadline. Timers clear on every terminal path; identical global
  controls coalesce and incompatible overlap rejects rather than accumulating
  work
  ([deadline construction](../../adapters/chromium/src/background/native-client.ts#L104),
  [three-second default](../../adapters/chromium/src/background/native-client.ts#L115),
  [`native-client.ts`](../../adapters/chromium/src/background/native-client.ts#L118),
  [`native-client.ts`](../../adapters/chromium/src/background/native-client.ts#L202),
  [`native-client.ts`](../../adapters/chromium/src/background/native-client.ts#L279)).
- Connections, sessions, event queues, wire queues, hello, idle, provider, and
  writer drain paths have explicit ceilings. One owned reader task preserves
  partial frame state while outbound events are multiplexed; shutdown aborts
  and joins the reader and drains or aborts the writer
  ([`server.rs`](../../broker/src/server.rs#L25),
  [`server.rs`](../../broker/src/server.rs#L230),
  [`server.rs`](../../broker/src/server.rs#L327)).
- Provider work has broker-wide admission with a default of four and a hard cap
  of sixteen. Cancellation, timeout, and shutdown are explicit, and no new
  provider work is admitted after terminal shutdown
  ([default/hard cap](../../broker/src/engine.rs#L30),
  [configuration clamp](../../broker/src/engine.rs#L224),
  [`engine.rs`](../../broker/src/engine.rs#L342),
  [`engine.rs`](../../broker/src/engine.rs#L485)).
- Clean close, connection loss, a three-second context-silence lease, pause,
  supersession, and terminal failed/stale/blocked commit results revoke retained
  context and edit authority
  ([clean close](../../broker/src/engine.rs#L314),
  [`engine.rs`](../../broker/src/engine.rs#L355),
  [`engine.rs`](../../broker/src/engine.rs#L989)).
- `badictl` validates hello correlation, negotiated protocol and limits,
  response type, request ID, control action, reason, and action-specific paused
  state before accepting a response
  ([hello validation](../../broker/src/bin/badictl.rs#L146),
  [control validation](../../broker/src/bin/badictl.rs#L241)).

### Text, hardware, and model contracts

- Rust and TypeScript consume one multilingual accept-word fixture. Chromium
  uses `Intl.Segmenter` for word and grapheme boundaries, including the tested
  combining-mark, emoji-ZWJ, Persian, Arabic, and Han cases. This is a shared
  tested corpus, not a claim of universal Unicode equivalence
  ([`accept-word-fixtures.json`](../../protocol/v1/accept-word-fixtures.json#L1),
  [`segment.rs`](../../broker/src/segment.rs#L128),
  [`context.test.ts`](../../adapters/chromium/test/context.test.ts#L76)).
- Model advice is an explicit `candidate | no_fit` V2 sum type. Both branches
  require `runtime_ready: false`; a candidate carries an exact pinned artifact
  and a non-executing download plan, while `no_fit` carries neither
  ([`model_selection.rs`](../../broker/src/model_selection.rs#L259),
  [`badi.model-advice.v2.schema.json`](../../broker/schemas/badi.model-advice.v2.schema.json#L1)).
- Selection subtracts a 2 GiB host reserve from both total and currently
  available memory, then accounts for artifact bytes and runtime headroom.
  Unknown architecture or memory fails closed; unknown power, battery,
  non-AVX2, and hybrid GPU states conservatively cap the tier
  ([reserve arithmetic](../../broker/src/model_selection.rs#L329),
  [unknown memory](../../broker/src/model_selection.rs#L365),
  [tier caps](../../broker/src/model_selection.rs#L380)).
- GPU detection and backend-validated usable capacity are separate. The direct
  `nvidia-smi` child has a two-second best-effort deadline, a 16 KiB output cap,
  and kill/reap handling
  ([GPU contract](../../broker/src/model_selection.rs#L68),
  [detection](../../broker/src/model_selection.rs#L531),
  [bounded child](../../broker/src/model_selection.rs#L610),
  [deadline limitation](../architecture/model-selection.md#L48),
  [usable-capacity boundary](../architecture/model-selection.md#L59)).
- The six catalog artifacts remain revision-, filename-, byte-, digest-,
  quantization-, and license-pinned to official Qwen/Hugging Face sources. The
  Apache-2.0 Qwen2.5-Coder 7B entry avoids the non-commercial 3B release. No
  candidate is described as having passed Badi's runtime quality gate
  ([model verification](2026-08-30-independent-adversarial-audit.md#model-catalog-verification),
  [`model-selection.md`](../architecture/model-selection.md#L91)).

### Evidence and handoff integrity

- Ordinary receipt validation is deliberately historical. V2 reads linked
  blobs from the receipt's recorded commit and reports
  `current_links=not-checked`; unanchored V1 is identified separately
  ([historical validation](../../scripts/check-capabilities.mjs#L391),
  [recorded-commit reads](../../scripts/check-capabilities.mjs#L422),
  [stable mode output](../../scripts/check-capabilities.mjs#L824)).
- Strict current mode additionally compares generated adapter artifacts and
  every current file under `broker/` and `protocol/v1/`, plus the workspace
  Cargo inputs, byte for byte with the receipt's clean recorded commit
  ([input collection](../../scripts/check-capabilities.mjs#L141),
  [byte comparison](../../scripts/check-capabilities.mjs#L185),
  [current-link validation](../../scripts/check-capabilities.mjs#L239),
  [evidence guide](../../capabilities/README.md#validation-modes)).
- A durable browser run refuses a dirty tree, records its starting commit, and
  rechecks a clean tree and unchanged `HEAD` before writing evidence. Durable
  mode requires a new explicit ID, binds that ID to the filename, refuses an
  existing target, and creates the file with exclusive-write semantics. Smoke
  output may be dirty because it is diagnostic and cannot satisfy the promoted
  receipt schema
  ([durable-ID policy](../../adapters/chromium/live/run-live.mjs#L54),
  [clean-tree capture](../../adapters/chromium/live/run-live.mjs#L176),
  [overwrite guard](../../adapters/chromium/live/run-live.mjs#L1320),
  [unchanged-HEAD check](../../adapters/chromium/live/run-live.mjs#L1545),
  [exclusive write](../../adapters/chromium/live/run-live.mjs#L1635),
  [`schema.json`](../../capabilities/v2/schema.json#L137)).
- The historical p95 figures are consistently labeled as historical. The
  README points to both the immutable baseline audit and this remediation
  boundary. The same-UID decision is explicit rather than called
  authentication
  ([`README.md`](../../README.md#current-status),
  [`VISION-V2.md`](../../VISION-V2.md#L437),
  [ADR 0001](../decisions/0001-same-uid-trust-boundary.md)).

## Audit-finding disposition

| Baseline finding | Current disposition | Remaining truth |
| --- | --- | --- |
| H1: lifecycle invalidation retained broker authority | Remediated in source and tests | Real renderer/worker crash and restart have not been reproduced against this tree. |
| H2: exact-document authority was not continuous | Remediated for the one declared document | This does not broaden support beyond the exact localhost fixture. |
| H3: `Shift+Tab` accepted all | Remediated and regression-tested | A plain Tab denied after asynchronous authorization remains a no-op; no synthetic replay is allowed. |
| H4: evidence and headline claims were stale or mutable | Partially remediated | Historical evidence is now labeled and immutable; no current live receipt exists. |
| H5: reviewers landed on the wrong tree | Remediated once promoted | The commit containing this handoff is the review identity; the baseline audit remains pinned to `b8d6786`. |
| M1: Rust/TypeScript word semantics diverged | Remediated for one shared corpus | Do not generalize the corpus into a universal-language claim. |
| M2: type-through fake omitted real clear behavior | Remediated at source/unit boundary | The current native/browser chain has not reproduced it. |
| M3: hello pause state was discarded | Remediated and regression-tested | Worker-restart inbound route loss remains; cross-process policy epochs are release work. |
| M4: model advice could overstate fit | Remediated as conservative offline advice | Runtime latency, memory, prompts, and usefulness remain unmeasured. |
| M5: same UID was described as authentication | Resolved by explicit ADR | Malicious same-UID processes remain an accepted local residual risk. |
| M6: page CSS could hide a logical ghost | Partially remediated | Common hiding fails closed; arbitrary occlusion requires headed/adversarial testing. |
| M7: connections and provider work were unbounded | Remediated with hard ceilings | A synchronously blocking future still requires backend offload; same-session supersession may transiently report busy. |
| M8: response and JSON contracts were nominal | Remediated for CLI and model/hardware output | JSON Schema owns structure; producer tests own cross-field arithmetic invariants. |
| M9: implemented visual loop did not match the vision | Open by design | The card ghost is an M2 engineering surface, not final product design. |
| M10: no project license | Open owner decision | Blocks public release, not private architecture review. |
| L1: historical binary hashes were not reproduced | Not reproduced | No current binary-performance claim is made. |
| L2: focused field was not adopted at startup | Remediated and regression-tested | Background-window acquisition remains fail-closed. |
| L3: event failure could create phantom broker state | Remediated and regression-tested | Health remains a point-in-time local snapshot, not authentication. |
| L4: documentation debt | Substantially remediated | Public release notes and changelog remain premature. |

## Remaining findings

### Critical

No Critical findings remain in the source-reviewed candidate.

### High

#### Immutable verification identity required before promotion

**Classification:** verification and handoff risk.
**Evidence:** at handoff capture, `HEAD` was still the audited baseline while
the remediation was a working-tree diff; the latest GitHub success and live
receipt both belonged to `b8d6786`. Strict current evidence rejected the
changed adapter and Rust-chain inputs.
**Impact:** until frozen and tested, a reviewer cannot bind the changes and CI
result to one immutable revision. The historical browser receipt must never be
presented as proof of the remediation.
**Smallest fix:** create one review commit and require CI at that exact SHA.
Produce a new live receipt only if current performance or browser behavior will
be claimed.
**Resolution criterion:** the commit containing this handoff plus successful
exact-SHA CI closes the source-promotion portion of this finding; it does not
close headed-browser or product-readiness gaps.
**Blocks:** merge until the resolution criterion is met, and head-of-Omarchy or
release framing until the remaining headed gates pass; not local source review.

### Medium

#### MV3 restart loses inbound UI routes until outbound recovery

**Classification:** availability/lifecycle defect.
**Evidence:** routes exist only in worker memory
([`service-worker.ts`](../../adapters/chromium/src/background/service-worker.ts#L22)).
After bootstrap, content recovery listeners are removed
([`content-script.ts`](../../adapters/chromium/src/content/content-script.ts#L62)).
An outbound trusted command can re-register the route, but inbound pause,
revocation, clear, and disconnect delivery depends on the missing registry
([`service-worker.ts`](../../adapters/chromium/src/background/service-worker.ts#L35),
[`service-worker.ts`](../../adapters/chromium/src/background/service-worker.ts#L72)).
**Impact:** UI state may remain stale until a later page command or local
expiry. A locally paused controller may emit no command at all after a missed
resume and remain inert until reload. Broker authorization still fails closed,
so this is not stale edit authority.
**Smallest fix:** prove a minimal document re-registration mechanism across
worker suspension/restart in real Chromium before claiming full M2 lifecycle
support.
**Blocks:** full M2 and release; not a narrow source-architecture critique.

#### Hostile-page occlusion is not proven

**Classification:** security/product risk on any future broad-site surface.
**Evidence:** computed hiding and geometry are checked
([`ghost-view.ts`](../../adapters/chromium/src/content/ghost-view.ts#L39)), but
the page-selected host cannot reliably prove absence of arbitrary overlap or
top-layer occlusion. Commit validation therefore proves logical visibility,
not what a human can see.
**Impact:** expanding beyond the controlled fixture could let a page obscure
the suggestion while a genuine shortcut remains eligible.
**Smallest fix:** retain exact-document scope and add headed adversarial
occlusion cases before any origin expansion.
**Blocks:** broad-site support and release; neither current fixture source
review nor broker architecture review.

#### Acceptance shortcuts can be consumed when authorization is denied

**Classification:** accepted UX risk, not a hidden input fallback.
**Evidence:** after synchronous document, policy, visibility, selection,
constraint, and duplicate-authorization validation succeeds, trusted plain Tab
or Ctrl/Command+Right is prevented before asynchronous broker authorization completes
([`field-controller.ts`](../../adapters/chromium/src/content/field-controller.ts#L594)).
The code explicitly refuses to synthesize or replay navigation or caret movement.
**Impact:** a late denial leaves the field unchanged but converts one genuine
Tab navigation or word-wise caret movement into a no-op.
**Smallest fix:** document and test the product decision; change the shortcut
before release if “never steals a native key action” is a requirement.
**Blocks:** unqualified keyboard-UX claims; not architecture review.

#### Model recommendations are candidates, not validated providers

**Classification:** product-readiness risk.
**Evidence:** the broker executable constructs the deterministic phrase
provider rather than a model loader
([`main.rs`](../../broker/src/main.rs#L6),
[`main.rs`](../../broker/src/main.rs#L17)); every advice branch reports
`runtime_ready: false`
([`model_selection.rs`](../../broker/src/model_selection.rs#L259)), and no
candidate is documented as having passed the latency, memory, prompt, or
quality gate.
**Impact:** model fit arithmetic cannot establish responsive inline completion
or writing quality.
**Smallest fix:** keep the boundary advisory and make no semantic-inference
claim until a named candidate passes the frozen runtime gate.
**Blocks:** semantic product claims and release; neither architecture review nor
offline JSON review.

### Low

- Same-session request supersession can transiently return `ProviderBusy` at a
  fully saturated provider because cancellation precedes immediate
  non-blocking permit acquisition
  ([`engine.rs`](../../broker/src/engine.rs#L485)). This is bounded
  availability behavior, not stale authority.
- Bootstrap stops after three foreground, event-driven retries
  ([`content-script.ts`](../../adapters/chromium/src/content/content-script.ts#L91)).
  Repeated transient failures can leave the document inert until reload.
- Abrupt silence revokes context authority but deliberately retains an empty,
  reusable session. Repeated renderer losses on one still-live native
  connection can therefore consume the 64-session connection allowance until
  the connection closes; a safe reaper needs an adapter liveness/reopen
  contract
  ([`engine.rs`](../../broker/src/engine.rs#L355),
  [`server.rs`](../../broker/src/server.rs#L25)).
- A timed-out global control has an uncertain remote outcome: a late broker
  pause or resume reply is ignored after the local operation is settled. The
  deadline bounds local memory, but complete resolution needs reconnect/state
  reconciliation or protocol-level idempotency
  ([`native-client.ts`](../../adapters/chromium/src/background/native-client.ts#L279)).
- The `nvidia-smi` deadline is best-effort rather than an absolute process-tree
  wall clock; kill/reap and an inherited stdout descriptor could outlive the
  direct-child timeout. The limitation is documented
  ([`model-selection.md`](../architecture/model-selection.md#L52)).
- The partial-frame server regression uses a short debounce rather than an
  explicit reader-consumed-byte handshake
  ([`server.rs`](../../broker/src/server.rs#L845)). The implementation itself
  uses one owned reader and is structurally cancellation-safe.

## Verification record

All build and test commands are serialized under one verifier. No durable
browser-evidence command is run in this working checkout.

This table is the lead verifier's contemporaneous command log. Most commands
were captured while `HEAD` still named the baseline; documentation and naming
checks were rerun while finalizing the review identity. It is not a signed
transcript. The GitHub Actions run for the eventual review commit is the
authoritative, reproducible source-verification gate; the historical receipt
remains authoritative only for its own baseline browser run.

| Command | Local review result |
| --- | --- |
| `git rev-parse HEAD` | `b8d6786a451defed5e186c3ba1fdf90a29b7099e` |
| `git branch --show-current` | `develop` |
| `git diff --check` | Exit 0; no whitespace errors. |
| `cargo fmt --all --check` | Exit 0. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0. |
| `cargo test --workspace` | Exit 0; 111 passed, 0 failed, 0 ignored: 79 library, 2 broker CLI, 3 native host, 2 manifest, 9 `badictl`, 2 model CLI, 4 model schema/invariant, 2 native bridge, 2 manifest integration, 4 protocol schema, and 2 shutdown tests. |
| `cargo +1.85.0 check --workspace --all-targets --locked` | Exit 0. |
| `npm ci` | Exit 0 earlier in the remediation session; zero audited vulnerabilities. It was not repeated after source-only edits because the dependency graph and `package-lock.json` did not change. |
| `npm run check` | Exit 0; typecheck; 99 tests in 7 files; two deterministic three-file builds; live-runner syntax; naming across 133 tracked files; 138 local Markdown links; historical evidence validation. |
| `npm audit --audit-level=moderate` | Exit 0; `found 0 vulnerabilities`. |
| `rg -n -i 'wtype\|xdotool\|ydotool\|evdev\|uinput\|navigator\.clipboard\|clipboard\|synthetic typing\|send[_-]?keys' broker/src adapters/chromium/src` | Exit 1 with no matches; the reviewed production source contains no named synthetic-input or clipboard path. |
| `node adapters/chromium/live/run-live.mjs` | Exit 1 before any build/browser work: durable mode requires an explicit unique `--evidence-id`. |
| `npm run capabilities:check:current` | Exit 1 by design; both the adapter artifact identity and complete Rust-chain input identity differ from the historical receipts. |
| `gh run list --branch develop` | At handoff capture, [run 33306824009](https://github.com/ahuray/badi/actions/runs/33306824009) was the latest success and belonged to baseline SHA `b8d6786`; no run existed for the working tree. |
| `gh pr list --base main --head develop` | At handoff capture, output was `[]`; no review PR existed. |

The ordinary evidence checker reported exactly:

```text
Validated 2 capability receipt(s) (mode=historical, v2_recorded_commit=1, v1_unanchored=1, current_links=not-checked).
Current linked sources and generated artifacts were not checked; use --require-current for that gate.
V1 receipts have no recorded commit or raw run; their historical validation covers only schema and safe declared paths.
```

The strict current gate reported exactly:

```text
chromium-dom-foundation.v1.json: artifact hashes differ from the generated build manifest
chromium-native-live.v2.json: current Rust-chain source/build/test input set differs from recorded commit 068db9fe389fd7777bd903021b9c2baf3bde5140
```

That failure is required integrity behavior. It is not a reason to rewrite the
old raw evidence or receipt under the same identity.

One intermediate focused run of the newly added native-client deadline tests
failed because the tests did not flush fake-timer microtasks before inspecting
timer state. The tests were corrected without changing the production design;
the focused file then passed 19/19 and the complete TypeScript lane passed
97/97 at that stage. A later shortcut-hardening run initially failed one legacy
expectation that a second shortcut should also be consumed while authorization
was pending; the production behavior and test were aligned to preserve that
second native key, after which the focused controller file passed 46/46. The
final 99/99 full-lane result in the table supersedes both intermediate totals.
No failure is omitted from the handoff.

The first pushed review candidate, `599c4bc1fbbbe64f97cc677117f28a7083249f4c`,
also exposed a clean-checkout-only documentation issue: both Chromium jobs in
[run 33322243108](https://github.com/ahuray/badi/actions/runs/33322243108)
passed typechecking, all 99 tests, and deterministic builds, then failed because
the name checker treated three historical former-name occurrences in the newly
tracked immutable audit as current product naming. The correction count-locks
exactly those occurrences to that one audit file; any count drift still fails.
The superseded SHA is not the final review identity.

The durable live command and disposable browser smoke command were both
skipped. The former would create new durable evidence from a dirty checkout;
the latter would still not prove headed Manifest V3 or Omarchy behavior and was
not needed to validate the source-only deadline change.

## Current-machine model result

This is a contemporaneous `badictl` stdout observation; raw probe JSON was not
promoted as a repository artifact. The final post-remediation probe observed
x86-64, 20 logical CPUs, AVX2,
15,663 MiB total memory, 11,105 MiB available at the hardware sample, Intel
integrated graphics, no backend-validated usable GPU memory, and
`on_battery: null`. Unknown power correctly caps the result at balanced. The
two following samples returned:

- writing: Qwen3 1.7B Q8_0, 9,049 MiB usable and 2,956 MiB required host
  memory;
- code: Qwen2.5-Coder 1.5B Q4_K_M, 2,101 MiB conservative host-memory
  requirement from 9,047 MiB usable host memory.

Both were `badi.model-advice.v2` candidates with `runtime_ready: false`.
`badictl` calls hardware detection and recommendation only
([`badictl.rs`](../../broker/src/bin/badictl.rs#L42)); its download plans are
data, not execution. The lead operator did not invoke a weight download or
model execution command. Available memory is live input, so this observation
is neither a stable machine guarantee nor independently reproducible from the
repository alone.

## Architecture assessment

### What is now strong

- The adapter performs the sole reviewed page mutation through `setRangeText`
  after final authorization and constraint checks
  ([`field-controller.ts`](../../adapters/chromium/src/content/field-controller.ts#L975)).
  The broker exposes a text completion-provider interface and grants a
  revision-bound commit; the production-source search in the verification
  table found no named synthetic-input or clipboard path
  ([`provider.rs`](../../broker/src/provider.rs#L32)).
- Authority is addressed by session, focus epoch, revision, fingerprint, and
  suggestion ID, then checked again at the mutation boundary.
- Native messaging and Unix-socket framing are explicit and bounded. The
  reader and writer are owned through teardown; event and provider work has
  bounded admission plus cancellation and shutdown paths. A synchronously
  blocking provider still requires backend offload.
- Model selection is a content-free, offline, versioned advisory boundary. The
  executable still installs only the deterministic provider
  ([`main.rs`](../../broker/src/main.rs#L17)); advice construction is not an
  installer, model loader, or hidden runtime switch
  ([`model_selection.rs`](../../broker/src/model_selection.rs#L259)).
- Historical evidence and current-tree validation are different commands with
  different, stable output. Drift now fails visibly.

### Complexity that must not grow

The real complexity is concentrated in two places: Chromium authority spans a
controller, service worker, native port, and broker; broker authority spans
context, provider, visible suggestion, commit, and lease timers. Those are
genuine distributed lifecycle costs, but the product has not earned another
generic policy bus, framework abstraction, runtime-model registry, or
Omarchy-patching layer.

The next change should favor deletion or one narrow mechanism. In particular,
MV3 recovery should not become a general service-worker persistence framework,
and policy epochs should not become a cross-platform orchestration subsystem.

### Linux and Omarchy fit

XDG runtime paths, Unix sockets, peer UID checks, native messaging, content-free
hardware JSON, and `badictl` are sensible Linux-native seams. The versioned JSON
is a credible future Quickshell/Omarchy boundary because consumers can render a
candidate or `no_fit` without embedding hardware policy. Nothing in the
candidate addresses an Omarchy-owned path; the stronger assertion that no
external configuration changed is an operator attestation below, not something
the Git tree alone can prove.

That fit is architectural, not experiential. Focus, workspaces, IME,
accelerators, permission consent, native undo, battery behavior, packaging, and
visual integration still require physical headed validation.

## Scope truth

### Works in source and deterministic tests

- bounded Rust broker, policy, lifecycle, deterministic provider, native host,
  and `badictl` protocol behavior;
- one exact top-level `http://localhost:4173/chromium.html` textarea surface;
- pre-acquisition field denial, suggestion display/clear, exact type-through,
  word/all authorization, dismissal, pause bootstrap, session close, stale
  rejection, and shutdown paths;
- offline hardware reporting and conservative, versioned model candidates;
- historical receipt validation with explicit provenance limits.

### Foundation only

- the current native Chromium chain beyond the historical `b8d6786` receipt;
- the card ghost, shortcut ergonomics, and global pause synchronization;
- hardware/model JSON as a future Omarchy or Quickshell consumer boundary;
- the deterministic phrase provider as a lifecycle fixture.

### Unsupported

- semantic inference or runtime model loading;
- arbitrary sites, subframes, framework editors, `contenteditable`, broad
  Chromium compatibility, and hostile pages;
- Obsidian/CodeMirror, terminal/Fcitx, Ghostty, and other native applications;
- synthetic typing, clipboard insertion, global input capture, or hidden
  automation fallback;
- confidentiality against malicious processes already running as the same UID.

### Requires physical/headed Omarchy validation

- Manifest V3 permission consent, service-worker suspension/restart,
  `documentId` behavior, renderer crash, native disconnect, and reopen;
- Hyprland workspace and focus changes, background visibility, Tab and command
  accelerators, IME/composition, native undo, and accessibility;
- zoom, scrolling, clipping, overlay/top-layer occlusion, hostile CSS, and final
  visual placement;
- battery, integrated/hybrid GPU, and low-memory behavior on named target
  laptops.

## GrillMe challenge log

### Initial challenge

GrillMe rejected the first remediation synthesis for overstating its status. Its
verbatim verdict was:

> I do not approve presenting this working tree to the head of Omarchy as a review-ready remediation. It is an uncommitted, cross-cutting candidate with no CI or live-browser evidence at its own SHA; pause/resume is broken on the supported fixture, bootstrap can leave pages permanently inert, Tab can be consumed without insertion, provider concurrency remains unbounded, model advice does not preserve its claimed host reserve, and the handoff materially overstates Unicode and evidence coverage. The tree is useful only as an explicitly unfinished internal critique target until those defects and claims are corrected; “ready after minor handoff cleanup” is not supported.

### Challenge resolution

| GrillMe objection | Resolution |
| --- | --- |
| Pause/resume and paused-at-load were broken | Bootstrap now returns hello pause state before content acquisition; pause/resume advance revision and focused fields can be re-adopted. |
| Bootstrap could leave pages permanently inert | Successful background bootstrap is retained until eligible; failed bootstrap gets three foreground event-driven attempts. The bounded-retry residual remains explicit. |
| A shortcut could be consumed before insertion was possible | `Shift+Tab` remains native, impossible insertions are rejected synchronously, and a duplicate key during pending authorization remains native. A late asynchronous denial can still turn plain Tab or Ctrl/Command+Right into a no-op because synthetic replay is prohibited. |
| Provider concurrency was unbounded | Global admission defaults to four, clamps at sixteen, and races cancellation, timeout, and shutdown. Same-session transient busy remains explicit. |
| Host reserve was not preserved | Both total and available memory subtract 2 GiB; artifact-specific arithmetic and saturation are tested. |
| Unicode coverage was overstated | Both languages consume one named multilingual corpus; all universal-equivalence wording was removed. |
| Evidence proved less than claimed | Historical and current modes are separate; strict mode covers the complete Rust input set; durable evidence requires a clean, unchanged commit. The current tree intentionally has no new receipt. |
| Pending native operations could grow indefinitely | Post-hello suggestion, commit, and global-control waits are timed; identical controls coalesce and incompatible overlap rejects. |
| A future durable run could overwrite its historical raw file | Durable mode now requires a new ID/filename, refuses an existing target before work, and uses an exclusive create for the final write. |

### Final GrillMe round

GrillMe reopened the current source and tests without editing or running the
verification lane. It confirmed that ancestor `contain: paint | content |
strict` now fails before context capture
([`field-policy.ts`](../../adapters/chromium/src/content/field-policy.ts#L152),
[`field-controller.ts`](../../adapters/chromium/src/content/field-controller.ts#L693))
and that the `contain: paint` regression observes zero value reads and zero
transport requests
([test case](../../adapters/chromium/test/field-controller.test.ts#L380),
[assertions](../../adapters/chromium/test/field-controller.test.ts#L463)).
It found no new blocker introduced by the conservative denial. Exercising the
other two tokens separately would be low-priority test hardening, not a missing
code branch.

Its three strongest reasons not to approve a head-of-Omarchy send are:

1. The current work has no immutable SHA, exact-SHA CI, or headed receipt.
2. Real MV3 restart, shortcut, and human-visibility behavior remains unproved;
   the paused-controller route loss is still a concrete lifecycle risk.
3. The implemented surface is one exact localhost deterministic fixture, not a
   packaged semantic co-writer or an actual Omarchy integration.

The claims most likely to be overstated are that local source checks establish
browser/Omarchy readiness, that reconstructing a route on an outbound command
restores every missed inbound pause transition, and that logical ghost
visibility proves human-visible placement. Performance results, model
responsiveness, and GitHub status must remain historical or unvalidated: the
[1,000 insertion trials and 100 stale trials](../../capabilities/evidence/chromium-native-live-run.v1.json#L286),
[12.6/0.6 ms p95 figures](../../capabilities/chromium-native-live.v2.json#L230),
and [run 33306824009](https://github.com/ahuray/badi/actions/runs/33306824009)
belong to `b8d6786`, while no catalog candidate has passed the runtime quality
gate.

Three tests can remain green while the real product is broken: jsdom geometry
tests cannot exercise compositor or top-layer occlusion; mocked worker tests do
not reproduce MV3 suspension; and the complete Rust lane can pass while the
only provider remains
[four deterministic phrase rules](../../broker/src/provider.rs#L65). GrillMe expects a busy
external reviewer to ask which immutable SHA was tested, where the useful
Omarchy-integrated co-writer is, and why this state-machine surface should be
reviewed before headed restart, IME, undo, accessibility, packaging, and model
quality proof.

#### Disagreements and resolution

| Disagreement | Evidence resolution |
| --- | --- |
| The browser pass initially treated common clipping denial as sufficient; GrillMe identified ancestor paint containment. | The policy now rejects `paint`, `content`, and `strict` containment before capture, and the instrumented `paint` test records zero reads. GrillMe's source re-check marks the defect resolved. |
| Route reconstruction was initially summarized as MV3 recovery. | Source tracing showed that only outbound traffic reconstructs the in-memory route; a locally paused controller can miss resume and emit no such traffic. The report retains this as a Medium lifecycle finding pending headed reproduction. |
| The asynchronous-denial residual was initially described as a Tab-only issue. | Both keyboard branches consume the trusted event before awaiting the same authorization path. The finding and checklist now cover Tab and Ctrl/Command+Right. |
| Passing local tests was at risk of being treated as current product evidence. | The local 111/99 results remain useful source verification, but the strict current gate fails and every live performance/CI statement is explicitly tied to the baseline commit. |

No disagreement was resolved by vote. The remaining questions are whether a
minimal restart-safe re-registration mechanism works in real Chromium, whether
either acceptance shortcut may deliberately sacrifice its native action after
late denial, whether the ghost is actually visible under hostile stacking and
top-layer cases, how the large boundary changes should be split for review, and
which license and semantic runtime the owner will eventually choose.

GrillMe's answers to the final round are correspondingly direct: the least
supported conclusion would be real-browser or Omarchy readiness; installation
or connectivity fails first on an ordinary laptop, followed by MV3 lifecycle
behavior in a prepared developer setup; restart-durable routing, model loading,
and an Omarchy consumer look complete only at the document-contract level; and
the controller, engine, mapper, and model-selection surface is more code than
one fixture and a four-rule provider currently earns. GrillMe would approve
only an explicitly private owner/source critique.

This verdict was issued against the pre-freeze working tree. A review commit
and exact-SHA CI close its identity objection only; its headed and product-scope
objections remain. GrillMe's final verdict, verbatim:

> The current Badi remediation has closed the concrete pre-acquisition paint-containment gap, and I find no new blocker introduced by that fix, but I still do not approve sending this working tree to the head of Omarchy. It remains a large uncommitted candidate with no immutable review SHA, exact-SHA CI, or headed Omarchy/MV3 proof; worker restart can strand a paused controller, late authorization denial can consume either acceptance shortcut without performing its native action, and arbitrary human-visible placement and occlusion remain unproved. Because the implemented surface is still one exact localhost deterministic phrase fixture with no semantic provider, packaging, license, or actual Omarchy integration, it is appropriate only for owner/private source critique until a frozen commit passes CI and the headed lifecycle gate.

### Foundation-only promotion decision

For a source promotion to `main`, the lead explicitly accepts—not resolves—the
MV3 paused-route and asynchronous shortcut residuals. They remain blockers for
full M2, broad-origin, product-demonstration, and release claims. The promotion
makes no current browser-performance claim, so it must not create or rewrite a
durable receipt merely to make strict current validation green. The owner
requested one immutable snapshot; the audit and this handoff provide the
boundary-by-boundary review map for that deliberately large commit.

## Minimal handoff checklist

### Required before head-of-Omarchy review

- owner-review the cross-cutting diff and freeze it in an immutable review SHA;
- attach passing CI to that exact SHA;
- perform a headed exact-document Chromium/Omarchy lifecycle pass, including
  worker restart, route recovery, focus/workspace changes, Tab denial, IME,
  native disconnect, and hostile visibility cases;
- present this as an incomplete M2 architecture surface, not a finished
  co-writer or broad Chromium integration.

### Required before merging to `main`

- freeze exactly the reviewed repository delta in one `develop` commit;
- require all four push and pull-request CI jobs to pass at that exact head SHA;
- confirm `main` has not advanced unexpectedly and merge with a head-SHA guard;
- keep `main` as the default branch and make its repository description,
  README entry point, and foundation-only scope consistently describe Badi;
- retain `develop`; do not fold unrelated dependency pull requests into this
  promotion.

### Required before release

- choose a project license and complete package/trademark review;
- implement and pass semantic-provider quality, latency, memory, cancellation,
  prompt, and privacy gates before enabling inference;
- add native-host packaging, upgrade, rollback, and removal paths owned by the
  package or user—not `/usr/share/omarchy`;
- complete the headed device/application matrix for every advertised surface;
- implement cross-process policy epochs and validate the final visual and
  accessibility behavior.

### Optional improvements

- synchronize the partial-frame regression with an explicit reader handshake;
- expose a dedicated provider-busy reason if consumers need to distinguish
  saturation from provider failure;
- add changelog/release notes once there is a first reviewable release;
- evaluate Obsidian and terminal cells only after the exact Chromium boundary
  is complete.

## Change-safety statement

The following is a lead-operator attestation about actions taken during the
audit and remediation, not a comprehensive forensic comparison of host state.
The remediation and its verification changed only files inside the Badi
repository plus ignored build/dependency outputs. The authorized promotion may
change this repository's Git/GitHub history and descriptive metadata; GitHub
history and Actions, not this static document, record whether that occurred.
No command in the work modified
`/usr/share/omarchy`, user Omarchy/Hyprland configuration, the real Chromium
profile, native-host installation paths, packages, services, or model caches.
No model-weight download was invoked, and no project license was selected.

## Recommended next action

After exact-SHA CI and the foundation-only merge, reproduce the paused-controller
worker-restart sequence once in a disposable headed Chromium/Omarchy session.
Treat that as a lifecycle test, not permission to make a new performance claim.
