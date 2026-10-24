---
name: verify-badi
description: Drive Badi, the local Linux writing assistant, and capture proof. Use when a change needs a user-flow proof artifact, a doctor check, or a feature-map journey. Headless runs use an isolated phrase broker; grey text on a real app needs Omarchy, Hyprland and Fcitx 5.
---

# Verify Badi

Badi suggests the next few words while someone types, on one machine. The primary surface is a Hyprland session with Fcitx 5: grey text in a field, accepted from the keyboard. The same keys exist in Bash, Neovim and Obsidian. LibreOffice Writer has its own spelling replacement. The Omarchy bar opens settings. `badi` is the desktop command; Prediction Lab compares models on drafts the user supplies.

A cloud VM or GitHub-hosted runner has no Hyprland seat and no Fcitx. It can prove the headless journeys below. It cannot prove grey text, a real key, an accessibility field, or the bar. Say which one a proof is.

The map in [features/README.md](features/README.md) is the maintained list. Driving one entry point does not verify the others.

## Launch

Headless launch builds the broker once, then starts one isolated phrase broker. The phrase provider is the deterministic fixture from `badi-broker --provider phrase` (`thank you` completes as ` for your time`). It does not load the local model. An empty data directory is the instance root; the only seed is a Bash grant.

From the repository root:

```sh
cargo build -p badi-broker --bins --locked
export BADI_VERIFY_RUN_ID="verify-$(date +%Y%m%d%H%M%S)"
.cursor/skills/verify-badi/verify-badi.sh launch
```

Ready is a line `verify-badi ready` plus a socket at `/tmp/badi-verify-$BADI_VERIFY_RUN_ID/runtime/badi/broker.sock` that `badictl status` can read. The broker stays quiet: a phrase broker that prints diagnostics is not healthy. Two runs may exist together when their run ids differ. Launch refuses a second broker for the same id.

This launch does not start `badi-broker.service`, Fcitx, the accessibility observer, or the session helper. It does not read or write `~/.config/badi` or the desktop `XDG_RUNTIME_DIR`.

Desktop launch, only in a graphical session the user asked to install:

```sh
python3 scripts/install-desktop.py
badi doctor
```

Ready there means `badi doctor` names the installed commit and does not exit 1. Model startup can take a few seconds after the service answers. That command is the installed user service, not a verify-badi run id.

