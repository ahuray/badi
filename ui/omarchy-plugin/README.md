# Badi Omarchy panel artifact

This directory is the exact root artifact for the disabled-by-default Omarchy
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

- installed Omarchy package: `4.0.2-1`
- official source: `https://github.com/omacom/omarchy.git`
- official source tag: `v4.0.2`
- source commit: `346e69e1cec6c4e8924531874af6ba010a1bc99e`
- Quickshell: `0.3.1-1`
- Qt declarative: `6.11.2-1`

The installed `shell.qml`, `PluginRegistry.qml`, `qs.Ui`/`qs.Commons` module
indexes, and plugin validator were byte-compared with that source commit. Their
hashes are recorded in `compatibility.json`.

The manifest declares only `panel`. `Panel.qml` exposes the host-injected
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

Set `BADI_SUMMON_CYCLES=100` to run the bounded lifecycle stress check. The
supported fake states are `healthy`, `unavailable`, `degraded`, `stale`,
`capacity`, `memory-repair`, `malformed`, `timeout`, and `term-ignoring`. The
last state includes an immediate hide/reopen check and verifies every recorded
fake PID is gone. `term-ignoring-mutation` is reserved for the headless client
harness.

## Deliberate limits

- The artifact is not installed or enabled on this device.
- A packaged `badictl` on `omarchy-shell`'s PATH is required before live use;
  this device currently has no installed `badictl` command.
- The fake-client and isolated copied-shell checks are source/process evidence,
  not visual theme, focus, scaling, screen-reader, or multi-monitor proof.
- Full distribution, install/update/remove/rollback, and the headed Omarchy
  matrix remain later gates.
