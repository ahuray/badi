#!/usr/bin/env python3
"""Install the Badi broker and cooperative addon into this Omarchy user session."""

import argparse
import errno
import hashlib
import importlib.util
import json
import mmap
import os
from pathlib import Path
import re
import shutil
import shlex
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
# Each launcher's user flags file and how that wrapper turns it into arguments:
# "glib" is /usr/bin/chromium's GLib shell parsing, "words" strips # comments
# and splits on whitespace without quote handling (Codex: read -a; VS Code:
# sed plus unquoted expansion), and "line" passes every non-comment line as one
# argument (mapfile). Discord has no such file; see the accessibility runbook.
OBSERVED_APP_FLAGS = {
    "chromium": ("chromium-flags.conf", "glib"),
    "brave-origin": ("brave-origin-flags.conf", "line"),
    "chatgpt": ("codex-flags.conf", "words"),
    "code": ("code-flags.conf", "words"),
    "cursor": ("cursor-flags.conf", "line"),
}
# Measured on Chromium 152: "basic" and "form-controls" omit the HTML tag the
# observer's purpose gate needs and the character extents its caret geometry
# needs. The bare switch also selects the complete mode. Native Wayland
# text-input-v3 is the Chromium 152 / Electron 42 default, so no input flags.
OBSERVED_FLAG = "--force-renderer-accessibility=complete"
ACCESSIBILITY_SWITCH = "--force-renderer-accessibility"
COMPLETE_ACCESSIBILITY = (ACCESSIBILITY_SWITCH, OBSERVED_FLAG)
COMPAT_LIBRARY = Path(".local/lib/badi/compat")
COMPAT_DROPIN = Path(".config/systemd/user/omarchy-fcitx5.service.d/60-badi-wayland-compat.conf")
COMPAT_FILES = ("launch.py", "build-receipt.json", "addons/libwaylandim.so")
# Retired frontends are removed only while no service command references them.
OBSOLETE_COMPAT = ("5.1.21",)
STOCK_FCITX_COMMAND = "/usr/bin/fcitx5 --disable notificationitem"
# The removed Chromium extension and its native messaging host, which the
# former install-editors.py --chromium installed.
RETIRED_EXTENSION = Path(".local/lib/badi/chromium")
RETIRED_HOSTS = (Path(".local/lib/badi/badi-native-host"), Path(".local/lib/badi/badi-native-manifest"))
RETIRED_HOST_MANIFEST = "io.github.ahuray.badi.json"
CHROMIUM_CONFIGS = ("chromium", "google-chrome", "google-chrome-beta", "google-chrome-unstable",
                    "BraveSoftware/Brave-Browser", "BraveSoftware/Brave-Browser-Beta",
                    "BraveSoftware/Brave-Browser-Nightly", "BraveSoftware/Brave-Origin")


def run(command, **kwargs):
    return subprocess.run(command, check=True, **kwargs)


def chromium_line_tokens(line):
    # GLib starts comments at an unquoted token boundary. shlex's built-in
    # commenters would also truncate a literal hash within an option value.
    quote, escaped, boundary = None, False, True
    for index, char in enumerate(line):
        if escaped:
            escaped = False
            continue
        if char == "\\" and quote != "'":
            escaped, boundary = True, False
        elif quote is not None:
            if char == quote:
                quote = None
        elif char in "\"'":
            quote, boundary = char, False
        elif char == "#" and boundary:
            line = line[:index]
            break
        else:
            boundary = char.isspace()
    return shlex.split(line, comments=False, posix=True)


def observed_tokens(text, parser):
    """The arguments a launcher's flags file supplies, as that wrapper parses it."""
    if parser == "glib":
        try:
            return [token for line in text.splitlines() for token in chromium_line_tokens(line)]
        except ValueError:
            raise RuntimeError("Inspect unbalanced quoting in Chromium startup flags before changing them.") from None
    # Bash reads lines at \n and splits words at the default IFS only.
    if parser == "words":
        return [token for line in text.split("\n") for token in re.split(r"[ \t]+", line.split("#", 1)[0]) if token]
    if parser == "line":
        return [line for line in text.split("\n") if line.strip(" \t") and not line.lstrip(" \t").startswith("#")]
    raise RuntimeError("Unknown observed application startup parser")


