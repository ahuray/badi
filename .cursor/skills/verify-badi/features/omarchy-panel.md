# Omarchy panel

The **b** mark on the Omarchy bar opens Badi's settings and shows whether suggestions are on. Right-click pauses or resumes.

## Sub-features

- `panel-open` opens the popout under the mark.
- `panel-pause` pauses from the mark or the pause switch and dims the mark.
- `panel-model` shows the model in use and the recommendation.
- `panel-search` filters rows from `/`.

## How to get to it (user POV)

- Left-click the **b** mark. The popout shows the model pill, a mode such as `SUGGESTING` or `PAUSED`, and the pause switch.
- Right-click the mark. Suggestions pause and the mark dims. Right-click again resumes.
- With the popout focused, j and k move, h and l change a row, and `/` searches.

## Driving it with verify-badi

Preconditions:

- The panel is installed with `python3 scripts/install-omarchy-ui.py` on Omarchy, with the bar visible.
- The broker it talks to is the user service, not a verify-badi run id.
- A cloud VM has no Omarchy bar. The journey fails there because the mark is not on a display. Record that, and do not substitute a source check.

- **Open.** Left-click the **b** mark. A screenshot shows the popout with the model pill and the pause switch.
- **Pause from the mark.** Right-click the mark. The mark dims, and `badi status` reports paused. Right-click again, and status reports the model ready.
- **Search.** Press `/`, type a row name, and confirm the other rows are hidden. Escape or clearing the query shows them again.
- **Source check, not the panel.** `npm run omarchy:check` exits 0 when the repo's fast QML and keyboard checks pass. That command never opens the bar. A green run is not `panel-open`.

## Gotchas

- The popout polls the broker only while it is open. A status change made while it is closed shows up on the next open.
- Installing the panel is a separate step from `install-desktop.py`, and it waits for an unlocked session.
- Keyboard navigation is the popout's own model. Driving `badictl` does not prove j, k, or `/`.
