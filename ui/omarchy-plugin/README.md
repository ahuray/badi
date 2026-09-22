# Badi Omarchy writing controls

The development device has a user-local copy of this plugin enabled immediately
before `omarchy.network`. Its speech-bubble **b** mark opens `DesktopPanel.qml`, with editor
launch buttons, a session selector, runtime counters and pause/resume. The existing
`Panel.qml` privacy panel remains available through the shell's summon API.

The writing settings use three views:

- **Writing:** model state, persistent pause/resume, editor launch buttons,
  Tab instructions, counters, and a concise privacy explanation.
- **Applications:** independent native app permissions. Blocking an app revokes
  context reading, suggestion generation, and display. Other subjects are preserved;
  learning stays blocked and retention stays `none`. Unsupported apps are listed
  explicitly. The trial selector appears only when multiple sessions exist.
- **System:** login startup, model restart/stop/start, service status, and selectable
  terminal commands. Turning startup off takes effect at the next graphical login;
  stopping the process releases model memory now. Neither changes app permissions.

Left-click the Badi mark to open/close settings; right-click to persistently
pause/resume. A paused icon is dimmed and its tooltip reports the state. Escape
closes the floating panel; buttons are keyboard focusable with accessible labels.
The panel uses Omarchy colors, typography, spacing and button components.

### Install and update

`python scripts/install-desktop.py` installs `badi`, `badi-desktop`, `badictl`, and a
standard Badi application launcher entry, alongside the native runtime described
in the Fcitx runbook. The helpers use the installed broker CLI, so everyday controls
do not require the checkout. The advanced `badictl` interface remains available.

`python scripts/install-desktop.py --broker-only` updates the model service and
commands without restarting Fcitx or touching its addon/profile. It can run while
the desktop is locked. The complete installer requires confirmed unlocked state
before replacing native files and again before restarting the input service.
Both modes retain backups and update the changed-file map as files are installed.

To update the already installed Omarchy plugin:

```sh
python scripts/install-omarchy-ui.py
# Or stage the validated UI and wait up to one hour for normal unlock:
python scripts/install-omarchy-ui.py --wait-for-unlock 3600
```

The updater verifies the existing plugin identity and QML, snapshots the source,
waits for explicit unlocked state, backs up replaced plugin files under
`~/.local/state/badi/ui-backups/`, and uses the supported Omarchy shell restart.
It then verifies that the new panel answers IPC. It never unlocks the desktop.
The restart keeps editor windows but clears cached nested QML components.

### Terminal controls

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
fail visibly rather than retrying over another change. `doctor` exits nonzero
when the broker is unreachable/degraded or the running native addon is missing.
The CLI uses Linux user services; the GUI and desktop installer target Omarchy.
This plugin is not a portable StatusNotifier tray implementation.

When startup fails, the System view shows the classified current-invocation
error. `badi doctor` includes actionable `problems` for missing model files,
resource or integrity failures, unreachable broker, degraded settings and an
unloaded addon. Journal prose is not copied into these diagnostics. Starting or
restarting a failed service clears its restart limit; service activation still
precedes model readiness, which the broker health probe verifies separately.

`scripts/badi-desktop.py` is installed as `~/.local/bin/badi` and `badi-desktop`. It starts the
persistent `badi-broker.service` and opens regular editors through transient user
services, preserving Wayland/runtime values and selecting their Fcitx modules.
The private desktop socket is listed first as **Desktop writing**. Separate
`scripts/try-native.py` trials register private, content-free socket
receipts under `$XDG_RUNTIME_DIR/badi/native-trials/` and remove their own receipt
at shutdown. The desktop client discovers live owned sockets, obtains settings
through `badictl`, and binds each mutation to the selected trial ID. It never
silently switches a mutation to another trial. Closing a trial leaves its saved
files and diagnostics under the repository's ignored `output/native-trials/`.

The bar is Omarchy-native; this is not yet a freedesktop StatusNotifier tray
implementation for other Linux desktops. Native prediction remains manual,
limited to the two visually tested apps. English, German and Persian input-method
languages are routed; the native physical trials used English. Tab requests words at the
end of a phrase; the next Tab accepts. Candidates have a five-second
reading window and retain text, focus, one-shot acceptance and expiry guards.

To disable only the Badi icon, run `omarchy plugin disable io.github.ahuray.badi`.
The pre-install shell configuration was backed up beside `shell.json` with a
`shell.json.badi-before-` timestamp. Do not restore that entire backup over later
unrelated changes. The ordinary Fcitx instance is not restarted by the launcher.

Additional checks: `PYTHONDONTWRITEBYTECODE=1 python scripts/test-desktop.py` and
the real-model/native UI evidence in
[visibility and desktop controls](../../what-have-been.md).

## Original panel artifact contract

This directory is the exact root artifact for the originally disabled Omarchy
plugin `io.github.ahuray.badi`. It is deliberately repo-local for this
milestone. A future publishing job can deterministically extract this tree into
a dedicated plugin repository; it must not add files from elsewhere in Badi.
For an immutable Badi commit, the root-artifact contract is exactly:

```sh
git archive --format=tar "$BADI_COMMIT:ui/omarchy-plugin" >badi-omarchy-plugin.tar
```

The publishing receipt must record `BADI_COMMIT` and the resulting tar SHA-256.

