# Adversarial review: should Badi be built?

> **Historical V1 gate:** its risks remain relevant, but
> [Vision V2](../../VISION-V2.md) and the
> [V2 implementation plan](../plan/vision-v2-implementation.md) supersede its
> three-target/48-hour delivery contract.

Status: research gate, 2026-08-30

This is the promised grill of the current proposal. It reviews the
[vision](../../VISION.md), [competitive evidence](competitive-landscape.md),
[Linux architecture](linux-architecture.md), and
[two-day delivery contract](../plan/two-day-delivery.md) as if the goal were to
find a reason **not** to build them.

## Verdict

**Go for one 48-hour integration experiment. Do not yet go for a public
product, universal Linux claim, or package launch.**

The opportunity is credible: the reviewed projects do not combine Cotypist's
low-friction co-writing loop with Linux-native compatibility and explicit
policy. The proposed broker-plus-adapters architecture addresses the real
platform boundary instead of treating an LLM call as the product.

The full three-target proof is not yet proven feasible. Chromium and Obsidian
have app-owned text APIs and are plausible. A safe, correctly positioned
Fcitx candidate and `commitString` path inside a live Codex TUI in Ghostty is a
low-confidence hypothesis. Local model usefulness and latency on this
integrated-GPU machine are also unmeasured. The build is justified only as a
risk-first experiment with permission to report a failed terminal target.

The exact `OmaType` name collision was a public-launch blocker and prompted the
rename to **Badi**. A separate active AI workflow CLI already uses the bare
`badi` command, so this project uses `badictl` and owned technical identities.
Package/trademark clearance is still required before distribution.

## Claims under pressure

| Claim or choice | Current confidence | What would falsify it |
|---|---:|---|
| Quiet partial acceptance is a valuable Linux product wedge | Medium | In the blinded 12-prompt session, fewer than 8 next words are useful, or the user reports that visible suggestions disrupt flow even after timing is tuned. |
| A broker plus app-owned adapters is the right architecture | High | Chromium and Obsidian cannot share lifecycle/policy semantics without target-specific exceptions leaking into the broker. |
| Rust broker + TypeScript app adapters is worth the language split | Medium-high | Contract plumbing consumes the H12 window or requires duplicated policy in adapters. |
| A thin C++ Fcitx addon can coexist with the current input method | Low | It misses printable events, disturbs foreign preedit/candidates, cannot obtain a usable cursor rectangle, or requires replacing the selected engine. |
| Ghostty/Codex can be a safe manual terminal cell | Low | Any of 20 trials misplaces the candidate, inserts into the wrong target, changes bytes, or submits input; unarmed typing emits any context/provider call. |
| A small local model can feel immediate and useful on 16 GB/iGPU | Low | Warm p50 exceeds 250 ms, p95 exceeds 500 ms, results routinely expire at 600 ms, or the blind usefulness gate fails. |
| `Always`, `Manual`, and `Never` can be trustworthy | Medium | An adapter serializes one byte from a hard-denied field, identity ambiguity inherits `Always`, or a stale revision inserts once. |
| Compatibility work can become a moat | Medium | Maintaining target adapters costs more than the acceptance value users report, or major apps do not expose stable enough APIs. |

These confidence labels are judgments from the available evidence, not measured
probabilities.

## The hardest questions

### Is this actually different from IBus Typing Booster or SmartComplete?

Not yet in shipped behavior. Today it is a researched position.

IBus Typing Booster already demonstrates mature, learned Linux completion.
SmartComplete demonstrates a young Fcitx-shaped deterministic pipeline.
Mac projects demonstrate stronger generative interaction. Badi becomes
distinct only if it proves all of the following together:

- app-owned, well-placed suggestions in high-value apps;
- one-word and whole-suffix acceptance without stale insertion;
- local semantic continuation that beats a phrase table in blind use;
- policy that visibly explains why context is allowed, manual, or blocked; and
- honest capability receipts rather than a blanket “works everywhere” badge.

If the 48-hour result is only a phrase list plus a popup, it is plumbing—not a
new product category.

