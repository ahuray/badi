# Fcitx 5.1.22 Wayland frontend backport

A narrow, version-pinned backport of an upstream Fcitx 5.1.23 fix, rebuilt
privately for Fcitx **5.1.22** on Hyprland **0.56.2**. Without it,
Chromium-based apps stop publishing surrounding text. It affects **every
input-method-v2 client** of this Fcitx instance, not only Badi's; Badi's own
checks stay separate. The full desktop installer selects it on exactly this
Fcitx version. Retire it once Omarchy ships Fcitx 5.1.23: [launch.py](launch.py)
already falls back to the system frontend after an upgrade.

## Why

Chromium queues each surrounding-text publication behind the previous `done`.
Hyprland sends the client `done` only after an input-method `commit`, and stock
5.1.22 sends no commit while idle, so the text stalls.
Upstream [`1e00551`](https://github.com/fcitx/fcitx5/commit/1e00551899f9d0fa5418d899f468b6e421401adf)
([PR #1690](https://github.com/fcitx/fcitx5/pull/1690)) answers every focused
`done` with a forced preedit update and one `commit`. It sends no insertion,
deletion, key or new preedit text.

The 5.1.22 core lacks the new `InputContext::updatePreedit(bool)`, so
[the pinned patch](done-preedit-refresh.patch) changes only
`waylandimserverv2.cpp`. It posts the same `UpdatePreeditEvent` through public
API. Two differences remain:
- the core's last-empty-preedit marker is not refreshed, so at most one extra
  empty preedit commit can follow;
- the 5.1.23 capability reset on activation is left out.

The exchange continues only while a client publishes after each `done`, which
Chromium and GTK 4 do not.

## Build and test

Needs Python 3.12+, CMake, Ninja, a C++20 compiler, `pkg-config`, GNU `patch`,
Wayland and xkbcommon development files, and exactly the 5.1.22 Fcitx
development files and runtime. Protocol tests also need `wayland-server`,
`dbus-run-session` and Fcitx's keyboard addon.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 packaging/fcitx5-wayland-compat/build.py \
  --work-dir output/extensionless/fcitx-wayland-compat-5.1.22 --jobs 2 --check
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s packaging/fcitx5-wayland-compat/tests -p 'test_*.py' -v   # no build needed
```

- **Source.** The work directory must be new. The source archive pinned in
  [manifest.json](manifest.json) is verified by commit, size and SHA-256, as
  is the patch; `--archive PATH` uses an offline copy.
- **Output.** `libwaylandim.so` and `build-receipt.json`; nothing is installed.
- **`--check`** runs the real Fcitx against a private compositor, comparing
  the stock and patched frontends on commits, idle quiet, publication chains,
  queued `done` events, preedit, XCompose, insertion, deletion and focus
  changes.
- **License.** The patch and upstream code are LGPL-2.1-or-later
  ([license](LICENSE.LGPL-2.1-or-later)).

## Runtime selection

The module lives outside every Fcitx addon directory, at
`~/.local/lib/badi/compat/fcitx5-5.1.22/addons/libwaylandim.so`.
[launch.py](launch.py) prepends that directory only when all of these hold:
- the service arguments are `--disable notificationitem`;
- `/usr/bin/fcitx5`, the core libraries and the normal `libwayland.so` match
  the hashes recorded at build time;
- the module matches its receipt;
- the Hyprland instance and version match.

Hyprland may not answer at login, so the check retries for up to 4 s. Any
mismatch logs a content-free reason and runs the stock `/usr/bin/fcitx5`
unchanged, so input never stops.

## Installation

A complete, unlocked `scripts/install-desktop.py` run selects the frontend.
`--no-wayland-compat` and `--broker-only` leave it alone. The installer:

1. Requires `fcitx5 --version` 5.1.22 and a stock or Badi `ExecStart`; anything
   else stops it before any change.
2. Reuses the installed frontend when its receipt still verifies; otherwise it
   runs `build.py --check` in a fresh `output/` directory and deletes it after
   installing (`--wayland-compat-build DIR` installs an existing build).
3. Installs `launch.py`, the receipt and the module, and this drop-in, backing
   up whatever it replaces:

   ```ini
   # ~/.config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf
   [Service]
   ExecStart=
   ExecStart=/usr/bin/python3 -B %h/.local/lib/badi/compat/fcitx5-5.1.22/launch.py --disable notificationitem
   ```

4. Reloads systemd, restarts Fcitx once and checks that the running service
   maps the installed module, saying so if it fell back to the stock frontend.

The restart briefly interrupts input-method handling desktop-wide, including
XCompose.

## Rollback

Delete the drop-in to return to the stock frontend:

```sh
systemctl --user stop omarchy-fcitx5.service
rm ~/.config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf
systemctl --user daemon-reload
systemctl --user start omarchy-fcitx5.service
grep -F libwaylandim.so /proc/$(systemctl --user show omarchy-fcitx5.service -p MainPID --value)/maps
```

The stock frontend maps `/usr/lib/fcitx5/libwaylandim.so`. To restore the exact
earlier files instead, restore the backup the installer printed, limited with
`--only .local/lib/badi/compat` and `--only` the drop-in path. Rerunning the
installer selects the frontend again unless `--no-wayland-compat` is given.
