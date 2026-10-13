# ADR 0007: retire protocol v1

- Status: accepted
- Date: 2026-10-13
- Scope: the broker's wire protocol, the settings document schema, and every
  client that negotiates with the broker

## Context

The broker accepts wire protocol v1 and v2 (`MIN_PROTOCOL_VERSION` is 1,
`CURRENT_PROTOCOL_VERSION` is 2 in
[`broker/src/protocol.rs`](../../broker/src/protocol.rs)). No client needs v1:
every client in the public history negotiates v2 only — the Fcitx addon
(`adapters/fcitx5/src/transport.cpp`), the shared editor client behind
Obsidian, Bash and Neovim (`adapters/shared/broker-client.mjs`), the session
helper (`adapters/session/link.py`), the Writer extension
(`adapters/libreoffice/extension/pythonpath/badi_libreoffice/broker.py`),
`badictl` (which the Omarchy panel shells out to) and the battery trial. The
Lab worker links the broker library instead of speaking the wire protocol.
v1 acceptance therefore protects only pre-public development builds, while
keeping a second behavior surface alive in the broker:

- **Negotiation floor.** `select_version` accepts 1, and the
  `WireEnvelope::session`/`global` constructors stamp `v: 1` until a caller
  overrides it with `at_version`.
- **Offset units.** v1 context selections count UTF-16 code units; v2 counts
  Unicode scalar values (`validate_for_version` on the context payload).
- **Target kinds.** v1 rejects `desktop_application`, `neovim` and
  `libreoffice` targets; v2 additionally requires a browser target to carry
  an origin.
- **Hello and handler gates.** `long_reading_window`, the `terminal`,
  `session`, `neovim` and `libreoffice` adapter kinds, the
  `desktop_signals` and `style_key` capabilities and the probe handler are
  already refused below v2, one check at a time.
- **Suggestion lease.** v2 native-panel and editor sessions get the human
  reading window; a v1 session keeps the configured lease
  (`has_reading_window` in `broker/src/engine/mod.rs`).
- **Health shape.** A v1 connection gets the frozen v1 counter set
  (`MetricsSnapshot::for_protocol_v1` drops the no-suggestion breakdown and
  word savings) and no `desktop` block.
- **Policy reason.** Browser defaults report `matched_rule` instead of
  `matched_default` because v1 clients do not know the newer reason.
- **Settings over the wire.** A v1 connection must send a
  `badi.settings.v1` document; the broker merges it over the current v2
  policy (`preserving_v2_policy_from`) and projects settings status back per
  connection version.
- **Settings on disk.** A `badi.settings.v1` file is migrated atomically to
  canonical v2 on load (`broker/src/settings/store.rs`), independently of
  the wire version. This is the only keeper that protects user state rather
  than a client.
- **Artifacts and tests.** `protocol/v1/schema.json` and its examples,
  `broker/schemas/badi.settings.v1.schema.json`, and the v1 arms of the
  protocol, server, metrics and settings tests.
  `protocol/v1/accept-word-fixtures.json` is word-segmentation test data
  used by `broker/src/segment.rs`, not wire contract.

## Decision

Retire v1 in this order. Each step ships on its own and fails closed;
nothing below weakens field binding, revision, fingerprint, caret, focus or
expiry checks.

1. **Raise the floor.** Set `MIN_PROTOCOL_VERSION` to 2 and stamp new
   envelopes with the current version instead of 1. A v1 `hello` then fails
   negotiation before any session, grant or settings access exists; the
   refusal itself is stamped v2, since a v1-only peer cannot proceed either
   way. Add a regression test that a v1 hello is refused and opens nothing.
2. **Delete the wire differentials.** With only v2 negotiable, remove the
   UTF-16 offset arm, the version arms of target validation, the per-check
   hello and probe gates (their conditions become unconditional), the
   version test in `has_reading_window`, the `for_protocol_v1` health
   projection and `desktop` omission, and the `matched_rule` alias, so
   browser defaults report `matched_default` like every other target.
3. **One settings schema on the wire.** Remove the per-connection schema
   expectation, the `preserving_v2_policy_from` merge and the versioned
   settings-status projection; the control plane speaks only
   `badi.settings.v2`.
4. **Retire the on-disk migration last.** Keep the load-time
   `badi.settings.v1` → v2 migration for one release after step 1: it costs
   nothing at runtime and is the only step whose removal can strand a
   settings file written by a pre-v2 broker that has not run since. After
   that release, delete `SETTINGS_SCHEMA_V1`, the migration and
   `badi.settings.v1.schema.json`; an unrecognized settings schema then
   fails the settings load closed for `badi doctor` to report, as corrupt
   settings already do.
5. **Remove the artifacts.** Delete `protocol/v1/schema.json` and its
   examples, move `accept-word-fixtures.json` beside the segment tests that
   consume it, and update the broker runbook's protocol section and the
   Fcitx [wire document](../../adapters/fcitx5/WIRE_PROTOCOL.md).

## Consequences

- One offset unit, one lease rule, one health shape and one settings schema
  remain, so a new capability no longer needs a v1 gating decision and the
  validation surface shrinks to the contract every shipped client already
  uses.
- A pre-public v1-only build stops at `hello` with no session, instead of
  running with silently different offsets, leases and counters.
- Until step 4 lands, an old settings file still migrates itself on first
  load; afterwards, the same file is a doctor-reported configuration error
  rather than a silent downgrade of the user's policy.