def observed_flags(text, app="chromium"):
    """Add only the renderer accessibility switch; None when it is already effective.

    Comments and every other option stay byte-identical. A reduced or disabled
    accessibility choice is the user's and is reported instead of overridden.
    """
    tokens = observed_tokens(text, OBSERVED_APP_FLAGS[app][1])
    if any(token.split("=", 1)[0] == "--disable-renderer-accessibility" for token in tokens):
        raise RuntimeError(f"Existing --disable-renderer-accessibility in {OBSERVED_APP_FLAGS[app][0]} conflicts with the observer's accessibility requirement; startup flags were not changed.")
    current = [token for token in tokens if token.split("=", 1)[0] == ACCESSIBILITY_SWITCH]
    if any(token not in COMPLETE_ACCESSIBILITY for token in current):
        raise RuntimeError(f"Existing {ACCESSIBILITY_SWITCH} value in {OBSERVED_APP_FLAGS[app][0]} conflicts with the measured complete accessibility mode; startup flags were not changed.")
    if current:
        return None
    return text + ("\n" if text and not text.endswith("\n") else "") + \
        "# Badi: renderer accessibility for the focused-field observer\n" + OBSERVED_FLAG + "\n"


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


def enable_accessibility(backup):
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
    receipt = backup / "accessibility-setting.json"
    with receipt.open("x") as stream:
        os.chmod(receipt, 0o600)
        json.dump({"bus_enabled": previous, "toolkit_accessibility": toolkit == "true"}, stream)
    if not previous:
        require_unlocked()
        run(["busctl", "--user", "--timeout=2s", "set-property", "org.a11y.Bus", "/org/a11y/bus",
             "org.a11y.Status", "IsEnabled", "b", "true"], capture_output=True, timeout=3)
    if not status():
        raise RuntimeError("Desktop accessibility did not enable; Fcitx was not restarted.")


def require_unlocked():
    result = run(["omarchy-shell", "lock", "status"], capture_output=True, text=True, timeout=4)
    state = json.loads(result.stdout)
    if not isinstance(state, dict) or not all(state.get(key) is False for key in ("locked", "secure", "requested", "pending", "sessionLocked")):
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
    result = run(["systemctl", "--user", "show", unit, "--property=LoadState", "--value"],
                 capture_output=True, text=True, timeout=4)
    return result.stdout.strip() != "not-found"


def probe_model(cli, endpoint):
    def service_pid():
        result = run(["systemctl", "--user", "show", "badi-broker.service", "--property=MainPID", "--value"],
                     capture_output=True, text=True, timeout=3)
        return int(result.stdout.strip())

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
        result = run(["systemctl", "--user", "show", "omarchy-fcitx5.service", "--property=ActiveState,MainPID"],
                     capture_output=True, text=True, timeout=3)
        return dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)

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
    deadline = time.monotonic() + 10
    while not native_addon_loaded(addon):
        if time.monotonic() >= deadline:
            raise RuntimeError("The running Fcitx service did not load the installed Badi addon. Inspect badi doctor and the Fcitx user-service journal.")
        time.sleep(.2)


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


