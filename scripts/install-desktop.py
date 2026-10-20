#!/usr/bin/env python3
"""Install the Badi broker and cooperative addon into this Omarchy user session."""

import argparse
from dataclasses import dataclass
import hashlib
import importlib.util
import json
import mmap
import os
from pathlib import Path
import re
import socket
import stat
import struct
import subprocess
import sys

import badi_install
from badi_install import (ACCESSIBILITY_SWITCH, COMPLETE_ACCESSIBILITY, HYPR_REQUIRE,  # noqa: F401
                          OBSERVED_APP_FLAGS, OBSERVED_FLAG, chromium_line_tokens, hyprland_with_badi,
                          observed_flags, observed_tokens)

ROOT = Path(__file__).resolve().parents[1]
# Every module the installed observer imports, beside its daemon.
OBSERVER_MODULES = ("daemon.py", "contract.py", "desktop.py", "field.py", "geometry.py", "preview.py", "health.py",
                    "debugtrace.py")
SESSION_MODULES = ("daemon.py", "link.py", "style_key.py")
SESSION_SERVICE = "badi-session.service"
OMARCHY_REPAIR_HOOKS = ("post-boot.d", "post-update.d")
# The retired Fcitx 5.1.22 Wayland frontend backport. Fcitx 5.1.23 ships the
# upstream fix, so a complete install removes exactly what earlier installs
# left here, backed up like any replaced file.
RETIRED_COMPAT_LIBRARY = Path(".local/lib/badi/compat")
RETIRED_COMPAT_FILES = ("fcitx5-5.1.22/launch.py", "fcitx5-5.1.22/build-receipt.json",
                        "fcitx5-5.1.22/addons/libwaylandim.so")
RETIRED_COMPAT_DIRECTORIES = ("fcitx5-5.1.22/addons", "fcitx5-5.1.22", "")
RETIRED_COMPAT_DROPIN = Path(".config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf")
RETIRED_COMPAT_DROPIN_TEXT = (
    "[Service]\n# Badi: pinned Fcitx Wayland frontend; launch.py falls back to /usr/bin/fcitx5.\n"
    f"ExecStart=\nExecStart=/usr/bin/python3 -B %h/{RETIRED_COMPAT_LIBRARY}/fcitx5-5.1.22/launch.py --disable notificationitem\n")


def run(command, **kwargs):
    return subprocess.run(command, check=True, **kwargs)


def observed_config_target(home, config_home, filename):
    home = home.resolve()
    if not config_home.is_absolute():
        raise RuntimeError("Observed app startup configuration needs an absolute configuration directory.")
    if ".." in config_home.parts:
        raise RuntimeError("Inspect ambiguous parent traversal in the app startup configuration directory before changing it.")
    normalized = Path(os.path.normpath(config_home))
    resolved = config_home.resolve()
    if not resolved.is_relative_to(home):
        raise RuntimeError("Observed app startup configuration must be inside the user home for recoverable installation.")
    if normalized != resolved:
        raise RuntimeError("Inspect the app startup configuration directory symlink before changing it.")
    target = resolved / filename
    if target.is_symlink():
        raise RuntimeError("Inspect the app startup configuration symlink before changing it.")
    return target


def enable_accessibility(installation):
    def status():
        value = run(["busctl", "--user", "--timeout=2s", "get-property", "org.a11y.Bus",
                     "/org/a11y/bus", "org.a11y.Status", "IsEnabled"],
                    capture_output=True, text=True, timeout=3).stdout.strip()
        if value not in ("b true", "b false"):
            raise RuntimeError("Cannot determine the prior accessibility state; it was not changed.")
        return value == "b true"
    previous = status()
    # IsEnabled can persist the GNOME toolkit setting. Record both values so
    # rollback does not mistake this for a process-local environment change.
    toolkit = run(["gsettings", "get", "org.gnome.desktop.interface", "toolkit-accessibility"],
                  capture_output=True, text=True, timeout=3).stdout.strip()
    if toolkit not in ("true", "false"):
        raise RuntimeError("Cannot determine the prior toolkit accessibility state; it was not changed.")
    receipt = installation.note("accessibility-setting.json",
                                {"bus_enabled": previous, "toolkit_accessibility": toolkit == "true"})
    print(f"Prior accessibility settings recorded in {receipt}", flush=True)
    if not previous:
        require_unlocked()
        run(["busctl", "--user", "--timeout=2s", "set-property", "org.a11y.Bus", "/org/a11y/bus",
             "org.a11y.Status", "IsEnabled", "b", "true"], capture_output=True, timeout=3)
    if not status():
        raise RuntimeError("Desktop accessibility did not enable; Fcitx was not restarted.")