### Is “a ghostwriter everywhere on Linux” an honest promise?

No. It is a direction, and must remain phrased that way.

Wayland does not give one process a generic API for another application's text,
caret, sensitive purpose, and verified edit. Fcitx itself documents different
paths and limitations across compositors, toolkits, Chromium/Electron, and
XWayland. Browser DOM, CodeMirror, Fcitx, future IBus, and target-specific APIs
will remain separate capability cells.

The honest product claim is: **one consistent co-writing experience in every
tested adapter, with visible unsupported states elsewhere.**

### Why not make Fcitx the universal adapter?

Because breadth and quality are different questions. Fcitx is the right early
experiment for terminal reach and a valuable later compatibility layer. It does
not guarantee app identity, complete surrounding text, distinct inline styling,
or reliable candidate placement on every Wayland route. Making it universal
would weaken browser-domain policy and the in-field rendering that makes the
product worth using.

The opposite extreme—only first-party plugins—also fails the vision by creating
an integration maintenance business. The proposed ladder is defensible only if
the broker contract and compatibility suite keep each adapter small.

### Is manual terminal arming actually safe?

It reduces exposure; it does not eliminate it.

Ghostty cannot reliably tell Badi that a raw-mode TUI is currently asking
for a sudo, SSH, token, or password value. Fcitx sensitive-purpose flags help
when the client supplies them, but absence is not proof of safety. A user can
still arm the addon at the wrong prompt.

Therefore terminal mode must remain local, ephemeral, learning-off, visibly
armed, and short-lived. It must reject every control character and never append
Enter. This is residual risk the user must see, not a privacy claim to bury in
implementation notes.

### Does `commitString` prove a successful terminal edit?

No. It proves that Fcitx dispatched text to its current input context. It does
not give Badi an application-level acknowledgment from the Codex TUI. That
is why terminal telemetry must say `dispatched-unverified`, never `applied`, and
must never retry automatically.

The 20 physical trials provide compatibility evidence, not a transactional
guarantee.

### Is the three-language stack too ambitious for two days?

Possibly. It is acceptable only because the boundaries match native extension
surfaces:

- Rust owns one small broker, policy state machine, cancellation, CLI, and
  model sidecar lifecycle.
- TypeScript owns Chromium and Obsidian, where the official APIs are JavaScript.
- C++ is confined to the Fcitx addon API.

Multi-agent work does not make integration free. The JSON Schema and golden
transcripts must freeze before lanes fan out. If agents invent three private
protocols, parallelism will make the deadline worse. Do not add a settings UI,
generic Electron injection, IBus, Firefox, or packaging to this slice.

### Is Rust premature when a Python prototype already exists?

The old `cotype` prototype is faster to modify, but its core assumptions are
the ones being rejected: global `evdev`, pointer-positioned layer-shell UI,
title denylisting, and `wtype`. Porting that daemon would risk preserving the
wrong boundaries because it is convenient.

Rust is justified if the initial broker stays narrow. The deterministic
provider should make broker/UI work independent of model integration. If the
broker alone cannot reach a fake-provider Chromium loop by H4, the team should
cut features, not replace the architecture mid-slice.

### Does “local” mean private?

No. Local inference removes a major network boundary, but context capture,
logs, native-messaging permissions, socket peers, crash dumps, memory copies,
swap, downloaded model licenses, and optional future providers all remain in
scope.

The first slice should make the smaller claim: no remote provider, no raw prose
in normal logs, no clipboard or screen context, bounded in-memory context, and
zero bytes transported from hard-denied fields. It should not promise that
plaintext leaves no physical remnants.

### Is the market evidence strong enough?

No. The interaction hypothesis is supported by Cotypist's behavior and a small
set of reviews, not by a broad Linux user study. Repository stars and recent
commits show awareness and activity, not willingness to adopt a persistent
input-layer tool.

The first live sessions are product discovery as much as engineering
validation. A technically correct suggestion that is rarely accepted or is
visually distracting is a failed product result.

### Could the name collision be ignored because the products differ?

