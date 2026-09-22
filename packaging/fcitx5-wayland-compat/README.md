# Opt-in Fcitx Wayland acknowledgement compatibility

This is a narrow compatibility experiment for Fcitx **5.1.21**, the inspected
Hyprland **0.56.2** commit `efb50993780079460b0cbed1363e2166a2de1d9f`, and the
Chromium 151 / Brave Origin 152 text-input-v3 publication stall. It is not a
general upstream fix or a claim of physical editor compatibility. When enabled,
it affects **all idle input-method-v2 clients of this Fcitx instance**, including
applications other than Badi's allowed targets. Badi's acquisition, policy,
field-identity, composition and acceptance checks remain separate.

Chromium serializes a new state publication behind the previous `done`.
Hyprland forwards `done` after an input-method commit. An idle Fcitx frontend
otherwise sends no commit, so surrounding text stalls. The patch sends a bare
protocol commit for an unacknowledged current serial while focused and idle.
It sends no insertion, deletion, synthetic key, or fabricated preedit request.
Existing commits for that serial suppress the acknowledgement. Sent preedit,
local preedit and XCompose suppress it too; protocol activation resets only the
previous field's preedit bookkeeping before focus callbacks run.

The new `IdleDoneAcknowledgement` option defaults to **False**. A bare commit
can clear a client's preedit, which is why those composition guards are required.
This patch does not repair the separate text-input-v1 `commit_state` mismatch.
Fcitx 5.1.22 and the inspected newer upstream source do not contain this idle
acknowledgement change.

