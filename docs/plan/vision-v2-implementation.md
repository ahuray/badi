# Omatype Vision V2 Implementation Plan

Status: execution contract, 2026-08-30

This plan implements [Vision V2](../../VISION-V2.md). It converts the product
promise into narrow capability proofs. A milestone is complete only when its
evidence is reproducible; a demo is not a compatibility claim.

`Omatype` remains an internal codename. Public naming, package identifiers,
domains, and distribution are blocked until the collision described in the
vision is resolved. The source license is also an explicit user decision. No
third-party implementation may be copied into this repository before that
decision and a dependency/license review.

## Outcome and order of proof

The program proves four things in this order:

1. **Trust substrate** — denied targets disclose zero text, every result is
   revision-bound, and pause/cancellation are reliable.
2. **Signature loop** — one calm suggestion can be displayed, typed through,
   dismissed, accepted by word, or accepted in full through a supported target
   API.
3. **Useful local prediction** — a measured local semantic provider improves
   on the deterministic lane without breaking latency or quietness gates.
4. **Capability breadth** — each additional app earns a compatibility cell
   through its own supported APIs and tests.

This ordering prevents model work or broad Linux hooks from hiding unsafe
target behavior.

## Current vertical slice

The first repository slice is intentionally controlled:

- a strict JSON Schema for protocol version 1;
- a Rust broker core with bounded frames, policy, session state, latest-wins
  cancellation, deterministic generation, segmentation, pause, and
  content-free metrics;
- a strict TypeScript Manifest V3 Chromium adapter for ordinary `input` and
  `textarea` elements;
- a deterministic fake transport and controlled web fixture executed under
  jsdom;
- tests for sensitive-field non-acquisition, stale results, display,
  type-through, dismiss, accept-word, accept-all, and pause; and
- repeatable format, lint, type, test, build, and documentation checks.

It does not yet claim production native-host packaging, arbitrary-page browser
compatibility, semantic-model quality, Obsidian support, or terminal support.

## System boundary

```text
eligible adapter-supported field
        |
        v
Chromium field controller ---- ghost view
  |  eligibility first          | display only
  |  bounded snapshot           | no mutation authority
  v                             |
MV3 runtime transport ----------+
        |
        v
native-messaging boundary
        |
        v
Rust broker
  protocol -> policy -> session/revision -> provider -> output filter
        |
        +---- content-free health and capability receipt
```

The adapter is the only component allowed to read or mutate its field. The
broker never drives the keyboard or edits an application. The view never owns
acceptance. Transport carries bounded protocol messages and has no authority
to weaken policy.

## Non-negotiable invariants

### Target identity

Every context-bearing request and every suggestion action is bound to:

- `session_id` — one adapter connection;
- `focus_epoch` — one continuous focused target;
- `revision` — one observed target state;
- `fingerprint` — a fresh session-scoped opaque race detector for the bounded
  text/selection state;
- `suggestion_id` — one broker result; and
- `ttl_ms` — a bounded relative lifetime converted to receiver-local expiry.

Display and insertion require every available value to match. Any focus,
selection, value, composition, policy, pause, or connection change invalidates
the candidate. A newer request supersedes all older work even when an older
provider call finishes last.

Fingerprints are neither logged nor persisted and do not authenticate an
adapter. Sender-local monotonic timestamps may order events from that sender;
they are never compared between processes.

### Acquisition before inference

Field eligibility is decided before value or surrounding text is serialized.
Fields positively recognized by supported metadata as password, PIN, OTP,
payment-secret, hidden, ambiguous, non-editable, or foreign-composition states
send no prose and invoke no provider. Unknown identity is denied before value
capture unless a later explicit mode can prove that activation preceded
acquisition. Unmarked or incorrectly marked secret-like ordinary fields remain
a disclosed residual risk; content inspection is not used to guess that a
secret is present. The broker repeats policy because an adapter assertion is
not a trust boundary.

The adapter owns only the pre-acquisition metadata gate. The broker owns the
authoritative inference decision for validated session metadata and global
pause. M1 intentionally has no persisted origin-policy store. M2 adds one with
a monotonic policy epoch, broadcast/ack synchronization, and fail-closed races;
adding an epoch field without that state protocol would provide false
assurance. Stored origin grants/preferences receive their own visibility,
revocation, migration, export, and deletion behavior.

M1 pause stops provider work and new broker authorization and clears each
notified adapter locally. A controller that starts or restarts while the broker
is already paused—or an existing controller not reached because pause came
from a separate CLI connection—can still acquire context and send repeated
requests that the broker rejects before any provider call. M1 cannot make
adapter acquisition or transport atomic with an external connection's pause.
It also does not claim a cross-connection happens-before between a CLI pause
acknowledgment and an already transmitted `commit.prepare`. M2 must add a
policy epoch plus bounded broadcast/ack and commit-revocation barriers before
claiming zero outbound context or physical mutation after pause
acknowledgement.

