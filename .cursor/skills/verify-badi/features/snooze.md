# Snooze

Snooze turns Badi off for a few minutes in the app or site of the field you are typing in. Other apps keep suggesting. `badi resume` ends it. A snooze is not saved: it ends when the broker stops.

## Sub-features

- `snooze-focused` withholds the focused app or site for 10 minutes, or the minutes you pass.
- `snooze-others` leaves every other app and site able to suggest.
- `snooze-end` ends every snooze from `badi resume`, `badi snooze end`, or the panel.
- `snooze-no-field` refuses when no field Badi follows is focused.

## How to get to it (user POV)

- While typing in an allowed field, run `badi snooze`. That app or site stays quiet for 10 minutes. `badi snooze 5` uses five minutes.
- The session helper's Snooze shortcut runs `badi snooze` for the focused app or site. The panel's Snoozed row ends that snooze.
- Run `badi resume` or `badi snooze end` to end every snooze now.
- Ctrl+Shift+Tab is a different key: it types a real Tab. It does not snooze.

## Driving it with verify-badi

Preconditions:

- Headless: doctor has exited 0, and no session is open at the start of the drive.
- The Bash grant is the seeded one. Snooze keys off that identity only while the bridge holds a focused field.
- A Hyprland shortcut or the panel's Snoozed row needs the desktop. This drive does not press them.

- **No field.** The drive first runs `badictl snooze` with no session. Exit code is 1 and stderr contains `error_code=rejected:no_context`. Status has no `snoozes` key.
- **Focused Bash field.** The same drive then asks the Bash bridge for `thank you`, and while that field is still open runs `badictl snooze`. Status lists one snooze whose subject is Bash (`shell` / `bash`) with `remaining_ms` above 0. A further bridge status is `BLOCKED`. `badictl snooze end` drops `snoozes` from status. Helper exit code is 0. Evidence is `drive-snooze.txt` and `status-snoozed.json`.
- **Desktop.** In a real field, `badi snooze`, type there and get nothing, type in a different allowed app and still get suggestions, then `badi resume`. A screenshot of the quiet field is the desktop proof. A VM cannot run it.

## Gotchas

- Snooze is runtime broker state. Citing it does not prove anything that survives a broker restart.
- The bridge's blocked line is the same text it uses while paused. The snooze proof is `snoozes` naming Bash, not that line alone.
- `badictl status` omits `snoozes` when the list is empty. A missing key is the idle state. A present key is a recorded subject.
- `snooze-others` is not driven headless: the instance grants Bash only, so a second app cannot suggest. Do not treat Bash going quiet as proof that another app still works.
- Snooze while already paused is refused. Resume the pause before snoozing.
