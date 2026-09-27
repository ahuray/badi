# Fcitx Wayland done-refresh backport

This is a narrow, version-pinned backport for Fcitx **5.1.22**, the inspected
Hyprland **0.56.2** commit `efb50993780079460b0cbed1363e2166a2de1d9f`, and the
Chromium 151 / Brave Origin 152 text-input-v3 publication stall. It carries the
upstream Fcitx change released in 5.1.23 into a privately rebuilt frontend. It
is not a claim of physical editor compatibility. When selected, it affects
**all input-method-v2 clients of this Fcitx instance**, including applications
other than Badi's allowed targets. Badi's acquisition, policy, field-identity,
composition and acceptance checks remain separate. The full desktop installer
builds and selects it by default on exactly this Fcitx version; see
[installation](#installation) for the opt-out.

Chromium serializes a new state publication behind the previous `done`.
Hyprland relays each client commit to the input method as `done`, but sends the
client `done` only after an input-method `commit`. Stock 5.1.22 sends no commit
while idle, so surrounding text stalls. Upstream commit
[`1e00551`](https://github.com/fcitx/fcitx5/commit/1e00551899f9d0fa5418d899f468b6e421401adf)
([PR #1690](https://github.com/fcitx/fcitx5/pull/1690)) answers every focused
`done` with a forced preedit update: the current client preedit, if any, and one
`commit` for the current serial. It sends no insertion, deletion, synthetic key,
or new preedit content.

Upstream adds `InputContext::updatePreedit(bool)` to the core library. This
slice links the system 5.1.22 core, so [the pinned patch](done-preedit-refresh.patch)
changes only `waylandimserverv2.cpp`. It posts the same `UpdatePreeditEvent` and,
unless an addon filters it, calls the frontend's preedit delegate, which is the
delivery path of the upstream call. Two core-private details are not reproduced.
The core's last-empty-preedit marker is not refreshed, which can cause at most
one extra empty preedit commit later. The event is also not queued while an
addon blocks client events; no in-tree 5.1.22 caller blocks them. The separate
5.1.23 capability reset on activation is not included.

### Differences from the retired 5.1.21 acknowledgement

The earlier `idle-done-ack.patch` (in Git history) does not apply to 5.1.22.
The upstream behavior replaces it with three semantic differences:

- There is no `IdleDoneAcknowledgement` option. The launcher and service
  drop-in below are the opt-in; the selected module always refreshes.
- Active client preedit is re-sent instead of suppressing the commit. Hyprland
  clears a client's preedit on any input-method commit without one, so the
  re-send keeps composition visible. Panel-only preedit and XCompose state
  without client preedit receive a bare commit; the client has nothing to clear.
- There is no per-serial duplicate suppression. If a focus callback already
  committed during that `done`, one extra same-serial commit follows, and
  Hyprland sends the client one extra `done`.

The feedback cycle is one client commit, one input-method `done`, and one
refresh commit. Hyprland never answers an input-method commit with another
input-method `done`. The exchange continues only while the client publishes after
a `done`. Chromium 151 commits only pending content type, cursor rectangle or
surrounding text. GTK 4.22 republishes only when a `done` changed its commit or
preedit text; an identical re-sent preedit changes neither. A client that commits
after every `done` would loop. The upstream maintainer cites KWin, which already
gives text-input-v3 clients one `done` per commit.

Chromium sets surrounding text before `enable`, and `enable` resets it
([Chromium issue 565066842](https://issues.chromium.org/issues/565066842)).
Hyprland therefore forwards no initial surrounding text on focus. The refresh
unblocks later publications only. [PR #1689](https://github.com/fcitx/fcitx5/pull/1689)
reported that an earlier, activation-only revision of #1690 did not unblock
Electron 43. The merged revision refreshes on every `done`; it has not been
physically verified with Chromium here.

Primary sources: [pinned Fcitx frontend](https://github.com/fcitx/fcitx5/blob/c7ecdb931d8b378ccdcd87382a3ef7ff0bd10def/src/frontend/waylandim/waylandimserverv2.cpp),
[input-method-v2 protocol](https://github.com/fcitx/fcitx5/blob/c7ecdb931d8b378ccdcd87382a3ef7ff0bd10def/src/lib/fcitx-wayland/input-method-v2/input-method-unstable-v2.xml),
[Chromium 151 state queue](https://github.com/chromium/chromium/blob/151.0.7922.173/ui/ozone/platform/wayland/host/zwp_text_input_v3.cc),
[Hyprland relay](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/managers/input/TextInput.cpp).

## Upstream status and retirement

Fcitx 5.1.23 contains the fix. Arch `extra` has published `fcitx5 5.1.23-1`
since 2026-09-24. On 2026-09-26, Omarchy's stable mirror still served
`5.1.22-1`. After an upgrade to 5.1.23, [launch.py](launch.py) detects the
changed runtime hashes and runs the system frontend, which includes the same
refresh. The installer then installs nothing new and leaves an existing drop-in
in place; remove it with the [rollback](#rollback).

## Reproducible isolated build

Requirements: Python 3.12+, CMake, Ninja, a C++20 compiler, `pkg-config`, GNU
`patch`, Wayland client/scanner/protocols and xkbcommon development files, and
exactly 5.1.22 Fcitx Core/Config/Utils development files and runtime. Protocol
tests additionally need `wayland-server`, `dbus-run-session` and the installed
Fcitx executable/keyboard addon. This standalone slice builds the frontend, its
shared virtual input-context sources and original protocol wrappers. It links
the existing system Fcitx libraries. It does not rebuild the core, compositor,
or normal Wayland module. No ECM, Plasma-protocol or Yoga installation is needed.

From the repository root, choose a **new** work directory:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 packaging/fcitx5-wayland-compat/build.py \
  --work-dir output/extensionless/fcitx-wayland-compat-5.1.22 --jobs 2 --check
```

`--archive /absolute/path/fcitx5-source.tar.gz` uses an offline copy of the exact
archive in [manifest.json](manifest.json). The source commit, archive byte count,
SHA256 and patch SHA256 are verified. The archive's tar stream matches `git
archive` of the signed `5.1.22` tag used by Arch's package. Existing work
directories are refused. Source and full license notices remain alongside the
build. The frontend patch and linked upstream code are LGPL-2.1-or-later; the
full [license text](LICENSE.LGPL-2.1-or-later) is retained here. Protocol files
also retain their original notices in the source archive. The output is
`libwaylandim.so` and `build-receipt.json`; nothing is installed or enabled.

The real-Fcitx/private-compositor tests compare the unpatched baseline with the
backport. They cover one bare latest-serial commit on activation and idle quiet.
They also cover later publications, an eight-publication chain that drains and
stops, and twenty queued `done` events yielding exactly twenty ordered refreshes.
Further cases re-send client preedit and keep panel-only preedit and XCompose
refreshes bare. The rest cover explicit insertion/deletion, deactivation,
reactivation after uncleared preedit, and one same-serial refresh after a
focus-callback commit. They use private D-Bus, XDG state, a short `/tmp` Wayland
socket, and fixed synthetic text. They do not connect to the user's compositor,
exercise application detection, or prove Chromium rendering, editing, or undo.

The portable packaging/selector source check is:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s packaging/fcitx5-wayland-compat/tests -p 'test_*.py' -v
```

## Runtime selection and upgrade fallback

Keep the candidate **outside** all ordinary Fcitx addon directories. The intended
location is `~/.local/lib/badi/compat/fcitx5-5.1.22/addons/libwaylandim.so`.
Never copy it to `~/.local/lib/fcitx5/` or `/usr/lib/fcitx5/`.

[launch.py](launch.py) prepends that isolated directory only for the inspected
service arguments `--disable notificationitem`, matching binary hashes of
`/usr/bin/fcitx5`, Core/Config/Utils and the normal `libwayland.so` recorded before
the build, and the exact candidate hash. It refuses loader overrides, a separate
user `libwayland.so` override, an unrecognized/missing receipt, or another active
compositor/display. The selected Hyprland instance, Wayland socket, exact version,
commit and clean-build status must match the inspected cell. Build-time runtime
files must remain unchanged through receipt creation.

An upgrade or mismatch executes `/usr/bin/fcitx5` with the original arguments,
environment and addon paths. Only a previously inherited exact compatibility
directory is removed. Thus it falls back to the normal frontend instead of
leaving Wayland input unavailable. Other user addon paths remain intact. This
does not sandbox unrelated addons or change how Fcitx loads the user's own addons.

## Installation

The complete, unlocked `python3 scripts/install-desktop.py` run selects this
frontend by default. `--no-wayland-compat` leaves the Fcitx service command
unchanged, and `--broker-only` never touches it. The installer:

1. Reads `/usr/bin/fcitx5 --version` before building. Any version other than
   5.1.22 gets no frontend (5.1.23 and later contain the refresh). The service's
   effective `ExecStart` must be the stock `/usr/bin/fcitx5 --disable
   notificationitem` or a Badi drop-in command; anything else stops the
   installation before any file changes.
2. Runs `build.py --check` in a new
   `output/extensionless/fcitx-wayland-compat-5.1.22-<ns>` directory, which
   downloads the pinned archive. `--wayland-compat-build DIR` instead reuses an
   existing checked build. Either way, the receipt must name this manifest,
   passed protocol checks, the installed Core/Config/Utils version, today's
   runtime hashes and the artifact hash.
3. Installs `launch.py`, `build-receipt.json` and `addons/libwaylandim.so` under
   `~/.local/lib/badi/compat/fcitx5-5.1.22/`, plus this drop-in:

   ```ini
   # ~/.config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf
   [Service]
   ExecStart=
   ExecStart=/usr/bin/python3 -B %h/.local/lib/badi/compat/fcitx5-5.1.22/launch.py --disable notificationitem
   ```

   The existing `50-badi.conf` environment drop-in is kept. Replaced files go to
   the installer's backup directory and `changed-files.json`; the install
   receipt records the new digests.
4. Removes the retired `fcitx5-5.1.21` directory, after backing it up, only when
   no service command references it and it holds exactly its three files.
   Otherwise the installer keeps it and names it in its output.
5. After `daemon-reload`, confirms that the effective `ExecStart` is the new
   command. Then it performs the single Fcitx restart that the addon update
   already needs, and checks that the running service maps the installed module
   (exact inode, stable PID). If `launch.py` fell back to the stock frontend,
   the installer says so. Input continues with the stock frontend.

The restart briefly interrupts input-method handling for the whole desktop,
including XCompose. A leftover 5.1.21 `IdleDoneAcknowledgement` option in
`~/.config/fcitx5/conf/waylandim.conf` is ignored by 5.1.22 and left unchanged.

After installation, run the disposable physical Chromium/Brave v3 trial:
automatic surrounding text after the first page change, visible prediction,
explicit acceptance, native undo, stale focus and real IME composition. Report
it separately from the synthetic protocol proof.

## Rollback

Use the directory that the installer printed as `Rollback files and
changed-file list`. The desktop must be unlocked; this helper rechecks all five
lock flags before each service operation:

```sh
compat_fcitx_service() {
  python3 - "$1" <<'PY'
import json, subprocess, sys
action = sys.argv[1]
if action not in ('start', 'stop'):
    raise SystemExit('Only the reviewed start/stop actions are supported')
reply = subprocess.run(['omarchy-shell', 'lock', 'status'], check=True,
                       capture_output=True, text=True, timeout=4)
state = json.loads(reply.stdout)
flags = ('locked', 'secure', 'requested', 'pending', 'sessionLocked')
if not isinstance(state, dict) or not all(state.get(key) is False for key in flags):
    raise SystemExit('Desktop is locked or its state is unknown; service unchanged')
subprocess.run(['systemctl', '--user', action, 'omarchy-fcitx5.service'], check=True)
PY
}
```

To return to the stock frontend, delete only the drop-in and restart:

```sh
compat_fcitx_service stop
rm ~/.config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf
systemctl --user daemon-reload
compat_fcitx_service start
```

To restore the exact pre-install frontend files instead, including a retired
5.1.21 directory, replace the `rm` line with this. Other Badi files keep their
update:

```sh
python3 - ~/.local/state/badi/install-backups/NNN <<'PY'
import json, pathlib, shutil, sys
home, backup = pathlib.Path.home(), pathlib.Path(sys.argv[1])
for relative in json.loads((backup / 'changed-files.json').read_text()):
    if not (relative.startswith('.local/lib/badi/compat/') or relative.endswith('/60-badi-wayland-compat.conf')):
        continue
    target, saved = home / relative, backup / relative
    if saved.exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(saved, target)
    else:
        target.unlink(missing_ok=True)
PY
```

Verify the result in the running process instead of inferring it from a
successful start:

```sh
grep -F libwaylandim.so /proc/$(systemctl --user show omarchy-fcitx5.service -p MainPID --value)/maps
```

The stock frontend maps `/usr/lib/fcitx5/libwaylandim.so`. Retain the backup.
Rerunning the installer reinstalls the frontend unless `--no-wayland-compat` is
given. Do not remove `50-badi.conf`, the Badi addon, the broker, the
accessibility helper, or unrelated user configuration as part of this
frontend-only rollback.