### One mutation

Acceptance first sends one addressed control action and awaits the matching
broker `commit.prepare`. Immediately after authorization, the adapter re-reads
the active element, eligibility, selection, value, revision, and fingerprint.
A valid action attempts one target-API edit and emits the target's normal input
event. A failed precondition reports stale/blocked and clears the suggestion;
it never retries with a synthetic typing or clipboard path. Browser DOM cells
remain `dispatched-unverified` until a live framework/undo suite proves more.

### Content-free operations

Normal logs and metrics may contain versions, message kinds, counters, byte
counts, timing buckets, policy reasons, and error codes. They must not contain
field text, suggestion text, URLs/origins, window titles, file paths,
fingerprints, stable target identifiers, or accepted prose.

### Wire units and output

Frames are capped at 64 KiB of encoded bytes. Browser selection offsets are
full-target JavaScript-safe counters tagged `utf16_code_units`; the broker
treats them as opaque state unless a named conversion exists. Transmitted
context is independently capped at 512 Unicode scalars before and 128 after.
Ill-formed UTF-16 context fails closed before transport while valid astral
scalars remain intact. Suggestion output is capped at 64 scalars and eight
words against shared Rust/TypeScript multilingual fixtures.

The initial browser profile rejects NUL, C0/C1 controls, DEL, CR/LF/Tab,
unpaired surrogates, and dangerous invisible/bidirectional override controls.
It renders text nodes only. Leading whitespace belongs to the first accepted
word. After partial acceptance the old remainder is cleared and a new request
is scheduled. An exact typed-through remainder may stay visible without
flicker, but remains display-only until a provider response binds it to the new
revision and fingerprint.

The target page is untrusted: it can observe/remove the extension host, mutate
the field during dispatch, intercept the input event, navigate, or trigger
application/network side effects. Closed shadow DOM is isolation, not secrecy.
M2 therefore includes hostile-page observation, DOM sabotage, event
interception, and navigation cases before any non-fixture origin is eligible.

## State machine

| State | Entry | Allowed exit | Mandatory invalidation |
| --- | --- | --- | --- |
| `idle` | no eligible focused field | focus eligible field | disconnect/pause |
| `observing` | eligible target identified | debounce/request | field becomes denied |
| `pending` | revision-bound request sent | show/abstain | any newer revision |
| `visible` | matching unexpired result | type-through/accept/dismiss | focus, selection, composition, revision, policy, pause |
| `committing` | action revalidated | idle/observing | any mismatch before mutation |
| `paused` | controller receives authoritative pause state | explicit authoritative resume | notified controller cancels, clears, and sends no new request; broker repeatedly denies any unnotified controller before provider work |

Transitions are events, not timers polling mutable global state. Timer
callbacks carry the identity they were created for and become harmless when
that identity is no longer current.

## Repository contracts

| Path | Responsibility | Must not own |
| --- | --- | --- |
| `protocol/v1/schema.json` | wire shape, bounds, message discrimination | product policy implementation |
| `broker/` | validation, policy, state, cancellation, provider lifecycle, segmentation, metrics | DOM/UI or synthetic input |
| `adapters/chromium/` | DOM eligibility, bounded context, ghost view, target revalidation and edit | provider/model policy |
| `fixtures/web/` | deterministic supported/denied/race surfaces | production integration claims |
| `docs/research/` | primary-source facts and explicit inferences | shipped behavior claims |
| `docs/plan/` | execution and acceptance contracts | unverified status claims |

Protocol changes begin in the schema and land with producer, consumer, and
negative fixtures in the same change. Unknown versions, message kinds,
properties, overlong strings, oversized frames, and invalid state transitions
fail closed.

## Delivery milestones

### M0 — Contract and research baseline

Deliver:

- Vision V2 and the source-backed V2 landscape;
- this implementation plan;
- an explicit codename warning and unresolved-license gate; and
- protocol and capability terminology shared by code and docs.

Exit evidence:

- at least twelve directly relevant products/projects compared from primary
  sources;
- verified facts separated from product inferences;
- local documentation links resolve; and
- the repository status no longer says implementation has not started.

### M1 — Browser adapter trust substrate

Deliver:

- strict protocol V1 and bounded framing;
- broker policy, deterministic provider, latest-wins state, pause, and
  metadata-only metrics;
- Chromium ordinary-field controller and ghost view; and
- jsdom fixtures, a deterministic race harness, and a machine-readable
  simulated-evidence capability record.

