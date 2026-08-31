# Develop branch roadmap

- Status: architecture roadmap; next-sprint order superseded on 2026-08-31
- Branch: `develop`
- Baseline: `0a1aaf37c2b561b2c415ddc8437f0e853d4c65e0`
- Source of truth: [Vision V2](../../VISION-V2.md) and the
  [V2 implementation plan](vision-v2-implementation.md)

> **Execution update:** the [GrillMe product-proof plan](grillme-product-proof.md)
> governs the next implementation slices. This document remains architecture
> and decision history; its old first-sprint order is not the active queue.

## Purpose

`main` remains the verified M2A Chromium/native-live baseline. `develop` is the
integration branch for completing the vision through small, reversible vertical
slices. This roadmap translates the broader vision into the order in which the
next code should be written.

The sequence is deliberate:

1. Complete the browser authority and permission boundary.
2. Prove the same target-owned editing model in Obsidian/CodeMirror.
3. Add a local semantic provider behind the stable core contracts.
4. Add quiet personalization with explicit user control.
5. Expand Linux coverage only where the evidence supports a truthful claim.

This is not a rewrite. Existing verified behavior stays working while each new
capability earns its place through a named gate and an evidence receipt.

## Delivery principles

- Build vertical slices that end in observable user behavior and evidence.
- Define schemas and contracts before changing their producers or consumers.
- Keep policy in the broker/core and platform mechanics in adapters.
- Treat the target application's edit API as the only commit path.
- Fail closed when identity, permission, focus, visibility, or policy is stale.
- Never use clipboard automation, synthetic typing, or keystroke replay.
- Never log document content, context text, suggestions, or reconstructed text.
- Invalidate pending suggestions and commit authority whenever policy changes.
- Keep compatibility explicit through protocol negotiation; never reinterpret an
  older message silently.
- Prefer small green commits. Refactor only when a tested capability needs it.
- Refresh hash-linked evidence whenever a measured runner, fixture, adapter, or
  extension source changes.

## Dependency map

| Order | Milestone | Depends on | Exit signal |
| --- | --- | --- | --- |
| D0 | Development guardrails | M2A baseline | Decisions and branch rules are documented |
| D1 | Browser authority contracts | D0 | Protocol and broker reject every stale authority path |
| D2 | Headed Chromium lifecycle proof | D1 | Permission, focus, pause, restart, and revocation pass in a real browser |
| D3 | Obsidian/CodeMirror slice | D1, stable edit contract | One editor surface passes the same safety loop |
| D4 | Local semantic provider | D2, D3 target cells | Quality beats the baseline within latency and memory budgets |
| D5 | Quiet personalization | D4 | Opt-in learning is inspectable, resettable, and privacy-preserving |
| D6 | Linux breadth and terminal decision | D2-D5 | Each claimed surface has evidence; unsupported paths remain explicit |

Work may be parallelized inside a milestone only after its input contract is
merged. Milestones do not skip their exit signal.

## D0 — development guardrails

Before widening behavior:

- Record architecture decisions for browser authority, policy epochs, protocol
  compatibility, and evidence renewal.
- Keep `main` as the stable release baseline and `develop` green as the
  integration branch.
- Use focused branches from `develop` when a slice needs review or isolation.
- Do not widen manifest permissions during exploratory work.
- Preserve the existing M2A receipt as historical evidence; publish a new
  capability version rather than rewriting what it proved.

### Blocking feasibility question

Prove what the supported Chromium permission APIs and match patterns can
actually constrain for scheme, host, and port. Do not assume exact-port consent
is enforceable by the browser. If Chrome collapses ports or requires broader host
access, stop and narrow the product claim or redesign the boundary. An
application-level URL check must not be presented as browser-enforced consent.

The feasibility spike must produce:

- a minimal disposable extension/profile reproduction;
- observed grant, denial, removal, and restart behavior;
- the exact manifest and runtime permission shapes tested;
- a short decision record with the strongest truthful supported claim; and
- no permission expansion in the production extension until that decision is
  accepted.

## D1 — browser authority contracts

### 1. Define the authority model

