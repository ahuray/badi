# Badi session helper

`badi-session` tells the broker, without any text, whether the login session
is in the foreground, whether the machine is about to sleep, and whether it
runs on battery or in the power-saver profile. It also offers two desktop
shortcuts through the GlobalShortcuts portal. It never reads a field, opens a
suggestion session, edits anything or changes a system setting.

| Signal | Source | Broker effect |
| --- | --- | --- |
| Session not in the foreground | logind display session `Active` | Hold: every grant is revoked until it returns |
| Preparing to sleep | logind `PrepareForSleep(true)` | Hold, acknowledged before the delay inhibitor is released |
| Resumed | `PrepareForSleep(false)` | Hold ends; adapters acquire their fields afresh |
| On battery | UPower `OnBattery` | Low power: automatic scheduling follows `badi battery` |
| Power-saver profile | power-profiles-daemon `ActiveProfile` | Low power, as above |

## Broker contract

The helper speaks protocol v2 with the single capability `desktop_signals`
(adapter kind `session`) and sends `desktop.state`
`{session: active|inactive|sleeping, on_battery, power_saver}` whenever one
changes; the reply is `desktop.status`
([schema](../../protocol/v2/schema.json), [broker runbook](../../broker/README.md#desktop-session-signals)).

- A report only withholds. Entering or leaving a hold advances the authority
  epoch: entering it revokes context, pending inference, visible suggestions,
  pending commits and the type-through carry; leaving it makes every adapter
  reacquire. A Tab pressed before reacquisition finds no grant.
- The report is bound to the helper's connection. If the helper exits, the
  broker drops its hold and low-power state; after a broker restart the helper
  reconnects (1, 2, 5, 10, then 30 s) and reports again.
- Unknown is permissive only where nothing is at stake: without logind the
  session counts as active (adapters keep their own focus, lock and expiry
  checks), and without UPower or power-profiles-daemon the machine counts as
  on mains power.
- `LockedHint` is not used. It is a hint, not proof of unlock; the observer
  keeps its fresh Omarchy lock check.

## Sleep

At start, and again after each resume, the helper takes a logind `delay`
inhibitor for `sleep`. On `PrepareForSleep(true)` it reports `sleeping`, waits
for the broker's acknowledgement (at most 1.5 s), then closes the inhibitor so
sleep proceeds. A broker that is offline or slow never holds sleep longer than
that. A restarted logind forgets inhibitors; the helper takes a new one when
the name returns.

## Battery

`badi battery quiet|normal|explicit` sets the broker's `battery_mode`. It
applies only while the helper reports battery power or the power-saver
profile; explicit requests are never delayed or refused, and the model stays
resident in every mode.

- `quiet` (default): an automatic request waits 300 ms before the model starts. A
  newer key or edit cancels it first, so keys typed closer together never
  start the model and a burst of typing starts one generation at the pause
  instead of one per key. The model keeps its full writing budget once it
  starts.
- `normal`: as on mains power, where every key in a known field starts the
  model at once.
- `explicit`: automatic requests abstain before the model
  (`request_abstained`); Ctrl+Shift+Space, Tab in manual fields and editor
  request keys still ask.

`badi status` prints the reported session and power state and the resulting
automatic scheduling.

The [paced battery trial](../../evaluation/power/battery-trial.py) compares
`quiet` with `normal`. On 2026-10-13 (battery, power-saver, release build,
752 disposable keys per mode at 150 ms per key, 450 ms after words and 1.5 s
after sentences) `quiet` made 122 model calls against 586, used 182 against
464 runtime CPU seconds, showed 313 against 257 suggestions with 211 against
190 matching the reference, and had visible p95 550 against 525 ms. Charging
in the performance profile the same day (one repeat, 376 keys per mode) it
made 45 against 220 calls and used 50 against 148 CPU seconds, but covered 27
against 31 distinct reference words at p95 456 against 211 ms. On 2026-10-17
in power-saver, a run-on sentence typed at 450 ms per word showed 15
suggestions from 62 requests under `quiet`, most cancelled by the next word
after generation began. Still unmeasured: the balanced profile on battery,
real typing intervals, energy from a privileged power counter rather than
runtime CPU time, and a reviewed count of fully accepted additions.

## Style key

When it connects to a broker, the helper looks up the style-learning key in
the Secret Service ([style_key.py](style_key.py), libsecret) and hands it over
with `style.key`. It never prompts: a locked keyring is retried every two
minutes, and no stored key sends nothing. The keyring item is labelled
`Badi style learning key` with the single attribute `purpose=style-learning`;
its value is two random hex strings. `badi style on` creates it (the keyring
may ask to unlock) and delivers it at once; `badi style forget` deletes it.
The helper can deliver a key but cannot read samples.

## Shortcuts

The helper registers the host application id `io.github.ahuray.badi` with the
portal and binds `pause-toggle` (runs `badi pause toggle`, the same
compare-and-swap settings write as the bar's right-click) and `open-settings`
(runs `badi settings`). The compositor chooses the keys; on Hyprland 0.56,
for example in `~/.config/hypr/bindings.lua`:

```lua
hl.bind("SUPER + ALT + B", hl.dsp.global("io.github.ahuray.badi:pause-toggle"))
hl.bind("SUPER + ALT + COMMA", hl.dsp.global("io.github.ahuray.badi:open-settings"))
```

A pause or resume shows a desktop notification. Interface version 1 is all
this needs: it has no `ConfigureShortcuts`, and the helper never calls it. A
shortcut grants no access to an application or field; there is deliberately
no request shortcut, because a request needs an admitted adapter field
(Ctrl+Shift+Space already does that through Fcitx). xdg-desktop-portal-hyprland
1.4.1 keeps listing the two shortcuts in `hyprctl globalshortcuts` after the
helper stops; while it is stopped they do nothing, and a restart reuses them.

## Install and service

`scripts/install-desktop.py` (also with `--broker-only`) installs
`daemon.py` and `link.py` in `~/.local/lib/badi/session/` and
[badi-session.service](../../packaging/systemd/badi-session.service), restarts
it after the broker and waits up to six seconds for the broker to hear from
it. A unit you disabled stays disabled; `systemctl --user enable --now
badi-session.service` turns it back on. `badi doctor` reports the unit and
what the broker last heard. The journal holds fixed, content-free lines only.

## Tests

```sh
npm run session:check         # broker link, report de-duplication, sleep order
npm run session:integration   # real phrase broker, private bus with fake logind, UPower, profiles and portal,
                              # and an isolated GNOME Keyring when gnome-keyring-daemon is installed
python3 evaluation/power/battery-trial.py --broker target/release/badi-broker   # real model, paced typing
```

The integration lane covers startup on battery, mains and power-saver
changes (no authority change), sleep with the hold acknowledged before the
inhibitor is released, resume with a new inhibitor, a session switch and
return, a restarted logind, both shortcuts and an unknown one, a broker
restart with the report resent, and helper exit releasing the hold, the
inhibitor and the portal session. With `gnome-keyring-daemon` installed, a
second test stores a key in an isolated keyring on the private bus, restarts
the broker and checks that the helper delivered it without logging it.

### Tested on this workstation

On 2026-10-13 (Hyprland 0.56.2, xdg-desktop-portal 1.22.1,
xdg-desktop-portal-hyprland 1.4.1, systemd logind, UPower 1.91.3,
power-profiles-daemon 0.30), the helper ran against a temporary phrase broker
on the real buses: it reported `on_battery=true power_saver=true`, held a
`delay` sleep inhibitor (`systemd-inhibit --list`), registered both shortcuts
under `io.github.ahuray.badi` (`hyprctl globalshortcuts`), and
`hyprctl dispatch 'hl.dsp.global("io.github.ahuray.badi:pause-toggle")'` ran
the controller command. Stopping it released the inhibitor and the broker
report. A real suspend and resume, and a real VT switch, were not exercised.
