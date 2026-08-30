# Badi Quickshell control center

This directory is a repo-owned Quickshell 0.3.1 control surface for Badi. It is
not a policy engine and it does not install, edit, or patch Omarchy or user
configuration.

The UI is deliberately thin:

- all reads come from `badictl overview --json`;
- all mutations use fixed `Process` argument arrays;
- no command runs through a shell;
- settings changes replace one complete, validated `badi.settings.v1` document
  through the broker with compare-and-swap protection;
- QML never reads or writes Badi's JSON settings or aggregate store directly;
- missing or malformed state renders as unavailable, never as allowed or ready.

Both status reads and mutations have a five-second UI deadline. The process is
sent `SIGTERM`, followed by `SIGKILL` after 500 ms if it does not stop. The CLI
also owns its shorter broker handshake and response deadlines.

## Run from the repository

Quickshell can load a configuration from an explicit path without copying it
into the user's configuration:

```sh
qs --path "$PWD/ui/quickshell/badi/shell.qml"
```

Run that command from the repository root. It does not install the control
center. `badictl` must be available on `PATH`; the broker must be running for
live status and pause controls.

The first launch opens a normal `FloatingWindow`. Once the shell is running,
its window can be controlled through Quickshell's per-instance IPC:

```sh
qs --path "$PWD/ui/quickshell/badi/shell.qml" ipc call badi show
qs --path "$PWD/ui/quickshell/badi/shell.qml" ipc call badi hide
qs --path "$PWD/ui/quickshell/badi/shell.qml" ipc call badi toggle
qs --path "$PWD/ui/quickshell/badi/shell.qml" ipc call badi refresh
```

Keyboard behavior:

- `Ctrl+R` refreshes content-free status;
- `Ctrl+P` persistently pauses or resumes through a revision-checked settings
  replacement when authority is coherent; if settings are unavailable while
  Badi is active, it can still issue a process-local emergency pause, but it
  never resumes a degraded broker;
- `Escape` hides the window; and
- controls participate in normal Qt Tab/Shift+Tab focus traversal.

Headed keyboard and assistive-technology behavior still requires physical
Omarchy/Hyprland validation; the source does not claim that static inspection
is accessibility evidence.

The configuration sets its own Quickshell shell ID and application ID. It does
not occupy the default Quickshell config and does not modify
`/usr/share/omarchy`.

## Required `badictl` contract

`BadiClient.qml` accepts only a top-level `badi.overview.v1` object. Every
settings mutation additionally requires `overview.settings` to be a complete,
strict `badi.settings.v1` document and requires
`overview.broker.settings_revision == overview.settings.revision`. `badictl`
retries that two-read coherence check at most three times. Missing nested
values display as `Not reported` or blocked, and missing or degraded control
authority disables mutation.

The broker coherence token consumed by the UI has this shape:

```json
{
  "broker": {
    "reachable": true,
    "provider": "phrase_v1",
    "paused": false,
    "authority_epoch": 11,
    "settings_revision": 7,
    "control_plane_degraded": false,
    "sessions": 0,
    "socket_mode": "0600",
    "max_frame_bytes": 65536
  }
}
```

Runtime pause and persisted `settings.paused` are separate facts and may
legitimately differ. Mutations require matching settings revisions, not matching
pause values. A process-local pause acknowledgment is ordered after all
previously queued aggregate outcomes; new context acquisition is revoked before
that fence is awaited.

The exact settings document edited by the UI is:

```json
{
  "schema": "badi.settings.v1",
  "revision": 7,
  "paused": false,
  "subjects": [
    {
      "identity": {
        "kind": "browser_origin",
        "adapter": "chromium",
        "scheme": "http",
        "host": "localhost",
        "port": 4173
      },
      "permissions": {
        "suggest": "allow",
        "display": "allow",
        "context_read": "allow",
        "learn": "block",
        "retention": { "mode": "none" }
      }
    }
  ]
}
```

The UI verifies exact keys, supported identity shape, permission dependencies,
canonical subject ordering, no more than 64 subjects, and the safe-integer
revision before enabling a mutation. Rust validation remains normative. A
maximum-shape 64-subject settings document is tested against both the 64 KiB
private-file and wire-frame limits.

The relevant text-free overview fields are:

```json
{
  "privacy": {
    "context": "focused_supported_field_only",
    "max_before_chars": 512,
    "max_after_chars": 128,
    "clipboard": false,
    "screen": false,
    "network": false,
    "adaptive_writing_memory": "not_implemented",
    "outcome_aggregates": "disabled",
    "aggregate_semantics": "broker_emitted_and_commit_requested_not_delivery_confirmed",
    "stored_metadata": "origin_provider_utc_day_counts",
    "max_retention_days": null,
    "memory_records": 0,
    "memory_bytes": 0,
    "memory_store_available": true,
    "memory_command_available": true,
    "memory_integrity": "healthy",
    "memory_write_failures": 0,
    "memory_dropped_signals": 0,
    "learning_available": false
  },
  "support": {
    "browser_permission": "static_exact_document",
    "badi_policy": "exact_origin_subjects",
    "scope": "http://localhost:4173/chromium.html",
    "evidence_class": "historical_not_current_tree_proof",
    "evidence_commit": null,
    "adapters": [
      "chromium_fixture",
      "obsidian_unsupported",
      "terminal_unsupported"
    ]
  }
}
```

When the optional aggregate file is corrupt or has ambiguous persistence state,
the broker preserves it and reports `memory_store_available: false`, null
`memory_records`/`memory_bytes`, and `memory_integrity: "unavailable"`. It does
not silently replace evidence or enable learning. The explicit `memory clear`
operation removes the safely identified private file and reopens an empty
store; settings mutations remain blocked until that repair succeeds. A settings
commit whose durability is unknown keeps the whole broker fail-closed and
requires a coherent settings reload/restart rather than treating memory clear
as authority repair. Subject/retention controls are disabled while the store is
unavailable. Subject-identical pause-only settings transitions remain valid,
and the process-local pause command remains an emergency fail-closed control.

`models.writing.advice` is the broker's model-advice document;
`models.writing.configured` and `models.writing.installed` are separate facts.
Hardware fit remains candidate advice; `runtime_ready` is the quality/runtime
gate and is never inferred by QML.

The UI invokes these mutations:

```text
badictl overview --json
badictl settings replace --if-revision N --json DOCUMENT
badictl pause on
badictl pause off
badictl memory clear
```

For each settings action, QML deep-clones the validated document, increments
`revision` exactly once, applies one bounded change, validates the result, and
sends the complete document:

- pause changes only top-level `paused`;
- the localhost suggestion bundle changes `context_read`, `display`, and
  `suggest` together because the schema forbids suggesting without acquisition
  and display authority;
- blocking that bundle also blocks `learn` and sets retention to `none`;
- outcome collection first enables `learn` with memory-only retention; and
- persistence is a separate choice of memory only, 7, 30, or 90 days.

In the current implementation, `learn` authorizes only daily per-origin
interaction counters. The persisted keys are origin, provider, UTC day, and
counts. “Shown” means emitted by the broker and “accepted” means a commit was
requested; neither proves browser display or applied text. This does not mean
adaptive writing, and no typed text, accepted text, phrase, style feature,
document, or fine-grained timestamp is retained.

Retention is enforced when signals arrive, settings change, status is read,
the broker starts, and by a 60-second idle reconciliation sweep. A failed expiry
write preserves and disables the store rather than reporting stale data as
healthy.

The revision precondition and the overview coherence check are mandatory. A
human or agent changing settings in parallel causes a conflict that the CLI
must reject; the UI then refreshes instead of silently overwriting the newer
state.

## Honest policy scope

The editable subject is the development origin `http://localhost:4173`. Badi's
identity gate is origin-wide. Independently, the tracked Chromium development
manifest injects only into `http://localhost:4173/chromium.html`; widening that
manifest would not make Badi's origin subject document-specific.

This control center is not yet a generic app/site allowlist. Native application
identity, arbitrary browser origins, Manual activation, Obsidian, and terminal
controls remain unsupported and are never simulated. The visible scope rows call
out the currently relevant adapter exclusions.

## Structure

- `shell.qml` owns only the window and harmless show/hide/refresh IPC.
- `BadiClient.qml` is the only process/JSON boundary.
- `ControlCenter.qml` owns information hierarchy and interaction state.
- `SectionCard.qml`, `InfoRow.qml`, `SettingRow.qml`, `ToggleControl.qml`,
  `StatusPill.qml`, and `ActionButton.qml` are reusable presentation pieces.
- `BadiTheme.qml` contains the compact visual token set.

Keep policy semantics in Rust and target mechanics in adapters. When the JSON
contract evolves, update `BadiClient.qml` and this document together; do not
teach individual visual components to infer policy.