The positive M1 cell is only a connected, visible, enabled, editable,
stable-identity, top-level `input type="text"` or `textarea`, with a collapsed
selection, no active composition, and no insertion-altering constraint. It
excludes other input types, selection replacement, readonly/disabled,
contenteditable, frameworks, iframes, shadow DOM, detached/replaced fields,
arbitrary origins, `maxlength`/validation behavior, native undo, and page
side-effect claims.

Exit evidence:

- recognized denied and unknown-identity fields cause no value read, outbound
  context, or provider call;
- every named invalidator plus 100 permuted late-result schedules cause zero
  stale display and zero stale insertion;
- accept-word, accept-all, type-through, dismiss, and pause pass deterministic
  tests, and acceptance waits for broker authorization; and
- Rust and TypeScript checks pass from lockfiles.

### M2 — Real Chromium native-message proof

Deliver:

- a minimal native host that translates Chrome native-message framing to the
  broker protocol without logging content;
- an unpacked-extension development installer that prints, but does not
  silently make, profile/system changes;
- one controlled origin with ordinary, sensitive, dynamic, and iframe cases;
- browser-driven tests for focus, geometry, zoom, scroll, navigation,
  composition, undo, and input events; and
- a signed capability receipt containing versions and outcomes.

Permissions begin empty and are runtime-granted for one exact
scheme/host/port. M2 forbids `<all_urls>`, restricted/file/incognito access and
arbitrary frames; validates service-worker sender tab/frame/origin; requires a
visible document, active tab, and focused browser window; and allowlists only
the development extension ID in the native-host manifest. Revocation,
navigation, window/tab loss, disconnect, and MV3 worker restart cancel/clear
before another read or commit. The persisted grant store and all messages use
the M2 policy epoch synchronization contract.

Exit evidence:

- 100/100 exact insertion and caret trials on named Chromium and OS versions;
- invalidation-to-hide p95 at or below 32 ms;
- accept-to-verified-insert p95 at or below 30 ms; and
- no permission broader than the controlled proof requires.

Latency gates use at least 1,000 measured interactions after 50 warmups, an
otherwise idle named machine, receiver-local monotonic endpoints, and
nearest-rank p95. Layout tests include end and mid-line caret (or explicitly
exclude mid-line), wrap, scroll, zoom, cleanup, no focus capture, first-word
emphasis, and high-contrast/accessibility behavior.

Stop if extension policy, isolated worlds, contenteditable semantics, or native
messaging cannot uphold the invariants. Narrow the advertised capability
instead of adding synthetic input.

### M3 — Obsidian signature-loop proof

Deliver:

- a CodeMirror 6 extension using documented editor transactions and
  decorations;
- Markdown-specific cursor, selection, undo, composition, and multi-pane
  fixtures;
- the same broker contract and acceptance actions as Chromium; and
- a capability receipt tied to the tested Obsidian/CodeMirror versions.

Exit evidence mirrors M2. Ordinary CodeMirror Markdown is a separate cell from
canvas, embeds, properties, search, and third-party editors.

### M4 — Local semantic provider gate

Deliver:

- a provider process boundary with cancellation, timeout, health, and model
  provenance;
- a frozen, non-sensitive evaluation set and deterministic evaluation runner;
- output shaping and confidence/abstention calibration; and
- documented model, tokenizer, data, and redistribution obligations.

Exit evidence on the named i7-12700H/16 GB machine:

- warm edit-to-visible p50 at or below 250 ms and p95 at or below 500 ms;
- result age above 600 ms displayed zero times;
- blind next-word usefulness at least 8/12; and
- measured retained text or net keystroke value above the deterministic lane.

Reject a model that misses the combined latency, usefulness, privacy, or
license gate. The deterministic lane remains a valid degraded product.

### M5 — Quiet personalization

Deliver only after M2–M4:

- explicit vocabulary/instruction import, export, provenance, and delete-all;
- accepted-only learning behind an independent switch;
- per-app quieting based on dismiss/contradict/erase signals; and
- an offline replay evaluator that compares policies without uploading prose.

Default broader raw-history learning remains out of scope. A store cannot ship
until delete-all survives a restart test with zero remaining records.

### M6 — Linux breadth and terminal go/no-go

Test Fcitx5/Ghostty/Codex independently under a short-lived manual arm. Use
Fcitx preedit/candidate/commit APIs only. Publish unsupported when identity,
composition coexistence, geometry, or commit cannot be proved.

Then test named GTK, Qt, XWayland, Firefox, and IBus tuples one cell at a time.
No result generalizes to a toolkit or Linux as a whole without its matrix
evidence.

M6 is deliberately P3, after the two target-API cells and quietness work. An
earlier independent Fcitx risk spike may inform feasibility, but cannot change
that product order or become an overnight delivery claim.

