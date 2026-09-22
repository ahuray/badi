#!/usr/bin/env python3
"""Update the installed Badi panel without disturbing a locked Omarchy shell."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
FILES = ("manifest.json", "BarWidget.qml", "BadiMark.qml", "DesktopPanel.qml", "BadiClient.qml", "Panel.qml")


def ipc(target, method):
    result = subprocess.run(["omarchy-shell", target, method],
                            capture_output=True, text=True, check=True, timeout=4)
    return json.loads(result.stdout)


def unlocked():
    state = ipc("lock", "status")
    # Missing or unavailable lock state is not permission to restart the shell.
    return all(state.get(key) is False for key in ("locked", "secure", "requested", "pending", "sessionLocked"))


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
    with tempfile.TemporaryDirectory(prefix="badi-ui-update-") as directory:
        stage = Path(directory)
        for name in FILES:
            shutil.copy2(ROOT / "ui/omarchy-plugin" / name, stage / name)
        deadline = time.monotonic() + args.wait_for_unlock
        waiting = False
        while not unlocked():
            if time.monotonic() >= deadline:
                raise RuntimeError("Desktop locked: no UI files changed. Unlock and rerun this command.")
            if not waiting:
                print("Validated UI staged. Waiting for normal desktop unlock; the lock client remains intact.", flush=True)
                waiting = True
            time.sleep(2)
        backup = Path.home() / ".local/state/badi/ui-backups" / time.strftime("%Y%m%d-%H%M%S")
        backup.mkdir(parents=True, mode=0o700)
        for name in FILES:
            destination = target / name
            if destination.exists() or destination.is_symlink():
                shutil.copy2(destination, backup / name, follow_symlinks=False)
            # Atomic replacement avoids the shell reading a partially written QML file.
            with tempfile.NamedTemporaryFile(dir=target, delete=False) as stream:
                temporary = Path(stream.name)
            try:
                shutil.copy2(stage / name, temporary)
                temporary.replace(destination)
            finally:
                temporary.unlink(missing_ok=True)
        print(f"Plugin backup: {backup}", flush=True)
        # The supported restart performs its own lock check. It also clears Qt's
        # cached nested components, which a plugin rescan alone may retain.
        subprocess.run(["omarchy", "restart", "shell"], check=True, timeout=30)
        deadline = time.monotonic() + 20
        while True:
            try:
                state = ipc("badi-writing", "state")
                if "page" in state and "service" in state:
                    print("Badi writing, application and system controls loaded in the Omarchy bar.", flush=True)
                    break
            except (ValueError, subprocess.SubprocessError):
                pass
            if time.monotonic() >= deadline:
                raise RuntimeError("Updated panel did not answer IPC; inspect the Omarchy shell log")
            time.sleep(.25)


if __name__ == "__main__":
    main()
