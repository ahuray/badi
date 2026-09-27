#!/usr/bin/env python3
"""Select a verified private frontend or retain normal system Fcitx behavior."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

VERSION = "5.1.22"
COMMIT = "c7ecdb931d8b378ccdcd87382a3ef7ff0bd10def"
COMPOSITOR_VERSION = "0.56.2"
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
# At login the service can start before Hyprland answers IPC.
COMPOSITOR_WAIT_SECONDS = 4.0


class Fallback(Exception):
    """A fixed, content-free reason to execute the system frontend."""


class CompositorStarting(Fallback):
    """Hyprland did not answer yet; the check may succeed when repeated."""


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def hyprctl(query: str, environ: dict[str, str]):
    try:
        raw = subprocess.check_output(["/usr/bin/hyprctl", "-j", query], env=environ,
                                      stderr=subprocess.DEVNULL, timeout=.5)
    except FileNotFoundError:
        raise Fallback("hyprctl missing") from None
    except (OSError, subprocess.SubprocessError):
        raise CompositorStarting(f"hyprctl {query} failed") from None
    if len(raw) > 8192:
        raise Fallback(f"hyprctl {query} reply too large")
    try:
        return json.loads(raw)
    except ValueError:
        raise CompositorStarting(f"hyprctl {query} reply unreadable") from None


def check_compositor(environ: dict[str, str]) -> None:
    """Raise unless this session's Hyprland instance and display are the inspected build."""
    signature = environ.get("HYPRLAND_INSTANCE_SIGNATURE")
    wayland_display = environ.get("WAYLAND_DISPLAY")
    if not signature or not wayland_display:
        raise Fallback("no Hyprland session")
    instances = hyprctl("instances", environ)
    if not isinstance(instances, list) or not all(isinstance(item, dict) for item in instances):
        raise Fallback("hyprctl instances reply unexpected")
    matching = [item for item in instances if item.get("instance") == signature]
    if not matching:
        raise CompositorStarting("Hyprland instance not listed")
    if len(matching) != 1 or matching[0].get("wl_socket") != wayland_display:
        raise Fallback("Hyprland instance serves another display")
    version = hyprctl("version", environ)
    if (not isinstance(version, dict) or version.get("version") != COMPOSITOR_VERSION or
            version.get("commit") != COMPOSITOR_COMMIT or version.get("dirty") is not False):
        raise Fallback("Hyprland build differs")


def wait_for_compositor(environ: dict[str, str], clock=time.monotonic, sleep=time.sleep) -> None:
    """check_compositor, repeated with backoff while Hyprland is still starting."""
    deadline = clock() + COMPOSITOR_WAIT_SECONDS
    delay = .05
    while True:
        try:
            return check_compositor(environ)
        except CompositorStarting:
            remaining = deadline - clock()
            if remaining <= 0:
                raise
            sleep(min(delay, remaining))
            delay = min(delay * 2, .5)


def verify_build(arguments: list[str], environ: dict[str, str], selected: dict[str, str], root: Path) -> None:
    """Raise unless the service command, runtime and private module match the build receipt."""
    if arguments != ["--disable", "notificationitem"]:
        raise Fallback("service arguments differ")
    if any(environ.get(key) for key in LOADER_OVERRIDES):
        raise Fallback("loader override set")
    try:
        receipt = json.loads((root / "build-receipt.json").read_text())
        if (receipt["schema"] != "badi.fcitx-wayland-compat-build.v1" or
                receipt["source"]["upstream_version"] != VERSION or
                receipt["source"]["upstream_commit"] != COMMIT or
                receipt["installed_build_versions"] != {name: VERSION for name in ("Fcitx5Core", "Fcitx5Config", "Fcitx5Utils")}):
            raise Fallback("build receipt differs")
        expected = receipt["runtime_files_sha256"]
        if set(expected) != set(RUNTIME_FILES):
            raise Fallback("build receipt differs")
        if any(digest(Path(path)) != expected[path] for path in RUNTIME_FILES):
            raise Fallback("Fcitx runtime changed since the build")
        # The rebuilt frontend shares pinned protocol-wrapper types with the
        # normal wayland addon. A separate user override must keep its own path.
        for directory in selected.get("FCITX_ADDON_DIRS", "/usr/lib/fcitx5").split(":"):
            candidate = Path(directory) / "libwayland.so"
            if directory and candidate.exists():
                if candidate.resolve() != Path("/usr/lib/fcitx5/libwayland.so").resolve():
                    raise Fallback("another wayland addon overrides the system one")
                break
        if digest(root / "addons/libwaylandim.so") != receipt["artifact_sha256"]:
            raise Fallback("private frontend module changed")
    except (OSError, ValueError, KeyError, TypeError):
        raise Fallback("build receipt or module unreadable") from None


def select_environment(arguments: list[str], environ: dict[str, str], root: Path) -> tuple[dict[str, str], str | None]:
    """The environment to execute, and why the private frontend was not selected (None if it was)."""
    selected = dict(environ)
    addon_directory = str(root / "addons")
    original = selected.get("FCITX_ADDON_DIRS")
    # A restarting process may inherit a prior selection. Remove only this
    # wrapper's exact private directory before deciding against today's runtime.
    if original is not None:
        selected["FCITX_ADDON_DIRS"] = ":".join(part for part in original.split(":") if part != addon_directory)
    try:
        verify_build(arguments, environ, selected, root)
        wait_for_compositor(selected)
    except Fallback as reason:
        return selected, str(reason)
    existing = selected.get("FCITX_ADDON_DIRS", "/usr/lib/fcitx5")
    selected["FCITX_ADDON_DIRS"] = addon_directory + ":" + existing
    return selected, None


def main() -> None:
    arguments = sys.argv[1:]
    environment, reason = select_environment(arguments, dict(os.environ), Path(__file__).resolve().parent)
    if reason is not None:
        print(f"Badi Fcitx compatibility override inactive ({reason}); launching the system frontend.", file=sys.stderr)
    os.execve(PROGRAM, [PROGRAM, *arguments], environment)


if __name__ == "__main__":
    main()