## Testing strategy

### Layer 1 — Pure and exhaustive where practical

- schema accept/reject fixtures for every message variant;
- Unicode-aware segmentation and length boundaries;
- field-policy tables, including adversarial autocomplete and type values;
- getter sentinels proving denial occurs before `.value` access;
- hostile output including controls, bidi overrides, unpaired surrogates, and
  HTML-shaped text;
- reducer/state-machine transition tables;
- frame size, truncation, unknown-property, and malformed-input cases; and
- provider spy assertions for zero calls under deny/pause/stale states.

### Layer 2 — Deterministic concurrency

Use controllable clocks and deferred providers. Test edit, selection,
composition, blur, visibility, DOM removal/replacement, navigation,
policy/permission/pause change, expiry, broker disconnect, MV3 restart, and
accept-versus-edit individually. Then permute at least 100 late-result
schedules, including two simultaneously focused broker sessions. Count stale
display and insertion separately, and add property-based sequences around
every discovered failure.

### Layer 3 — Target integration

Run the adapter against controlled real targets. Assert both DOM/editor state
and user-visible state. Verify normal app shortcuts when no suggestion is
visible, native input/undo semantics, caret placement, and absence of a second
mutation path.

### Layer 4 — Product evaluation

Freeze prompts before tuning. Score useful next-word, retained graphemes,
correction cost, bad appearances, latency, and abstention. Report hardware,
versions, sample size, and failures with the result.

## Continuous integration

Every change runs in a clean environment:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm ci
npm run check
git diff --check
```

CI pins major toolchain versions, restores dependency caches only, and builds
generated extension artifacts from source. Lockfile changes receive the same
review as source. No workflow receives repository write permission or a
secret for pull-request checks.

Release candidates add schema-fixture compatibility, dependency/license and
vulnerability review, browser-driven receipts, reproducible artifact hashes,
and an explicit human approval. Public release remains blocked on naming and
license decisions.

## Multi-agent workflow

Parallel work follows contract-first ownership:

| Role | Owns | Required handoff |
| --- | --- | --- |
| research lead | source matrix and inferences | citations, date, uncertainty, design consequences |
| protocol/broker lead | schema and Rust core | format/lint/test output, invariants, known gaps |
| adapter lead | target code and fixtures | type/test/build output, supported surface, known gaps |
| adversarial reviewer | cross-boundary review | ranked must-fix findings without implementation ownership |
| integrator | vision, plan, CI, docs, final merge | contract reconciliation and clean end-to-end evidence |

Agents use separate worktrees when available; otherwise they receive disjoint
path ownership. Shared protocol changes require an explicit schema handoff.
The integrator reads and tests every boundary instead of merging on agent
confidence. Each milestone ends with a fresh adversarial pass against the
vision, not only the implementation ticket.

Small commits should preserve a green main branch. Generated files are either
reproducible from a checked-in command or omitted. Decisions that narrow a
capability are recorded in the compatibility receipt and docs in the same
change.

## Observability and receipts

Each supported tuple emits a content-free auditable receipt with:

- source revision and protocol version;
- adapter/app/OS/compositor/toolkit versions;
- supported field and rendering/edit modes;
- policy certainty and denied-field cases;
- test suite identifiers and pass/fail counts;
- latency distributions and race count;
- known limitations and unsupported surfaces; and
- artifact hashes.

The receipt is evidence within the declared trusted-local-component boundary,
not cryptographic proof. Same-UID impersonation remains in the threat model.
Local suppression produces a local receipt even though no prose or broker
request exists.

Operational counters cover requests, abstentions, suppressions by reason,
cancellations, stale drops, displayed suggestions, actions, commit outcomes,
and latency buckets. They do not contain prose or stable cross-app behavioral
identifiers.

## Rollout and rollback

1. Deterministic fake transport on controlled fixtures.
2. Developer-only unpacked adapter on one controlled origin.
3. Opt-in local dogfood with a visible global pause and instant uninstall.
4. Closed alpha for two proven target-API cells.
5. Wider capability cells only after receipt review.

Every layer can be disabled independently: target, adapter, provider, learning,
and retention. A protocol or policy regression disables generation rather than
falling back. Rollback means returning to the last receipt-bearing build, not
silently changing the insertion mechanism.

## Decisions reserved for the user

Before public distribution, the user must choose:

- the public product and technical namespace;
- the source license and contribution model;
- the first semantic model and its distribution posture;
- whether remote providers exist at all;
- whether Tab is offered per adapter while a suggestion is visible; and
- the exact closed-alpha app/version matrix.

None of these decisions blocks the local, deterministic M1 foundation.
