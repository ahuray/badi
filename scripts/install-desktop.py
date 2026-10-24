#!/usr/bin/env python3
"""Install the Badi broker and cooperative addon into this user session.

On Omarchy it also wires the Hyprland preview rule, Omarchy's hooks and
omarchy-fcitx5.service. Elsewhere (GNOME, KDE Plasma, another compositor) it
leaves those out, and Fcitx loads the addon at the next login.
"""

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
                          OBSERVED_APP_FLAGS, OBSERVED_FLAG, OBSERVED_LAUNCHERS, chromium_line_tokens,
                          hyprland_with_badi, observed_flags, observed_launcher, observed_tokens)

ROOT = Path(__file__).resolve().parents[1]
# Every module the installed observer imports, beside its daemon.
OBSERVER_MODULES = ("daemon.py", "contract.py", "desktop.py", "field.py", "geometry.py", "preview.py", "health.py",
                    "debugtrace.py")
SESSION_MODULES = ("daemon.py", "link.py", "style_key.py")
SESSION_SERVICE = "badi-session.service"
OMARCHY_REPAIR_HOOKS = ("post-boot.d", "post-update.d")
# Packaged launchers; a user's copy of the same name shadows one.
SYSTEM_APPLICATIONS = Path("/usr/share/applications")


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


def observed_launcher_update(home, app):
    """(target, original, text) for the user's copy of `app`'s launcher; text
    is None when every Exec line has the renderer switch already."""
    name = OBSERVED_LAUNCHERS[app]
    applications = home / ".local/share/applications"
    target = applications / name
    if applications.is_symlink() or target.is_symlink():
        raise RuntimeError(f"Inspect the {name} launcher symlink before changing it.")
    original = target.read_text() if target.exists() else None
    packaged = SYSTEM_APPLICATIONS / name
    if original is None and not packaged.is_file():
        raise RuntimeError(f"No {name} launcher was found in {SYSTEM_APPLICATIONS}; nothing was changed.")
    return target, original, observed_launcher(original if original is not None else packaged.read_text(), app)


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
    mapped = process_maps_file(pid, addon)
    # A load log can precede a crash/restart; verify the process again after
    # reading maps instead of treating a historical message as current health.
    final = state()
    return mapped and final.get("ActiveState") == "active" and final.get("MainPID") == pid


def process_maps_file(pid, path):
    """Whether process `pid` maps exactly this installed file: its inode on
    its filesystem, not a replaced earlier build. False when unreadable."""
    target = path.resolve(strict=True)
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
    return mapped


def wait_native_addon(addon):
    if not badi_install.poll(lambda: native_addon_loaded(addon), 10):
        raise RuntimeError("The running Fcitx service did not load the installed Badi addon. Inspect badi doctor and the Fcitx user-service journal.")


# Where the compositor offers input-method-v2, Fcitx serves apps through its
# wayland_v2 frontend, which keeps Chromium-based apps' text around the caret
# current only from Fcitx 5.1.23 (upstream PR #1690; the adapter runbook's
# tested-with section). GNOME and KDE Plasma do not offer it.
MIN_FCITX_VERSION = (5, 1, 23)
INPUT_METHOD_V2 = "zwp_input_method_manager_v2"


def fcitx_version(output):
    """The (major, minor, patch) that `fcitx5 --version` printed, or None."""
    match = re.fullmatch(r"\s*(\d+)\.(\d+)\.(\d+)\S*\s*", output or "")
    return tuple(int(part) for part in match.groups()) if match else None


def require_fcitx_version(wayland_v2=True, platform=badi_install.OMARCHY):
    """The installed Fcitx version; refused below 5.1.23 where its wayland_v2
    frontend serves apps (always on Omarchy, which is Hyprland)."""
    try:
        result = run(["fcitx5", "--version"], capture_output=True, text=True, timeout=5)
        version = fcitx_version(result.stdout)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError):
        version = None
    needed = ".".join(map(str, MIN_FCITX_VERSION))
    if version is None:
        floor = f"; Badi needs Fcitx {needed} or later" if wayland_v2 else ""
        raise RuntimeError(f"Could not read the Fcitx version with fcitx5 --version{floor}. Install Fcitx 5 first.")
    if wayland_v2 and version < MIN_FCITX_VERSION:
        update = ("Update the system (omarchy-update) before installing." if platform == badi_install.OMARCHY
                  else "Update Fcitx before installing; on a desktop without input-method-v2 (GNOME, KDE Plasma) "
                       "this requirement does not apply.")
        raise RuntimeError(f"Badi needs Fcitx {needed} or later; this desktop has {'.'.join(map(str, version))}. "
                           "This compositor offers input-method-v2, where Fcitx's wayland_v2 frontend before "
                           f"{needed} stops Chromium-based apps sending the text around the caret. " + update)
    return version