Primary sources: [pinned Fcitx frontend](https://github.com/fcitx/fcitx5/blob/1319952f284eae17a36cba9e800843ca61a163c1/src/frontend/waylandim/waylandimserverv2.cpp),
[input-method-v2 protocol](https://github.com/fcitx/fcitx5/blob/1319952f284eae17a36cba9e800843ca61a163c1/src/lib/fcitx-wayland/input-method-v2/input-method-unstable-v2.xml),
[Chromium 151 state queue](https://github.com/chromium/chromium/blob/151.0.7922.173/ui/ozone/platform/wayland/host/zwp_text_input_v3.cc),
[Hyprland relay](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/managers/input/TextInput.cpp).

## Reproducible isolated build

Requirements: Python 3.12+, CMake, Ninja, a C++20 compiler, `pkg-config`, GNU
`patch`, Wayland client/scanner/protocols and xkbcommon development files, and
exactly 5.1.21 Fcitx Core/Config/Utils development files and runtime. Protocol
tests additionally need `wayland-server`, `dbus-run-session` and the installed
Fcitx executable/keyboard addon. This standalone slice builds only the frontend
and its original protocol wrappers; it links the existing system Fcitx libraries.
It does not rebuild the core, compositor, or normal Wayland module. No ECM,
Plasma-protocol or Yoga installation is needed for this slice.

From the repository root, choose a **new** work directory:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 packaging/fcitx5-wayland-compat/build.py \
  --work-dir output/extensionless/fcitx-wayland-compat-verified --jobs 2 --check
```

`--archive /absolute/path/fcitx5-source.tar.gz` uses an offline copy of the exact
archive in [manifest.json](manifest.json). The source commit, archive byte count,
SHA256 and patch SHA256 are verified. Existing work directories are refused.
Source and full license notices remain alongside the build. The frontend patch
and linked upstream code are LGPL-2.1-or-later; the full [license text](LICENSE.LGPL-2.1-or-later)
is retained here. Protocol files also retain their original notices in the source
archive. The output is `libwaylandim.so` and `build-receipt.json`; nothing is
installed or enabled by this command.

The real-Fcitx/private-compositor tests compare the unpatched baseline, disabled
option, and enabled candidate. They cover current serials, an eight-publication
feedback chain that drains and stops, zero document operations for bare commits,
sent/local preedit and XCompose, uncleared composition across reactivation,
explicit insertion/deletion, and duplicate suppression when a focus callback
already committed. They use private D-Bus, XDG state, a short `/tmp` Wayland
socket, and fixed synthetic text. They do not connect to the user's compositor
or prove Chromium rendering, editing, or native undo.

The portable packaging/selector source check is:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s packaging/fcitx5-wayland-compat/tests -p 'test_*.py' -v
```

## Runtime selection and upgrade fallback

Keep the candidate **outside** all ordinary Fcitx addon directories. The intended
location is `~/.local/lib/badi/compat/fcitx5-5.1.21/addons/libwaylandim.so`.
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

## Installation recipe for the reviewed local candidate

These commands are a reviewable recipe, **not an instruction to install while
the desktop is locked**. First verify an unlocked session and the existing
service command, active/enabled state, current addon paths and relevant files:

```sh
omarchy-shell lock status
systemctl --user cat omarchy-fcitx5.service
systemctl --user show omarchy-fcitx5.service -p ActiveState -p UnitFileState -p MainPID
```

The lock response must report false for `locked`, `secure`, `pending`,
`requested`, and `sessionLocked`. The inspected base service uses
`/usr/bin/fcitx5 --disable notificationitem`. Preserve the existing `50-badi.conf`
environment drop-in; this recipe adds only `60-badi-wayland-compat.conf`.
Stopping/restarting this service briefly interrupts input-method handling for
the whole desktop, including XCompose. Save disposable trial text first.
Run each block only after the previous block succeeded; stop on any error.
Use this wrapper for **every** stop/start below, including rollback. It rechecks
all five lock flags immediately before the service operation because the session
can relock during a build or trial:

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

Before stopping or copying, refuse symbolic links (including dangling links) or
unexpected target types. This file-copy recipe cannot preserve their topology;
inspect and adapt the concrete backup before proceeding if this preflight fails:

```sh
python3 - <<'PY'
from pathlib import Path
home = Path.home()
targets = [
  '.local/lib/badi/compat/fcitx5-5.1.21/launch.py',
  '.local/lib/badi/compat/fcitx5-5.1.21/build-receipt.json',
  '.local/lib/badi/compat/fcitx5-5.1.21/addons/libwaylandim.so',
  '.config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf',
  '.config/fcitx5/conf/waylandim.conf', '.config/fcitx5/profile',
]
for relative in targets:
    target = home / relative
    for path in (target, *target.parents):
        if path == home:
            break
        assert not path.is_symlink(), f'Symlink requires a preserving backup: {path}'
        if path.exists():
            assert path.is_file() if path == target else path.is_dir(), path
print('Exact compatibility targets are ordinary files or absent')
PY
```

Choose the successfully checked build and create a private backup directory:

```sh
compat_build="$PWD/output/extensionless/fcitx-wayland-compat-verified"
compat_root="$HOME/.local/lib/badi/compat/fcitx5-5.1.21"
mkdir -p "$HOME/.local/state/badi/compat-backups"
compat_backup=$(mktemp -d "$HOME/.local/state/badi/compat-backups/install-XXXXXXXX")
systemctl --user is-active omarchy-fcitx5.service > "$compat_backup/active-before.txt"
systemctl --user is-enabled omarchy-fcitx5.service > "$compat_backup/enabled-before.txt"
compat_fcitx_service stop
```

Back up these exact targets **after stopping**, so Fcitx cannot rewrite them
during the copy. Record absent files too; the profile is included because Fcitx
can save input-method choices on shutdown:

```sh
python3 - "$compat_backup" <<'PY'
import json, pathlib, shutil, sys
home = pathlib.Path.home()
backup = pathlib.Path(sys.argv[1])
files = [
  '.local/lib/badi/compat/fcitx5-5.1.21/launch.py',
  '.local/lib/badi/compat/fcitx5-5.1.21/build-receipt.json',
  '.local/lib/badi/compat/fcitx5-5.1.21/addons/libwaylandim.so',
  '.config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf',
  '.config/fcitx5/conf/waylandim.conf', '.config/fcitx5/profile',
]
entries = []
for index, relative in enumerate(files):
    source = home / relative
    entries.append({'path': relative, 'existed': source.exists(), 'backup': str(index)})
    if source.exists():
        shutil.copy2(source, backup / str(index))
(backup / 'files.json').write_text(json.dumps(entries, indent=2) + '\n')
PY
install -Dm644 "$compat_build/libwaylandim.so" "$compat_root/addons/libwaylandim.so"
install -Dm644 "$compat_build/build-receipt.json" "$compat_root/build-receipt.json"
install -Dm644 packaging/fcitx5-wayland-compat/launch.py "$compat_root/launch.py"
mkdir -p "$HOME/.config/systemd/user/omarchy-fcitx5.service.d"
cat > "$HOME/.config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf" <<'UNIT'
[Service]
ExecStart=
ExecStart=/usr/bin/python3 -B %h/.local/lib/badi/compat/fcitx5-5.1.21/launch.py --disable notificationitem
UNIT
```

Set the one top-level option while retaining other settings and sections:

```sh
python3 - <<'PY'
from pathlib import Path
p = Path.home() / '.config/fcitx5/conf/waylandim.conf'
p.parent.mkdir(parents=True, exist_ok=True)
lines = p.read_text().splitlines() if p.exists() else []
kept, top_level = [], True
for line in lines:
    if line.lstrip().startswith('['):
        top_level = False
    if top_level and line.split('=', 1)[0].strip() == 'IdleDoneAcknowledgement':
        continue
    kept.append(line)
p.write_text('IdleDoneAcknowledgement=True\n' + '\n'.join(kept) + '\n')
PY
systemctl --user daemon-reload
if [ "$(cat "$compat_backup/active-before.txt")" = active ]; then
  compat_fcitx_service start
fi
systemctl --user is-enabled omarchy-fcitx5.service
```

The enabled state must still match the saved state; this recipe never enables or
disables the service. Verify the running process actually loaded the intended
module, instead of inferring selection from a successful service start. Reuse
the installer's check of exact file inode/device and a stable service PID:

```sh
python3 - "$compat_root/addons/libwaylandim.so" <<'PY'
from pathlib import Path
import importlib.util, sys
spec = importlib.util.spec_from_file_location('desktop_installer', 'scripts/install-desktop.py')
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)
assert installer.native_addon_loaded(Path(sys.argv[1])), 'Current service does not map the exact candidate file'
print('Verified candidate frontend in the current service process')
PY
```

Then run the disposable physical Chromium/Brave v3 trial: automatic surrounding
text, visible prediction, explicit acceptance, native undo, stale focus, and real
IME composition. Report this separately from the synthetic protocol proof.

## Rollback

Stop `omarchy-fcitx5.service` before restoring any files. Compare the saved file
map against current files and preserve edits made after the trial, especially
the profile and other Fcitx configuration options. Restore an existing original
file from its numbered backup; remove only a target recorded as originally
absent. For an unchanged trial installation, the exact restoration is:

```sh
compat_fcitx_service stop
python3 - "$compat_backup" <<'PY'
import json, pathlib, shutil, sys
home, backup = pathlib.Path.home(), pathlib.Path(sys.argv[1])
for entry in json.loads((backup / 'files.json').read_text()):
    target = home / entry['path']
    if entry['existed']:
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(backup / entry['backup'], target)
    else:
        target.unlink(missing_ok=True)
PY
systemctl --user daemon-reload
if [ "$(cat "$compat_backup/active-before.txt")" = active ]; then
  compat_fcitx_service start
fi
systemctl --user is-enabled omarchy-fcitx5.service
```

Verify the saved enabled state and `/proc/<MainPID>/maps` again; a fresh
installation rollback should load `/usr/lib/fcitx5/libwaylandim.so`. Retain the
backup. Do not remove `50-badi.conf`, the Badi addon, the broker, the accessibility
helper, or unrelated user configuration as part of this frontend-only rollback.
