# Omarchy panel

The **b** mark on the Omarchy bar opens Badi's settings and shows whether suggestions are on. Right-click pauses or resumes.

## Sub-features

- `panel-open` opens the popout under the mark.
- `panel-pause` pauses from the mark or the pause switch and dims the mark.
- `panel-model` shows the model in use on the hero pill. After All settings, the Writing model row names that model's licence, the device recommendation, and every model that fits, with ★ on the recommended one.
- `panel-search` filters rows from `/`.
- `panel-length` sets Short or Normal on Suggestion length.
- `panel-snooze` shows a Snoozed row while an app or site is quiet, and ending it resumes that target.
- `panel-stats` shows seven-day bars and today's and the week's word counts in the History header. The leading-apps line and Forget usage statistics appear only after All settings. Forget asks Omarchy to confirm.

## How to get to it (user POV)

- Left-click the **b** mark. The popout shows the model pill, a mode such as `SUGGESTING` or `PAUSED`, and the pause switch.
- Right-click the mark. Suggestions pause and the mark dims. Right-click again resumes.
- With the popout focused, j and k move, h and l change a row, and `/` searches.
- Compact rows are Languages, Suggestion length and context, plus All settings (`a`). The Snoozed row appears only while a snooze is running. History shows the week's bars and word counts while the popout is open. All settings opens the Writing model row, the "most this week" line, and Forget usage statistics. `badi model` lists the same models; `badi model use` switches one.

## Driving it with verify-badi

Preconditions:

- The panel is installed with `python3 scripts/install-omarchy-ui.py` on Omarchy, with the bar visible.
- The broker it talks to is the user service, not a verify-badi run id.
- A cloud VM has no Omarchy bar. The journey fails there because the mark is not on a display. On a machine without `omarchy-shell`, `python3 scripts/install-omarchy-ui.py` exits 1 before writing any panel file and prints `The Badi panel lives in Omarchy's bar, and this desktop has no omarchy-shell; nothing was changed. The badi commands set the same things (badi --help).` The line `plugin source gate: pinned official manifest validator is required` is a later gate, when host checks are required and that validator is missing. Record the line this machine prints. Do not substitute a source check.

- **Open.** Left-click the **b** mark. A screenshot shows the popout with the model pill and the pause switch.
- **Pause from the mark.** Right-click the mark. The mark dims, and `badi status` reports paused. Right-click again, and status reports the model ready.
- **Search.** Press `/`, type a row name, and confirm the other rows are hidden. Escape or clearing the query shows them again.
- **Length, snooze, and stats on the panel.** Set Short on Suggestion length and confirm the next local-model suggestion is one or two words. Snooze from the focused field and confirm the Snoozed row names that app or site. The History header's week matches `badi stats`. Open All settings for the leading apps and for Forget usage statistics. These are not on a cloud VM.
- **Source check, not the panel.** `npm run omarchy:check` exits 0 when the repo's fast QML and keyboard checks pass. That command never opens the bar. A green run is not `panel-open`.

## Gotchas

- The popout polls the broker only while it is open. A status change made while it is closed shows up on the next open.
- Installing the panel is a separate step from `install-desktop.py`. A locked desktop exits with `Desktop locked: no UI files changed. Unlock and rerun this command.` `--wait-for-unlock SECONDS` waits for an unlock before that exit.
- Keyboard navigation is the popout's own model. Driving `badictl` does not prove j, k, or `/`.