Tear a headless run down with [Cleanup](#cleanup). Stop using a desktop install only with the installer's own restore path, and only when the user asked.

## Doctor

Run doctor before the first drive, and again after a drive that failed, before retrying.

Headless, for the run just launched:

```sh
.cursor/skills/verify-badi/verify-badi.sh doctor
```

Exit 0 prints `verify-badi doctor ok`. It requires all of the following, and writes them under the evidence directory:

- the recorded pid is still `badi-broker --provider phrase`
- `target/debug/badictl --socket "$BADI_SOCKET" status` exits 0
- `provider` is `phrase_v1`, `paused` is false, `control_plane_degraded` is false, `socket_mode` is `0600`, `sessions` is 0
- the broker log is empty

Anything else means the instance is not worth driving. Do not point this doctor at a broker you did not launch.

Installed desktop, when judging that install rather than a run id:

```sh
python3 scripts/badi-desktop.py doctor
python3 scripts/badi-desktop.py doctor --json
```

Exit 0 means `badi.desktop-health.v1` lists no problems. Exit 1 means it does not. This talks to the user service manager and the session socket. It does not see a verify-badi instance, and a healthy verify-badi instance does not make `badi doctor` pass. With no user D-Bus, this command exits 1 and prints systemd's `Failed to connect to bus: …` line instead of the health report. No user runtime directory yields `No medium found`. A bus address whose socket file is missing yields `No such file or directory`. When the user service manager answers but the broker socket does not, the report's problem code is `broker_unreachable`.

## Drive

The harness is `.cursor/skills/verify-badi/verify-badi.sh`. Headless drives speak to the Bash bridge, `adapters/shell/bridge.mjs`, which is what Bash starts on Ctrl-X, then Tab. Commands are JSON lines on its stdin. Suggestion text is base64. The seeded grant is Bash only (`shell` / `bash`).

```sh
.cursor/skills/verify-badi/verify-badi.sh drive word-suggestion
.cursor/skills/verify-badi/verify-badi.sh drive spelling
.cursor/skills/verify-badi/verify-badi.sh drive pause
.cursor/skills/verify-badi/verify-badi.sh drive length
.cursor/skills/verify-badi/verify-badi.sh drive stats
.cursor/skills/verify-badi/verify-badi.sh drive snooze
```

`word-suggestion` types `thank you`, accepts ` for your time`, and checks `commits_applied` increased by one. `spelling` types `We need an adress `, accepts replacement `address ` for `adress `, and checks the same counter. `pause` turns pause on, shows the bridge refuse the suggestion with `BLOCKED app_disabled_or_model_paused`, checks the context counter did not move, then turns pause off. `length` sets `suggestion_length` to `short`, checks the phrase fixture is still ` for your time`, and restores normal. `stats` reads today's counts after `word-suggestion` has applied one acceptance, then forgets them. `snooze` refuses with `rejected:no_context` while idle, snoozes Bash while the bridge holds a focused field, and ends that snooze. Each drive waits until `sessions` is 0 again. After snooze ends, status omits `snoozes`; `badictl status` leaves that key out when nothing is snoozed.

Doctor must have exited 0 for this run id first. The helper enforces that.

These drives prove the broker's reply and the bridge's authorized insert. They do not prove grey pixels, a physical key, or `commitString` landing in an app. Fcitx `commitString` is dispatched, not applied.

Further headless harnesses already in the repo, each with its own process and fixture. They are not substitutes for a desktop trial, and a passing one does not cover a feature entry the map lists separately:

| Command | What a pass shows | What it does not show |
| --- | --- | --- |
| `python3 adapters/shell/build-preview.py` then `python3 adapters/shell/test.py`, with this run's environment exported | Bash in a PTY: grey ANSI preview, accept into the Readline buffer, undo, no command submitted | A Hyprland window |
| `cargo build -p badi-broker --bins --locked` and the Fcitx test client, then `python3 adapters/fcitx5/tests/broker-smoke.py` | C++ client and the real broker agree on two native identities | A display or a loaded Fcitx module |
| `PYTHONDONTWRITEBYTECODE=1 python3 adapters/fcitx5/tests/observed-desktop.py` | Private D-Bus, synthetic fields, dispatch of CommitString | An applied edit, AT-SPI of a real app, or Hyprland |
| `npm run editors:integration` | Obsidian, Neovim and Bash clients against a phrase broker the test starts | Those editors on screen |
| `cargo build --release --locked -p badi-writing-lab` then `target/release/badi-writing-lab --inspect-device` | A content-free device description | A writing comparison; the worker errors when model files are missing |
| `npm run check` and the table in [AGENTS.md](../../../AGENTS.md#verification) | Source and unit contracts | A user flow |

`adapters/shell/test.py` needs Bash headers (`builtins.h`, Ubuntu package `bash-builtins`), Readline headers (`readline/readline.h`, Ubuntu package `libreadline-dev`), and a built `adapters/shell/build/badi-preview.so`. It reads the broker from `XDG_RUNTIME_DIR` and `target/debug/badictl`. Export the variables from the run's `instance.env` before starting it. Do not run it against the desktop socket.

Desktop drive, Hyprland and Fcitx only, disposable text: [Live trials](../../../AGENTS.md#live-trials). Keys are in [README](../../../README.md#keys). `badi debug on --text` records text; turn it off when the transcript is saved. A screenshot or recording has to show the grey suggestion and the field after accept.

## Evidence

Headless evidence is `/tmp/badi-verify-evidence/$BADI_VERIFY_RUN_ID/`. Cleanup never removes that directory.

| File | Contents |
| --- | --- |
| `instance.env` | Run id, instance root, socket, broker pid |
| `commit.txt`, `broker-version.txt`, `badictl-version.txt` | Build identity |
| `broker.log` | Broker stdout and stderr; empty on a healthy phrase broker |
| `doctor.json`, `doctor.err`, `doctor.exit` | `badictl status` and its exit code |
| `drive-<feature>.commands.jsonl` | The bridge commands, the action |
| `drive-<feature>.txt`, `drive-<feature>.err`, `drive-<feature>.exit` | The bridge transcript and exit code |
| `status-before-<feature>.json`, `status-after-<feature>.json` | Broker counters around the drive |
| `cleanup.txt` | Which pid was signaled, and this evidence path |

A proof cites the feature id, the entry point, the command, and the exit code. For `word-suggestion` the transcript's `SUGGEST` and `INSERT` payloads both decode to ` for your time`, and `commits_applied` is one higher afterwards. For `spelling`, both payloads are `address ` replacing `adress `. For `pause`, the transcript contains `BLOCKED app_disabled_or_model_paused` while status says paused, and status says not paused after resume.

Phrase-broker output is fixture evidence. It is not a local-model score and not a live-app trial. Report those three separately when more than one exists.

Desktop evidence is a screenshot or recording of the grey text and the accepted field, plus `badi debug log --json` when a trace was on. Name the app, its version, and the installed commit. Keep the trace off after copying the log; the log lives in the desktop runtime directory and is not this evidence directory.

Mocks stop at the phrase provider, which is the broker's own integration fixture. The Bash bridge, `badictl`, and the broker process are the real ones. Do not replace them with a stub and call the journey verified.

## Cleanup

```sh
.cursor/skills/verify-badi/verify-badi.sh cleanup
```

Cleanup reads `instance.env`, sends TERM only when that pid's command line is still `badi-broker --provider phrase`, and deletes `/tmp/badi-verify-$BADI_VERIFY_RUN_ID`. It does not delete `/tmp/badi-verify-evidence/$BADI_VERIFY_RUN_ID`. It does not signal a process by name. If the pid's command line is something else, it leaves the pid alone and says so in `cleanup.txt`.

Run cleanup after a failed launch or drive too, so a broker and its scratch root do not stay behind.

After cleanup, confirm the proof is still there:

```sh
test ! -e "/tmp/badi-verify-${BADI_VERIFY_RUN_ID}"
pid="$(sed -n 's/^BADI_BROKER_PID=//p' "/tmp/badi-verify-evidence/${BADI_VERIFY_RUN_ID}/instance.env")"
if [[ -n "$pid" && -r "/proc/${pid}/cmdline" ]] && tr '\0' ' ' < "/proc/${pid}/cmdline" | grep -q badi-broker; then
  echo "broker still running"; exit 1
fi
test -s "/tmp/badi-verify-evidence/${BADI_VERIFY_RUN_ID}/drive-word-suggestion.txt"
echo "evidence_survives exit=$?"
```

The pid is `BADI_BROKER_PID` in `instance.env`. The check is the command line, not a recycled pid number alone. `drive-word-suggestion.txt` is the stand-in when that was the feature driven; use the transcript of the feature that was actually driven.

## Helpers

`.cursor/skills/verify-badi/verify-badi.sh` is the only helper. It is executable. From the repository root, with `BADI_VERIFY_RUN_ID` set:

```sh
.cursor/skills/verify-badi/verify-badi.sh launch
.cursor/skills/verify-badi/verify-badi.sh doctor
.cursor/skills/verify-badi/verify-badi.sh drive word-suggestion
.cursor/skills/verify-badi/verify-badi.sh drive spelling
.cursor/skills/verify-badi/verify-badi.sh drive pause
.cursor/skills/verify-badi/verify-badi.sh drive length
.cursor/skills/verify-badi/verify-badi.sh drive stats
.cursor/skills/verify-badi/verify-badi.sh drive snooze
.cursor/skills/verify-badi/verify-badi.sh cleanup
```

`launch` runs `cargo build -p badi-broker --bins --locked` only when `target/debug/badi-broker` or `target/debug/badictl` is missing. `drive` exits 1 until `doctor.exit` is `0`. The helper waits for the bridge's `READY` line before it writes each command. Passing the command file as the bridge's initial stdin hangs: those lines arrive while the bridge is still connecting, and it never answers. A reply that does not arrive within 8 seconds fails the drive.

## Maintenance

When user-facing behavior changes, keep this map honest with `/maintain-verification-skill`. That pass re-reads every feature file and drives the journeys. Edit the skill, the feature files, and this helper. Do not change product code from that pass to make a journey match the map.