def wayland_globals(environment=None, timeout=2):
    """The interface names the compositor's Wayland registry advertises, or
    None when they cannot be read. A plain wire-protocol read: get_registry,
    then sync, collecting each global until the sync's done event."""
    environment = os.environ if environment is None else environment
    display = environment.get("WAYLAND_DISPLAY") or ""
    if not display:
        return None
    path = display if display.startswith("/") else os.path.join(environment.get("XDG_RUNTIME_DIR") or "", display)
    # wl_display (object 1): get_registry (opcode 1) as object 2, sync (opcode 0) as object 3.
    request = struct.pack("=IIIIII", 1, 12 << 16 | 1, 2, 1, 12 << 16 | 0, 3)
    names, buffer = set(), b""
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(timeout)
            connection.connect(path)
            connection.sendall(request)
            while len(buffer) < 1 << 20:
                chunk = connection.recv(65536)
                if not chunk:
                    return None
                buffer += chunk
                while len(buffer) >= 8:
                    sender, word = struct.unpack_from("=II", buffer)
                    size, opcode = word >> 16, word & 0xFFFF
                    if size < 8 or size % 4:
                        return None
                    if len(buffer) < size:
                        break
                    body, buffer = buffer[8:size], buffer[size:]
                    if sender == 2 and opcode == 0 and len(body) >= 8:  # wl_registry.global
                        length = struct.unpack_from("=I", body, 4)[0]
                        names.add(body[8:8 + max(0, length - 1)].decode("ascii", "replace"))
                    elif sender == 3 and opcode == 0:  # wl_callback.done
                        return names
                    elif sender == 1 and opcode == 0:  # wl_display.error
                        return None
    except (OSError, struct.error):
        return None
    return None


def offers_input_method_v2(platform):
    """Whether the 5.1.23 floor applies: on Omarchy always (Hyprland), and
    elsewhere when the compositor offers input-method-v2 or cannot be asked."""
    if platform == badi_install.OMARCHY:
        return True
    names = wayland_globals()
    return names is None or INPUT_METHOD_V2 in names


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
        hint = (" (Debian: python3-gi-cairo gir1.2-atspi-2.0 gir1.2-gtk-4.0 gir1.2-gtk4layershell-1.0; Ubuntu 24.04 "
                "has no gtk4-layer-shell package)" if badi_install.distribution_family() == "debian" else "")
        raise RuntimeError("The accessibility helper needs system Python with PyGObject, pycairo, AT-SPI, GTK4 and "
                           f"gtk4-layer-shell. Install the missing runtime{hint} before updating native integration.") from None


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
    checkout: dict | None = None
    # hyprland.lua change that loads Badi's layer rule: (target, original, new).
    hypr_update: tuple | None = None
    # The user disabled the session helper; it is updated but stays disabled.
    session_disabled: bool = False
    session_ready: bool | None = None
    # badi_install.OMARCHY or GENERIC. A generic desktop starts Fcitx its own
    # way, so the installer writes FCITX_ENVIRONMENT and never restarts it.
    platform: str = badi_install.OMARCHY
    # Generic only: the system Fcitx's own addon directory.
    system_addons: Path | None = None
    # Whether this is a Hyprland session, where the field observer runs.
    observer_session: bool = True
    # Generic only, after the install: the running Fcitx and whether it maps
    # the installed addon (None when no Fcitx runs or it cannot be inspected).
    fcitx_process: int | None = None
    addon_loaded: bool | None = None

    @property
    def native(self):
        return not self.args.broker_only

    @property
    def omarchy(self):
        return self.platform == badi_install.OMARCHY