def enable_screen_reader_accessibility(installation):
    """Opt into desktop-wide AT-SPI activation, recording its persistent state."""
    bus = ["busctl", "--user", "--timeout=2s"]
    address = ["org.a11y.Bus", "/org/a11y/bus", "org.a11y.Status", "ScreenReaderEnabled"]
    setting = ["org.gnome.desktop.a11y.applications", "screen-reader-enabled"]

    def state():
        active = run([*bus, "get-property", *address], capture_output=True, text=True, timeout=3).stdout.strip()
        persistent = run(["gsettings", "get", *setting], capture_output=True, text=True, timeout=3).stdout.strip()
        if active not in ("b true", "b false") or persistent not in ("true", "false"):
            raise RuntimeError("Cannot determine the prior screen-reader accessibility state; it was not changed.")
        return {"screen_reader_enabled": active == "b true", "screen_reader_setting": persistent == "true"}

    previous = state()
    if all(previous.values()):
        return  # An unchanged install must not overwrite the original rollback values.
    require_unlocked()
    receipt = installation.note("accessibility-screen-reader.json", previous)
    print(f"Prior desktop-wide accessibility settings recorded in {receipt}", flush=True)
    run(["gsettings", "set", *setting, "true"], capture_output=True, timeout=3)
    run([*bus, "set-property", *address, "b", "true"], capture_output=True, timeout=3)
    if not all(state().values()):
        raise RuntimeError("Desktop-wide accessibility did not enable; inspect the recorded prior settings before retrying.")


def require_unlocked():
    if not badi_install.explicitly_unlocked(badi_install.lock_state()):
        raise RuntimeError("Unlock the desktop before updating the native input addon. --broker-only can update the model and controls while locked.")


def session_runtime():
    result = run(["loginctl", "show-user", str(os.getuid()), "--property=RuntimePath", "--value"],
                 capture_output=True, text=True, timeout=4)
    runtime = Path(result.stdout.strip())
    supplied = Path(os.environ.get("XDG_RUNTIME_DIR", ""))
    if not runtime.is_absolute() or not supplied.is_absolute() or supplied.resolve() != runtime.resolve():
        raise RuntimeError("Run the installer in the graphical user's actual runtime, outside any isolated trial environment.")
    return runtime


def service_installed(unit="badi-broker.service"):
    return badi_install.unit_value(unit, "LoadState", timeout=4) != "not-found"


def probe_model(cli, endpoint):
    def service_pid():
        return int(badi_install.unit_value("badi-broker.service", "MainPID"))

    pid = service_pid()
    if pid <= 0:
        return None
    try:
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(1)
            connection.connect(str(endpoint))
            peer, uid, _ = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i")))
        if peer != pid or uid != os.getuid():
            raise RuntimeError("The desktop socket belongs to a different process than badi-broker.service; readiness was not verified.")
    except (FileNotFoundError, ConnectionRefusedError, TimeoutError):
        return None
    try:
        probe = subprocess.run([str(cli), "--socket", str(endpoint), "status"], capture_output=True, text=True, timeout=3)
    except subprocess.TimeoutExpired:
        return None
    if probe.returncode or service_pid() != pid:
        return None
    return json.loads(probe.stdout)


def wait_model(cli, endpoint):
    probe = badi_install.poll(lambda: probe_model(cli, endpoint), 60)
    if probe is None:
        raise RuntimeError("Model startup failed: inspect journalctl --user -u badi-broker.service")
    require_model_health(probe)
    return probe


def wait_session_helper(cli, endpoint):
    """Whether the broker hears from the session helper within a few seconds.
    A missing report is reported, not fatal: the model and adapters still work."""
    def observed():
        try:
            status = subprocess.run([str(cli), "--socket", str(endpoint), "status"], capture_output=True,
                                    text=True, timeout=3)
            desktop = json.loads(status.stdout).get("desktop") if status.returncode == 0 else None
        except (OSError, ValueError, subprocess.SubprocessError):
            return None
        return isinstance(desktop, dict) and desktop.get("observed") is True
    return bool(badi_install.poll(observed, 6))


def require_model_health(probe):
    if not isinstance(probe, dict) or probe.get("provider") != "local_model":
        raise RuntimeError("The installed broker is not using the local model")
    if probe.get("control_plane_degraded") is not False:
        raise RuntimeError("The model started, but persistent settings are degraded or unverified. Run badi doctor and restore valid private settings; the installer did not change existing policy.")
    if not isinstance(probe.get("paused"), bool):
        raise RuntimeError("The broker did not report a valid pause state. Inspect badi doctor before using predictions.")


