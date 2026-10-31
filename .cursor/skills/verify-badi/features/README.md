# Badi verification map

This directory is the maintained source for verifying Badi's user-facing behavior. Read the index before driving, then use the matching feature file.

## Baseline preconditions

Two surfaces. A pass on one does not verify the other.

**Headless instance (CI and a cloud VM).** An isolated phrase broker started by [verify-badi](../SKILL.md).

- `BADI_VERIFY_RUN_ID` is set, and launch has printed `verify-badi ready`.
- The instance root is `/tmp/badi-verify-$BADI_VERIFY_RUN_ID` with its own runtime, config and data directories.
- Settings grant Bash (`shell` / `bash`) context read, display and suggestions, and block learning. No other app is granted.
- `verify-badi.sh doctor` has exited 0 for this run.
- Never drive a broker this run did not start, and never point the run at the desktop socket or `~/.config/badi`.

**Desktop (Omarchy on Hyprland with Fcitx 5).** The installed user service.

- The user authorized `python3 scripts/install-desktop.py` in a graphical session.
- `badi doctor` names the installed commit and exits 0.
- Typing uses disposable text.
- A cloud VM has no Hyprland seat and no Fcitx. Leave desktop journeys unverified there.

## Driving conventions

- Start from the baseline for the surface the step names.
- Headless commands are `verify-badi.sh drive word-suggestion`, `drive spelling`, `drive pause`, `drive length`, `drive stats`, and `drive snooze`.
- Desktop keys are Tab, Ctrl+Right, Escape, and Ctrl+Shift+Space. Bash uses Ctrl-X, then Tab.
- After a pause journey, pause is off.
- Cleanup removes the instance root and the broker pid this run started. It keeps `/tmp/badi-verify-evidence/$BADI_VERIFY_RUN_ID`.

## Proof and skip reporting

- Capture the action and the resulting state.
- Headless proof is the command, the transcript, the status JSON, and the exit code in the evidence directory.
- Desktop proof of an input flow is a Hyprland screenshot or recording of the grey text and the field after accept.
- Phrase-provider output is fixture evidence, not a local-model score and not a live-app trial.
- Report an unreachable path with the command tried and the unmet precondition.
- Do not report a skipped entry point as verified through a different path.

## Feature entry contract

Each feature file starts with an H1 and one paragraph of user-visible behavior, then exactly these four H2 sections:

1. `Sub-features`
2. `How to get to it (user POV)`
3. `Driving it with verify-badi`
4. `Gotchas`

## Features

- [Word suggestion](./word-suggestion.md) covers grey text, the next word, the whole suggestion, another continuation, dismissal, an explicit request, and a literal Tab.
- [Suggestion length](./suggestion-length.md) covers one or two words versus up to four.
- [Spelling](./spelling.md) covers an unambiguous English correction on the Bash path, and the real-model and Writer paths that a VM cannot stand in for.
- [Selection revision](./selection-revision.md) covers the grammar fix and refined wording of one selected line, applying and dismissing them, and the refusals.
- [Snooze](./snooze.md) covers turning Badi off in the focused app or site, and the refusal when nothing is focused.
- [Pause and access](./pause-and-access.md) covers pause, app grants, status, and doctor.
- [Screen-share hold](./screen-share-hold.md) covers holding every suggestion while the screen is shared, and keeping them on request.
- [Usage statistics](./usage-stats.md) covers the content-free counts of suggestions, acceptances and words saved.
- [Omarchy panel](./omarchy-panel.md) covers the bar mark and the settings popout.
- [Prediction Lab](./prediction-lab.md) covers the two-tab page: a local-model comparison, and word completion from text you supply, without changing the installed broker.