def parse_arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--broker-only", action="store_true",
                        help="Update only the broker, commands and service; leave the native input method running")
    parser.add_argument("--observed-app", action="append", choices=(*OBSERVED_APP_FLAGS, *OBSERVED_LAUNCHERS),
                        default=[], help=f"Add {OBSERVED_FLAG} to this app's user flags file, or to Discord's "
                                         "launcher, for its next launch (backed up)")
    parser.add_argument("--model", metavar="ID",
                        help="When no writing model is installed, download this catalog model (pinned, hash-checked) "
                             "instead of the one recommended for this device")
    parser.add_argument("--accept-model-license", action="store_true",
                        help="Accept the terms that come with the downloaded model (Gemma, Llama) without asking")
    args = parser.parse_args()
    if args.broker_only and args.observed_app:
        parser.error("Accessibility options require the complete unlocked native installation")
    return args


def preflight(args):
    """Check the session and decide every change before building or touching a file."""
    home = Path.home().resolve()
    if not os.environ.get("WAYLAND_DISPLAY") or not os.environ.get("XDG_RUNTIME_DIR"):
        raise RuntimeError("Run the installer from your graphical session")
    platform = badi_install.desktop_platform()
    observer_session = bool(os.environ.get("HYPRLAND_INSTANCE_SIGNATURE"))
    system_addons = None
    if not args.broker_only and platform == badi_install.OMARCHY:
        require_unlocked()
        require_accessibility_runtime()
        require_fcitx_version()
    elif not args.broker_only:
        if args.observed_app:
            raise RuntimeError("--observed-app changes desktop-wide accessibility for the field observer and needs "
                               "Omarchy's lock status; on this desktop it is not available. Nothing was changed.")
        system_addons = badi_install.fcitx_addon_directory()
        if system_addons is None:
            raise RuntimeError("Could not find the system Fcitx's addon directory (fcitx5/ beside libFcitx5Core). "
                               "Install Fcitx 5 and its development files first (Debian, Ubuntu: fcitx5, "
                               "libfcitx5core-dev, fcitx5-modules-dev).")
        version = require_fcitx_version(offers_input_method_v2(platform), platform)
        if version < MIN_FCITX_VERSION:
            print(f"Fcitx {'.'.join(map(str, version))}: this compositor does not offer input-method-v2, so the "
                  f"Fcitx {'.'.join(map(str, MIN_FCITX_VERSION))} requirement of the wayland_v2 frontend does not "
                  "apply here.", flush=True)
        if observer_session:
            require_accessibility_runtime()
    config_home = badi_install.config_home(home)
    flag_updates, flags_present = {}, []
    for app in dict.fromkeys(args.observed_app):
        if app in OBSERVED_LAUNCHERS:
            target, original, text = observed_launcher_update(home, app)
        else:
            target = observed_config_target(home, config_home, OBSERVED_APP_FLAGS[app][0])
            original = target.read_text() if target.exists() else None
            text = observed_flags(original or "", app)
        if text is None:
            flags_present.append(app)
        else:
            flag_updates[target] = (original, text)
    runtime = session_runtime()
    broker_updating = service_installed()
    accessibility_enabled, hypr_update = False, None
    if not args.broker_only:
        accessibility_enabled = unit_enabled("badi-accessibility.service")
    if not args.broker_only and platform == badi_install.OMARCHY:
        run(["systemctl", "--user", "is-active", "omarchy-fcitx5.service"], capture_output=True)
        hyprland = config_home / "hypr/hyprland.lua"
        if hyprland.exists() and not hyprland.is_symlink():
            original = hyprland.read_text()
            updated = hyprland_with_badi(original)
            if updated is not None:
                hypr_update = (hyprland, original, updated)
    session_disabled = badi_install.unit_value(SESSION_SERVICE, "UnitFileState", timeout=4) == "disabled"
    return Plan(args, home, config_home, runtime, flag_updates, flags_present,
                broker_updating, accessibility_enabled, hypr_update=hypr_update,
                session_disabled=session_disabled, platform=platform, system_addons=system_addons,
                observer_session=observer_session or platform == badi_install.OMARCHY)