Create an opaque authority value that cannot become valid again after a process
restart. The default design is a boot-scoped instance identity plus a monotonic
policy epoch. A resettable counter by itself is not sufficient.

The broker owns authority generation. The following events advance or revoke
authority and invalidate pending work:

- pause or resume;
- permission grant, removal, or scope change;
- active tab or active window change;
- navigation or document replacement;
- extension service-worker restart;
- broker/native connection replacement; and
- any policy configuration change affecting eligibility or commit.

Pause acknowledgement is sent only after pending suggestions and commit leases
have been revoked and the UI-clear ordering is defined.

### 2. Introduce an explicit protocol revision

Specify protocol V2 before implementation:

- negotiate supported versions at connection startup;
- include authority on context, suggestion, presentation, and commit messages
  wherever a stale result could cross a boundary;
- provide canonical examples plus negative and boundary fixtures;
- define unknown-field and unknown-message behavior;
- cap frame, queue, and context sizes; and
- keep V1 behavior available only through explicit negotiated compatibility.

Protocol work is complete when Rust and TypeScript consume the same fixtures and
both reject stale, malformed, oversized, and mismatched messages.

### 3. Centralize broker enforcement

The broker must:

- issue and compare authority values in one policy component;
- bind each suggestion and commit lease to a connection, target, document, and
  authority value;
- revoke leases across connections when policy changes;
- clear bounded queues on revocation;
- reject replay, reordering, overflow, and post-restart stale results; and
- expose only content-free counters and timings for diagnostics.

Required tests include same-connection and cross-connection staleness, restart,
pause races, permission races, sequence rollover/overflow, bounded backpressure,
and clear-before-ack ordering.

### 4. Isolate Chromium lifecycle mechanics

Put browser permission and lifecycle APIs behind small adapter interfaces. The
controller must verify current authority before context acquisition and again
before presentation or commit.

Cover at least:

- active versus inactive tabs;
- focused versus background windows;
- hidden documents and unsupported fields;
- same-document navigation and full document replacement;
- permission removal while a suggestion is pending;
- extension worker suspension/restart; and
- native host disconnect/reconnect.

Suggestion routes are tied to the live document and authority value. Any
unverifiable state clears ghost UI and becomes ineligible.

## D2 — headed Chromium lifecycle proof

Promote D1 only after the entire safety loop runs through a real Chromium build
with a disposable profile. Fakes remain useful for exhaustive state tests but do
not replace headed evidence.

The headed suite must visibly exercise:

- runtime consent, denial, and permission removal;
- exact supported origin semantics established by the D0 spike;
- active/inactive tab and window transitions;
- visible/background document transitions;
- navigation and document replacement;
- pause/resume with pending work;
- extension service-worker restart;
- broker disconnect/reconnect; and
- accepted edits through the target DOM editing path.

Native undo is a release decision, not an assumption. If undo integration cannot
be demonstrated reliably, record it as unsupported instead of approximating it
with synthetic input.

Publish a new machine-readable receipt containing environment versions,
scenario results, exclusions, timing distributions, source hashes, isolation,
and cleanup status. Never mutate the M2A receipt to imply wider coverage.

## D3 — Obsidian/CodeMirror vertical slice

Start with one pinned Obsidian and CodeMirror version and one editor surface.
Reuse the core request, suggestion, authority, cancellation, and metrics
contracts; add only a target-specific adapter.

The first slice must prove:

1. eligibility without reading unrelated panes or vault content;
2. bounded context obtained through supported editor APIs;
3. ghost text rendered without mutating the document;
4. acceptance as one target-native transaction;
5. rejection and invalidation with no document mutation;
6. IME, selection, composition, and undo behavior; and
7. pause and policy changes clearing all pending UI and authority.

Keep the supported claim narrow: one documented editor surface and pinned
versions. Expand only after the compatibility matrix and evidence suite pass.

## D4 — local semantic provider

Add semantic intelligence behind the existing provider port; do not place model
logic in either adapter.

The content-free hardware probe and pinned candidate catalog are already a
foundation on `develop`. They intentionally stop before download or activation;
the following evaluation gate still decides whether any candidate becomes a
provider.

