# Usage statistics

Usage statistics count suggestions shown, acceptances that landed, and words those acceptances saved. The counts are per day and per app or site. They contain no typed text and no suggestion text.

## Sub-features

- `stats-show` reads today's counts, the last seven days, and the leading apps and sites.
- `stats-after-accept` increases shown, accepted, and words after an acceptance lands.
- `stats-forget` deletes every count.
- `stats-paused` adds nothing while Badi is paused or the app may not suggest.

## How to get to it (user POV)

- Run `badi stats`. It prints today, the last seven days, each day, and the leading apps and sites.
- Run `badi stats --json` for the same counts.
- Run `badi stats forget` to delete them. Counting starts again afterwards.
- The Omarchy popout shows the week's words and Forget usage statistics.

## Driving it with verify-badi

Preconditions:

- Headless: doctor has exited 0, and `drive word-suggestion` has already applied one Bash acceptance in this same run. The stats drive does not accept a suggestion itself.
- Counts are content-free. A proof must not expect the words `thank you` in the stats document.
- The panel's bars need Omarchy. `badi stats` on the user service is the desktop command; this drive uses `badictl stats` on the isolated broker.

- **Read, then forget.** Run `.cursor/skills/verify-badi/verify-badi.sh drive stats`. `badictl stats status` has `available` true, and today's `shown` and `accepted` are at least 1. A subject is Bash. `badictl stats forget` then leaves today's `shown`, `accepted`, and `words` at 0. Helper exit code is 0.
- **Paused.** Pause, ask the bridge for a suggestion, and read stats again. The counts stay where they were. Resume afterwards. The headless `drive pause` already checks that a paused suggest does not update `context_updates`; it does not by itself assert the stats file.
- **Desktop.** `badi stats` on the installed service matches the panel's week. A VM without the user service cannot run that command.

## Gotchas

- Nothing is counted while paused or where suggestions are off. An earlier count remains until it expires or is forgotten.
- Counts reach disk within a minute and on pause or shutdown. `badictl stats status` includes counts already queued, so the drive does not wait a minute.
- Forget is destructive. Capture the status JSON before forgetting. The drive does that, then forgets, so a later stats read in the same run sees zeros.
- A stats file that cannot be read reports `available` false and counts nothing until forget replaces it. Do not treat that as a zero day of real use.
