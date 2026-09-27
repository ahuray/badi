#!/usr/bin/env python3
"""Update the installed Badi panel without disturbing a locked Omarchy shell."""

import argparse
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import time

import badi_install

ROOT = Path(__file__).resolve().parents[1]
FILES = ("manifest.json", "BarWidget.qml", "BadiMark.qml", "DesktopPanel.qml", "BadiClient.qml")
# Files earlier versions installed. An update backs them up and removes them so
# the plugin directory holds only what the current manifest loads.
OBSOLETE = ("Panel.qml",)


def ipc(target, method):
    result = subprocess.run(["omarchy-shell", target, method],
                            capture_output=True, text=True, check=True, timeout=4)
    return json.loads(result.stdout)


def unlocked():
    # Missing or unavailable lock state is not permission to restart the shell.
    return badi_install.explicitly_unlocked(ipc("lock", "status"))


def panel_ready():
    try:
        state = ipc("badi-writing", "state")
    except (ValueError, subprocess.SubprocessError):
        return False
    return isinstance(state, dict) and "page" in state and "service" in state


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wait-for-unlock", type=int, default=0, metavar="SECONDS")
    args = parser.parse_args()
    if not 0 <= args.wait_for_unlock <= 86400:
        parser.error("wait must be between 0 and 86400 seconds")
    target = Path.home() / ".config/omarchy/plugins/io.github.ahuray.badi"
    if target.is_symlink() or target.stat().st_uid != os.getuid():
        raise RuntimeError("Inspect the installed Badi plugin directory before replacing it")
    if json.loads((target / "manifest.json").read_text())["id"] != "io.github.ahuray.badi":
        raise RuntimeError("The installed plugin identity does not match Badi")
    subprocess.run(["bash", "ui/omarchy-plugin/tests/check-source.sh"], cwd=ROOT,
                   env={**os.environ, "BADI_OMARCHY_REQUIRE_HOST_CHECKS": "1"}, check=True)
    # Stage the validated bytes: a checkout edited while waiting is not installed.
    staged = {name: ((ROOT / "ui/omarchy-plugin" / name).read_bytes(),
                     stat.S_IMODE((ROOT / "ui/omarchy-plugin" / name).stat().st_mode)) for name in FILES}
    if not unlocked():
        if args.wait_for_unlock:
            print("Validated UI staged. Waiting for normal desktop unlock; the lock client remains intact.", flush=True)
        if not badi_install.poll(unlocked, args.wait_for_unlock, 2):
            raise RuntimeError("Desktop locked: no UI files changed. Unlock and rerun this command.")
    backup = Path.home() / ".local/state/badi/ui-backups" / time.strftime("%Y%m%d-%H%M%S")
    backup.mkdir(parents=True, mode=0o700)
    for name, (data, mode) in staged.items():
        destination = target / name
        if destination.exists() or destination.is_symlink():
            shutil.copy2(destination, backup / name, follow_symlinks=False)
        # Atomic replacement avoids the shell reading a partially written QML file.
        badi_install.atomic_write(destination, data, mode)
    for name in OBSOLETE:
        stale = target / name
        if stale.exists() or stale.is_symlink():
            shutil.copy2(stale, backup / name, follow_symlinks=False)
            stale.unlink()
    print(f"Plugin backup: {backup}", flush=True)
    # The supported restart performs its own lock check. It also clears Qt's
    # cached nested components, which a plugin rescan alone may retain.
    subprocess.run(["omarchy", "restart", "shell"], check=True, timeout=30)
    if not badi_install.poll(panel_ready, 20, .25):
        raise RuntimeError("Updated panel did not answer IPC; inspect the Omarchy shell log")
    print("Badi writing, application and system controls loaded in the Omarchy bar.", flush=True)


if __name__ == "__main__":
    main()