def mapped_file_device(target, metadata):
    # Btrfs reports a subvolume device from stat, but the filesystem device in
    # proc maps. Compare two mappings of the same installed inode instead of
    # discarding the device check or requiring privileged map_files access.
    with target.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        if (opened.st_dev, opened.st_ino) != (metadata.st_dev, metadata.st_ino):
            return None
        with mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ):
            devices = set()
            for line in Path("/proc/self/maps").read_text().splitlines():
                parts = line.split(maxsplit=5)
                if len(parts) != 6 or parts[5] != str(target):
                    continue
                try:
                    if int(parts[4]) == metadata.st_ino:
                        devices.add(os.makedev(*(int(part, 16) for part in parts[3].split(":"))))
                except ValueError:
                    continue
            return devices.pop() if len(devices) == 1 else None


def native_addon_loaded(addon):
    """Check the current service process maps the exact newly installed file."""
    def state():
        return badi_install.unit_properties("omarchy-fcitx5.service", "ActiveState", "MainPID")

    initial = state()
    pid = initial.get("MainPID", "0")
    if initial.get("ActiveState") != "active" or not pid.isdigit() or int(pid) <= 0:
        return False
    target = addon.resolve(strict=True)
    metadata = target.stat()
    try:
        mappings = Path(f"/proc/{pid}/maps").read_text().splitlines()
    except OSError:
        return False
    mapped = False
    expected_device = metadata.st_dev
    for line in mappings:
        parts = line.split(maxsplit=5)
        if len(parts) != 6 or parts[5] != str(target):
            continue
        try:
            major, minor = (int(part, 16) for part in parts[3].split(":"))
            device = os.makedev(major, minor)
            if device != expected_device and int(parts[4]) == metadata.st_ino:
                expected_device = mapped_file_device(target, metadata)
            mapped = int(parts[4]) == metadata.st_ino and device == expected_device
        except ValueError:
            continue
        if mapped:
            break
    # A load log can precede a crash/restart; verify the process again after
    # reading maps instead of treating a historical message as current health.
    final = state()
    return mapped and final.get("ActiveState") == "active" and final.get("MainPID") == pid


def wait_native_addon(addon):
    if not badi_install.poll(lambda: native_addon_loaded(addon), 10):
        raise RuntimeError("The running Fcitx service did not load the installed Badi addon. Inspect badi doctor and the Fcitx user-service journal.")


# Chromium-based apps keep the text around the caret current only from Fcitx
# 5.1.23 on Hyprland (the adapter runbook's tested-with section).
MIN_FCITX_VERSION = (5, 1, 23)


def fcitx_version(output):
    """The (major, minor, patch) that `fcitx5 --version` printed, or None."""
    match = re.fullmatch(r"\s*(\d+)\.(\d+)\.(\d+)\S*\s*", output or "")
    return tuple(int(part) for part in match.groups()) if match else None


def require_fcitx_version():
    try:
        result = run(["fcitx5", "--version"], capture_output=True, text=True, timeout=5)
        version = fcitx_version(result.stdout)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError):
        version = None
    needed = ".".join(map(str, MIN_FCITX_VERSION))
    if version is None:
        raise RuntimeError(f"Could not read the Fcitx version with fcitx5 --version; Badi needs Fcitx {needed} or later.")
    if version < MIN_FCITX_VERSION:
        raise RuntimeError(f"Badi needs Fcitx {needed} or later; this desktop has {'.'.join(map(str, version))}. "
                           "Update the system (omarchy-update) before installing.")


def require_accessibility_runtime():
    if not os.environ.get("HYPRLAND_INSTANCE_SIGNATURE") or "/" in os.environ["HYPRLAND_INSTANCE_SIGNATURE"]:
        raise RuntimeError("Run the native installer in the current Hyprland session. --broker-only does not require the accessibility helper.")
    # Use the service's exact interpreter and actually load native libraries;
    # require_version alone only checks that typelib metadata can be located.
    code = """import ctypes, gi, cairo
ctypes.CDLL('libgtk4-layer-shell.so.0')
gi.require_version('Atspi', '2.0')
gi.require_version('Gtk', '4.0')
gi.require_version('Gtk4LayerShell', '1.0')
from gi.repository import Atspi, Gtk, Gtk4LayerShell, GLibUnix
"""
    try:
        run(["/usr/bin/python3", "-B", "-c", code], capture_output=True, text=True, timeout=4)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError):
        raise RuntimeError("The accessibility helper needs system Python with PyGObject, pycairo, AT-SPI, GTK4 and gtk4-layer-shell. Install the missing runtime before updating native integration.") from None


