# Badi independent adversarial audit

Status: immutable audit snapshot
Audit date: 2026-08-30
Review branch: `develop`
Reviewed commit: `b8d6786a451defed5e186c3ba1fdf90a29b7099e`
Reviewed tree: `cc751e82d3e518473c6d683a0c4b9b101ea3c672`

Evidence-link policy: repository source links are pinned to the reviewed commit
so later remediation cannot silently change the cited lines. Before the audit's
first publication, only those link targets and this provenance note were added;
the findings, classifications, command record, and verdict were not rewritten.

This document records the findings against the exact tree above. It is not a
rolling status page and must not be rewritten to make later remediation appear
to have existed in the audited tree. Current remediation and reviewer handoff
belong in a separate document.

## Executive verdict

**Requires material engineering before review.**

Badi contains a credible, deliberately narrow broker, native-messaging bridge,
and Chromium foundation. The reviewed Rust and TypeScript checks pass, its
transport boundaries are generally careful, and the product does not contain a
clipboard or synthetic-keystroke fallback.

It was not ready to be presented as an Omarchy-aligned product or approval
candidate at the reviewed commit because:

- browser lifecycle changes did not revoke broker-held context and session
  authority end to end;
- exact-document enforcement was not continuous through same-document
  navigation, display, and mutation;
- `Shift+Tab` was intercepted as accept-all;
- Rust and TypeScript disagreed on valid word segmentation;
- model advice could exceed available memory and no candidate had passed a
  runtime quality gate;
- the latest live evidence was historical, mutable under stable IDs, and
  contradicted by headline documentation; and
- the public default branch still presented the former Omatype product, with no
  review pull request or concise reviewer handoff.

The tree was suitable for a private architecture critique, but not for an
approval request, product demonstration, or release claim.

## Scope and method

Seven independent specialist passes covered:

1. Rust architecture, policy, state, provider, IPC, and CLI behavior;
2. Manifest V3, controller lifecycle, DOM policy, transport, and commit logic;
3. Git, CI, builds, tests, evidence, hashes, and reproducibility;
4. hardware detection, model selection, official model metadata, and licensing;
5. product vision, milestone status, Linux/XDG behavior, and Omarchy fit;
6. security and privacy trust boundaries; and
7. documentation, naming, reviewer discoverability, and release handoff.

The first passes were completed independently. A separate GrillMe agent then
received the provisional synthesis and was instructed to assume it was wrong.
Material disagreements were resolved by source inspection, a second primary
source, reproduction where allowed, or an explicit unresolved classification.

Only the verification specialist ran builds and tests. They ran in an isolated
temporary clone of the reviewed commit. The durable browser-evidence runner was
not rerun because doing so would rewrite the evidence under review.

## Findings

### Critical

No Critical findings.

No remote execution, credential-exfiltration, model-execution, clipboard
insertion, or synthetic-keystroke fallback path was found. The most serious
issues were local lifecycle/authority defects and misleading handoff evidence,
so High was the appropriate severity ceiling.

### High

#### H1. Browser invalidation leaves broker context and session authority alive

**Classification:** defect.
**Blocks:** architecture review and release.

Focus loss only cancelled pending work, cleared local UI, and dropped the local
field reference in
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L314).
Generic invalidation likewise cancelled and hid only local state at
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L1077).
No Chromium runtime command closed or deactivated the broker session. The broker
already supported explicit session closure at
[`server.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/server.rs#L241), while `badictl` could address the
broker's reported active locator at
[`badictl.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/bin/badictl.rs#L149).

The last context could therefore remain in broker memory after blur, visibility
loss, route change, or tab teardown. A same-UID controller could request
generation against that stale context even though the browser controller would
no longer display it. This violated the documented rule that focus changes
invalidate authority.

The smallest sufficient fix was an explicit context-deactivation or
session-close operation on focus loss, page lifecycle termination, route
invalidation, and controller disposal, backed by a bounded broker lease for
orphaned sessions.

#### H2. Exact-document authority is not continuous through display and mutation

**Classification:** high-impact risk; not reproduced.
**Blocks:** architecture review and release.

The exact URL was checked when the content script started at
[`content-script.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/content-script.ts#L9),
and the worker validated a `MessageSender` snapshot at
[`fixture-boundary.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/background/fixture-boundary.ts#L4).
Response display, authorization handling, and final mutation at
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L640),
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L698),
and
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L809)
did not recheck the current URL. No `pushState`, `replaceState`, `popstate`, or
`pagehide` test covered the gap.

