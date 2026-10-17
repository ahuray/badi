#!/usr/bin/env python3
"""Update the installed Badi panel without disturbing a locked Omarchy shell."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess

import badi_install

ROOT = Path(__file__).resolve().parents[1]
FILES = ("manifest.json", "BarWidget.qml", "BadiMark.qml", "DesktopPanel.qml", "SettingsView.qml", "BadiClient.qml")
FCITX_THEME = ROOT / "ui/omarchy-plugin/fcitx-theme.py"
HOOK_EVENTS = ("theme-set", "font-set")


def fcitx_theme():
    spec = importlib.util.spec_from_file_location("badi_fcitx_theme", FCITX_THEME)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def install_fcitx_theme(installation, home, generator):
    """Fcitx's panel, where terminals show suggestions, in the Omarchy theme,
    kept current by Omarchy's theme-set and font-set hooks."""
    target = home / ".local/lib/badi/omarchy/fcitx-theme.py"
    installation.write(target, generator, 0o755)
    hook = (f"#!/bin/sh\n# Badi: dress Fcitx's panel in the new Omarchy theme or font.\n"
            f"exec python3 '{target}' \"$@\"\n").encode()
    hooks = badi_install.config_home(home) / "omarchy/hooks"
    for event in HOOK_EVENTS:
        installation.write(hooks / f"{event}.d/badi-fcitx-theme", hook, 0o755)
    try:
        written = fcitx_theme().files(home)
    except (OSError, KeyError, ValueError) as error:
        print(f"Fcitx panel theme skipped until the next Omarchy theme change: {error}", flush=True)
        return
    for path, text in written:
        installation.write(path, text.encode(), 0o644)


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


def plugin_directory(home):
    """The Omarchy plugin directory, honouring XDG_CONFIG_HOME."""
    return badi_install.config_home(home) / "omarchy/plugins/io.github.ahuray.badi"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wait-for-unlock", type=int, default=0, metavar="SECONDS")
    args = parser.parse_args()
    if not 0 <= args.wait_for_unlock <= 86400:
        parser.error("wait must be between 0 and 86400 seconds")
    home = Path.home()
    target = plugin_directory(home)
    fresh = not target.exists()
    if not fresh:
        if target.is_symlink() or not target.is_dir() or target.stat().st_uid != os.getuid():
            raise RuntimeError("Inspect the installed Badi plugin directory before replacing it")
        manifest = target / "manifest.json"
        if manifest.exists() and json.loads(manifest.read_text())["id"] != "io.github.ahuray.badi":
            raise RuntimeError("The installed plugin identity does not match Badi")
    subprocess.run(["bash", "ui/omarchy-plugin/tests/check-source.sh"], cwd=ROOT,
                   env={**os.environ, "BADI_OMARCHY_REQUIRE_HOST_CHECKS": "1"}, check=True)
    # Stage the validated bytes: a checkout edited while waiting is not installed.
    staged = {name: ((ROOT / "ui/omarchy-plugin" / name).read_bytes(),
                     stat.S_IMODE((ROOT / "ui/omarchy-plugin" / name).stat().st_mode)) for name in FILES}
    generator = FCITX_THEME.read_bytes()
    if not unlocked():
        if args.wait_for_unlock:
            print("Validated UI staged. Waiting for normal desktop unlock; the lock client remains intact.", flush=True)
        if not badi_install.poll(unlocked, args.wait_for_unlock, 2):
            raise RuntimeError("Desktop locked: no UI files changed. Unlock and rerun this command.")
    installation = badi_install.Installation(home, "omarchy-ui")
    target.mkdir(parents=True, exist_ok=True)
    for name, (data, mode) in staged.items():
        # Atomic replacement avoids the shell reading a partially written QML
        # file; a development link to a checkout file is replaced (and saved).
        installation.write(target / name, data, mode, replace_link=True)
    for child in target.iterdir():
        if child.name not in FILES and child.is_file() and not child.is_symlink():
            installation.remove(child)
    install_fcitx_theme(installation, home, generator)
    installation.finish()
    fcitx_theme().reload()
    if fresh:
        subprocess.run(["omarchy", "plugin", "enable", "io.github.ahuray.badi"], check=True, timeout=30)
    badi_install.write_receipt(home, "omarchy-ui", badi_install.source_identity(ROOT), installation.current)
    # The supported restart performs its own lock check. It also clears Qt's
    # cached nested components, which a plugin rescan alone may retain.
    subprocess.run(["omarchy", "restart", "shell"], check=True, timeout=30)
    if not badi_install.poll(panel_ready, 20, .25):
        raise RuntimeError("Updated panel did not answer IPC; inspect the Omarchy shell log")
    print("Badi writing, application and system controls loaded in the Omarchy bar.", flush=True)


if __name__ == "__main__":
    main()