def accessibility_health_module():
    spec = importlib.util.spec_from_file_location("badi_accessibility_health", ROOT / "adapters/accessibility/health.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def probe_accessibility(helper, endpoint):
    """Verify the exact service process and private metadata-only endpoint."""
    def state():
        return badi_install.unit_properties("badi-accessibility.service", "ActiveState", "MainPID")

    initial = state()
    pid = initial.get("MainPID", "0")
    if initial.get("ActiveState") != "active" or not pid.isdigit() or int(pid) <= 0:
        return None
    process = Path("/proc") / pid
    try:
        with (process / "cmdline").open("rb") as stream:
            argv = stream.read(4097).split(b"\0")
        valid = (process.stat().st_uid == os.getuid()
                 and (process / "exe").resolve(strict=True) == Path("/usr/bin/python3").resolve(strict=True)
                 and argv == [b"/usr/bin/python3", b"-B", os.fsencode(helper), b""])
    except OSError:
        return None
    if not valid:
        raise RuntimeError("badi-accessibility.service is not running the installed helper with its expected interpreter. Inspect service overrides before using native predictions.")
    health = accessibility_health_module()
    try:
        status = health.probe(endpoint, int(pid))
    except health.ProbeError as error:
        if str(error) in ("service_unavailable", "timeout", "transport_unavailable"):
            return None
        raise RuntimeError(f"Accessibility readiness was not verified ({error}). Inspect badi doctor and the helper user-service journal.") from None
    final = state()
    return status if final.get("ActiveState") == "active" and final.get("MainPID") == pid else None


def wait_accessibility(helper, endpoint):
    if badi_install.poll(lambda: probe_accessibility(helper, endpoint), 10) is None:
        raise RuntimeError("The accessibility helper did not become ready. Inspect badi doctor and journalctl --user -u badi-accessibility.service; Fcitx was not restarted.")


def fcitx_command():
    """The effective argv of the Fcitx service's single ExecStart, verbatim."""
    value = badi_install.unit_value("omarchy-fcitx5.service", "ExecStart")
    commands = re.findall(r"argv\[\]=(.*?) ; ignore_errors=", value)
    return commands[0] if len(commands) == 1 else None


def retired_compat_files(home):
    """The retired frontend's files an earlier install left, to remove now.

    Only Badi's exact files qualify. Anything else there stops the install
    before any change, so a file Badi did not write is never deleted.
    """
    library = home / RETIRED_COMPAT_LIBRARY
    for path in (library, *library.parents):
        if path == home:
            break
        if path.is_symlink():
            raise RuntimeError(f"Inspect the symlink {path} before installing; Badi no longer installs a Fcitx frontend there.")
    found = []
    if library.exists():
        files = {library / name for name in RETIRED_COMPAT_FILES}
        directories = {library / name for name in RETIRED_COMPAT_DIRECTORIES}
        for path in sorted(library.rglob("*")):
            if path.is_symlink() or not ((path in files and path.is_file()) or (path in directories and path.is_dir())):
                raise RuntimeError(f"Inspect unexpected files in {library} before installing; "
                                   "Badi no longer installs a Fcitx frontend there.")
            if path.is_file():
                found.append(path)
    dropin = home / RETIRED_COMPAT_DROPIN
    if os.path.lexists(dropin):
        if dropin.is_symlink() or not dropin.is_file() or dropin.read_text() != RETIRED_COMPAT_DROPIN_TEXT:
            raise RuntimeError(f"Inspect {dropin} before installing: it is not the drop-in an earlier Badi install wrote.")
        found.append(dropin)
    return found


@dataclass
class Plan:
    """What one installer run will change, decided before anything is built."""
    args: argparse.Namespace
    home: Path
    config_home: Path
    runtime: Path
    # Observed-app flags files to change: target -> (original text or None, new text).
    flag_updates: dict
    flags_present: list
    broker_updating: bool
    # True only when the user unit is enabled. A unit file left behind by an
    # interrupted first install is loaded but not enabled, and must be enabled.
    accessibility_enabled: bool
    # Files the retired Fcitx frontend backport left, removed by this run.
    retired_compat: list
    checkout: dict | None = None
    # hyprland.lua change that loads Badi's layer rule: (target, original, new).
    hypr_update: tuple | None = None
    # The user disabled the session helper; it is updated but stays disabled.
    session_disabled: bool = False
    session_ready: bool | None = None

    @property
    def native(self):
        return not self.args.broker_only


def parse_arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--broker-only", action="store_true",
                        help="Update only the broker, commands and service; leave the native input method running")
    parser.add_argument("--observed-app", action="append", choices=tuple(OBSERVED_APP_FLAGS), default=[],
                        help=f"Add {OBSERVED_FLAG} to this app's user flags file for its next launch (backed up)")
    parser.add_argument("--screen-reader-accessibility", action="store_true",
                        help="Enable desktop-wide AT-SPI ScreenReaderEnabled, including Discord; record prior settings for rollback")
    args = parser.parse_args()
    if args.broker_only and (args.observed_app or args.screen_reader_accessibility):
        parser.error("Accessibility options require the complete unlocked native installation")
    return args


