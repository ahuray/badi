#!/usr/bin/env python3
"""Install the Badi broker and cooperative addon into this Omarchy user session."""

import argparse
import hashlib
import importlib.util
import json
import mmap
import os
from pathlib import Path
import shutil
import shlex
import socket
import struct
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
OBSERVED_APP_FLAGS = {"chromium": "chromium-flags.conf", "chatgpt": "codex-flags.conf"}
OBSERVED_FLAGS = ("--ozone-platform=wayland", "--enable-wayland-ime",
                  "--wayland-text-input-version=3", "--force-renderer-accessibility=complete")


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


def observed_flags(text, app="chromium"):
    """Preserve comments and unrelated options in the installed line-based wrappers."""
    if app == "chromium":
        try:
            current = [token for line in text.splitlines() for token in chromium_line_tokens(line)]
        except ValueError:
            raise RuntimeError("Inspect unbalanced quoting in Chromium startup flags before changing them.") from None
    elif app == "chatgpt":
        # This wrapper uses Bash read -a after stripping comments, without
        # evaluating shell quotes; Chromium's launcher uses GLib shell parsing.
        current = [token for line in text.splitlines() for token in line.split("#", 1)[0].split()]
    else:
        raise RuntimeError("Unknown observed application startup parser")
    missing = []
    for flag in OBSERVED_FLAGS:
        key = flag.split("=", 1)[0]
        matches = [token for token in current if token.split("=", 1)[0] == key]
        if matches and any(token != flag for token in matches):
            raise RuntimeError(f"Existing {key} conflicts with the measured Wayland input/accessibility recipe; startup flags were not changed.")
        if not matches:
            missing.append(flag)
    if not missing:
        return text
    return text + ("\n" if text and not text.endswith("\n") else "") + \
        "# Badi: cooperative Wayland input and focused accessibility\n" + "\n".join(missing) + "\n"


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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--broker-only", action="store_true",
                        help="Update only the broker, commands and service; leave the native input method running")
    parser.add_argument("--observed-app", action="append", choices=tuple(OBSERVED_APP_FLAGS), default=[],
                        help="Prepare diagnostic accessibility/input observation on next launch; native browser/Codex editing is disabled")
    args = parser.parse_args()
    if args.broker_only and args.observed_app:
        parser.error("--observed-app requires the complete unlocked native installation")
    home = Path.home().resolve()
    if not os.environ.get("WAYLAND_DISPLAY") or not os.environ.get("XDG_RUNTIME_DIR"):
        raise RuntimeError("Run the installer from your graphical session")
    if not args.broker_only:
        require_unlocked()
        require_accessibility_runtime()
    flag_updates = {}
    config_home = Path(os.environ.get("XDG_CONFIG_HOME") or home / ".config")
    for app in dict.fromkeys(args.observed_app):
        target = observed_config_target(home, config_home, OBSERVED_APP_FLAGS[app])
        original = target.read_text() if target.exists() else None
        flag_updates[target] = (original, observed_flags(original or "", app))
    runtime = session_runtime()
    updating = service_installed()
    accessibility_updating = False
    if not args.broker_only:
        accessibility_updating = service_installed("badi-accessibility.service")
        run(["systemctl", "--user", "is-active", "omarchy-fcitx5.service"], capture_output=True)
    run(["cargo", "build", "--release", "--locked", "--workspace", "--bins"], cwd=ROOT)
    if not args.broker_only:
        run(["npm", "run", "fcitx5:check"], cwd=ROOT)
        require_unlocked()
    backup = home / ".local/state/badi/install-backups" / str(time.time_ns())
    backup.mkdir(parents=True, mode=0o700)
    changes = []

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
                staged.chmod(0o600)
                install(staged, target)
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
    print(f"Rollback files and changed-file list: {backup}", flush=True)
    run(["systemd-analyze", "--user", "verify", str(unit)])
    if not args.broker_only:
        run(["systemd-analyze", "--user", "verify", str(home / ".config/systemd/user/badi-accessibility.service")])
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
    require_unlocked()
    profile = home / ".config/fcitx5/profile"
    previous = hashlib.sha256(profile.read_bytes()).digest()
    run(["systemctl", "--user", "restart", "omarchy-fcitx5.service"])
    wait_native_addon(home / ".local/lib/fcitx5/libbadi-fcitx5.so")
    if hashlib.sha256(profile.read_bytes()).digest() != previous:
        raise RuntimeError("Fcitx rewrote the keyboard profile; inspect it before continuing")
    print("Local model and accessibility helper ready. Desktop addon loaded by the normal Fcitx service. Keyboard profile preserved. Application accessibility and exact target policy determine coverage; use badi doctor to inspect setup." + pause_note)
    if args.observed_app:
        print("Diagnostic startup flags prepared for " + ", ".join(dict.fromkeys(args.observed_app)) +
              ". Native browser/Codex writing is disabled because the external input path lacks safe editor transaction authority. Relaunch normally only for diagnostic observation. Existing windows were not closed; app/site policy is unchanged.")


if __name__ == "__main__":
    main()
