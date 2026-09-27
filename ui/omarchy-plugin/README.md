# Badi Omarchy writing controls

This directory is the Omarchy plugin `io.github.ahuray.badi`: a bar widget
(`BarWidget.qml`) whose speech-bubble **b** mark (`BadiMark.qml`) opens the
floating settings window (`DesktopPanel.qml`). `BadiClient.qml` is the shared
`badictl` client. Everything the plugin loads lives here; it uses only the
host's `qs.Ui` and `qs.Commons` modules, `badictl`, and the installed
`~/.local/bin/badi-desktop` helper.

The settings window has three views:

- **Writing:** model state, persistent pause/resume, editor launch buttons,
  Tab instructions, counters, and a concise privacy explanation.
- **Applications:** independent native app permissions. Blocking an app revokes
  context reading, suggestion generation, and display. Other subjects are preserved;
  learning stays blocked and retention stays `none`. Unsupported apps are listed
  explicitly.
- **System:** login startup, model restart/stop/start, service status, and selectable
  terminal commands. Turning startup off takes effect at the next graphical login;
  stopping the process releases model memory now. Neither changes app permissions.

Left-click the Badi mark to open/close settings; right-click to persistently
pause/resume. A paused icon is dimmed and its tooltip reports the state. Escape
closes the floating window; buttons are keyboard focusable with accessible labels.
The window uses Omarchy colors, typography, spacing and button components.
[README.md](../../README.md) lists which applications currently get suggestions.

## Install and update

`python scripts/install-desktop.py` installs `badi`, `badi-desktop`, `badictl`, and a
standard Badi application launcher entry, alongside the native runtime described
in the Fcitx runbook. Each command is installed once under `~/.local/lib/badi/`;
the names in `~/.local/bin` are links to it. The helpers use the installed broker
CLI, so everyday controls do not require the checkout. The advanced `badictl`
interface remains available.

`python scripts/install-desktop.py --broker-only` updates the model service and
commands without restarting Fcitx or touching its addon/profile. It can run while
the desktop is locked. The complete installer requires confirmed unlocked state
before replacing native files and again before restarting the input service.
Both modes skip files that are already identical, back up each file they
replace or remove, and write its `changes.json` map before the change; see the
Fcitx runbook for the layout and `scripts/badi_install.py restore`.

To update the already installed Omarchy plugin:

```sh
python scripts/install-omarchy-ui.py
# Or stage the validated UI and wait up to one hour for normal unlock:
python scripts/install-omarchy-ui.py --wait-for-unlock 3600
```

The updater verifies the existing plugin identity and runs the strict source
checks, snapshots the source, waits for explicit unlocked state, and backs up
replaced plugin files under `~/.local/state/badi/ui-backups/<UTC time>/` (newest
three kept; identical files are neither backed up nor rewritten). It also backs up
and removes files earlier versions installed but the current manifest no longer
loads (the former `Panel.qml`). It then uses the supported Omarchy shell restart
and verifies that the new window answers IPC. It never unlocks the desktop. The
restart keeps editor windows but clears cached nested QML components.

To disable only the Badi icon, run `omarchy plugin disable io.github.ahuray.badi`.
The ordinary Fcitx instance is not restarted by the launcher.

## Terminal controls

```sh
badi status                    # readable health and counters
badi status --json             # broker health for scripts
badi settings                  # toggle the Omarchy settings window
badi settings --json           # read the persistent settings document
badi pause                     # persists through model restart
badi resume
badi app omawrite off          # block context reading and predictions
badi app omawrite on
badi app xournalpp off
badi autostart off             # leave the current model process running
badi service stop              # release model memory
badi service start
badi service restart
badi service status            # JSON; available while the broker is stopped
badi doctor                    # JSON health, including the loaded native addon
badi logs                      # last 60 broker journal entries
```

Settings mutations use the broker's revision compare-and-swap API. Conflicts
fail visibly rather than retrying over another change. Window writes carry the
optional `all_web_origins` flag (`badi site all on`) through unchanged. `doctor` exits nonzero
when the broker is unreachable/degraded or the running native addon is missing.
The CLI uses Linux user services; the GUI and desktop installer target Omarchy.
This plugin is not a portable StatusNotifier tray implementation.