A same-document SPA transition could therefore leave the controller running
outside the claimed path. At minimum, a pending result could display after the
route changed; a narrow authorization-to-mutation race could also permit an
edit. Chrome's match patterns and sender metadata do not replace a fresh local
mutation-time check:
[match patterns](https://developer.chrome.com/docs/extensions/develop/concepts/match-patterns),
[`MessageSender`](https://developer.chrome.com/docs/extensions/reference/api/runtime).

The smallest fix was one canonical current-document predicate checked at
acquisition, display, shortcut handling, and immediately before mutation, plus
page/history lifecycle invalidation and real-browser SPA transition tests.

#### H3. `Shift+Tab` is treated as accept-all

**Classification:** defect.
**Blocks:** release and product demonstration.

The Tab branch excluded Alt, Ctrl, and Meta but not Shift at
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L471).
The vision explicitly required normal application shortcuts to remain normal at
[`VISION-V2.md`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/VISION-V2.md#L75).

A user attempting backward focus traversal could unexpectedly insert the entire
suggestion. The smallest fix was to require `!event.shiftKey`, add negative
tests, and include the behavior in a trusted real-browser keyboard run.

#### H4. Evidence is mutable under stable IDs and headline performance is stale

**Classification:** evidence and documentation defect.
**Blocks:** architecture handoff, merge, and release.

Commit `b8d6786` modified only the existing capability and evidence documents:

```text
M capabilities/chromium-dom-foundation.v1.json
M capabilities/chromium-native-live.v2.json
M capabilities/evidence/chromium-native-live-run.v1.json
```

The roadmap required the existing M2A receipt to remain historical and a new
capability version to be published rather than rewritten at
[`develop-roadmap.md`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/docs/plan/develop-roadmap.md#L68). The record still used
`chromium-native-live.v2` at
[`chromium-native-live.v2.json`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/capabilities/chromium-native-live.v2.json#L4).
The README called 8.4 ms and 0.7 ms the current result at
[`README.md`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/README.md#L46), while the receipt reported 12.6 ms and 0.6 ms
at
[`chromium-native-live.v2.json`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/capabilities/chromium-native-live.v2.json#L230).

The smallest fix was to preserve immutable historical identifiers, issue a new
run/receipt identity for each durable run, update the README's current values,
and distinguish historical source-linked evidence from a current reproduction.

#### H5. The public reviewer entry point presents the wrong product state

**Classification:** handoff defect.
**Blocks:** head-of-Omarchy review.

Remote inspection returned:

```json
{"defaultBranchRef":{"name":"main"},"description":"","licenseInfo":null,"url":"https://github.com/ahuray/badi"}
```

`git show origin/main:README.md` began with `# Omatype`, `origin/main` still
contained `omatypectl`, and
`gh pr list --state open --base main --head develop` returned `[]`.

A reviewer arriving at the repository would see the former product, old command
names, and no discoverable review delta. The smallest fix was a focused
`develop` to `main` review PR with one concise reviewer brief, exact commit and
evidence scope, and explicit blockers. Opening or merging that PR remained an
external owner action rather than part of this audit.

### Medium

#### M1. Rust and TypeScript implement different output and word contracts

**Classification:** defect.
**Blocks:** release and semantic-provider work.

Rust used UAX #29 `unicode_word_indices()` at
[`segment.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/segment.rs#L52), while TypeScript used a narrower
letter/number/mark regex at
[`context.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/context.ts#L135). The locked
Rust dependency treats `can't` as one word. Rust therefore produced `" can't"`
for `" can't wait"`, TypeScript derived `" can"`, and the controller silently
rejected the response at
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L656).
The shared fixtures at
[`accept-word-fixtures.json`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/protocol/v1/accept-word-fixtures.json#L4)
omitted contractions and decimals.

The smallest fix was to make the fixture suite the canonical executable
contract and require both implementations to prove identical contractions,
decimals, punctuation, combining marks, emoji, and multilingual cases.

#### M2. Type-through continuity is not validated against the real broker

**Classification:** confirmed shortcut defect plus unresolved race risk.
**Blocks:** release.

Locally reconciled text was shown with `brokerBound: false` while retaining the
old broker address at
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L1040).
The next broker context retired the old suggestion and emitted a clear at
[`engine.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/engine.rs#L294) and
[`engine.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/engine.rs#L912). The controller's clear matched
that old address at
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L239).
The jsdom test used a fake transport that never emitted the broker clear at
[`field-controller.test.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/test/field-controller.test.ts#L714).
It also codified that immediate Tab was prevented but inserted nothing at
[`field-controller.test.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/test/field-controller.test.ts#L751).

The smallest fix was to avoid intercepting acceptance shortcuts while a suffix
was display-only and add a real broker/browser type-through scenario observing
both the old clear and replacement result.

#### M3. Broker pause state from hello is validated and discarded

**Classification:** privacy and coherence risk; disclosed foundation limit.
**Blocks:** release or origin expansion.

The mapper validated `paused` at
[`protocol-mapper.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/background/protocol-mapper.ts#L558),
but `NativeBrokerClient` resolved readiness without propagating it at
[`native-client.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/background/native-client.ts#L228).
The limitation was honestly disclosed at
[`vision-v2-implementation.md`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/docs/plan/vision-v2-implementation.md#L132).

A controller created or restarted while the broker was paused could still
acquire text and send it to the broker. The broker rejected before provider
work, so this was not inference leakage, but pause was not an acquisition
barrier. The smallest fix was to propagate authoritative paused state before
activating a controller, followed by a monotonic policy epoch for the wider M2
claim.

#### M4. Hardware and model advice is not resource-safe enough for an Omarchy boundary

**Classification:** defect and future integration risk.
**Blocks:** release of model advice as a stable interface.

Balanced selection required only 1,536 MiB available at
[`model_selection.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/model_selection.rs#L223), while the
balanced writing artifact itself was 1,834,426,016 bytes at
[`model_selection.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/model_selection.rs#L104), before runtime
and KV-cache overhead. `download_bytes` was not used in selection, every profile
received a tier, unknown battery state could still permit quality, and total/max
VRAM was used rather than currently usable memory. `nvidia-smi` ran with no
deadline or output cap at
[`model_selection.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/model_selection.rs#L358). No tracked
hardware or model-advice JSON Schema existed.

The smallest fix was a bounded NVIDIA probe, an explicit no-fit result,
artifact-size plus runtime-headroom checks, conservative unknown-data behavior,
and formal versioned schemas including backend compatibility.

#### M5. Same UID is a trust boundary, not authentication

**Classification:** accepted residual risk requiring an explicit decision.
**Blocks:** architecture review until decided.

The socket verified only UID and mode at
[`ipc.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/ipc.rs#L92). The broker accepted and echoed
self-declared capabilities at
[`server.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/server.rs#L143). Addressed `ControlRequest` did
not require session ownership at
[`server.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/server.rs#L271), allowing `badictl` to control
another connection's active session. The receipt explicitly excluded same-UID
impersonation at
[`chromium-native-live.v2.json`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/capabilities/chromium-native-live.v2.json#L299).

The correct minimal action was an architecture decision: either explicitly
trust all same-UID processes and retain the compact design, or introduce a real
broker-issued capability if that attacker belongs in scope. Decorative
pseudo-authentication would be worse than a clear boundary.

#### M6. Hostile pages can make a logically visible suggestion visually absent

**Classification:** future-origin security risk.
**Blocks:** release to arbitrary origins.

View visibility checked internal state, connection, and `hidden` only at
[`ghost-view.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/ghost-view.ts#L39). The page
could target the host's `data-badi-owned` attribute created at
[`ghost-view.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/ghost-view.ts#L92). The live
oracle likewise checked only `!host.hidden` at
[`run-live.mjs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/live/run-live.mjs#L440). Chrome content
scripts have an isolated JavaScript world but share the page DOM:
[official content-script documentation](https://developer.chrome.com/docs/extensions/develop/concepts/content-scripts).

Hostile CSS could make the host transparent or displaced while Badi still
intercepted Tab. The smallest fix was hostile CSS/removal/geometry coverage, a
computed render-visibility precondition before shortcut interception, and a
clear statement that extension DOM is not a secrecy boundary.

#### M7. Connection and provider work is insufficiently bounded

**Classification:** availability and maintainability risk.
**Blocks:** release.

Every accepted socket was detached with `tokio::spawn` at
[`server.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/server.rs#L27); writer and event queues were
unbounded at [`server.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/server.rs#L126). There were no
connection or frame-idle deadlines. Provider and expiry tasks were detached;
provider timeout did not explicitly signal its cancellation token at
[`engine.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/engine.rs#L403). Browser native-client handshake
timeout reset local state without disconnecting the port at
[`native-client.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/background/native-client.ts#L212).

The smallest fix was bounded channels, connection/session caps, read and
handshake deadlines, tracked tasks, and explicit timeout/shutdown cancellation.

#### M8. Several nominal contracts are not fully validated by their consumers

**Classification:** contract defect.
**Blocks:** release.

`badictl` accepted hello based only on message type at
[`badictl.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/bin/badictl.rs#L107); it did not correlate ID or
validate the selected limits, capabilities, or paused state. Context payload was
validated before activation was clamped to the session's stricter activation at
[`engine.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/engine.rs#L280). The latter did not invoke the
provider or retain denied context, but weakened the claimed boundary.

The smallest fix was strict shared hello decoding and clamping authoritative
policy inputs before context-dependent validation.

#### M9. The implemented visual loop does not match the vision

**Classification:** product gap, not an architecture defect.
**Blocks:** product demonstration and release.

The vision called for a caret suffix with a visually stronger first word at
[`VISION-V2.md`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/VISION-V2.md#L66). The implementation was a generic panel
positioned above or below the whole field with one uniformly styled suggestion
at [`ghost-view.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/ghost-view.ts#L22) and
[`ghost-view.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/ghost-view.ts#L108).

The smallest product step was to define and validate one Linux/Omarchy visual
interaction before adding surfaces: caret-aligned suffix, first-word emphasis,
scaling, contrast, and quiet dismissal.

#### M10. No project license exists

**Classification:** release and legal risk.
**Blocks:** release and public reuse.

No tracked `LICENSE` or `COPYING` file existed and GitHub reported
`"licenseInfo": null`. The absence was disclosed at
[`README.md`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/README.md#L9). Selecting a license is an owner decision, not
an implementation detail; an SPDX license must be added before public
distribution.

### Low

#### L1. Recorded debug binary hashes were not reproduced

The raw record contained broker SHA `633ee7...` and native-host SHA `a6f63c...`
at
[`chromium-native-live-run.v1.json`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/capabilities/evidence/chromium-native-live-run.v1.json#L286).
Current isolated builds had the same sizes but different hashes. Path-dependent
debug data was plausible but not proven. Release evidence should bind normalized
or reproducible artifacts rather than incidental debug binaries.

#### L2. A field already focused at `document_idle` is not adopted

`start()` registered `focusin` but did not inspect `document.activeElement` at
[`field-controller.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/field-controller.ts#L148).
Autofocused fields remained inactive until focus moved away and back.

#### L3. Broker bookkeeping can report phantom or prematurely consumed state

The broker recorded a suggestion as visible and incremented `shown` even when
event delivery failed at [`engine.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/engine.rs#L495).
`commit_result` consumed the pending lease before full address and authority
validation at [`engine.rs`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/broker/src/engine.rs#L758).

#### L4. Minor documentation debt remains

The historical two-day plan simultaneously said `Status: execution contract` at
[`two-day-delivery.md`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/docs/plan/two-day-delivery.md#L3), and runtime transport
reported `an Badi message` at
[`runtime-transport.ts`](https://github.com/ahuray/badi/blob/b8d6786a451defed5e186c3ba1fdf90a29b7099e/adapters/chromium/src/content/runtime-transport.ts#L66).

## Claim-verification matrix

| Claim | Status | Evidence and caveat |
| --- | --- | --- |
| Review branch and expected commit | Verified | Clean `develop`; HEAD and `origin/develop` were both `b8d6786...` |
| `main...develop` scope | Verified | Four commits; 60 files; 1,668 insertions and 496 deletions |
| 75 Rust tests pass | Verified | Current isolated `cargo test --workspace`; not headed product proof |
| 68 Chromium/TypeScript tests pass | Verified | Current isolated `npm run check`; primarily jsdom and fakes |
| CI passes Rust 1.85/1.98 and Node 22/24 | Verified | Exact-SHA [Actions run 33306824009](https://github.com/ahuray/badi/actions/runs/33306824009) succeeded |
| Raw evidence records 1,000 insertion and 100 stale trials | Partially verified | The record and runner agree; the historical run was not reproduced |
| Latest receipt reports 12.6 ms and 0.6 ms p95 | Partially verified | Raw and promoted records agree; README contradicted them |
| 1,000 edit-to-visible trials are additional independent trials | Contradicted | They were observations from the same insertion loop, not another independent trial set |
| Current machine receives balanced writing/code advice | Verified | `badictl` selected Qwen3 1.7B and Qwen2.5-Coder 1.5B; this did not prove runtime fit |
| No model weights were downloaded | Partially verified | This audit downloaded none and the code never executed its download plan; all historical filesystem activity could not be proven |
| Semantic inference is absent and disabled | Verified | Only the deterministic phrase provider existed; `runtime_ready` was always false |
| No clipboard or synthetic typing path exists | Verified by source | One `setRangeText` mutation plus a synthetic notification `InputEvent`; no keystroke or clipboard fallback |
| Badi is consistently renamed in tracked `develop` | Verified | Only legitimate references to the unrelated OmaType project remained; public `main` was stale |
| No persistent Omarchy/browser/system configuration was modified | Partially verified | The audit itself was isolated and read-only; historical evidence remained self-reported |
| All six model records are pinned and Apache-2.0 | Verified | Official Hugging Face/Qwen metadata matched every field; product utility remained unverified |
| The non-commercial Qwen Coder 3B problem was avoided | Verified | The selected 7B quality artifact is Apache-2.0; the excluded 3B uses Qwen's research license |
| Badi is ready for broad Chromium or Omarchy use | Contradicted | The receipt itself excluded headed, arbitrary-site, framework, undo, policy-epoch, and semantic-model support |

## Model catalog verification

Every catalog record matched the official Hugging Face repository API at its
pinned revision.

| Use and tier | Pinned artifact | Intended-use and runtime judgment |
| --- | --- | --- |
| Writing compact | [`Qwen/Qwen3-0.6B-GGUF`](https://huggingface.co/api/models/Qwen/Qwen3-0.6B-GGUF/revision/23749fefcc72300e3a2ad315e1317431b06b590a?blobs=true), rev `23749fefcc72300e3a2ad315e1317431b06b590a`, `Qwen3-0.6B-Q8_0.gguf`, 639,446,688 bytes, SHA `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031`, Q8_0, Apache-2.0 | Official general Qwen3 assistant GGUF; not evaluated for inline suffix quality |
| Writing balanced | [`Qwen/Qwen3-1.7B-GGUF`](https://huggingface.co/api/models/Qwen/Qwen3-1.7B-GGUF/revision/90862c4b9d2787eaed51d12237eafdfe7c5f6077?blobs=true), rev `90862c4b9d2787eaed51d12237eafdfe7c5f6077`, `Qwen3-1.7B-Q8_0.gguf`, 1,834,426,016 bytes, SHA `061b54daade076b5d3362dac252678d17da8c68f07560be70818cace6590cb1a`, Q8_0, Apache-2.0 | General assistant, not an inline model; the old 1.5 GiB threshold could not safely fit it |
| Writing quality | [`Qwen/Qwen3-4B-GGUF`](https://huggingface.co/api/models/Qwen/Qwen3-4B-GGUF/revision/bc640142c66e1fdd12af0bd68f40445458f3869b?blobs=true), rev `bc640142c66e1fdd12af0bd68f40445458f3869b`, `Qwen3-4B-Q4_K_M.gguf`, 2,497,280,256 bytes, SHA `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5`, Q4_K_M, Apache-2.0 | Official llama.cpp artifact; latency, template, and inline usefulness unmeasured |
| Code compact | [`Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF`](https://huggingface.co/api/models/Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF/revision/ebb2015119c907b064c512bf053e945850b5875f?blobs=true), rev `ebb2015119c907b064c512bf053e945850b5875f`, `qwen2.5-coder-0.5b-instruct-q4_k_m.gguf`, 491,400,064 bytes, SHA `1d9614638d18024d0fbb36575a15f1302a3adf044df10345688ec4f6e1c4ff32`, Q4_K_M, Apache-2.0 | Coding-instruct/chat artifact; selected card did not prove fill-in-the-middle product quality |
| Code balanced | [`Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF`](https://huggingface.co/api/models/Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF/revision/f86cb2c1fa58255f8052cc32aeede1b7482d4361?blobs=true), rev `f86cb2c1fa58255f8052cc32aeede1b7482d4361`, `qwen2.5-coder-1.5b-instruct-q4_k_m.gguf`, 1,117,320,768 bytes, SHA `cc324af070c2ecbfd324a30884d2f951a7ff756aba85cb811a6ec436933bb046`, Q4_K_M, Apache-2.0 | Plausible candidate; base-model FIM support did not prove this Instruct artifact's Badi fitness |
| Code quality | [`Qwen/Qwen2.5-Coder-7B-Instruct-GGUF`](https://huggingface.co/api/models/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/revision/13fb94bfda8c8cf22497dc57b78f391a9acb426a?blobs=true), rev `13fb94bfda8c8cf22497dc57b78f391a9acb426a`, `qwen2.5-coder-7b-instruct-q4_k_m.gguf`, 4,683,073,536 bytes, SHA `509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c`, Q4_K_M, Apache-2.0 | Correctly avoided the 3B license problem; old headroom policy was too coarse for artifact plus runtime state |

Official Qwen documentation required llama.cpp b5092 or newer for Qwen3, while
Badi recorded only `llama.cpp` and no minimum backend or prompt-template
contract: [Qwen llama.cpp guide](https://github.com/QwenLM/Qwen3/blob/main/docs/source/run_locally/llama.cpp.md).

## Architecture assessment

```text
untrusted page DOM
      ⇅ bounded context and one target-API edit
Chromium field controller ── ghost view
      ↓
MV3 service worker and immutable document route
      ↓ nativeMessaging
Rust native host and exact extension identity
      ↓ private 0600 Unix socket
Rust broker ── policy/state/cancellation ── provider
      ↑
   badictl (same-UID global and addressed control)
```

### Strongest decisions

- The adapter alone reads and mutates the target; the broker cannot simulate
  typing.
- Eligibility precedes value capture for recognized denied fields.
- Browser, native, and socket frames are bounded before body allocation.
- The native host verifies the exact extension origin, private socket metadata,
  and peer UID.
- Session, focus, revision, fingerprint, suggestion identity, latest-wins
  cancellation, receiver-local TTL, and single-use commit preparation are the
  right primitives.
- The provider port is small and keeps inference logic out of adapters.
- Pinned data-only GGUF artifacts and `runtime_ready: false` avoid executable
  remote code and premature activation.

### Missing boundaries

- A document and focus lifecycle lease revoking broker context.
- Continuous current-document authority at display and mutation.
- One canonical cross-language segmentation and output contract.
- A pause/policy epoch propagated before acquisition.
- An explicit same-UID threat decision.
- A resource-fit and runtime-compatibility gate for model advice.
- Immutable evidence identities.

### Complexity and maintainability

The core layering was not decorative, but some mass preceded product proof. The
490-line model selector existed before a semantic provider or quality corpus;
global active-session CLI control preceded a final threat model; multiple schema
and manual validators overlapped without one executable semantic contract; and
the evidence system was substantial while its IDs remained mutable.

The provider trait, transport boundaries, authority coordinates, and
target-native edit path earned their abstractions. The model advisor and some
evidence/handoff machinery did not yet earn their maintenance cost. No new
generic framework was warranted: small explicit lifecycle methods, bounded
resources, and canonical fixtures were sufficient.

### Linux and Omarchy fit

The XDG runtime socket and user-owned native-messaging path fit Linux well. The
tree did not patch `/usr/share/omarchy`. `badictl --json` was a sensible future
Quickshell/Omarchy boundary, but not yet a stable one because its schemas were
nominal, probing could block, and model-fit semantics were unsafe.

## Test and evidence report

| Command | Result |
| --- | --- |
| `git status --short --branch` | `## develop...origin/develop` |
| `git log --oneline --decorate -10` | Expected HEAD; four commits above `main` |
| `git diff --check` | Exit 0 |
| `git diff --stat main...develop` | 60 files, 1,668 insertions, 496 deletions |
| `cargo fmt --all --check` | Exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0 |
| `cargo test --workspace` | 75 passed |
| `cargo +1.85.0 check --workspace --all-targets --locked` | Exit 0 |
| `npm ci` | 105 packages in the disposable clone; 107 audited; zero vulnerabilities |
| `npm run check` | Exit 0; typecheck, 68 tests, deterministic build, syntax, naming, docs, and capability checks |
| `npm audit --audit-level=moderate` | Zero vulnerabilities |
| `gh run list --branch develop` | Exact-SHA workflow succeeded |
| TODO/FIXME/HACK/name/path/number searches | No TODO/FIXME/HACK; rename clean; stale performance and archived-plan contradictions found |
| `gh pr list --base main --head develop` | `[]` |

Rust's 75 tests comprised 51 library, 2 broker CLI, 3 native-host CLI, 2
manifest CLI, 5 `badictl`, 2 model-selection CLI, 2 native bridge, 2 manifest
integration, 4 protocol/schema, and 2 signal/shutdown tests. TypeScript's 68
tests comprised 32 field-controller, 14 context, 11 native-client, 5
protocol-mapper, and 6 fixture-boundary tests.

The raw record and receipt were bound to base commit
`068db9fe389fd7777bd903021b9c2baf3bde5140`, not HEAD directly. HEAD's only
changes after that base were the three evidence files; runtime source was
unchanged. Current extension, fixture, runner, policy, and raw-record hashes
matched the receipt. The raw record contained 18 scenario rows: 15 pass and 3
explicitly unsupported. The 1,000 insertion and 1,000 edit-to-visible
observations came from the same loop; the 100 delayed stale trials were
separate. The durable browser run remained historical rather than a current
reproduction.

## Scope truth

### Works in the reviewed tree

- Current Rust and TypeScript builds and deterministic checks.
- Private XDG socket, UID/mode checks, bounded framing, and native bridge.
- One static localhost development document and extension identity.
- Ordinary visible, writable, uniquely identified light-DOM text inputs and
  textareas.
- Recognized sensitive-field pre-acquisition denial.
- Deterministic phrase suggestions.
- Adapter-owned panel UI and one broker-authorized DOM edit.
- Dismissal, stale-result suppression, TTL, disconnect invalidation, and
  broker-authoritative provider pause within the declared M1 limits.
- Content-free hardware/model JSON with non-executing download plans.

### Foundation only

- Exact-document trust.
- Type-through continuity.
- Cross-process pause coherence.
- `badictl` as an Omarchy/Quickshell API.
- Model recommendations.
- Capability receipts as durable provenance.
- Product visual language.

### Unsupported

- Arbitrary sites or runtime origin grants.
- `contenteditable`, shadow DOM, iframes, framework-controlled editors, and
  selection replacement.
- Browser-native undo guarantees and physical IME behavior.
- Headed accelerator, background visibility, compositor rendering, and
  accessibility proof.
- MV3 restart and cross-connection policy epochs.
- Semantic inference or verified model quality and latency.
- Obsidian/CodeMirror, terminal, Fcitx5, IBus, and personalization.
- Production packaging, installation, and stable public deployment.
- Resistance to hostile same-UID processes.
- A project license.

### Milestone status

| Milestone | Status at reviewed commit |
| --- | --- |
| M0 contract and research | Complete |
| M1 trust foundation | Partial |
| M2A controlled Chromium/native slice | Partial foundation with historical evidence |
| Full M2 Chromium | Unstarted beyond M2A |
| M3 Obsidian/CodeMirror | Unstarted |
| M4 semantic provider | Catalog only |
| M5 personalization | Unstarted |
| M6 Linux breadth/terminal | Unstarted |
| D0 guardrails | Partial; immutable-evidence rule violated |
| D1-D3 | Unstarted |
| D4 | Hardware/catalog foundation only |
| D5-D6 | Unstarted |

### Required headed Omarchy validation

- Caret and field geometry under Hyprland scaling and multiple monitors.
- Contrast, blur, opacity, browser zoom, and compositor rendering.
- Trusted Tab, Shift+Tab, and configurable shortcut behavior.
- Physical IME composition and browser-native undo/redo.
- Active/background window and workspace transitions.
- Runtime permission consent and revocation.
- Full and same-document navigation, BFCache, tab close, discard/freeze, and
  MV3 restart.
- Native-host installation in disposable user-owned paths.
- Hostile page CSS, DOM, and event interference.
- Real Obsidian/CodeMirror if retained in product scope.

## GrillMe challenge log

### Strongest reasons not to approve

1. Lifecycle changes revoked browser UI but not broker context and authority.
2. The signature interaction contained concrete keyboard and cross-language
   semantic defects.
3. Evidence and public handoff were historical, mutable, internally
   inconsistent, and difficult to discover.

### Claims most likely to be overstated

1. **Exact document:** exact at injection and message snapshots, not continuously
   at display and mutation.
2. **Exact type-through without flicker:** tested against a fake omitting the
   real broker clear.
3. **Hardware-aware recommendation:** deterministic tier output, not
   artifact-fit or runtime-quality-aware advice.

### Tests that could pass while the product remained broken

1. The jsdom type-through test passed because `FakeTransport` never emitted the
   old broker clear.
2. The live ghost oracle passed whenever `hidden` was false, even if hostile CSS
   made the UI invisible.
3. `npm run check` passed because `live:check` parsed the browser runner but did
   not run Chromium.

### Material disagreement resolution

| Disagreement | Resolution |
| --- | --- |
| Exact-document issue is a confirmed exploit versus harmless | Source proved a local TOCTOU gap; Chrome could refresh sender URL on later messages. Classified High risk and explicitly unresolved without live reproduction. |
| Type-through definitely flickers versus is safe | Source showed the old clear could match the locally retained address; fake tests omitted it. Classified Medium unresolved race. Immediate swallowed Tab was confirmed. |
| Same-UID behavior is a defect versus an intended CLI feature | Receipt explicitly excluded same-UID impersonation and `badictl` intentionally controlled other sessions. Classified accepted residual risk requiring an ADR. |
| Composite emoji proves contract divergence | Second-source inspection showed its ZWJ was rejected by the sanitizer before end-to-end output. That counterexample was withdrawn; contractions remained confirmed. |
| Model catalog is sound versus misleading | Metadata and license were sound; responsive inline suitability and resource fit were not. Claims were split. |
| Evidence proves the current tree versus is obsolete | Runtime source and source hashes matched the recorded base, but live results were not reproduced and binary hashes differed. Classified source-linked historical evidence. |

### Final GrillMe verdict

> I would not send Badi to the head of Omarchy for approval or a product demonstration in its current state. It contains a credible narrow broker and controlled Chromium architecture slice, but document and session authority survive lifecycle changes, same-document mutation authority is not continuous, keyboard and cross-language output semantics are broken, model advice is not resource-safe, and the public evidence and handoff are historical and internally inconsistent. After lifecycle leases and authority epochs, canonical shared semantics, corrected immutable evidence, a discoverable reviewer brief, and an explicit same-UID decision, it would be suitable for a private architecture critique; it is not yet an Omarchy-ready product.

## Minimal handoff checklist

### Before head-of-Omarchy review

- Revoke context and session authority on every browser lifecycle change.
- Enforce current-document authority at acquisition, display, shortcut, and
  mutation.
- Fix Shift+Tab and canonicalize Rust/TypeScript output semantics.
- Validate type-through against the real broker clear path.
- Correct headline performance and preserve immutable evidence identities.
- Publish a concise reviewer brief and focused review PR.
- Record the same-UID decision.
- Make model advice resource-safe or remove it from the reviewer-facing product
  surface.

### Before merging to `main`

- Complete the review blockers above.
- Bound connection, channel, and provider lifecycles.
- Tighten `badictl` negotiation and formalize public JSON schemas.
- Produce fresh isolated evidence linked to the final commit.
- Add the owner-selected project license if `main` is intended for public reuse.
- Make public README, description, names, commands, and default branch present
  Badi consistently.

### Before release

- Complete the headed Chromium/Hyprland lifecycle matrix.
- Implement permission revocation and policy epochs.
- Validate physical IME, undo, focus, workspace, hostile-page, and accessibility
  behavior.
- Provide production native-host packaging and removal.
- Complete the Obsidian cell if retained in product scope.
- Pass semantic-provider quality, cancellation, latency, and memory gates, or
  make no semantic claim.
- Resolve project license and public package/trademark clearance.

## Recommended bounded remediation

The first engineering slice should be D1 lifecycle authority:

- explicit context deactivation or session close on blur, page lifecycle, tab
  removal, and controller disposal;
- one current-document predicate at capture, display, authorization response,
  and mutation;
- a bounded broker lease for abrupt controller or worker loss; and
- regression tests for `pushState`, `replaceState`, `pagehide`, worker death,
  stale commit authorization, and zero retained active context/session state.

The exit criterion should be immutable evidence showing that every lifecycle
transition leaves zero visible suggestion, zero commit authority, and zero
retained active context.
