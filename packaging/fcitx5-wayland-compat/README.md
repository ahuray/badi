# Fcitx Wayland done-refresh backport

A narrow, version-pinned backport for Fcitx **5.1.22** on the inspected Hyprland
**0.56.2** commit `efb50993780079460b0cbed1363e2166a2de1d9f`. It carries the
upstream fix released in Fcitx 5.1.23 into a privately rebuilt frontend so
Chromium-based text-input-v3 clients keep publishing surrounding text. When
selected it affects **every input-method-v2 client of this Fcitx instance**, not
only Badi's targets; Badi's policy, field-identity, composition and acceptance
checks stay separate. The full desktop installer selects it by default on
exactly this Fcitx version ([installation](#installation)).

## Why

Chromium serializes a new state publication behind the previous `done`.
Hyprland relays each client commit to the input method as `done`, but sends the
client `done` only after an input-method `commit`. Stock 5.1.22 sends no commit
while idle, so surrounding text stalls. Upstream commit
[`1e00551`](https://github.com/fcitx/fcitx5/commit/1e00551899f9d0fa5418d899f468b6e421401adf)
([PR #1690](https://github.com/fcitx/fcitx5/pull/1690)) answers every focused
`done` with a forced preedit update: the current client preedit, if any, and one
`commit` for the current serial. It sends no insertion, deletion, synthetic key
or new preedit content.

Upstream adds `InputContext::updatePreedit(bool)` to the core library. This
slice links the system 5.1.22 core, so [the pinned patch](done-preedit-refresh.patch)
changes only `waylandimserverv2.cpp`: it posts the same `UpdatePreeditEvent` and,
unless an addon filters it, calls the frontend's preedit delegate, the delivery
path of the upstream call. Two core-private details differ: the core's
last-empty-preedit marker is not refreshed (at most one extra empty preedit
commit later), and the event is not queued while an addon blocks client events
(no in-tree 5.1.22 caller blocks them). The 5.1.23 capability reset on
activation is not included.

The feedback cycle is one client commit, one input-method `done` and one refresh
commit; Hyprland never answers an input-method commit with another `done`, so
the exchange continues only while the client publishes after a `done`. Chromium
commits only pending content type, cursor rectangle or surrounding text, and
GTK 4.22 republishes only when a `done` changed its commit or preedit text. A
client that commits after every `done` would loop. Chromium sets surrounding
text before `enable`, and `enable` resets it
([Chromium issue 565066842](https://issues.chromium.org/issues/565066842)), so
Hyprland forwards no initial surrounding text on focus; the refresh unblocks
later publications only.

The live Chromium 152, VS Code 1.138 and Cursor 3.21 trials of 2026-09-27 ran
with this frontend selected. Primary sources:
[pinned Fcitx frontend](https://github.com/fcitx/fcitx5/blob/c7ecdb931d8b378ccdcd87382a3ef7ff0bd10def/src/frontend/waylandim/waylandimserverv2.cpp),
[input-method-v2 protocol](https://github.com/fcitx/fcitx5/blob/c7ecdb931d8b378ccdcd87382a3ef7ff0bd10def/src/lib/fcitx-wayland/input-method-v2/input-method-unstable-v2.xml),
[Chromium 151 state queue](https://github.com/chromium/chromium/blob/151.0.7922.173/ui/ozone/platform/wayland/host/zwp_text_input_v3.cc),
[Hyprland relay](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/managers/input/TextInput.cpp).

## Retirement

Fcitx 5.1.23 contains the fix and is in Arch `extra`; Omarchy's stable mirror
still served 5.1.22 on 2026-09-26. After an upgrade, [launch.py](launch.py)
detects the changed runtime hashes and runs the system frontend, and the
installer installs nothing new. Remove the leftover drop-in with the
[rollback](#rollback).

## Build and tests

Requirements: Python 3.12+, CMake, Ninja, a C++20 compiler, `pkg-config`, GNU
`patch`, Wayland client/scanner/protocols and xkbcommon development files, and
exactly 5.1.22 Fcitx Core/Config/Utils development files and runtime. Protocol
tests also need `wayland-server`, `dbus-run-session` and the installed Fcitx
executable and keyboard addon. Only the frontend, its shared virtual
input-context sources and protocol wrappers are built; they link the system
Fcitx libraries.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 packaging/fcitx5-wayland-compat/build.py \
  --work-dir output/extensionless/fcitx-wayland-compat-5.1.22 --jobs 2 --check
```

The work directory must be new. `--archive /absolute/path/fcitx5-source.tar.gz`
uses an offline copy of the archive pinned in [manifest.json](manifest.json);
its commit, size, SHA-256 and the patch SHA-256 are verified, and its tar stream
matches `git archive` of the signed `5.1.22` tag Arch packages. The output is
`libwaylandim.so` and `build-receipt.json`; nothing is installed. The patch and
linked upstream code are LGPL-2.1-or-later ([license](LICENSE.LGPL-2.1-or-later)).

`--check` runs the real Fcitx against a private compositor, comparing the
unpatched baseline with the backport: one bare latest-serial commit on
activation and idle quiet, later publications, an eight-publication chain that
drains and stops, twenty queued `done` events yielding twenty ordered refreshes,
re-sent client preedit, bare panel-only and XCompose refreshes, insertion and
deletion, deactivation, reactivation after uncleared preedit, and one
same-serial refresh after a focus-callback commit. It uses private D-Bus and XDG
state and never connects to the user's compositor. The packaging and selector
checks run without a build:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s packaging/fcitx5-wayland-compat/tests -p 'test_*.py' -v
```

## Runtime selection

The module lives **outside** every ordinary Fcitx addon directory, at
`~/.local/lib/badi/compat/fcitx5-5.1.22/addons/libwaylandim.so`; never copy it to
`~/.local/lib/fcitx5/` or `/usr/lib/fcitx5/`.

[launch.py](launch.py) prepends that directory only when the service arguments
are `--disable notificationitem`, the hashes of `/usr/bin/fcitx5`,
Core/Config/Utils and the normal `libwayland.so` match those recorded at build
time, and the module matches its receipt. It refuses loader overrides, a user
`libwayland.so` override, a missing or unknown receipt, and any other
compositor or display; the Hyprland instance, socket, version, commit and
clean-build status must match. Local checks run first. At login the service can
start before Hyprland answers IPC, so an unreadable reply or an instance not yet
listed is retried with backoff (50 ms up to 0.5 s, at most 4 s); a definite
mismatch is not retried.

Any mismatch logs a content-free reason, for example `Badi Fcitx compatibility
override inactive (Hyprland instance not listed); launching the system
frontend.`, and executes `/usr/bin/fcitx5` with the original arguments,
environment and addon paths, removing only an inherited compatibility
directory. Input falls back to the normal frontend instead of stopping.

## Installation

A complete, unlocked `python3 scripts/install-desktop.py` run selects this
frontend; `--no-wayland-compat` leaves the service command unchanged and
`--broker-only` never touches it. The installer:

1. Reads `/usr/bin/fcitx5 --version`. Any version other than 5.1.22 gets no
   frontend. The effective `ExecStart` must be the stock `/usr/bin/fcitx5
   --disable notificationitem` or a Badi drop-in command; anything else stops
   the installation before any change.
2. Reuses the installed frontend when its `build-receipt.json` still verifies
   against its module: no download, no compile. Otherwise it runs
   `build.py --check` in a new `output/extensionless/fcitx-wayland-compat-5.1.22-<ns>`
   directory and deletes it once installed; `--wayland-compat-build DIR`
   installs an existing checked build instead. The receipt must name this
   manifest, passed protocol checks, the installed Core/Config/Utils version,
   today's runtime hashes and the module hash, so an upgrade or a changed pin
   rebuilds.
3. Installs `launch.py`, `build-receipt.json` and `addons/libwaylandim.so` under
   `~/.local/lib/badi/compat/fcitx5-5.1.22/` and this drop-in, keeping
   `50-badi.conf`, with every replaced file backed up:

   ```ini
   # ~/.config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf
   [Service]
   ExecStart=
   ExecStart=/usr/bin/python3 -B %h/.local/lib/badi/compat/fcitx5-5.1.22/launch.py --disable notificationitem
   ```

4. Removes a retired `fcitx5-5.1.21` directory, backed up, only when no service
   command references it and it holds exactly its three files.
5. After `daemon-reload`, confirms the effective `ExecStart`, performs the one
   Fcitx restart the addon update needs, and checks that the running service
   maps the installed module (exact inode, stable PID). If `launch.py` fell back
   to the stock frontend, it says so.

The restart briefly interrupts input-method handling for the whole desktop,
including XCompose.

## Rollback

From an unlocked desktop, delete only the drop-in to return to the stock
frontend:

```sh
systemctl --user stop omarchy-fcitx5.service
rm ~/.config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf
systemctl --user daemon-reload
systemctl --user start omarchy-fcitx5.service
```

To restore the exact pre-install frontend files instead, replace the `rm` line
with a restore from the backup the installer printed as `Replaced files and
their rollback map`; a path changed since then is listed, not overwritten:

```sh
python3 scripts/badi_install.py restore ~/.local/state/badi/install-backups/<UTC time> \
  --only .local/lib/badi/compat \
  --only .config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf
```

Check the running process rather than a successful start; the stock frontend
maps `/usr/lib/fcitx5/libwaylandim.so`:

```sh
grep -F libwaylandim.so /proc/$(systemctl --user show omarchy-fcitx5.service -p MainPID --value)/maps
```

Rerunning the installer selects the frontend again unless `--no-wayland-compat`
is given. Leave `50-badi.conf`, the Badi addon, the broker and the observer in
place for this frontend-only rollback.
