#!/usr/bin/env python3
"""Select a verified private frontend or retain normal system Fcitx behavior."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

VERSION = "5.1.21"
COMMIT = "1319952f284eae17a36cba9e800843ca61a163c1"
COMPOSITOR_COMMIT = "efb50993780079460b0cbed1363e2166a2de1d9f"
PROGRAM = "/usr/bin/fcitx5"
RUNTIME_FILES = (
    PROGRAM,
    "/usr/lib/libFcitx5Core.so.7",
    "/usr/lib/libFcitx5Config.so.6",
    "/usr/lib/libFcitx5Utils.so.2",
    "/usr/lib/fcitx5/libwayland.so",
)
LOADER_OVERRIDES = ("LD_LIBRARY_PATH", "LD_PRELOAD", "LD_AUDIT")


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def compatible_compositor(environ: dict[str, str]) -> bool:
    signature = environ.get("HYPRLAND_INSTANCE_SIGNATURE")
    wayland_display = environ.get("WAYLAND_DISPLAY")
    if not signature or not wayland_display:
        return False
    try:
        replies = []
        for query in ("instances", "version"):
            raw = subprocess.check_output(["/usr/bin/hyprctl", "-j", query], env=environ,
                                          stderr=subprocess.DEVNULL, timeout=.5)
            if len(raw) > 8192:
                return False
            replies.append(json.loads(raw))
        instances, version = replies
        matching = [item for item in instances if item.get("instance") == signature]
        return (len(matching) == 1 and matching[0].get("wl_socket") == wayland_display and
                version.get("version") == "0.56.2" and version.get("commit") == COMPOSITOR_COMMIT and
                version.get("dirty") is False)
    except (OSError, ValueError, TypeError, AttributeError, subprocess.SubprocessError):
        return False


def select_environment(arguments: list[str], environ: dict[str, str], root: Path) -> tuple[dict[str, str], bool]:
    selected = dict(environ)
    addon_directory = str(root / "addons")
    original = selected.get("FCITX_ADDON_DIRS")
    # A restarting process may inherit a prior selection. Remove only this
    # wrapper's exact private directory before deciding against today's runtime.
    if original is not None:
        selected["FCITX_ADDON_DIRS"] = ":".join(part for part in original.split(":") if part != addon_directory)
    if arguments != ["--disable", "notificationitem"] or any(environ.get(key) for key in LOADER_OVERRIDES):
        return selected, False
    if not compatible_compositor(selected):
        return selected, False
    try:
        receipt = json.loads((root / "build-receipt.json").read_text())
        if (receipt["schema"] != "badi.fcitx-wayland-compat-build.v1" or
                receipt["source"]["upstream_version"] != VERSION or
                receipt["source"]["upstream_commit"] != COMMIT or
                receipt["installed_build_versions"] != {name: VERSION for name in ("Fcitx5Core", "Fcitx5Config", "Fcitx5Utils")}):
            return selected, False
        expected = receipt["runtime_files_sha256"]
        if set(expected) != set(RUNTIME_FILES):
            return selected, False
        if any(digest(Path(path)) != expected[path] for path in RUNTIME_FILES):
            return selected, False
        # The rebuilt frontend shares pinned protocol-wrapper types with the
        # normal wayland addon. A separate user override must keep its own path.
        for directory in selected.get("FCITX_ADDON_DIRS", "/usr/lib/fcitx5").split(":"):
            candidate = Path(directory) / "libwayland.so"
            if directory and candidate.exists():
                if candidate.resolve() != Path("/usr/lib/fcitx5/libwayland.so").resolve():
                    return selected, False
                break
        module = root / "addons/libwaylandim.so"
        if digest(module) != receipt["artifact_sha256"]:
            return selected, False
    except (OSError, ValueError, KeyError, TypeError):
        return selected, False
    existing = selected.get("FCITX_ADDON_DIRS", "/usr/lib/fcitx5")
    selected["FCITX_ADDON_DIRS"] = addon_directory + ":" + existing
    return selected, True


def main() -> None:
    arguments = sys.argv[1:]
    environment, enabled = select_environment(arguments, dict(os.environ), Path(__file__).resolve().parent)
    if not enabled:
        print("Badi Fcitx compatibility override inactive; launching the system frontend.", file=sys.stderr)
    os.execve(PROGRAM, [PROGRAM, *arguments], environment)


if __name__ == "__main__":
    main()