def preflight(args):
    """Check the session and decide every change before building or touching a file."""
    home = Path.home().resolve()
    if not os.environ.get("WAYLAND_DISPLAY") or not os.environ.get("XDG_RUNTIME_DIR"):
        raise RuntimeError("Run the installer from your graphical session")
    if not args.broker_only:
        require_unlocked()
        require_accessibility_runtime()
        require_fcitx_version()
    config_home = badi_install.config_home(home)
    flag_updates, flags_present = {}, []
    for app in dict.fromkeys(args.observed_app):
        target = observed_config_target(home, config_home, OBSERVED_APP_FLAGS[app][0])
        original = target.read_text() if target.exists() else None
        text = observed_flags(original or "", app)
        if text is None:
            flags_present.append(app)
        else:
            flag_updates[target] = (original, text)
    runtime = session_runtime()
    broker_updating = service_installed()
    accessibility_enabled, retired_compat, hypr_update = False, [], None
    if not args.broker_only:
        accessibility_enabled = unit_enabled("badi-accessibility.service")
        run(["systemctl", "--user", "is-active", "omarchy-fcitx5.service"], capture_output=True)
        retired_compat = retired_compat_files(home)
        hyprland = config_home / "hypr/hyprland.lua"
        if hyprland.exists() and not hyprland.is_symlink():
            original = hyprland.read_text()
            updated = hyprland_with_badi(original)
            if updated is not None:
                hypr_update = (hyprland, original, updated)
    session_disabled = badi_install.unit_value(SESSION_SERVICE, "UnitFileState", timeout=4) == "disabled"
    return Plan(args, home, config_home, runtime, flag_updates, flags_present,
                broker_updating, accessibility_enabled, retired_compat, hypr_update=hypr_update,
                session_disabled=session_disabled)


def unit_enabled(unit):
    """Whether systemd will start this user unit. LoadState is not enough:
    an interrupted install can leave the unit file loaded and still disabled."""
    return badi_install.unit_value(unit, "UnitFileState", timeout=4) == "enabled"


# Pinned bytes from semantic/pinned_runtime.rs. The installer verifies them
# and does not download; the broker has no network code of its own. Writing
# models come from the broker's own catalog (badictl models writing).
WRITING_ASSETS = (
    ("runtimes/llama-b10726/llama-server",
     "4c20c6b55baa75eafeb02c17f118ce93314ba69aef89a9b4156284d58dcbc0c8",
     17_896),
    ("runtimes/downloads/llama-b10726-bin-ubuntu-x64.tar.gz",
     "d3c4e406b2911c8c75d2d0858459645960f8f592c1ab372d565cf145b870c901",
     16_702_536),
)


def require_writing_assets(home):
    """Refuse to build until the pinned llama.cpp runtime matches.

    The broker reads runtimes/llama-b10726 and checks these hashes again at
    startup. A size mismatch is rejected before the file is read.
    """
    data = Path(home) / ".local/share/badi"
    problems = []
    for relative, digest, size in WRITING_ASSETS:
        path = data / relative
        if not path.is_file():
            problems.append(f"{path} is missing")
            continue
        if path.stat().st_size != size or badi_install.file_sha256(path) != digest:
            problems.append(f"{path} does not match the pinned hash")
    if problems:
        raise RuntimeError(
            "The pinned llama.cpp b10726 runtime is not ready. "
            "The desktop installer checks the pinned hashes before building. " + " ".join(problems)
        )