def receipt_module():
    spec = importlib.util.spec_from_file_location("badi_install_receipt", Path(__file__).with_name("install-receipt.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def accessibility_health_module():
    spec = importlib.util.spec_from_file_location("badi_accessibility_health", ROOT / "adapters/accessibility/health.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def probe_accessibility(helper, endpoint):
    """Verify the exact service process and private metadata-only endpoint."""
    def state():
        result = run(["systemctl", "--user", "show", "badi-accessibility.service", "--property=ActiveState,MainPID"],
                     capture_output=True, text=True, timeout=3)
        return dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)

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
    deadline = time.monotonic() + 10
    while probe_accessibility(helper, endpoint) is None:
        if time.monotonic() >= deadline:
            raise RuntimeError("The accessibility helper did not become ready. Inspect badi doctor and journalctl --user -u badi-accessibility.service; Fcitx was not restarted.")
        time.sleep(.2)


def compat_launcher():
    """The pinned frontend selector; its constants define the supported Fcitx cell."""
    spec = importlib.util.spec_from_file_location("badi_fcitx_compat_launch", ROOT / "packaging/fcitx5-wayland-compat/launch.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def compat_command(home, version):
    return f"/usr/bin/python3 -B {home}/{COMPAT_LIBRARY}/fcitx5-{version}/launch.py --disable notificationitem"


def compat_dropin(version):
    return ("[Service]\n# Badi: pinned Fcitx Wayland frontend; launch.py falls back to /usr/bin/fcitx5.\n"
            f"ExecStart=\nExecStart=/usr/bin/python3 -B %h/{COMPAT_LIBRARY}/fcitx5-{version}/launch.py --disable notificationitem\n")


def fcitx_command():
    """The effective argv of the Fcitx service's single ExecStart, verbatim."""
    result = run(["systemctl", "--user", "show", "omarchy-fcitx5.service", "--property=ExecStart", "--value"],
                 capture_output=True, text=True, timeout=3)
    commands = re.findall(r"argv\[\]=(.*?) ; ignore_errors=", result.stdout)
    return commands[0] if len(commands) == 1 else None


def retirable_compat(home, directory):
    """The exact files of a retired frontend; None when anything unexpected is present."""
    for path in (directory, *directory.parents):
        if path == home:
            break
        if path.is_symlink():
            raise RuntimeError(f"Inspect existing symlink before replacing {directory}")
    if not directory.exists():
        return []
    expected = {directory / name for name in COMPAT_FILES}
    files, folders = set(), set()
    for path in directory.rglob("*"):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            return None
        (folders if path.is_dir() else files).add(path)
    if not directory.is_dir() or files - expected or folders - {directory / "addons"}:
        return None
    return sorted(files)


def verify_compat_build(directory, launcher):
    """Accept only a checked build of the pinned source for today's exact runtime."""
    directory = Path(directory).expanduser().resolve()
    module = directory / "libwaylandim.so"
    try:
        receipt = json.loads((directory / "build-receipt.json").read_text())
        manifest = json.loads((ROOT / "packaging/fcitx5-wayland-compat/manifest.json").read_text())
        runtime = receipt["runtime_files_sha256"]
        valid = (receipt["schema"] == "badi.fcitx-wayland-compat-build.v1" and receipt["source"] == manifest
                 and receipt["protocol_checks_passed"] is True
                 and receipt["installed_build_versions"] == {name: launcher.VERSION for name in ("Fcitx5Core", "Fcitx5Config", "Fcitx5Utils")}
                 and set(runtime) == set(launcher.RUNTIME_FILES)
                 and all(launcher.digest(Path(path)) == runtime[path] for path in launcher.RUNTIME_FILES)
                 and not module.is_symlink() and module.is_file()
                 and launcher.digest(module) == receipt["artifact_sha256"])
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        valid = False
    if not valid:
        raise RuntimeError("The Wayland compatibility build is not a checked build of the pinned source for this system's Fcitx runtime. Rebuild it, or rerun with --no-wayland-compat.")
    return directory


def retired_browser_components(home, config_home):
    """Files and folders the removed Chromium integration left installed.

    A browser's native-messaging manifest counts only while it names Badi's
    installed host, and the extension folder only while it holds nothing but
    plain files and folders. Returns (files, folders deepest first, kept).
    """
    host = home / RETIRED_HOSTS[0]
    files = [home / path for path in RETIRED_HOSTS if (home / path).is_file() and not (home / path).is_symlink()]
    folders, kept = [], []
    extension = home / RETIRED_EXTENSION
    if extension.is_symlink() or (extension.exists() and not extension.is_dir()):
        kept.append(extension)
    elif extension.is_dir():
        found, expected = [], True
        for directory, subdirectories, names in os.walk(extension):
            directory = Path(directory)
            folders.append(directory)
            for path in (directory / name for name in (*subdirectories, *names)):
                if path.is_symlink() or not (path.is_file() or path.is_dir()):
                    expected = False
                elif path.is_file():
                    found.append(path)
        if expected:
            files += found
            folders.reverse()
        else:
            folders = []
            kept.append(extension)
    for browser in CHROMIUM_CONFIGS:
        manifest = config_home / browser / "NativeMessagingHosts" / RETIRED_HOST_MANIFEST
        if manifest.is_symlink() or not manifest.is_file() or not manifest.is_relative_to(home):
            continue
        try:
            named = json.loads(manifest.read_text()).get("path")
        except (OSError, ValueError, AttributeError):
            continue
        if isinstance(named, str) and Path(named).is_absolute() and Path(named).resolve() == host.resolve():
            files.append(manifest)
    return files, folders, kept


def plan_wayland_compat(home, build=None):
    """Decide the pinned frontend action before building or changing any file.

    Another Fcitx version gets no frontend: 5.1.23 contains the upstream
    refresh, and launch.py already falls back to the system frontend.
    """
    launcher = compat_launcher()
    try:
        version = run(["/usr/bin/fcitx5", "--version"], capture_output=True, text=True, timeout=5).stdout.strip()
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError):
        raise RuntimeError("Cannot determine the system Fcitx version. Rerun with --no-wayland-compat to leave its frontend unchanged.") from None
    if version != launcher.VERSION:
        if build is not None:
            raise RuntimeError(f"The system Fcitx is {version or 'unknown'}, not the pinned {launcher.VERSION}; the supplied compatibility build was not installed.")
        return {"install": False, "version": version, "pinned": launcher.VERSION}
    homes = dict.fromkeys((str(Path.home()), str(home)))
    command = fcitx_command()
    active = {old for old in OBSOLETE_COMPAT if command in {compat_command(item, old) for item in homes}}
    managed = {compat_command(item, old) for item in homes for old in (version, *OBSOLETE_COMPAT)}
    if command != STOCK_FCITX_COMMAND and (command not in managed or not (home / COMPAT_DROPIN).is_file()):
        raise RuntimeError("omarchy-fcitx5.service runs an unrecognized command. Inspect its overrides, or rerun with --no-wayland-compat.")
    root = home / COMPAT_LIBRARY / f"fcitx5-{version}"
    if retirable_compat(home, root) is None:
        raise RuntimeError(f"Inspect unexpected files in {root} before installing the compatibility frontend.")
    obsolete, kept = [], []
    for old in OBSOLETE_COMPAT:
        directory = home / COMPAT_LIBRARY / f"fcitx5-{old}"
        files = retirable_compat(home, directory)
        if files is None or (files and old in active):
            kept.append(directory)
        elif files:
            obsolete.append((directory, files))
    plan = {"install": True, "version": version, "pinned": launcher.VERSION, "root": root, "launcher": launcher,
            "homes": tuple(homes), "build": None, "obsolete": obsolete, "kept": kept}
    if build is not None:
        plan["build"] = verify_compat_build(build, launcher)
    return plan


def build_wayland_compat(plan):
    if plan["build"] is None:
        work = ROOT / "output/extensionless" / f"fcitx-wayland-compat-{plan['version']}-{time.time_ns()}"
        run([sys.executable, "-B", str(ROOT / "packaging/fcitx5-wayland-compat/build.py"),
             "--work-dir", str(work), "--jobs", "2", "--check"], cwd=ROOT)
        plan["build"] = verify_compat_build(work, plan["launcher"])
    return plan["build"]


def wait_compat_frontend(module):
    """Report whether launch.py selected the installed frontend; bounded."""
    deadline = time.monotonic() + 3
    while not native_addon_loaded(module):
        if time.monotonic() >= deadline:
            return False
        time.sleep(.2)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--broker-only", action="store_true",
                        help="Update only the broker, commands and service; leave the native input method running")
    parser.add_argument("--observed-app", action="append", choices=tuple(OBSERVED_APP_FLAGS), default=[],
                        help=f"Add {OBSERVED_FLAG} to this app's user flags file for its next launch (backed up)")
    parser.add_argument("--no-wayland-compat", action="store_true",
                        help="Leave omarchy-fcitx5.service on its current frontend (skip the pinned Fcitx 5.1.22 compatibility frontend)")
    parser.add_argument("--wayland-compat-build", type=Path, metavar="DIR",
                        help="Install this existing checked compatibility build instead of building one")
    args = parser.parse_args()
    if args.broker_only and args.observed_app:
        parser.error("--observed-app requires the complete unlocked native installation")
    if args.broker_only and args.wayland_compat_build:
        parser.error("--wayland-compat-build requires the complete unlocked native installation")
    if args.no_wayland_compat and args.wayland_compat_build:
        parser.error("--no-wayland-compat and --wayland-compat-build are exclusive")
    home = Path.home().resolve()
    if not os.environ.get("WAYLAND_DISPLAY") or not os.environ.get("XDG_RUNTIME_DIR"):
        raise RuntimeError("Run the installer from your graphical session")
    if not args.broker_only:
        require_unlocked()
        require_accessibility_runtime()
    flag_updates = {}
    config_home = Path(os.environ.get("XDG_CONFIG_HOME") or home / ".config")
    flags_present = []
    for app in dict.fromkeys(args.observed_app):
        target = observed_config_target(home, config_home, OBSERVED_APP_FLAGS[app][0])
        original = target.read_text() if target.exists() else None
        text = observed_flags(original or "", app)
        if text is None:
            flags_present.append(app)
        else:
            flag_updates[target] = (original, text)
    runtime = session_runtime()
    updating = service_installed()
    accessibility_updating = False
    compat = None
    if not args.broker_only:
        accessibility_updating = service_installed("badi-accessibility.service")
        run(["systemctl", "--user", "is-active", "omarchy-fcitx5.service"], capture_output=True)
        if not args.no_wayland_compat:
            compat = plan_wayland_compat(home, args.wayland_compat_build)
    receipts = receipt_module()
    # Record the checkout state that is about to be built and copied.
    checkout = receipts.source_identity(ROOT)
    run(["cargo", "build", "--release", "--locked", "--workspace", "--bins"], cwd=ROOT)
    if not args.broker_only:
        run(["npm", "run", "fcitx5:check"], cwd=ROOT)
        if compat and compat["install"]:
            build_wayland_compat(compat)
        require_unlocked()
    backup = home / ".local/state/badi/install-backups" / str(time.time_ns())
    backup.mkdir(parents=True, mode=0o700)
    changes = []
    installed = []

    def install(source, target):
        target.parent.mkdir(parents=True, exist_ok=True)
        managed_link = target == home / ".local/bin/badi-desktop" and target.resolve() == ROOT / "scripts/badi-desktop.py"
        if target.is_symlink() and not managed_link:
            raise RuntimeError(f"Inspect existing symlink before replacing {target}")
        if target.exists():
            saved = backup / target.relative_to(home)
            saved.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(target, saved, follow_symlinks=False)
        changes.append(str(target.relative_to(home)))
        installed.append(target)
        # Keep recovery discoverable even if a later target or startup fails.
        (backup / "changed-files.json").write_text(json.dumps(changes, indent=2))
        # Never truncate a library or executable that another process may map.
        with tempfile.NamedTemporaryFile(dir=target.parent, delete=False) as stream:
            staged = Path(stream.name)
        try:
            shutil.copy2(source, staged)
            staged.replace(target)
        finally:
            staged.unlink(missing_ok=True)

    def retire(files, folders):
        # Recorded like replaced files: restoring changed-files.json from the
        # backup recreates them, and they leave the install receipt's new set.
        for path in files:
            saved = backup / path.relative_to(home)
            saved.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, saved, follow_symlinks=False)
            changes.append(str(path.relative_to(home)))
            (backup / "changed-files.json").write_text(json.dumps(changes, indent=2))
            path.unlink()
        for folder in folders:
            try:
                folder.rmdir()
            except OSError as error:
                if error.errno not in (errno.ENOENT, errno.ENOTEMPTY):
                    raise

    retired_files, retired_folders, retired_kept = retired_browser_components(home, config_home)
    retire(retired_files, retired_folders)
    receipts.forget(home, "editors", retired_files)

    for name in ("badi-broker", "badictl"):
        install(ROOT / "target/release" / name, home / ".local/lib/badi" / name)
    install(ROOT / "scripts/badi-desktop.py", home / ".local/bin/badi")
    install(ROOT / "scripts/badi-desktop.py", home / ".local/bin/badi-desktop")
    install(ROOT / "target/release/badictl", home / ".local/bin/badictl")
    install(ROOT / "packaging/io.github.ahuray.badi.desktop",
            home / ".local/share/applications/io.github.ahuray.badi.desktop")
    install(ROOT / "packaging/io.github.ahuray.badi.svg",
            home / ".local/share/icons/hicolor/scalable/apps/io.github.ahuray.badi.svg")
    # The writing binary embeds a modified English lexicon; ship its complete
    # notices on both update paths even though inference needs no external file.
    for name in ("LICENSE", "README.md"):
        install(ROOT / "broker/data/writing-lexicon" / name,
                home / ".local/share/badi/licenses/writing-lexicon" / name)
    if not args.broker_only:
        install(ROOT / "adapters/fcitx5/build/libbadi-fcitx5.so", home / ".local/lib/fcitx5/libbadi-fcitx5.so")
        install(ROOT / "adapters/fcitx5/build/badi.conf", home / ".local/share/fcitx5/addon/badi.conf")
        for name in ("daemon.py", "contract.py", "preview.py", "health.py"):
            install(ROOT / "adapters/accessibility" / name, home / ".local/lib/badi/accessibility" / name)
        install(ROOT / "packaging/systemd/badi-accessibility.service",
                home / ".config/systemd/user/badi-accessibility.service")
        for target, (original, text) in flag_updates.items():
            if target.parent.resolve() != target.parent or target.is_symlink() or (target.read_text() if target.exists() else None) != original:
                raise RuntimeError("App startup configuration changed during the build; newer user settings were preserved.")
            with tempfile.TemporaryDirectory() as directory:
                staged = Path(directory) / target.name
                staged.write_text(text)
                staged.chmod(stat.S_IMODE(target.stat().st_mode) if target.exists() else 0o600)
                install(staged, target)
        if compat and compat["install"]:
            root = compat["root"]
            install(compat["build"] / "libwaylandim.so", root / "addons/libwaylandim.so")
            install(compat["build"] / "build-receipt.json", root / "build-receipt.json")
            install(ROOT / "packaging/fcitx5-wayland-compat/launch.py", root / "launch.py")
            with tempfile.TemporaryDirectory() as directory:
                staged = Path(directory) / COMPAT_DROPIN.name
                staged.write_text(compat_dropin(compat["version"]))
                staged.chmod(0o644)
                install(staged, home / COMPAT_DROPIN)
            for directory, files in compat["obsolete"]:
                retire(files, (directory / "addons", directory))
    unit = home / ".config/systemd/user/badi-broker.service"
    install(ROOT / "packaging/systemd/badi-broker.service", unit)
    if not args.broker_only:
        install(ROOT / "packaging/systemd/fcitx-badi.conf",
                home / ".config/systemd/user/omarchy-fcitx5.service.d/50-badi.conf")
    config = Path(os.environ.get("XDG_CONFIG_HOME") or home / ".config") / "badi/settings.json"
    if not config.exists():
        config.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        subjects = [{"identity": {"kind": "linux_app", "adapter": "fcitx", "app_id": app},
                     "permissions": {"context_read": "allow", "display": "allow", "suggest": "allow",
                                     "learn": "block", "retention": {"mode": "none"}}}
                    for app in ("com.github.xournalpp.xournalpp", "omawrite")]
        with config.open("x") as stream:
            os.chmod(config, 0o600)
            json.dump({"schema": "badi.settings.v2", "revision": 1, "paused": False, "subjects": subjects}, stream)
    # Preserve the upstream launcher, including its MIME types and action entries.
    # Only Xournal++ needs its GTK toolkit path selected explicitly on Wayland.
    desktop = Path("/usr/share/applications/com.github.xournalpp.xournalpp.desktop")
    if not args.broker_only and desktop.exists():
        target = home / ".local/share/applications" / desktop.name
        text = target.read_text() if target.exists() else desktop.read_text()
        text = "\n".join("Exec=env GTK_IM_MODULE=fcitx " + line[5:]
                         if line.startswith("Exec=") and "GTK_IM_MODULE=fcitx" not in line else line
                         for line in text.splitlines()) + "\n"
        with tempfile.TemporaryDirectory() as directory:
            staged = Path(directory) / desktop.name
            staged.write_text(text)
            install(staged, target)
    (backup / "changed-files.json").write_text(json.dumps(changes, indent=2))
    if retired_files:
        print("Removed the retired Badi browser extension and native host; if the unpacked Badi extension "
              "is still loaded, remove it in brave://extensions.", flush=True)
    for path in retired_kept:
        print(f"Kept {path}: it holds unexpected entries from the retired browser extension. Inspect and remove it.", flush=True)
    print(f"Rollback files and changed-file list: {backup}", flush=True)
    receipt = receipts.write_receipt(home, "desktop", checkout, installed)
    print(f"Install receipt: {receipt}", flush=True)
    run(["systemd-analyze", "--user", "verify", str(unit)])
    if not args.broker_only:
        run(["systemd-analyze", "--user", "verify", str(home / ".config/systemd/user/badi-accessibility.service")])
    if compat and compat["install"]:
        run(["systemd-analyze", "--user", "verify", "omarchy-fcitx5.service"])
    run(["systemctl", "--user", "daemon-reload"])
    if not updating:
        run(["systemctl", "--user", "enable", "badi-broker.service"])
    run(["systemctl", "--user", "reset-failed", "badi-broker.service"])
    run(["systemctl", "--user", "restart", "badi-broker.service"])
    cli = home / ".local/lib/badi/badictl"
    deadline = time.monotonic() + 60
    while True:
        probe = probe_model(cli, runtime / "badi/broker.sock")
        if probe is not None:
            require_model_health(probe)
            break
        if time.monotonic() >= deadline:
            raise RuntimeError("Model startup failed: inspect journalctl --user -u badi-broker.service")
        time.sleep(.2)
    pause_note = " Predictions remain paused; use badi resume when wanted." if probe["paused"] else ""
    if args.broker_only:
        print("Local model ready. Broker and controls updated; native input addon was not restarted." + pause_note)
        return
    require_unlocked()
    if not accessibility_updating:
        run(["systemctl", "--user", "enable", "badi-accessibility.service"])
    run(["systemctl", "--user", "reset-failed", "badi-accessibility.service"])
    run(["systemctl", "--user", "restart", "badi-accessibility.service"])
    wait_accessibility(home / ".local/lib/badi/accessibility/daemon.py", runtime / "badi/accessibility.sock")
    if args.observed_app:
        enable_accessibility(backup)
    if compat and compat["install"] and fcitx_command() not in {compat_command(item, compat["version"]) for item in compat["homes"]}:
        raise RuntimeError(f"Another omarchy-fcitx5.service override replaces the compatibility command, so Fcitx was not restarted. Inspect its drop-ins, restore from {backup}, or rerun with --no-wayland-compat.")
    require_unlocked()
    profile = home / ".config/fcitx5/profile"
    previous = hashlib.sha256(profile.read_bytes()).digest()
    run(["systemctl", "--user", "restart", "omarchy-fcitx5.service"])
    wait_native_addon(home / ".local/lib/fcitx5/libbadi-fcitx5.so")
    if hashlib.sha256(profile.read_bytes()).digest() != previous:
        raise RuntimeError("Fcitx rewrote the keyboard profile; inspect it before continuing")
    print("Local model and accessibility helper ready. Desktop addon loaded by the normal Fcitx service. Keyboard profile preserved. Application accessibility and exact target policy determine coverage; use badi doctor to inspect setup." + pause_note)
    if compat is None:
        print("Fcitx Wayland frontend left unchanged (--no-wayland-compat).")
    elif not compat["install"]:
        print(f"System Fcitx {compat['version'] or 'unknown'} is not the pinned {compat['pinned']}, so the Wayland compatibility frontend was not installed; Fcitx 5.1.23 and later include the upstream refresh.")
    elif wait_compat_frontend(compat["root"] / "addons/libwaylandim.so"):
        print(f"Pinned Fcitx {compat['version']} Wayland compatibility frontend verified in the running service.")
    else:
        print("The Wayland compatibility frontend is installed, but launch.py selected the system frontend for this runtime. Inspect journalctl --user -u omarchy-fcitx5.service; input continues with stock Fcitx.")
    for directory in compat["kept"] if compat and compat["install"] else ():
        print(f"Kept {directory}: it is still referenced or holds unexpected files. Rerun the installer after inspecting it.")
    if args.observed_app:
        prepared = [app for app in dict.fromkeys(args.observed_app) if app not in flags_present]
        if prepared:
            print(f"{OBSERVED_FLAG} added for " + ", ".join(prepared) +
                  ". Relaunch the app to apply it; existing windows were not closed and app/site policy is unchanged."
                  " Complete renderer accessibility costs some browser CPU and memory on every page.")
        if flags_present:
            print("Renderer accessibility was already enabled for " + ", ".join(flags_present) + "; its flags file was not changed.")


if __name__ == "__main__":
    main()
