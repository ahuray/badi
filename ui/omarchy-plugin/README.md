# Badi Omarchy panel

The Omarchy plugin `io.github.ahuray.badi`:

| File | Role |
| --- | --- |
| `BarWidget.qml` | The bar mark and its popout: an Omarchy `Panel` with a `KeyboardPanel` |
| `BadiMark.qml` | The **b** speech-bubble mark |
| `DesktopPanel.qml` | State and actions without visuals: the client, helper processes, every settings change and the typed rows |
| `SettingsView.qml` | The popout's content |
| `BadiClient.qml` | The shared `badictl` client |

It uses only the host's `qs.Ui` and `qs.Commons` modules, `badictl` and the
installed `badi-desktop` helper.

Left-click the mark to open or close the popout; right-click pauses or
resumes (a paused mark is dimmed). The popout hangs under the mark like
Omarchy's own panels, takes the keyboard and polls the broker every three
seconds only while open. From the top:

- **Hero:** the mark, the model as a pill, Badi's mode as a glyph and word
  (`⏵⏵ SUGGESTING`, `⏸ PAUSED`, `◌ LOADING`, `○ MODEL OFFLINE`,
  `◇ NO APP ALLOWED`) and a switch that pauses or resumes.
- **Activity:** while something runs, a spinner, a verb and the elapsed
  seconds (`Saving…`, `Loading the model… 3s`); otherwise a status dot and
  the counts since model start.
- **Usage:** a ten-cell acceptance meter, words saved and the last
  no-suggestion reason (`⎿ last miss: …`).
- **Search:** `/` filters every row by name, description or section.
- **Rows**, in sections: Writing (mid-sentence, Escape quiet, battery),
  Spelling (each language and its dictionary, naming the package when one is
  missing), Apps (the list mode and each supported app), Websites (the list
  mode, each rule, an add field: Enter allows, Shift+Enter blocks) and
  System (start at login, debug trace, restart, start or stop).

Keys: j/k or arrows move one cursor through the rows, space or Enter toggles
or runs, h/l changes a choice, x removes a site rule, p pauses, r refreshes,
Tab moves to the neighbouring bar panel and Escape closes. The footer shows
the last result (`✓ Spelling on`, `✗ …`) or these keys.

Settings writes use the broker's compare-and-swap API and fail visibly on a
conflict instead of retrying over another change. `badi settings` toggles the
popout through the `badi-writing` IPC target. The terminal equivalents are in
the [README](../../README.md#choose-where-badi-writes).
Per-app language overrides are currently set from the terminal. Panel writes
validate and preserve them.

## Install and update

`scripts/install-desktop.py` installs `badi`, `badi-desktop`, `badictl` and the
launcher entry. The panel itself updates with:

```sh
python3 scripts/install-omarchy-ui.py
python3 scripts/install-omarchy-ui.py --wait-for-unlock 3600   # stage and wait for unlock
```

The updater runs the source checks, waits for an explicitly unlocked desktop
(it never unlocks it), backs up replaced files under
`~/.local/state/badi/ui-backups/<UTC time>/` (newest three kept, plus the
original backup), removes files the manifest no longer loads, restarts the
Omarchy shell the supported way and checks that the new popout answers IPC.
A missing plugin directory is created and enabled with `omarchy plugin enable
io.github.ahuray.badi`; `XDG_CONFIG_HOME` selects that directory. `omarchy
plugin disable io.github.ahuray.badi` hides the mark.

## Suggestion panel theme

Terminals, and any field without the grey overlay, show Badi's suggestion
in Fcitx's panel. [`fcitx-theme.py`](fcitx-theme.py) dresses it in the
current Omarchy theme: the theme background with Omarchy's soft control
border (the foreground at 40%), square corners, the Omarchy font at Ghostty's
`font-size`, the word Tab takes in bold accent, and the rest softened to the
dimmest shade that still reads at 4.5:1 (the accent falls back to the
foreground below that). It writes `fcitx5/themes/badi-omarchy/theme.conf`
under `XDG_DATA_HOME` and, only when absent or written by Badi,
`fcitx5/conf/classicui.conf`; a classicui.conf you wrote is left alone and
named on the console. The updater installs the generator as
`~/.local/lib/badi/omarchy/fcitx-theme.py`, adds `badi-fcitx-theme` to
Omarchy's `theme-set.d` and `font-set.d` hooks, writes the theme for the
current one and reloads Fcitx's classic UI. All of it is in the backup map,
so a restore removes it. Badi cannot draw grey text inline in a terminal:
over Wayland's input-method-v2 the compositor places this panel at the
cursor, and Fcitx never learns where the caret is.

## Client contract

`BadiClient.qml` never reads document text, model state or settings files
directly. Every command is a fixed argv array through Quickshell `Process`;
settings JSON goes through the child's stdin, closed after writing. It runs at
most one overview read and one settings write, and a write invalidates any older read. Deactivation terminates
every child (SIGTERM, then SIGKILL), stale exit handlers change nothing, and a
reactivation during teardown queues exactly one fresh read.

## Statistics

The popout reports counters since model start, without storing prose:
- **Shown:** broker suggestions, excluding growth and type-through remainders.
- **Accepted:** explicit acceptance requests dispatched to the adapter, not a
  claim that every edit applied.
- **Words saved ≈:** word parts in successful results, including native
  `dispatched-unverified` results. Partial words count; punctuation-only parts,
  spelling replacements and failed results do not. Native insertions and later
  undo cannot be verified, so this is an estimate rather than retained words.

An older broker without word counters shows `—`, never a fabricated zero.
Restarting the model resets these counters. The broker retains verified and
unverified word counts separately in `word_savings`.

On 2026-10-15 the popout was rendered in the real Omarchy session (Tokyo
Night, square corners): j/k moved the cursor and scrolled, `l` on "On battery"
wrote `battery_mode` through stdin (`h` restored it), `/ghost` left only the
Ghostty row, and a restart from System showed `◌ LOADING` with
`Loading the model… 3s` until the broker answered.

## Pinned host

`compatibility.json` records the Omarchy host files the plugin depends on,
byte-compared with Omarchy `v4.0.3` (`0534987009061cbe2dacdde4ad564092ab698d12`)
under Quickshell 0.3.1 and Qt declarative 6.11.2, including every `qs.Ui`
component the popout uses (unchanged in 4.0.4). The strict check fails when the
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

These checks prove source and process contracts, not the popout's look, focus,
scaling or multi-monitor behavior, which need a real Omarchy session.