No. [Aayush9029/OmaType](https://github.com/Aayush9029/OmaType) is also a local,
Omarchy-adjacent “type anywhere” product. It performs dictation rather than
prediction, but the overlap is close enough to confuse search, packages,
services, issue reports, and community discussion.

This finding drove the later rename to Badi. The adjacent dictation project
still matters for positioning and interoperability, but its technical namespace
is no longer reused here.

## Rejected shortcuts

The following would make a demo easier and the product less trustworthy. They
are explicitly rejected as production or passing first-slice routes:

- raw `evdev` capture as the normal context source;
- `wtype`, virtual-keyboard, clipboard/paste, or `xdotool` as an insertion
  fallback;
- a pointer-positioned layer-shell pill described as caret-aligned ghost text;
- AT-SPI observation treated as verified insertion authority;
- a Bash/Readline hook described as support inside Codex or Claude TUI input;
- terminal process names, titles, or OSC markers treated as safe prompt
  detection;
- a loopback WebSocket substituted for the registered browser native host;
- a deterministic phrase result described as local-LLM quality; or
- password/sensitive hints treated as a complete security boundary.

App-owned field-corner or status UI may explain why inline rendering is
unsupported. It does not turn a failed ghost-rendering cell into a pass.

## Stop and pivot criteria

| Failure | Required response |
|---|---|
| Any forbidden-field byte reaches the broker/provider, any stale insertion occurs, or terminal text is submitted | Stop every ambient demo. Fix and rerun the complete safety suite; there is no waiver. |
| Fcitx interferes with the selected IME or fails context/candidate/commit gates | Mark Ghostty/Codex unsupported and deliver a two-adapter prototype. Research explicit terminal/agent APIs later; do not synthesize around it. |
| Chromium native messaging cannot be made reliable | Fail the Chromium target for the slice. Do not open an ad hoc TCP/WebSocket service to rescue the schedule. |
| Chromium or Obsidian cannot render and revalidate app-owned ghost text | Mark that target partial/unsupported; a status pill is diagnostic evidence, not the promised interaction. |
| Two approved local models miss latency or usefulness gates | Keep the deterministic provider for integration tests, but state that semantic completion is not ready. Stop model shopping inside the 48 hours. |
| The user finds suggestions distracting after debounce/TTL tuning | Pivot the product default toward explicit/manual completion; do not optimize suggestion volume. |
| Adapter exceptions force policy or insertion logic into the broker | Revisit the contract before adding more targets; the architecture is losing its main benefit. |
| Badi package/trademark clearance fails or technical IDs collide | Keep distribution private and choose cleared package/desktop/service identifiers before release. |

## Decisions still owed by the user

Research can recommend defaults, but implementation should not silently choose
these:

1. Confirm whether the Badi brand can clear normal package, domain, and
   trademark review before public launch; keep `badictl` as the CLI identity.
2. Confirm the exact terminal acceptance target: a live Codex prompt in Ghostty
   is the recommended gate because it matches the original request.
3. Approve the temporary CMake installation and backed-up user-local Fcitx
   addon/reload needed for the terminal spike.
4. Approve or decline up to two local model downloads, their licenses, storage,
   and size budget. Declining still permits the deterministic integration proof.
5. Confirm the proposed shortcuts after checking current Omarchy/Hyprland and
   Fcitx bindings.
6. Choose a source-code license before accepting code from projects with
   different MIT, GPL, and AGPL obligations. No competitor code should be copied
   during the research phase.

## Final recommendation

Proceed only with the bounded experiment in the delivery plan. Start with the
Fcitx feasibility probe and protocol contract in parallel, guarantee a real
Chromium loop for the first live session, then add Obsidian. Keep the terminal
cell binary: native Fcitx behavior passes on the named tuple, or it is reported
unsupported.

If the experiment produces three safe targets, a useful local model, and a user
who prefers the interaction to typing unaided, the concept has earned a product
iteration. Anything less still produces valuable compatibility evidence, but
does not justify “Cotypist for Linux” as a shipped claim.