def require_writing_model(home, badictl):
    """At least one model of the freshly built broker's writing catalog is
    installed with its pinned size and hash; the broker picks among them."""
    advice = json.loads(subprocess.run([str(badictl), "models", "writing", "--json"], capture_output=True,
                                       text=True, check=True, timeout=15).stdout)
    models = Path(home) / ".local/share/badi/models"
    for model in advice["catalog"]:
        path = models / model["filename"]
        if (path.is_file() and path.stat().st_size == model["download_bytes"]
                and badi_install.file_sha256(path) == model["sha256"]):
            return model
    recommended = (advice.get("recommended") or {}).get("name", "a catalog model")
    raise RuntimeError(f"No pinned writing model is installed in {models}. `{badictl} models writing` prints "
                       f"the pinned download of {recommended}, the one recommended for this device; "
                       "rerun the installer after it. Later, `badi model use ID` switches models.")


def build(plan):
    require_writing_assets(plan.home)
    # Record the checkout state that is about to be built and copied.
    plan.checkout = badi_install.source_identity(ROOT)
    # Build only the shipped package: installs never compile the Prediction Lab.
    run(["cargo", "build", "--release", "--locked", "-p", "badi-broker", "--bins"], cwd=ROOT)
    require_writing_model(plan.home, ROOT / "target/release/badictl")
    if plan.native:
        run(["npm", "run", "fcitx5:check"], cwd=ROOT)
        require_unlocked()


def default_settings():
    """The first settings document: predictions allowed in the natively tested editors."""
    subjects = [{"identity": {"kind": "linux_app", "adapter": "fcitx", "app_id": app},
                 "permissions": badi_install.grant("allow")}
                for app in ("com.github.xournalpp.xournalpp", "omawrite")]
    return {"schema": "badi.settings.v2", "revision": 1, "paused": False, "subjects": subjects}


def xournal_launcher(home):
    """Xournal++'s launcher with GTK_IM_MODULE=fcitx on every Exec line, or None.

    Only Xournal++ needs its GTK toolkit path selected explicitly on Wayland;
    the upstream launcher's MIME types and action entries are preserved.
    """
    desktop = Path("/usr/share/applications/com.github.xournalpp.xournalpp.desktop")
    if not desktop.exists():
        return None
    target = home / ".local/share/applications" / desktop.name
    text = target.read_text() if target.exists() else desktop.read_text()
    return target, "\n".join("Exec=env GTK_IM_MODULE=fcitx " + line[5:]
                             if line.startswith("Exec=") and "GTK_IM_MODULE=fcitx" not in line else line
                             for line in text.splitlines()) + "\n"


def flags_still_match(plan):
    """Reject flag files that changed while the build ran, before any write."""
    for target, (original, _text) in plan.flag_updates.items():
        if target.parent.resolve() != target.parent or target.is_symlink() or (target.read_text() if target.exists() else None) != original:
            raise RuntimeError("App startup configuration changed during the build; newer user settings were preserved.")
    if plan.hypr_update is not None:
        target, original, _text = plan.hypr_update
        if target.is_symlink() or not target.exists() or target.read_text() != original:
            raise RuntimeError("Hyprland configuration changed during the build; newer user settings were preserved.")


def install_files(plan):
    """Install every file this run owns; returns the Installation with its backup and current files."""
    if plan.native:
        flags_still_match(plan)
    installation = badi_install.Installation(plan.home, "desktop")
    try:
        return install_prepared_files(plan, installation)
    except Exception:
        badi_install.announce_stopped(installation)
        raise


