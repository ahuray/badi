# Pause and access

Pause stops suggestions until you resume. An app or site that is not allowed never receives a suggestion. Status and doctor say which situation you are in.

## Sub-features

- `pause-on` stops suggestions and survives a restart when set with `badi pause`.
- `pause-resume` allows suggestions again.
- `access-app` turns an app on or off, or switches allowlist and blocklist.
- `access-doctor` reports whether the installed desktop is worth using.

## How to get to it (user POV)

- Run `badi pause`, `badi resume`, or `badi pause toggle`. Right-click the Omarchy **b** mark for the same pause. `badi resume` also ends every snooze.
- Run `badi status` to see whether the model is ready and whether apps are allowlisted.
- Run `badi app NAME on`, `badi app NAME off`, `badi apps allowlist`, or `badi apps blocklist`.
- Run `badi doctor` when nothing appears. `badi doctor --json` is the same report for tools.

## Driving it with verify-badi

Preconditions:

- Headless pause uses the isolated broker. `badictl pause` is runtime only; it is not `badi pause`, which persists in the desktop settings file.
- Headless doctor is `verify-badi.sh doctor`. `badi doctor` is the installed user service and does not see this broker.
- Desktop access commands need the install. The headless settings file grants Bash only.

- **Pause refuses a suggestion.** Run `.cursor/skills/verify-badi/verify-badi.sh drive pause`. While pause is on, `badictl status` has `paused` true, the bridge transcript contains `BLOCKED app_disabled_or_model_paused`, and `context_updates` is unchanged. After the drive, `paused` is false. Helper exit code is 0.
- **Desktop pause persists.** Run `badi pause`, restart the user service, and run `badi status`. It still says paused. `badi resume` clears it. A VM without the user service cannot run this.
- **Doctor the install.** Run `python3 scripts/badi-desktop.py doctor` and record its exit code. Exit 0 is a desktop with no problems. With no user D-Bus the command exits 1 and prints `Failed to connect to bus: No such file or directory`, and it does not print the health report. When the service manager answers but the broker does not, the report names `broker_unreachable`. Do not treat either result as a failed headless doctor, or the reverse.

## Gotchas

- The bridge prints `READY` as soon as it connects, including while paused. The pause proof is the later `BLOCKED` line and `paused: true`, not that first `READY`.
- Pause withholds every app. A snooze withholds one app or site and leaves the others alone. This drive does not start a snooze. `badi resume` ends both a persistent pause and every snooze; `badictl pause off` is only the runtime pause.
- `badictl pause` ends when that broker exits. Citing it does not prove `badi pause` across a restart.
- Allowlist is the default. An app with no rule gets nothing. The headless instance will refuse Neovim, Obsidian, and Fcitx apps.
- A password field is refused in every mode. Do not use a real password to prove that.