def unit_enabled(unit):
    """Whether systemd will start this user unit. LoadState is not enough:
    an interrupted install can leave the unit file loaded and still disabled."""
    return badi_install.unit_value(unit, "UnitFileState", timeout=4) == "enabled"


# Pinned bytes from semantic/pinned_runtime.rs. The installer verifies them
# and does not download; the broker has no network code of its own. Writing
# models come from the broker's own catalog (badictl models writing).
WRITING_ASSETS = (
    ("runtimes/llama-b11840/llama-server",
     "a929dd0ce172a69aef809b39c0bd8ce61356e3587c71aad75cf292f0d108b33f",
     17_864),
    ("runtimes/downloads/llama-b11840-bin-ubuntu-x64.tar.gz",
     "5f6e72e772a1264df279d6b7a848570fdc00e5d968b72e7a917d19e07da96400",
     22_423_573),
)


def require_writing_assets(home):
    """Refuse to build until the pinned llama.cpp runtime matches.

    The broker reads runtimes/llama-b11840 and checks these hashes again at
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
            "The pinned llama.cpp b11840 runtime is not ready. "
            "The desktop installer checks the pinned hashes before building. " + " ".join(problems)
        )


def writing_model_choices(advice):
    """The writing models that fit this device: the recommended one first,
    then the most useful in Badi's blind development screen, unscreened ones
    last."""
    recommended = (advice.get("recommended") or {}).get("id")
    fitting = [model for model in advice["catalog"] if model.get("fits", True)]
    return sorted(fitting, key=lambda model: (model["id"] != recommended, model.get("screen_useful") is None,
                                              -(model.get("screen_useful") or 0)))


def licence_name(model):
    """How a model's licence reads to a person: its terms' name, or the SPDX id."""
    terms = badi_install.MODEL_TERMS.get(model.get("license"))
    return terms[0].removeprefix("the ") if terms else model.get("license") or "unknown licence"


def ask_writing_model(choices, recommended):
    """The model the person picks on a terminal, or None when they stop. A
    model's terms are asked for only once it is picked; declining them
    returns to the list."""
    print("Badi needs a writing model. It is downloaded once from Hugging Face, pinned and hash-checked.")
    print("These fit this device (useful suggestions of 135 in Badi's blind development screen):", flush=True)
    for number, model in enumerate(choices, 1):
        score = (f"{model['screen_useful']} of 135 useful" if model.get("screen_useful") is not None
                 else "not screened")
        mark = "  recommended" if model["id"] == recommended else ""
        print(f"  {number}  {model['name']:13} {model['download_bytes'] / 1e9:4.2f} GB  {score:18} "
              f"{licence_name(model)}{mark}")
    for _ in range(5):
        answer = input(f"Download which one? [1-{len(choices)}, Enter for 1, q to stop] ").strip().lower()
        if answer in ("q", "n", "no"):
            return None
        index = 1 if answer == "" else int(answer) if answer.isdigit() else 0
        if not 1 <= index <= len(choices):
            continue
        model = choices[index - 1]
        terms = badi_install.model_terms(model)
        if terms is None or input(f"{model['name']} is provided under {terms}. Do you accept them? [y/N] "
                                  ).strip().lower() in ("y", "yes"):
            return model
    return None


def require_writing_model(home, badictl, model_id=None, accept_license=False):
    """At least one model of the freshly built broker's writing catalog is
    installed with its pinned size and hash; the broker picks among them.

    Without one, a model is downloaded, pinned and hash-checked: on a
    terminal the person picks one of the models that fit, and accepts its
    terms if it has any; otherwise only `model_id`, with --accept-model-license
    for a model with terms."""
    advice = json.loads(subprocess.run([str(badictl), "models", "writing", "--json"], capture_output=True,
                                       text=True, check=True, timeout=15).stdout)
    models = Path(home) / ".local/share/badi/models"
    for model in advice["catalog"]:
        path = models / model["filename"]
        if (path.is_file() and path.stat().st_size == model["download_bytes"]
                and badi_install.file_sha256(path) == model["sha256"]):
            return model
    choices = writing_model_choices(advice)
    recommended = (advice.get("recommended") or {}).get("id")
    if model_id is None:
        if not choices:
            raise RuntimeError(f"No writing model is installed in {models}, and none fits this device.")
        if not sys.stdin.isatty():
            best = choices[0]
            terms = badi_install.model_terms(best)
            others = ", ".join(f"{model['id']} ({licence_name(model)})" for model in choices[1:])
            raise RuntimeError(f"No writing model is installed in {models}. Badi recommends {best['name']} "
                               f"({best['download_bytes'] / 1e9:.2f} GB)" + (f", provided under {terms}" if terms else "")
                               + f". Rerun with --model {best['id']}" + (" --accept-model-license" if terms else "")
                               + " to download it" + (f", or with another that fits: {others}" if others else "") + ".")
        model = ask_writing_model(choices, recommended)
        if model is None:
            raise RuntimeError("No writing model was downloaded; nothing was installed.")
    else:
        model = next((entry for entry in choices if entry["id"] == model_id), None)
        if model is None:
            fitting = " ".join(entry["id"] for entry in choices)
            raise RuntimeError(f"No writing model is installed in {models}, and {model_id} cannot be "
                               f"downloaded for this device. Rerun with --model ID, one of: {fitting}")
        terms = badi_install.model_terms(model)
        if terms and not accept_license:
            if not sys.stdin.isatty():
                raise RuntimeError(f"{model['name']} is provided under {terms}. Rerun with --model {model['id']} "
                                   "--accept-model-license to accept them and download it.")
            if input(f"{model['name']} is provided under {terms}. Do you accept them? [y/N] "
                     ).strip().lower() not in ("y", "yes"):
                raise RuntimeError("No writing model was downloaded; nothing was installed.")
    if badi_install.model_terms(model):
        badi_install.record_model_terms(home, model)
    badi_install.download_model(model, models)
    return model


def build(plan):
    require_writing_assets(plan.home)
    # Record the checkout state that is about to be built and copied.
    plan.checkout = badi_install.source_identity(ROOT)
    # Build only the shipped package: installs never compile the Prediction Lab.
    run(["cargo", "build", "--release", "--locked", "-p", "badi-broker", "--bins"], cwd=ROOT)
    require_writing_model(plan.home, ROOT / "target/release/badictl", plan.args.model, plan.args.accept_model_license)
    if plan.native:
        build_addon()
        if plan.omarchy:
            require_unlocked()


def build_addon():
    """Build the Fcitx addon against this system's Fcitx and run its tests,
    as `npm run fcitx5:check` does, without needing Node.

    Warnings stay errors in CI and in the check, which set BADI_WERROR=ON
    explicitly on the same build directory; an install, like the packages,
    keeps building when a newer compiler adds a warning."""
    build = ROOT / "adapters/fcitx5/build"
    run(["cmake", "-S", str(ROOT / "adapters/fcitx5"), "-B", str(build), "-G", "Ninja",
         "-DCMAKE_BUILD_TYPE=Release", "-DBUILD_TESTING=ON", "-DBADI_WERROR=OFF"], cwd=ROOT)
    run(["cmake", "--build", str(build)], cwd=ROOT)
    run(["ctest", "--test-dir", str(build), "--output-on-failure"], cwd=ROOT)


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
    desktop = SYSTEM_APPLICATIONS / "com.github.xournalpp.xournalpp.desktop"
    if not desktop.exists():
        return None
    target = home / ".local/share/applications" / desktop.name
    text = target.read_text() if target.exists() else desktop.read_text()
    return target, "\n".join("Exec=env GTK_IM_MODULE=fcitx " + line[5:]
                             if line.startswith("Exec=") and "GTK_IM_MODULE=fcitx" not in line else line
                             for line in text.splitlines()) + "\n"


def require_flag_file_unchanged(target, original):
    if target.parent.resolve() != target.parent or target.is_symlink() or \
            (target.read_text() if target.exists() else None) != original:
        raise RuntimeError("App startup configuration changed during the build; newer user settings were preserved.")


def require_hypr_unchanged(target, original):
    if target.is_symlink() or not target.exists() or target.read_text() != original:
        raise RuntimeError("Hyprland configuration changed during the build; newer user settings were preserved.")


def flags_still_match(plan):
    """Reject user configuration that changed while the build ran, before any
    write; each file is checked again just before it is written."""
    for target, (original, _text) in plan.flag_updates.items():
        require_flag_file_unchanged(target, original)
    if plan.hypr_update is not None:
        target, original, _text = plan.hypr_update
        require_hypr_unchanged(target, original)


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
    if plan.omarchy:
        # The launcher opens the Omarchy bar's settings panel.
        install(ROOT / "packaging/io.github.ahuray.badi.desktop",
                home / ".local/share/applications/io.github.ahuray.badi.desktop")
        install(ROOT / "packaging/io.github.ahuray.badi.svg",
                home / ".local/share/icons/hicolor/scalable/apps/io.github.ahuray.badi.svg")
    # The writing binary embeds a modified English lexicon; ship its complete
    # notices on both update paths even though inference needs no external file.
    for name in ("LICENSE", "README.md"):
        install(ROOT / "broker/data/writing-lexicon" / name,
                home / ".local/share/badi/licenses/writing-lexicon" / name)
    # The licences of the open-source code compiled into the binaries.
    install(ROOT / "THIRD-PARTY-NOTICES.txt", home / ".local/share/badi/licenses/THIRD-PARTY-NOTICES.txt")
    if native:
        install(ROOT / "adapters/fcitx5/build/libbadi-fcitx5.so", home / ".local/lib/fcitx5/libbadi-fcitx5.so")
        install(ROOT / "adapters/fcitx5/build/badi.conf", home / ".local/share/fcitx5/addon/badi.conf")
        for name in OBSERVER_MODULES:
            install(ROOT / "adapters/accessibility" / name, library / "accessibility" / name)
        install(ROOT / "packaging/systemd/badi-accessibility.service",
                home / ".config/systemd/user/badi-accessibility.service")
    if native and plan.omarchy:
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
            require_hypr_unchanged(target, original)
            installation.write(target, text.encode(), stat.S_IMODE(target.stat().st_mode))
        for target, (original, text) in plan.flag_updates.items():
            require_flag_file_unchanged(target, original)
            installation.write(target, text.encode(), stat.S_IMODE(target.stat().st_mode) if target.exists() else 0o600)
    install(ROOT / "packaging/systemd/badi-broker.service", home / ".config/systemd/user/badi-broker.service")
    # The session helper reports suspend, session and power state to the
    # broker and reads no text, so it updates with the broker on both paths.
    for name in SESSION_MODULES:
        install(ROOT / "adapters/session" / name, library / "session" / name)
    install(ROOT / "packaging/systemd" / SESSION_SERVICE, home / ".config/systemd/user" / SESSION_SERVICE)
    if native and plan.omarchy:
        install(ROOT / "packaging/systemd/fcitx-badi.conf",
                home / ".config/systemd/user/omarchy-fcitx5.service.d/50-badi.conf")
    elif native:
        installation.write(home / badi_install.FCITX_ENVIRONMENT,
                           badi_install.fcitx_environment(home, plan.system_addons).encode(), 0o644)
    settings = plan.config_home / "badi/settings.json"
    # The broker's unit keeps home read-only except these two directories,
    # so they must exist before it starts.
    (home / ".local/share/badi").mkdir(parents=True, exist_ok=True, mode=0o700)
    settings.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if not settings.exists():
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
    if not plan.omarchy:
        restart_generic(plan)
        return probe
    require_unlocked()
    if not plan.accessibility_enabled:
        run(["systemctl", "--user", "enable", "badi-accessibility.service"])
    run(["systemctl", "--user", "reset-failed", "badi-accessibility.service"])
    run(["systemctl", "--user", "restart", "badi-accessibility.service"])
    wait_accessibility(home / ".local/lib/badi/accessibility/daemon.py", runtime / "badi/accessibility.sock")
    if plan.args.observed_app:
        enable_accessibility(installation)
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


def restart_generic(plan):
    """A generic desktop's observer and Fcitx. The desktop, not Badi, starts
    Fcitx (im-config's autostart, or the compositor on KDE Plasma), and a
    replacement started from here would lose what that launcher gave it, so
    Fcitx is never restarted: it loads the addon at the next login. The
    observer starts only in a Hyprland session, which its unit requires."""
    home, runtime = plan.home, plan.runtime
    if not plan.accessibility_enabled:
        run(["systemctl", "--user", "enable", "badi-accessibility.service"])
    if plan.observer_session:
        run(["systemctl", "--user", "reset-failed", "badi-accessibility.service"])
        run(["systemctl", "--user", "restart", "badi-accessibility.service"])
        wait_accessibility(home / ".local/lib/badi/accessibility/daemon.py", runtime / "badi/accessibility.sock")
    plan.fcitx_process = badi_install.fcitx_bus_process()
    if plan.fcitx_process:
        try:
            plan.addon_loaded = process_maps_file(plan.fcitx_process, home / ".local/lib/fcitx5/libbadi-fcitx5.so")
        except OSError:
            plan.addon_loaded = None


def generic_report(plan):
    """What a generic desktop install leaves for the user to do."""
    environment = "~/" + str(badi_install.FCITX_ENVIRONMENT)
    if plan.addon_loaded:
        print("The running Fcitx already uses this addon build.")
    elif plan.fcitx_process:
        print(f"Fcitx loads the addon at your next login: {environment} adds ~/.local/lib/fcitx5 before "
              f"{plan.system_addons}. Log out and back in, then run badi doctor. Fcitx was not restarted.")
    else:
        print(f"Fcitx is not running in this session. Choose it as the input method (Debian, Ubuntu: im-config -n "
              f"fcitx5; KDE Plasma: Fcitx 5 as the Virtual Keyboard in System Settings), then log out and back "
              f"in; {environment} lets it load the addon. Run badi doctor afterwards.")
    if not plan.observer_session:
        print("Browser and other observed fields need the field observer, which runs only in a Hyprland session so "
              "far: Chromium-based apps, Zen, VS Code and Writer's grey preview get no suggestions on this desktop. "
              "Fcitx's manual fields (Tab requests in apps you allow with badi app ID on), the editors "
              "(Obsidian, Neovim, Bash: scripts/install-editors.py) and the LibreOffice Writer spelling extension "
              "work without it. The observer is installed and starts by itself in a Hyprland session.")


def report(plan, probe):
    if plan.session_disabled:
        print("The session helper is disabled (systemctl --user enable --now badi-session.service turns it on); "
              "suspend, session switches, battery modes and screen sharing are not reported to the broker.")
    elif plan.session_ready:
        print("Session helper reporting: sleep, session switches and screen sharing withdraw suggestions "
              "(badi screen-share); battery behavior follows badi battery; shortcuts are offered through the "
              "desktop portal.")
    else:
        print("The session helper did not report to the broker. Inspect badi doctor and "
              "journalctl --user -u badi-session.service; predictions still work without it.")
    pause_note = " Predictions remain paused; use badi resume when wanted." if probe["paused"] else ""
    if not plan.native:
        print("Local model ready. Broker and controls updated; native input addon was not restarted." + pause_note)
        return
    if not plan.omarchy:
        helper = " and accessibility helper" if plan.observer_session else ""
        print(f"Local model{helper} ready. Fcitx addon installed in ~/.local/lib/fcitx5. Application accessibility "
              "and exact target policy determine coverage; use badi doctor to inspect setup." + pause_note)
        generic_report(plan)
        return
    print("Local model and accessibility helper ready. Desktop addon loaded by the normal Fcitx service. Keyboard profile preserved. Application accessibility and exact target policy determine coverage; use badi doctor to inspect setup." + pause_note)
    if plan.args.observed_app:
        prepared = [app for app in dict.fromkeys(plan.args.observed_app) if app not in plan.flags_present]
        if prepared:
            print(f"{OBSERVED_FLAG} added for " + ", ".join(prepared) +
                  ". Relaunch the app to apply it; existing windows were not closed and app/site policy is unchanged."
                  " Complete renderer accessibility costs some browser CPU and memory on every page.")
        if plan.flags_present:
            print("Renderer accessibility was already enabled for " + ", ".join(plan.flags_present) +
                  "; its flags file or launcher was not changed.")


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
    badi_install.run_installer(main)
