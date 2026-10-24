# Badi session helper

`badi-session` tells the broker, without any text, whether the login session
is in the foreground, whether the machine is about to sleep, whether it runs
on battery or in the power-saver profile, and whether the screen is being
shared. It also offers three desktop shortcuts through the GlobalShortcuts
portal. It never reads a field, opens a suggestion session, edits anything or
changes a system setting.

| Signal | Source | Broker effect |
| --- | --- | --- |
| Session not in the foreground | logind display session `Active` | Hold: every grant is revoked until it returns |
| Preparing to sleep | logind `PrepareForSleep(true)` | Hold, acknowledged before the delay inhibitor is released |
| Resumed | `PrepareForSleep(false)` | Hold ends; adapters acquire their fields afresh |
| On battery | UPower `OnBattery` | Low power: automatic scheduling follows `badi battery` |
| Power-saver profile | power-profiles-daemon `ActiveProfile` | Low power, as above |
| Screen shared | A desktop portal screencast stream in PipeWire (`pw-dump --monitor`) | Hold until the stream ends, unless `badi screen-share suggest` |

## Broker contract

The helper speaks protocol v2 with the capabilities `desktop_signals` and
`style_key` (adapter kind `session`) and sends `desktop.state`
`{session: active|inactive|sleeping, on_battery, power_saver, screen_shared}`
whenever one changes; the reply is `desktop.status`
([schema](../../protocol/v2/schema.json), [broker runbook](../../broker/README.md#desktop-session-signals)).

- A report only withholds. Entering or leaving a hold advances the authority
  epoch: entering it revokes context, pending inference, visible suggestions,
  pending commits and the type-through carry; leaving it makes every adapter
  reacquire. A Tab pressed before reacquisition finds no grant.
- The report is bound to the helper's connection. If the helper exits, the
  broker drops its hold and low-power state, except that a live screen share
  stays held for 10 s or until a helper reports again; after a broker restart
  the helper reconnects (1, 2, 5, 10, then 30 s) and reports again.
- Unknown is permissive only where nothing is at stake: without logind the
  session counts as active (adapters keep their own focus, lock and expiry
  checks), and without UPower or power-profiles-daemon the machine counts as
  on mains power. Without PipeWire the screen counts as not shared, because
  a portal screencast cannot exist without it.
- `LockedHint` is not used. It is a hint, not proof of unlock; the observer
  keeps its fresh Omarchy lock check.

## Sleep

At start, and again after each resume, the helper takes a logind `delay`
inhibitor for `sleep`. On `PrepareForSleep(true)` it reports `sleeping`, waits
for the broker's acknowledgement, then closes the inhibitor so sleep
proceeds. Each exchange with the broker gives up after 1.5 s: one when the
helper is connected, the hello first when it has to reconnect. The style key
of a new connection is looked up only after the report, so the keyring never
holds sleep. A restarted logind forgets inhibitors; the helper takes a new
one when the name returns.

## Screen sharing

A browser's `getDisplayMedia`, OBS or a meeting app asks
`xdg-desktop-portal` to share the screen, a window or a region, and the
portal backend publishes the picture as a PipeWire video source for the
whole share. The helper runs `pw-dump --monitor` (part of PipeWire) and
counts a node as a share when its `media.class` is `Video/Source` and its
`node.name` starts with `xdg-desktop-portal-` or its `media.name` with
`xdph-streaming-`, the prefix Omarchy 4.5's notifications watch. A camera is
a video source too, but the session manager creates it, so it never counts.
Frame activity is not used: screenshots and the share picker's previews
produce it without a share.

- While any such stream exists the report says `screen_shared: true`, from
  the moment the first one appears until one second after the last one
  ends, so a share that changes streams (Chromium's preview becoming the
  share replaces one stream with the next within milliseconds) is never
  released in between. With
  settings `screen_share: hold` (the default, `badi screen-share hold`) the
  broker holds every grant as for an inactive session; `badi screen-share
  suggest` keeps suggesting. `badi status`, `badi doctor` and the panel name
  the hold, and the helper asks the bar mark to look at once (`omarchy-shell
  -q badi-writing refresh`), so it dims as for a pause; the debug trace shows
  it as `screen_share: held` on `desktop.status`.
- Only node ids are kept. Node names, applications and media titles (a
  playing song's, for example) are read to classify a node and dropped; the
  journal says only `screen_shared=true|false`.
- A helper that starts counts the screen as shared until its first dump, so
  a helper that restarted mid-share never tells the broker, which keeps that
  share held for its 10 s grace, that nothing is shared before it looked.
- If `pw-dump` ends after it showed anything, the helper restarts it (1, 2,
  5, 10, then 30 s) and keeps the streams it last saw until the new dump
  replaces them, so a crash never ends a hold early. A `pw-dump` that ends
  without output could not reach PipeWire, and the share ends. Without
  `pw-dump` nothing is detected, and `badi doctor` says so.
- Other portal backends' node names are not checked yet: only
  xdg-desktop-portal-hyprland has been tried ([tested with](#tested-with)).

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
`quiet` with `normal` on disposable keys typed at 150 ms per key, 450 ms
after words and 1.5 s after sentences (`quiet` against `normal`):

| Run (date) | Model calls | Runtime CPU s | Shown (matching) | Visible p95 |
| --- | --- | --- | --- | --- |
| Battery, power-saver, release build, 752 keys per mode (2026-10-13) | 122 / 586 | 182 / 464 | 313 (211) / 257 (190) | 550 / 525 ms |
| Charging, performance, 376 keys per mode, one repeat (2026-10-13) | 45 / 220 | 50 / 148 | 27 / 31 distinct reference words | 456 / 211 ms |
| Power-saver, a run-on sentence at 450 ms per word (2026-10-17) | 62 requests | | 15 under `quiet`, most cancelled by the next word | |

Still unmeasured: the balanced profile on battery, real typing intervals,
energy from a privileged power counter rather than runtime CPU time, and a
reviewed count of fully accepted additions.

## Style key

When it connects to a broker, the helper first sends its report, then looks
up the style-learning key in the Secret Service ([style_key.py](style_key.py),
libsecret) from the main loop and hands it over with `style.key`. It never
prompts: a locked keyring is retried every two
minutes, and no stored key sends nothing. The keyring item is labelled
`Badi style learning key` with the single attribute `purpose=style-learning`;
its value is two random hex strings. `badi style on` creates it (the keyring
may ask to unlock) and delivers it at once; `badi style forget` deletes it.
The helper can deliver a key but cannot read samples.

## Shortcuts

The helper registers the host application id `io.github.ahuray.badi` with the
portal and binds `pause-toggle` (runs `badi pause toggle`, the same
compare-and-swap settings write as the bar's right-click), `snooze` (runs
`badi snooze`: Badi stays off in the focused field's app or site for 10
minutes, like Cotypist's Ctrl+Option+Command+`) and `open-settings` (runs
`badi settings`). The compositor chooses the keys; on Hyprland 0.56, for
example in `~/.config/hypr/bindings.lua` (all three are free in Omarchy
4.0.6, whose `SUPER + ALT + COMMA` invokes the last notification):

```lua
hl.bind("SUPER + ALT + B", hl.dsp.global("io.github.ahuray.badi:pause-toggle"))
hl.bind("SUPER + ALT + GRAVE", hl.dsp.global("io.github.ahuray.badi:snooze"))
hl.bind("SUPER + ALT + PERIOD", hl.dsp.global("io.github.ahuray.badi:open-settings"))
```

A pause, resume or snooze shows on Omarchy's on-screen display, as volume
and the touchpad toggle do (`omarchy-osd -i media-pause -m "Badi paused"`;
a snooze reads `Snoozed in` and the app or site), and as a desktop
notification where `omarchy-osd` is missing. A snooze the controller
refused stays a notification with its own line, saying why nothing was
snoozed (no focused field Badi follows, or fields of two apps);
the snooze itself is [broker runtime state](../../broker/README.md#snooze)
that only withholds, and `badi resume` ends it sooner. Interface version 1 is
all this needs: it has no `ConfigureShortcuts`, and the helper never calls
it. A shortcut grants no access to an application or field; there is
deliberately no request shortcut, because a request needs an admitted adapter
field (Ctrl+Shift+Space already does that through Fcitx).
xdg-desktop-portal-hyprland 1.4.1 keeps listing the shortcuts in `hyprctl
globalshortcuts` after the helper stops; while it is stopped they do nothing,
and a restart reuses them.

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
npm run session:check         # broker link, report de-duplication, sleep order, report before the style key
npm run session:integration   # real phrase broker, private bus with fake logind, UPower, profiles and portal,
                              # a fake pw-dump, and an isolated GNOME Keyring when gnome-keyring-daemon is installed
python3 evaluation/power/battery-trial.py --broker target/release/badi-broker   # real model, paced typing
```

The integration lane covers startup on battery, mains and power-saver
changes (no authority change), sleep with the hold acknowledged before the
inhibitor is released, resume with a new inhibitor, a session switch and
return, a restarted logind, the three shortcuts and an unknown one, a broker
restart with the report resent, and helper exit releasing the hold, the
inhibitor and the portal session. A second test shares the screen through
the fake `pw-dump`: the broker holds, `screen-share suggest` lifts the hold
while the share lasts, `hold` restores it, the stream's removal ends it, and
a helper restarted mid-share keeps the hold without a new authority epoch;
no stream name reaches
the journal. The unit lane parses split and compact dump output, restarts
after a crash without lapsing the hold, and ends the share when PipeWire is
gone. With `gnome-keyring-daemon` installed, a
third test stores a key in an isolated keyring on the private bus, restarts
the broker and checks that the helper delivered it without logging it.

### Tested with

Live on the development workstation: Hyprland 0.56.2, xdg-desktop-portal
1.22.1, xdg-desktop-portal-hyprland 1.4.1, systemd logind, UPower 1.91.3,
power-profiles-daemon 0.30, PipeWire 1.6.9, disposable pages.

| What (installed commit, date) | Result | Boundary |
| --- | --- | --- |
| Real buses, temporary phrase broker (2026-10-13) | Reported `on_battery=true power_saver=true`; held a `delay` sleep inhibitor (`systemd-inhibit --list`); registered the shortcuts under `io.github.ahuray.badi`; a `pause-toggle` dispatch ran the controller; stopping released the inhibitor and the report | No real suspend and resume or VT switch: the session locks on sleep and needs someone to unlock it |
| Helper restart while a suggestion showed, Chromium 154, real model (`5e141ed`, 2026-10-19) | Reconnected within 0.2 s with the same active, mains state; nothing was withheld and Tab accepted into the same field ([matrix](../fcitx5/README.md#tested-with)) | |
| Snooze shortcut by `hl.dispatch`, Chromium 154 (`e1fa107`, 2026-10-20) | Hid the suggestion and snoozed only that origin (the notification, `badi status` and the panel named it; another port still suggested); `badi resume` restored it; `badi snooze 1` ran out after 60 s | A key bound with `hl.bind` not tried; the dispatch reaches the same portal signal |
| Entire-screen share from `getDisplayMedia`, Chromium 155 with Omarchy's picker (`7b2d5fc`, 2026-10-23) | Held from the picker's preview (already a portal stream) until about a second after the share stopped; `screen-share suggest` and `hold` switched it during the share; status, the panel, Omarchy's `notifications probe` and the text-free trace agreed | Not tried: window or region shares, OBS, meeting apps, other portal backends |
| Helper restart and SIGKILL mid-share (`29ec777`), and under the unit's sandbox limits (`827492a`) | The hold and the authority epoch stayed until the share ended; `pw-dump` ran as the helper's child under `RestrictRealtime`, `RestrictNamespaces`, `SystemCallArchitectures` and `LimitCORE` | |