Sequence:

1. Freeze deterministic text and code evaluation corpora with content-safe
   provenance and licenses.
2. Define baseline quality, cancellation, latency, memory, and package-size
   budgets before choosing a model.
3. Implement an interchangeable local provider with bounded context and bounded
   output.
4. Add aggressive cancellation and stale-result rejection at every boundary.
5. Benchmark cold start, warm latency, peak memory, and quality on supported
   hardware profiles.
6. Ship only if it beats the deterministic baseline enough to justify its cost.

Model weights, prompts, caches, and evaluation fixtures require explicit license
and provenance review. No network fallback is added implicitly.

## D5 — quiet personalization

Personalization remains local, opt-in, inspectable, and reversible.

- Learn only from explicit accept/reject outcomes and declared settings.
- Store compact features, never recoverable document text.
- Provide view, export, reset, and disable controls before enabling learning.
- Separate global defaults from target-specific preferences.
- Bound storage and retention.
- Prove that reset removes learned state and invalidates derived caches.
- Compare personalized quality against a non-personalized holdout before rollout.

The system must remain fully usable when personalization is disabled.

## D6 — Linux breadth and terminal decision

Add applications one capability cell at a time. Prefer supported editor APIs and
accessibility protocols with testable identity and commit semantics.

For every candidate, document:

- target identity and focus guarantees;
- context acquisition boundary;
- presentation mechanism;
- native commit and undo behavior;
- password/secret-field exclusion;
- Wayland/X11/session assumptions; and
- a reproducible integration and evidence plan.

Terminal support is a separate go/no-go decision because shell boundaries,
prompt detection, full-screen TUIs, secrets, and undo semantics differ from text
editors. Keep it unsupported unless a narrow terminal capability can meet the
same trust contract without reading or replaying arbitrary terminal content.

## First coding sprint

Implement the next work as five reviewable slices:

1. **Authority and permission ADR/spike** — prove Chromium permission semantics
   and settle the non-reusable authority design without widening production
   permissions.
2. **Protocol V2 contract** — add schemas, negotiation, shared fixtures, and
   rejection tests before wiring behavior.
3. **Broker authority enforcement** — implement instance identity, policy epoch,
   lease revocation, and cross-connection race tests.
4. **Chromium lifecycle ports** — add permission/lifecycle interfaces, fakes,
   fail-closed controller transitions, and unit/integration tests.
5. **Headed evidence runner** — exercise the accepted D0 permission claim and D1
   lifecycle matrix with a disposable browser profile, then publish a new
   receipt.

Each slice should be independently reviewable and keep existing M2A behavior
green. Code for Obsidian begins only after the shared authority/edit contract is
stable enough to reuse.

## Definition of done for every slice

- The user-visible capability and unsupported boundaries are explicit.
- New behavior has unit, contract, negative, and relevant integration tests.
- Formatting, linting, type checks, deterministic builds, and the full relevant
  test suite pass.
- Cancellation, revocation, restart, and stale-result paths are tested.
- Content does not appear in logs, metrics, receipts, or failure artifacts.
- Documentation, fixtures, and capability receipts match the shipped behavior.
- Any changed measured source either refreshes its linked evidence or clearly
  leaves the prior evidence attached only to its original source snapshot.
- The worktree is clean and the commit is pushed to `develop`.

## Decisions intentionally left open

- The strongest browser-enforced origin/port permission claim Chromium supports.
- Production extension identity, packaging, signing, and distribution.
- Whether native undo can be guaranteed per supported target.
- Badi package/trademark clearance; the source license is MIT.
- Semantic model/runtime choice and weight licenses.
- Which Linux editor becomes the next capability cell after Obsidian.

These are gates, not invitations to guess. Record evidence, choose explicitly,
and narrow the claim when the platform cannot uphold it.

## Promotion policy

`develop` is ready to promote only when the milestone's capability gate is
green, evidence is reproducible, unsupported surfaces are documented, and CI is
clean. Promotion to `main` should be a deliberate release action; incomplete
milestones remain on `develop` or focused branches.