The panel is a thin `badictl` client. It never reads document text, model state,
settings files, or aggregate files directly. All commands use fixed argv arrays
through Quickshell `Process`; JSON remains one non-executable argument and no
command is evaluated by a shell. Closing the panel terminates every outstanding
child: it invalidates the active lifecycle, requests SIGTERM, retains a bounded
SIGKILL escalation, and forces SIGKILL if the host unloads the panel first.
Stale exit handlers cannot update state or start another command. A reopen while
teardown is pending queues exactly one fresh overview for the new lifecycle.

The authority card matches the product adapter's exact top-level target,
`https://dillinger.io/`. Chromium gates that complete URL; the broker's durable
settings identity is its HTTPS origin (`https`, `dillinger.io`, port `443`).

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
`compatibility.json`.

The September 9, 2026 rerun first failed the previous 4.0.2 host hash gate.
Review of 4.0.3 found that third-party plugins now receive scoped shell and bar
interfaces. Badi's panel calls only `shell.hide` for its own plugin identifier,
which that interface retains. The unchanged panel and bar widget then passed
100 copied-shell summon/hide cycles and the TERM-ignoring close/reopen,
escalation, stale-exit and unload checks on the current Wayland host. The current
official validator and headless mutation lifecycle also passed. This updates
the source compatibility cell; it does not qualify native writing behavior,
visual quality or the installed panel. The initial failures and subsequent
receipts are preserved under `output/writing/2026-09-09-rerun-current/`.

The manifest declares `panel` and `bar-widget`. `Panel.qml` exposes the host-injected
`shell` and `manifest` properties plus `opened`, `open(payload)`, and `close()`;
user dismissal routes through `shell.hide(id)`. It owns no `ShellRoot` and uses
Omarchy's `Color`, `Style`, `Border`, `BorderSurface`, `Button`, section header,
and separator primitives.

## Validation without installation

From the repository root:

```sh
npm run omarchy:check
omarchy plugin validate ui/omarchy-plugin
bash ui/omarchy-plugin/tests/check-source.sh
bash ui/omarchy-plugin/tests/check-term-ignoring.sh
BADI_OMARCHY_REQUIRE_HOST_CHECKS=1 \
  bash ui/omarchy-plugin/tests/check-source.sh
bash ui/omarchy-plugin/tests/run-client-lifecycle.sh
```

`npm run omarchy:check` is the portable CI gate. It runs ShellCheck, JSON/source
lifecycle contracts, and proves that the fake process really ignores TERM but
dies to KILL. It also exercises private process-group escalation with nested
TERM-ignoring watchers. The portable baseline requires neither Omarchy nor
Quickshell; when `qs` is present it retains the same headless client lifecycle
coverage locally, and otherwise explicitly reports that the optional local run
was not executed. Required CI coverage never relies on that availability.

The required `Omarchy plugin / pinned Arch host` CI job is the runtime lane. It
uses a digest-pinned Arch container and the dated 2026-08-31 Arch Archive
snapshot, checks out Omarchy at the exact recorded commit, installs exactly
Quickshell `0.3.1-1` and Qt declarative `6.11.2-1`, and then fails closed unless
all of these run: the official manifest validator, recorded host-file hashes,
Qt 6 `qmllint`, and the real `BadiClient.qml` headless mutation lifecycle. That
lifecycle covers close-during-mutation, SIGKILL escalation, stale-exit
suppression, and same-instance reopen during teardown. `check-host.sh` is the
same strict entry point when invoked against an exact Omarchy Git checkout.

The separate `tests/run-isolated.sh term-ignoring` test exercises the complete
copied-shell panel path for close, escalation, stale-exit suppression, and host
unload/reopen. It requires a real Wayland/Omarchy session and is deliberately
device evidence rather than a headless CI substitute.

`tests/run-isolated.sh` copies the pinned shell and this artifact into a
temporary HOME/config/runtime tree, places a deterministic fake `badictl` first
on that process's PATH, and launches only the copied shell. Against that
temporary tree it proves disabled discovery, rescan, official enable/disable,
summon/hide/toggle, unrelated-config preservation, and official removal to a
recoverable temporary backup. It never calls `omarchy plugin add` or edits the
real `~/.config/omarchy` tree.

The combined panel/bar-widget manifest may be enabled in a bar layout instead
of the top-level plugin list. The harness requires exactly one placement across
both locations, and none after disabling. Its temporary `badi-desktop` bridge
accepts only service status and overview reads; all other commands fail. This
separate healthy fixture proves the bar widget loads without mixing its
background polling into the legacy panel's fault-scenario lifecycle counts.

Set `BADI_SUMMON_CYCLES=100` to run the bounded lifecycle stress check. The
supported fake states are `healthy`, `unavailable`, `degraded`, `stale`,
`capacity`, `memory-repair`, `malformed`, `timeout`, and `term-ignoring`. The
last state includes an immediate hide/reopen check and verifies every recorded
fake PID is gone. `term-ignoring-mutation` is reserved for the headless client
harness.

## Deliberate limits

- The historical compatibility receipt describes the original disabled artifact;
  it does not certify the new bar widget or this user-local development install.
- The original privacy panel requires a packaged `badictl` on the shell's PATH;
  the native desktop panel uses the installed `badi-desktop` bridge and `badictl`.
- The fake-client and isolated copied-shell checks are source/process evidence,
  not visual theme, focus, scaling, screen-reader, or multi-monitor proof.
- Full distribution, install/update/remove/rollback, and the headed Omarchy
  matrix remain later gates.