When startup fails, the System view shows the classified current-invocation
error. `badi doctor` includes actionable `problems` for missing model files,
resource or integrity failures, unreachable broker, degraded settings, an
unloaded addon, and a running broker that differs from the last install receipt.
Its notes name the content-free classes of requests that showed no suggestion.
Journal prose is not copied into these diagnostics. The service restarts a
failed broker with backoff; only a missing, unsupported or too-large model
(exit 78) leaves it stopped until Start or Restart after the fix. Service
activation still precedes model readiness, which the broker health probe
verifies separately.

`scripts/badi-desktop.py` is installed as `~/.local/lib/badi/badi-desktop.py`, beside the
`badi_install.py` helpers it imports, and linked as `~/.local/bin/badi` and `badi-desktop`. It starts the
persistent `badi-broker.service` and opens regular editors through transient user
services, preserving Wayland/runtime values and selecting their Fcitx modules.
The panel controls only the private desktop socket,
`$XDG_RUNTIME_DIR/badi/broker.sock`, and only while it is an owned private socket;
otherwise it reports the model offline. It obtains settings through `badictl`,
and every settings write is compare-and-swap bound to the revision it read.

## Client contract

`BadiClient.qml` never reads document text, model state, or settings files
directly. All commands use fixed argv arrays through Quickshell `Process`; JSON
remains one non-executable argument and no command is evaluated by a shell.
It runs at most one overview read and one settings write. A write first
invalidates any overview read that began before it. Deactivating or disposing
the client terminates every outstanding child: it invalidates the active
lifecycle, requests SIGTERM, retains a bounded SIGKILL escalation, and forces
SIGKILL on disposal. Stale exit handlers cannot update state or start another
command. A reactivation while teardown is pending queues exactly one fresh
overview for the new lifecycle.

## Pinned compatibility cell

- installed Omarchy package: `4.0.3-1`
- official source: `https://github.com/omacom/omarchy.git`
- official source tag: `v4.0.3`
- source commit: `0534987009061cbe2dacdde4ad564092ab698d12`
- Quickshell: `0.3.1-1`
- Qt declarative: `6.11.2-1`

The installed `shell.qml`, `PluginRegistry.qml`, `qs.Ui`/`qs.Commons` module
indexes, plugin validator, `PluginShellApi.qml`, and `PluginBarApi.qml` were
byte-compared with that source commit. Their hashes are recorded in
`compatibility.json`, and the strict source check fails when a host drifts.
The manifest declares only the `bar-widget` kind: the widget sits in the bar
layout, and `badi settings` toggles its window through the `badi-writing` IPC
target.

## Validation without installation

From the repository root:

```sh
npm run omarchy:check
omarchy plugin validate ui/omarchy-plugin
BADI_OMARCHY_REQUIRE_HOST_CHECKS=1 \
  bash ui/omarchy-plugin/tests/check-source.sh
bash ui/omarchy-plugin/tests/run-client-lifecycle.sh
```

`npm run omarchy:check` is the portable gate. It runs ShellCheck and the
JSON/source contracts, proves that the fake `badictl` really ignores TERM but
dies to KILL, and exercises private process-group escalation with nested
TERM-ignoring watchers. It requires neither Omarchy nor Quickshell; when `qs`
is present it also runs the headless client lifecycle, and otherwise reports
that this optional local run was skipped.

`tests/run-client-lifecycle.sh` loads the real `BadiClient.qml` in an offscreen
Quickshell with a fake `badictl` first on its PATH and a temporary HOME. It
checks settings validation, a compare-and-swap write that preserves every
subject and `all_web_origins`, deactivation during a TERM-ignoring write with
SIGKILL escalation and a discarded stale exit, reactivation during teardown,
and a write that invalidates an in-flight overview read.

`tests/check-host.sh` is the strict runtime lane used by CI. Against an exact
Omarchy Git checkout (`BADI_OMARCHY_ROOT`) with the pinned Quickshell and Qt
packages, it fails closed unless the official manifest validator, recorded
host-file hashes, Qt 6 `qmllint`, and the headless lifecycle all run.

## Deliberate limits

- The compatibility cell certifies source and host contracts, not visual theme,
  focus, scaling, screen-reader, or multi-monitor behavior.
- The fake-client checks are source/process evidence; the installed window's
  appearance needs a real Omarchy session.
- Distribution outside this user-local install, and a headed Omarchy matrix,
  remain later gates.
