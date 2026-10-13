# ADR 0005: desktop session signals withhold authority, never grant it

- Status: accepted
- Date: 2026-10-13
- Scope: the broker's runtime pause, automatic scheduling and the
  [session helper](../../adapters/session/README.md)

## Context

Suggestions are bound to a field's identity, revision, fingerprint, caret,
focus and expiry, but nothing told the broker that the machine was about to
sleep or that another login session had taken the screen. A suggestion shown
before suspend could still be inside its reading window after resume, and a
stale grant could meet a Tab pressed before the adapter reacquired its field.
Battery power was equally invisible: every key in a known field started a
generation that the next key cancelled.

logind, UPower and power-profiles-daemon expose these facts on D-Bus. The
broker runs with no network and no D-Bus client, and the observer belongs to
Fcitx fields only, so neither is the right owner.

## Decision

A separate user service, the session helper, observes logind and power state
and reports it to the broker over a dedicated protocol v2 capability,
`desktop_signals`. The broker treats a report as an input that can only remove
authority or reduce work:

- An inactive or sleeping session is a runtime pause. Entering it advances the
  authority epoch and revokes context, pending inference, visible suggestions,
  pending commits and the type-through carry before the broker answers.
  Leaving it advances the epoch again so every adapter reacquires.
- Low power selects automatic scheduling from the user's `battery_mode`. It
  never changes authority, explicit requests or the resident model.
- A report belongs to its connection. When the helper exits, its hold and
  low-power state go with it; the broker never stays held by a dead helper.
- The helper takes a logind `delay` sleep inhibitor and releases it only after
  the broker acknowledged the hold, or after a bounded failure. It never holds
  sleep longer than its 1.5 s report timeout.

Any same-UID process could send the same report (ADR 0001); the worst it can
do is make Badi quieter.

## Consequences

- Sleep and session switches no longer depend on each adapter's own expiry to
  retire suggestions. Adapters need no change: they already retire grants at
  every new epoch.
- Without the helper, behavior is exactly as before. Unknown session state
  counts as active and unknown power as mains, because the adapters' own
  focus, lock and expiry checks still apply.
- logind's `LockedHint` is not used as evidence of anything. The observer's
  fresh Omarchy lock check remains the unlock proof.
- Desktop shortcuts live in the same helper because they are desktop session
  integration, not field acquisition; a shortcut can pause or open settings
  but never request or accept.
