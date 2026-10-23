# ADR 0008: the debug trace records text only when asked, in memory, for minutes

- Status: accepted
- Date: 2026-10-14
- Scope: `badi debug`, and the trace lines written by the broker, the Fcitx
  addon, the field observer and the editor clients

## Context

`badi debug` kept per-component counters and a last reason. That said *that*
nothing appeared, but not which suggestion arrived, which display path showed
it, whether the grey text was really on screen, or where. A live trial on
2026-10-13 blamed Badi for a doubled pill that turned out to be another
program's ([future plans](../../future%20plans.md)); a trace that follows one
suggestion from model to screen would have settled that in minutes.

The product contract kept typed text out of all diagnostics. To judge
suggestion quality and display while using Badi, the user asked for a debug
mode that records the suggestions themselves.

## Decision

`badi debug on` starts one shared, append-only trace,
`$XDG_RUNTIME_DIR/badi/debug-trace.jsonl`, for 15 minutes. Every component
appends one JSON line per event: the broker each wire frame in and out (by an
allowlist of fields; settings, style keys and samples are recorded by type
only), each model call with its latency and outcome, and each type-through
decision; the Fcitx addon each key with the action it took, and each display
of a suggestion as the overlay or the panel; the observer each request, each
preview drawn with its placement and box, declined with its reason, mapped by
the compositor and hidden with its cause; the editors their activity events.
While `badi debug watch` runs, it also records whether Hyprland has the
preview's layer on screen.

The trace is content-free by default: lengths, ids, reasons and positions.
`badi debug on --text` adds context tails, typed characters and suggestion
text. In both modes:

- Only `badi debug on` creates the trace; writers append to an existing,
  private (0600), owned regular file and never follow a symlink.
  `badi debug off` deletes it, which stops every writer at once.
- A run expires after 15 minutes; writers check the control file, and a
  disabled one at most once a second.
- The trace lives in the runtime directory, which is memory-backed and
  cleared at logout. `badi debug on` also asks the user manager for a
  one-shot timer (`systemd-run --user`) that deletes the run's files when it
  ends, unless a newer run replaced it (amended 2026-10-23). Nothing is sent
  anywhere.
- Writers stop at 16 MiB.
- The broker never records context text from a field that reports itself
  sensitive or not plain prose. Sensitive fields are denied before any
  adapter acquires their text, so none reaches the trace.
- A typed key is recorded with its text only from a field Badi may read
  itself, and a password or sensitive field's keys leave no line at all, so
  neither a password nor its length and rhythm reach the trace (amended
  2026-10-23: the Fcitx addon used to record every printable key in a text
  run).

## Consequences

- A `--text` trace is a record of what the user wrote for up to 15 minutes.
  It is opt-in per run, named in the command's output, and gone at logout.
- Every component checks the run control on each event; while debug is off
  this costs at most one failed open a second per process.
- The trace's lines are diagnostics, not a protocol: their fields may change
  with any release.