def install_prepared_files(plan, installation):
    home, native = plan.home, plan.native
    install = installation.copy

    library = home / ".local/lib/badi"
    for name in ("badi-broker", "badictl"):
        install(ROOT / "target/release" / name, library / name)
    # One copy of each command: the CLI imports its helper module from its own
    # directory, and the PATH names are links (the Omarchy panel runs badi-desktop).
    install(ROOT / "scripts/badi-desktop.py", library / "badi-desktop.py")
    install(ROOT / "scripts/badi_install.py", library / "badi_install.py")
    commands = {"badi": "../lib/badi/badi-desktop.py", "badi-desktop": "../lib/badi/badi-desktop.py",
                "badictl": "../lib/badi/badictl"}
    for name, destination in commands.items():
        target = home / ".local/bin" / name
        # A command link replaces an earlier copy, or the development link to
        # this checkout's script; any other symlink is the user's.
        installation.link(target, destination,
                          replace_link=target.is_symlink() and target.resolve() == ROOT / "scripts/badi-desktop.py")
    badi_install.forget(home, "desktop", [home / ".local/bin" / name for name in commands])
    install(ROOT / "packaging/io.github.ahuray.badi.desktop",
            home / ".local/share/applications/io.github.ahuray.badi.desktop")
    install(ROOT / "packaging/io.github.ahuray.badi.svg",
            home / ".local/share/icons/hicolor/scalable/apps/io.github.ahuray.badi.svg")
    # The writing binary embeds a modified English lexicon; ship its complete
    # notices on both update paths even though inference needs no external file.
    for name in ("LICENSE", "README.md"):
        install(ROOT / "broker/data/writing-lexicon" / name,
                home / ".local/share/badi/licenses/writing-lexicon" / name)
    if native:
        install(ROOT / "adapters/fcitx5/build/libbadi-fcitx5.so", home / ".local/lib/fcitx5/libbadi-fcitx5.so")
        install(ROOT / "adapters/fcitx5/build/badi.conf", home / ".local/share/fcitx5/addon/badi.conf")
        for name in OBSERVER_MODULES:
            install(ROOT / "adapters/accessibility" / name, library / "accessibility" / name)
        install(ROOT / "packaging/systemd/badi-accessibility.service",
                home / ".config/systemd/user/badi-accessibility.service")
        install(ROOT / "packaging/hypr/badi.lua", plan.config_home / "hypr/badi.lua")
        # Omarchy runs these at every login and after omarchy-update; its
        # refresh commands replace hyprland.lua and the browser flag files.
        # omarchy-hook reads ~/.config whatever XDG_CONFIG_HOME says.
        hooks = home / ".config/omarchy/hooks"
        if hooks.is_dir():
            for name in OMARCHY_REPAIR_HOOKS:
                install(ROOT / "packaging/omarchy/badi-repair.hook", hooks / name / "badi-repair")
        if plan.hypr_update is not None:
            target, original, text = plan.hypr_update
            if target.is_symlink() or not target.exists() or target.read_text() != original:
                raise RuntimeError("Hyprland configuration changed during the build; newer user settings were preserved.")
            installation.write(target, text.encode(), stat.S_IMODE(target.stat().st_mode))
        for target, (original, text) in plan.flag_updates.items():
            if target.parent.resolve() != target.parent or target.is_symlink() or (target.read_text() if target.exists() else None) != original:
                raise RuntimeError("App startup configuration changed during the build; newer user settings were preserved.")
            installation.write(target, text.encode(), stat.S_IMODE(target.stat().st_mode) if target.exists() else 0o600)
        # launch.py replaced itself with /usr/bin/fcitx5 when the service
        # started, so removing it is safe; the Fcitx restart below runs the
        # stock command.
        for path in plan.retired_compat:
            if os.path.lexists(path):
                installation.remove(path)
        badi_install.forget(home, "desktop", plan.retired_compat)
        for name in RETIRED_COMPAT_DIRECTORIES:
            directory = home / RETIRED_COMPAT_LIBRARY / name
            if directory.is_dir() and not directory.is_symlink() and not any(directory.iterdir()):
                directory.rmdir()
    install(ROOT / "packaging/systemd/badi-broker.service", home / ".config/systemd/user/badi-broker.service")
    # The session helper reports suspend, session and power state to the
    # broker and reads no text, so it updates with the broker on both paths.
    for name in SESSION_MODULES:
        install(ROOT / "adapters/session" / name, library / "session" / name)
    install(ROOT / "packaging/systemd" / SESSION_SERVICE, home / ".config/systemd/user" / SESSION_SERVICE)
    if native:
        install(ROOT / "packaging/systemd/fcitx-badi.conf",
                home / ".config/systemd/user/omarchy-fcitx5.service.d/50-badi.conf")
    settings = plan.config_home / "badi/settings.json"
    if not settings.exists():
        settings.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        with settings.open("x") as stream:
            os.chmod(settings, 0o600)
            json.dump(default_settings(), stream)
    launcher = xournal_launcher(home) if native else None
    if launcher:
        installation.write(launcher[0], launcher[1].encode(), 0o644)
    installation.finish()
    receipt = badi_install.write_receipt(home, "desktop", plan.checkout, installation.current)
    print(f"Install receipt: {receipt}", flush=True)
    return installation


