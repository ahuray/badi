# Badi Omarchy panel

The Omarchy plugin `io.github.ahuray.badi`:

| File | Role |
| --- | --- |
| `BarWidget.qml` | The bar widget |
| `BadiMark.qml` | The **b** speech-bubble mark |
| `DesktopPanel.qml` | The floating settings window |
| `BadiClient.qml` | The shared `badictl` client |

It uses only the host's `qs.Ui` and `qs.Commons` modules, `badictl` and the
installed `badi-desktop` helper.

Left-click the mark to open or close settings; right-click pauses or resumes
(a paused mark is dimmed). The window has three views:

- **Writing:** model state, pause, editor launch buttons, key help, counters and
  a short privacy note.
- **Applications:** the app and website list modes, allow or block per
  supported app, and exact website rules. Blocking revokes context reading,
  suggestions and display; learning stays blocked and retention `none`.
- **System:** login startup, model start, stop and restart, service status and
  terminal commands.

Settings writes use the broker's compare-and-swap API and fail visibly on a
conflict instead of retrying over another change. `badi settings` toggles the
window through the `badi-writing` IPC target. The terminal equivalents are in
the [README](../../README.md#choose-where-badi-writes).

## Install and update

`scripts/install-desktop.py` installs `badi`, `badi-desktop`, `badictl` and the
launcher entry. The panel itself updates with:

```sh
python3 scripts/install-omarchy-ui.py
python3 scripts/install-omarchy-ui.py --wait-for-unlock 3600   # stage and wait for unlock
```

The updater runs the source checks, waits for an explicitly unlocked desktop
(it never unlocks it), backs up replaced files under
`~/.local/state/badi/ui-backups/<UTC time>/` (newest three kept), removes files
the manifest no longer loads, restarts the Omarchy shell the supported way and
checks that the new window answers IPC. `omarchy plugin disable
io.github.ahuray.badi` hides the mark.

## Client contract

`BadiClient.qml` never reads document text, model state or settings files
directly. Every command is a fixed argv array through Quickshell `Process`; JSON
is one non-executable argument. It runs at most one overview read and one
settings write, and a write invalidates any older read. Deactivation terminates
every child (SIGTERM, then SIGKILL), stale exit handlers change nothing, and a
reactivation during teardown queues exactly one fresh read.

## Pinned host

`compatibility.json` records the Omarchy host files the plugin depends on,
byte-compared with Omarchy `v4.0.3` (`0534987009061cbe2dacdde4ad564092ab698d12`)
under Quickshell 0.3.1 and Qt declarative 6.11.2. The strict check fails when the
installed host drifts. The manifest declares only the `bar-widget` kind.

## Checks

```sh
npm run omarchy:check                                     # portable gate
BADI_OMARCHY_REQUIRE_HOST_CHECKS=1 bash ui/omarchy-plugin/tests/check-source.sh
bash ui/omarchy-plugin/tests/run-client-lifecycle.sh      # needs qs
```

- **`omarchy:check`** runs ShellCheck, the JSON and source contracts and
  process-group escalation against a fake `badictl` that ignores TERM. When
  `qs` is present it also runs the headless client lifecycle.
- **`run-client-lifecycle.sh`** loads the real `BadiClient.qml` offscreen with
  a fake `badictl`. It covers settings validation, a compare-and-swap write that
  keeps every subject and both list flags, teardown during a write, and
  invalidation of an in-flight read.
- **`tests/check-host.sh`** is CI's strict lane. It runs against an exact
  Omarchy checkout (`BADI_OMARCHY_ROOT`) with the official validator, host-file
  hashes, `qmllint` and the lifecycle.

These checks prove source and process contracts, not the window's look, focus,
scaling or multi-monitor behavior, which need a real Omarchy session.
