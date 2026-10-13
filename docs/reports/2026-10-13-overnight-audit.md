# Overnight audit — 2026-10-13

Autonomous YOLO session fired by timer at 00:59 CEST while Ahura slept.
Codex had just committed `7cb975b` "Complete ranked desktop writing slices
and runtime verification" (77 files: conservative field-language selection in
`broker/src/language.rs`, AUR candidate under `packaging/aur/`, lock-state
handling in the accessibility observer, Lab/runtime updates). That work was
preserved untouched; this session built on top of it.

## Audit findings (what works, what is missing, what is thin)

- **Broker (Rust)** is the strong core: exact session/focus/revision/
  fingerprint binding, one-shot acceptance, context leases, type-through
  carry (ADR 0004), Escape quiet, compare-and-swap settings. Thin spots from
  the backlog: partial-vs-completed word discrimination before contextual
  lookup (`mire` → `Mirella` must complete, `marin` ↛ `Marinella`; dictionary
  membership is the wrong splitter), short-fragment language coverage,
  `broker/src/settings.rs` and `broker/src/engine/tests.rs` flagged for
  splitting, and `authority.ack` recorded but never enforced.
- **Fcitx5 addon + AT-SPI observer** implement IME-parity (ADR 0003,
  append-only `commitString`, dispatched-unverified) with foreign-composition
  yield and grey preview. Thin: browser event cost on large pages
  (`daemon.py` focus handling, `field.py` serialization); EditContext/Firefox/
  Discord/Grok surfaces need live trials and were excluded overnight.
- **Editor/shell clients** (Obsidian, Bash) own exact acquisition/mutation
  and native undo. Missing by design backlog: Fish/Zsh/Neovim via their own
  buffers.
- **Prediction Lab / model**: Qwen3-1.7B pinned; French/Spanish stay Lab-only;
  the frozen UD r2.16 confirmation set's first use is runtime-loading
  equivalence only, explicitly not qualification (full gates: p95 ≤ 550 ms,
  ≥ 40 cases/language, ≥ 60 % useful on-time, zero harmful, human review).
  Same-window context and style learning are barred from production by the
  VISION context firewall pending independent Lab evidence.
- **Protocol**: an ADR retiring protocol v1 is still unwritten (docs-only,
  low leverage tonight).

## Research notes

Comparable systems (Cotypist, TabType, Oma Tab, Fcitx5 prediction plugins,
llama.cpp inline completion) confirm Badi's existing direction rather than
replacing it: prefix/KV-cache reuse across keystrokes is the dominant latency
lever (already Badi's warm path at ~220–250 ms median first grey word),
speculative decoding did not help on this CPU (already measured in the
backlog), and local-only inference does not by itself remove prompt-injection
risk from page content — which is why Badi's context firewall and fail-closed
binding matter more than raw coverage. No external finding justified a model
or prompt change tonight; those remain gated on Lab qualification.

## Chosen target

**Enforce `authority.ack`** (backlog, Audit follow-ups). Rationale: it was the
only open item that is simultaneously safety-relevant (a client operating
under a new policy epoch without proving it retired old bindings), small
enough to finish and fully verify headlessly, and aligned with what every
real adapter already does (ack before opening sessions, per
`adapters/fcitx5/WIRE_PROTOCOL.md` and `adapters/shared/broker-client.mjs`).
Assumption recorded: gating the data plane (session open, context, suggestion
request) is sufficient because epoch advances already close owned sessions
server-side, so control/commit paths cannot be reached by an unacknowledged
connection.

## What changed

- `broker/src/engine/authority.rs`: `require_current_authority_ack` — a
  registered policy connection whose acknowledged epoch differs from the
  current epoch gets `BrokerError::Stale`; unregistered (non-policy)
  connections are unaffected.
- `broker/src/server.rs`: the gate runs at the head of `session.open`,
  `context.changed` and `suggest.request` handling. The error maps to a wire
  `error` frame; the connection stays open so the client can still ack.
- Tests: `authority_ack_gate_tracks_the_current_epoch` (engine: unacked
  denied, future ack rejected, current ack admitted, epoch advance re-blocks,
  stale re-ack insufficient, non-policy unaffected) and
  `unacknowledged_authority_cannot_open_a_session` (wire: open before ack →
  `error` + zero sessions, ack → session opens). The wire test was observed
  failing before the fix (no error frame; session opened silently).
- Docs: broker runbook "Authority acknowledgement" contract, Fcitx wire note,
  backlog item closed in `future plans.md`.

## Verification (all on 2026-10-13, this machine)

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | pass |
| `cargo test --workspace --all-features --locked` | pass (all suites, 0 failed) |
| `cargo +1.85.0 check --workspace --all-targets --all-features --locked` | pass |
| `npm run editors:integration` (real broker, shared JS client) | pass |
| `npm run fcitx5:integration` (real broker, native transport ×2 identities + type-through) | pass |
| `npm run docs:check` | pass (101 links) |

Evidence classes: fixture/unit (engine + wire tests) and real-broker lanes
(editors, Fcitx) — no real-model or live-app claims are made. The change is
not installed to the desktop; the running broker still predates it.

## Unverified / next

- Live desktop behavior after install (needs `scripts/install-desktop.py`
  and a graphical session trial) — deliberately not done overnight.
- Recommended next slices, in order: partial-vs-completed word splitter
  (broker unit tests on the named `mire`/`marin` cases, Lab screening before
  any production claim); browser event-cost fixtures for the observer; ADR
  retiring protocol v1.