def restart(plan, installation):
    """Apply the installed files to the running services; returns the model's health."""
    home, runtime = plan.home, plan.runtime
    units = home / ".config/systemd/user"
    run(["systemd-analyze", "--user", "verify", str(units / "badi-broker.service")])
    run(["systemd-analyze", "--user", "verify", str(units / SESSION_SERVICE)])
    if plan.native:
        run(["systemd-analyze", "--user", "verify", str(units / "badi-accessibility.service")])
    run(["systemctl", "--user", "daemon-reload"])
    if not plan.broker_updating:
        run(["systemctl", "--user", "enable", "badi-broker.service"])
    run(["systemctl", "--user", "reset-failed", "badi-broker.service"])
    run(["systemctl", "--user", "restart", "badi-broker.service"])
    probe = wait_model(home / ".local/lib/badi/badictl", runtime / "badi/broker.sock")
    if not plan.session_disabled:
        run(["systemctl", "--user", "enable", SESSION_SERVICE])
        run(["systemctl", "--user", "reset-failed", SESSION_SERVICE])
        run(["systemctl", "--user", "restart", SESSION_SERVICE])
        plan.session_ready = wait_session_helper(home / ".local/lib/badi/badictl", runtime / "badi/broker.sock")
    if not plan.native:
        return probe
    require_unlocked()
    if not plan.accessibility_enabled:
        run(["systemctl", "--user", "enable", "badi-accessibility.service"])
    run(["systemctl", "--user", "reset-failed", "badi-accessibility.service"])
    run(["systemctl", "--user", "restart", "badi-accessibility.service"])
    wait_accessibility(home / ".local/lib/badi/accessibility/daemon.py", runtime / "badi/accessibility.sock")
    if plan.args.observed_app or plan.args.screen_reader_accessibility:
        enable_accessibility(installation)
    if plan.args.screen_reader_accessibility:
        enable_screen_reader_accessibility(installation)
    if plan.retired_compat and str(RETIRED_COMPAT_LIBRARY) in (fcitx_command() or ""):
        raise RuntimeError("Another omarchy-fcitx5.service override still starts the retired Badi frontend launcher, so Fcitx was not restarted. "
                           "Inspect its drop-ins, or restore the newest backup in ~/.local/state/badi/install-backups.")
    require_unlocked()
    # A fresh Fcitx may not have written its profile yet; then only an
    # existing profile that changed counts as a rewrite.
    profile = home / ".config/fcitx5/profile"
    previous = hashlib.sha256(profile.read_bytes()).digest() if profile.exists() else None
    run(["systemctl", "--user", "restart", "omarchy-fcitx5.service"])
    wait_native_addon(home / ".local/lib/fcitx5/libbadi-fcitx5.so")
    if previous is not None and (not profile.exists() or hashlib.sha256(profile.read_bytes()).digest() != previous):
        raise RuntimeError("Fcitx rewrote the keyboard profile; inspect it before continuing")
    return probe


def report(plan, probe):
    if plan.session_disabled:
        print("The session helper is disabled (systemctl --user enable --now badi-session.service turns it on); "
              "suspend, session switches and battery modes are not reported to the broker.")
    elif plan.session_ready:
        print("Session helper reporting: sleep and session switches withdraw suggestions; "
              "battery behavior follows badi battery; shortcuts are offered through the desktop portal.")
    else:
        print("The session helper did not report to the broker. Inspect badi doctor and "
              "journalctl --user -u badi-session.service; predictions still work without it.")
    pause_note = " Predictions remain paused; use badi resume when wanted." if probe["paused"] else ""
    if not plan.native:
        print("Local model ready. Broker and controls updated; native input addon was not restarted." + pause_note)
        return
    print("Local model and accessibility helper ready. Desktop addon loaded by the normal Fcitx service. Keyboard profile preserved. Application accessibility and exact target policy determine coverage; use badi doctor to inspect setup." + pause_note)
    if plan.retired_compat:
        print("Removed the retired Fcitx 5.1.22 Wayland frontend backport and its service drop-in; "
              "Fcitx 5.1.23 and later include the upstream fix, and Fcitx now runs its own frontend.")
    if plan.args.observed_app:
        prepared = [app for app in dict.fromkeys(plan.args.observed_app) if app not in plan.flags_present]
        if prepared:
            print(f"{OBSERVED_FLAG} added for " + ", ".join(prepared) +
                  ". Relaunch the app to apply it; existing windows were not closed and app/site policy is unchanged."
                  " Complete renderer accessibility costs some browser CPU and memory on every page.")
        if plan.flags_present:
            print("Renderer accessibility was already enabled for " + ", ".join(plan.flags_present) + "; its flags file was not changed.")


def main():
    plan = preflight(parse_arguments())
    build(plan)
    installation = None
    try:
        installation = install_files(plan)
        report(plan, restart(plan, installation))
    except Exception:
        badi_install.announce_stopped(installation)
        raise


if __name__ == "__main__":
    main()
