#!/usr/bin/env python3
"""Shared contracts of Badi's user-local installers and its desktop CLI.

Installed next to the `badi` CLI, which imports it; the installers import it
from the checkout. It holds content-free install receipts, the backed-up file
changes of one installer run and their rollback, atomic file writes, the
systemd and lock-state queries both sides make, and the grant shape of a
settings subject.

Run as a script, it restores one installation backup:
    python3 scripts/badi_install.py restore BACKUP_DIRECTORY [--only PATH]...
"""

import argparse
import calendar
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import time

STATE_DIRECTORY = Path(".local/state/badi")
SCHEMA = "badi.install-receipt.v1"
RECEIPT_DIRECTORY = STATE_DIRECTORY / "receipts"
BACKUP_SCHEMA = "badi.install-backup.v1"
# Each installer keeps its backups in its own directory, newest KEEP_BACKUPS only.
BACKUP_DIRECTORIES = {"desktop": "install-backups", "editors": "editor-backups", "omarchy-ui": "ui-backups"}
KEEP_BACKUPS = 3
BACKUP_MAP = "changes.json"
# Records of desktop-wide settings as they were before an installation changed
# them. Pruning moves them to <backup directory>/notes/<backup name>/ first,
# since no later backup can say what the setting was before Badi.
KEPT_NOTES = "accessibility-*.json"
# Backup directory names: the current UTC time, and the two earlier formats
# (time_ns digits; local YYYYmmdd-HHMMSS) that pruning also recognizes.
BACKUP_NAME = re.compile(r"(\d{8}T\d{6})\.(\d{9})Z")
VERSIONED = frozenset(("badi-broker", "badictl"))
VERSION_LINE = re.compile(r"[a-z][a-z-]* \S+ commit=([0-9a-f]{40}|unknown) dirty=(true|false|unknown)")
UNKNOWN = {"commit": "unknown", "dirty": None}
# Omarchy's lock status is permission only when every flag is explicitly false.
LOCK_FLAGS = ("locked", "secure", "requested", "pending", "sessionLocked")

# Each launcher's user flags file and how that wrapper turns it into arguments:
# "glib" is /usr/bin/chromium's GLib shell parsing, "words" strips # comments
# and splits on whitespace without quote handling (Codex and Claude: read -a;
# VS Code and Grok Bot: sed plus unquoted expansion), and "line" passes every non-comment
# line as one argument (mapfile). Discord has no such file; see the
# accessibility runbook.
OBSERVED_APP_FLAGS = {
    "chromium": ("chromium-flags.conf", "glib"),
    "brave-origin": ("brave-origin-flags.conf", "line"),
    "chatgpt": ("codex-flags.conf", "words"),
    "code": ("code-flags.conf", "words"),
    "com.anthropic.claude": ("claude-desktop-flags.conf", "words"),
    "cursor": ("cursor-flags.conf", "line"),
    "grok-bot": ("grok-bot-flags.conf", "words"),
}
# Measured on Chromium 152: "basic" and "form-controls" omit the HTML tag the
# observer's purpose gate needs and the character extents its caret geometry
# needs. The bare switch also selects the complete mode. Native Wayland
# text-input-v3 is the Chromium 152 / Electron 42 default, so no input flags.
OBSERVED_FLAG = "--force-renderer-accessibility=complete"
ACCESSIBILITY_SWITCH = "--force-renderer-accessibility"
COMPLETE_ACCESSIBILITY = (ACCESSIBILITY_SWITCH, OBSERVED_FLAG)
# The line hyprland.lua needs to load Badi's preview layer rule.
HYPR_REQUIRE = 'require("hypr.badi")'


def hyprland_with_badi(text):
    """hyprland.lua loading Badi's layer rule, or None when it already does."""
    if HYPR_REQUIRE in text:
        return None
    return text.rstrip() + "\n\n-- Badi writing preview layer rule (no entrance animation).\n" + HYPR_REQUIRE + "\n"


def config_home(home):
    """The user's configuration directory, as the XDG base directory rules name it."""
    return Path(os.environ.get("XDG_CONFIG_HOME") or Path(home) / ".config")


def grant(decision):
    """The permissions of one settings subject: "allow" or "block" predictions.

    Learning stays blocked and nothing is retained either way.
    """
    return {"context_read": decision, "display": decision, "suggest": decision,
            "learn": "block", "retention": {"mode": "none"}}


def explicitly_unlocked(state):
    """True only for a lock status whose every flag is present and false."""
    return isinstance(state, dict) and all(state.get(flag) is False for flag in LOCK_FLAGS)


def lock_state(timeout=4):
    """The Omarchy shell's lock status document; raises when it cannot be read."""
    result = subprocess.run(["omarchy-shell", "lock", "status"], capture_output=True, text=True,
                            check=True, timeout=timeout)
    return json.loads(result.stdout)


# The desktops the installers and the CLI tell apart. Omarchy has the shell's
# lock IPC, bar panel, hooks and omarchy-fcitx5.service; any other desktop
# (GNOME, KDE Plasma, another compositor) starts Fcitx its own way, through
# im-config, XDG autostart or the compositor.
OMARCHY = "omarchy"
GENERIC = "generic"
# Where a generic desktop's Fcitx learns Badi's user-local addon directory:
# systemd's user environment generator reads it at login, and the session
# passes it on to whatever starts Fcitx. Always under ~/.config, as systemd
# reads it there whatever an interactive shell's XDG_CONFIG_HOME says.
FCITX_ENVIRONMENT = Path(".config/environment.d/50-badi-fcitx5.conf")
# The D-Bus name a running Fcitx 5 owns on the session bus.
FCITX_BUS_NAME = "org.fcitx.Fcitx5"


def desktop_platform():
    """OMARCHY where Omarchy's shell command is installed, otherwise GENERIC."""
    return OMARCHY if shutil.which("omarchy-shell") else GENERIC


def os_release(path=Path("/etc/os-release")):
    """The distribution's os-release fields; empty when it cannot be read."""
    fields = {}
    try:
        lines = Path(path).read_text().splitlines()
    except (OSError, UnicodeDecodeError):
        return fields
    for line in lines:
        key, separator, value = line.partition("=")
        if separator and re.fullmatch(r"[A-Z_][A-Z0-9_]*", key):
            try:
                fields[key] = " ".join(shlex.split(value)) if value else ""
            except ValueError:
                continue
    return fields


def distribution_family(release=None):
    """"debian" (Debian, Ubuntu and their derivatives), "arch", or None."""
    release = os_release() if release is None else release
    names = {release.get("ID", ""), *release.get("ID_LIKE", "").split()}
    if names & {"debian", "ubuntu"}:
        return "debian"
    if "arch" in names:
        return "arch"
    return None


def fcitx_addon_directory(root=Path("/"), multiarch=None):
    """The directory the system's Fcitx loads its own addons from, or None.

    Fcitx keeps them in fcitx5/ beside libFcitx5Core: /usr/lib/fcitx5 on
    Arch, /usr/lib/<multiarch>/fcitx5 on Debian and Ubuntu, /usr/lib64/fcitx5
    on Fedora. Setting FCITX_ADDON_DIRS replaces that default, so it must
    name this directory after Badi's own.
    """
    if multiarch is None:
        import sysconfig
        multiarch = sysconfig.get_config_var("MULTIARCH") or ""
    candidates = ([f"usr/lib/{multiarch}"] if multiarch else []) + ["usr/lib64", "usr/lib"]
    for relative in candidates:
        library = Path(root) / relative
        if (library / "fcitx5").is_dir() and any(library.glob("libFcitx5Core.so*")):
            return Path("/") / relative / "fcitx5"
    return None


def fcitx_environment(home, system_addons):
    """The environment.d file that adds Badi's user-local addon to Fcitx.

    The same two directories as Omarchy's service drop-in
    (packaging/systemd/fcitx-badi.conf), with this system's addon directory.
    """
    home = Path(home)
    return ("# Badi: Fcitx loads the user-local Badi addon beside the system's own addons.\n"
            f"FCITX_ADDON_DIRS={home}/.local/lib/fcitx5:{system_addons}\n"
            f"FCITX_DATA_DIRS={home}/.local/share/fcitx5:/usr/local/share/fcitx5:/usr/share/fcitx5\n")


def fcitx_bus_process(timeout=3):
    """The process id of the Fcitx 5 that owns its name on this session bus,
    0 when none does, or None when the bus cannot be asked."""
    def ask(method, reply):
        try:
            result = subprocess.run(["busctl", "--user", f"--timeout={timeout}s", "call", "org.freedesktop.DBus",
                                     "/org/freedesktop/DBus", "org.freedesktop.DBus", method, "s", FCITX_BUS_NAME],
                                    capture_output=True, text=True, timeout=timeout + 1)
        except (OSError, subprocess.SubprocessError):
            return None
        match = re.fullmatch(reply, result.stdout.strip()) if result.returncode == 0 else None
        return match[1] if match else None

    # Bus implementations word a missing owner differently, so ask first.
    owned = ask("NameHasOwner", r"b (true|false)")
    if owned != "true":
        return 0 if owned == "false" else None
    pid = ask("GetConnectionUnixProcessID", r"u (\d+)")
    return int(pid) if pid else None


def unit_properties(unit, *names, timeout=3):
    """The named properties of a systemd user unit, as a dict of strings."""
    result = subprocess.run(["systemctl", "--user", "show", unit, "--property=" + ",".join(names)],
                            capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "Cannot reach the user service manager")
    return dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)


def unit_value(unit, name, timeout=3):
    """One property of a systemd user unit, verbatim (values may contain '=')."""
    result = subprocess.run(["systemctl", "--user", "show", unit, f"--property={name}", "--value"],
                            capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "Cannot reach the user service manager")
    return result.stdout.strip()


def poll(check, seconds, interval=.2):
    """Call check() until it returns something truthy; None once `seconds` pass."""
    deadline = time.monotonic() + seconds
    while True:
        result = check()
        if result:
            return result
        if time.monotonic() >= deadline:
            return None
        time.sleep(interval)


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def private_directory(path, top):
    """Creates `path` 0700 and tightens it and every directory up to `top`
    (Badi's state directory) that an older install left group- or
    world-readable."""
    path, top = Path(path), Path(top)
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    directory = path
    while True:
        info = directory.lstat()
        if stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid() and info.st_mode & 0o077:
            directory.chmod(0o700)
        if directory == top or directory.parent == directory:
            return path
        directory = directory.parent


def atomic_write(path, data, mode=0o600):
    """Replace `path` with `data` in one rename.

    Readers see the old or the new file, never a partial one, and a mapped
    library or running executable keeps its old inode.
    """
    path = Path(path)
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as stream:
        staged = Path(stream.name)
        try:
            os.fchmod(stream.fileno(), mode)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            staged.unlink(missing_ok=True)
            raise
    try:
        staged.replace(path)
    finally:
        staged.unlink(missing_ok=True)


def receipt_path(home, installer):
    return Path(home) / RECEIPT_DIRECTORY / f"{installer}.json"


def home_relative(home, path):
    """The receipt/backup key of a path: relative to home when inside it."""
    path = Path(path)
    return str(path.relative_to(home)) if path.is_relative_to(home) else str(path)


def source_identity(root):
    """Return the checkout HEAD and whole-tree dirty flag; unknown without Git.

    Like build.rs, an exported tree nested in another repository is unknown
    rather than inheriting that repository's commit.
    """
    git = ["git", "-C", str(root)]
    try:
        top = subprocess.run([*git, "rev-parse", "--show-toplevel"], capture_output=True, text=True, timeout=10)
        if top.returncode or not isinstance(top.stdout, str) or not top.stdout.strip() \
                or Path(top.stdout.strip()).resolve() != Path(root).resolve():
            return dict(UNKNOWN)
        head = subprocess.run([*git, "rev-parse", "--verify", "HEAD^{commit}"],
                              capture_output=True, text=True, timeout=10)
        status = subprocess.run([*git, "status", "--porcelain", "--untracked-files=normal"],
                                capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.SubprocessError):
        return dict(UNKNOWN)
    commit = head.stdout.strip() if head.returncode == 0 and isinstance(head.stdout, str) else ""
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or status.returncode or not isinstance(status.stdout, str):
        return dict(UNKNOWN)
    return {"commit": commit, "dirty": bool(status.stdout.strip())}


def binary_version(path):
    """The `--version` line of an installed Badi binary, or None."""
    try:
        result = subprocess.run([str(path), "--version"], capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        return None
    line = result.stdout.strip() if result.returncode == 0 and isinstance(result.stdout, str) else ""
    return line if VERSION_LINE.fullmatch(line) else None


def version_identity(line):
    """The source identity a binary embedded at build time, or unknown."""
    match = VERSION_LINE.fullmatch(line or "")
    if not match or match[1] == "unknown":
        return dict(UNKNOWN)
    return {"commit": match[1], "dirty": {"true": True, "false": False}.get(match[2])}


def write_receipt(home, installer, source, files, now=None):
    """Atomically record each installed file's digest and source identity.

    Badi binaries record the identity embedded in their own `--version` line,
    since an installer may copy one built from an earlier checkout. Entries for
    files this run did not install are kept with their own earlier identity, so
    a partial update never claims untouched files are current; so are entries
    whose recorded digest still matches, since those bytes were not rewritten.
    """
    home = Path(home)
    path = receipt_path(home, installer)
    moment = now or datetime.datetime.now(datetime.timezone.utc)
    installed_at = moment.astimezone(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    entries = {}
    try:
        previous = json.loads(path.read_text())
        if isinstance(previous, dict) and previous.get("schema") == SCHEMA and isinstance(previous.get("files"), dict):
            entries.update(previous["files"])
    except (OSError, ValueError):
        pass
    for target in dict.fromkeys(Path(item) for item in files):
        key, digest = home_relative(home, target), file_sha256(target)
        if isinstance(entries.get(key), dict) and entries[key].get("sha256") == digest:
            continue  # Bytes this run found already installed keep their recorded identity.
        entry = {"sha256": digest, "installed_at": installed_at,
                 "commit": source["commit"], "dirty": source["dirty"]}
        if target.name in VERSIONED:
            entry["version"] = binary_version(target)
            entry.update(version_identity(entry["version"]))
        entries[key] = entry
    receipt = {"schema": SCHEMA, "installer": installer, "installed_at": installed_at,
               "source": {"commit": source["commit"], "dirty": source["dirty"]},
               "files": dict(sorted(entries.items()))}
    store(path, receipt)
    return path


def refresh_restored_receipts(home, keys):
    """Point receipt entries at bytes a restore just put back.

    A rolled-back broker whose receipt still names the newer build makes
    `badi doctor` report broker_build_mismatch after the service next starts.
    A removed file drops out of the receipt. Other entries stay.
    """
    home = Path(home)
    keys = list(dict.fromkeys(keys))
    if not keys:
        return
    for installer in BACKUP_DIRECTORIES:
        path = receipt_path(home, installer)
        try:
            receipt = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        if not isinstance(receipt, dict) or receipt.get("schema") != SCHEMA or not isinstance(receipt.get("files"), dict):
            continue
        files = receipt["files"]
        changed = False
        for key in keys:
            entry = files.get(key)
            if not isinstance(entry, dict):
                continue
            target = home / key
            if not target.is_file() or target.is_symlink():
                del files[key]
            else:
                entry["sha256"] = file_sha256(target)
                if target.name in VERSIONED:
                    entry["version"] = binary_version(target)
                    entry.update(version_identity(entry.get("version")))
            changed = True
        if changed:
            store(path, receipt)


def announce_stopped(installation):
    """Name the backup and the restore command once, after a failed install."""
    directory = getattr(installation, "directory", None)
    if directory is None or getattr(installation, "_announced", False):
        return
    installation._announced = True
    print(f"Install stopped after replacing files. Backup: {directory}", file=sys.stderr, flush=True)
    print(f"Restore with: python3 scripts/badi_install.py restore {directory}", file=sys.stderr, flush=True)


def forget(home, installer, files):
    """Drop the entries of removed files, keeping the receipt's other entries
    and its own install time and source identity. A missing or invalid receipt
    is left as it is."""
    home = Path(home)
    path = receipt_path(home, installer)
    try:
        receipt = json.loads(path.read_text())
    except (OSError, ValueError):
        return
    if not isinstance(receipt, dict) or receipt.get("schema") != SCHEMA or not isinstance(receipt.get("files"), dict):
        return
    removed = {home_relative(home, item) for item in files}
    if removed.isdisjoint(receipt["files"]):
        return
    receipt["files"] = {key: entry for key, entry in receipt["files"].items() if key not in removed}
    store(path, receipt)


def store(path, receipt):
    """Atomically write a private receipt."""
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    atomic_write(path, (json.dumps(receipt, indent=2) + "\n").encode())


def backup_time(name):
    """The creation time in ns encoded by a backup directory name, or None."""
    try:
        if match := BACKUP_NAME.fullmatch(name):
            return calendar.timegm(time.strptime(match[1], "%Y%m%dT%H%M%S")) * 10**9 + int(match[2])
        if re.fullmatch(r"\d{19}", name):
            return int(name)
        if re.fullmatch(r"\d{8}-\d{6}", name):
            return int(time.mktime(time.strptime(name, "%Y%m%d-%H%M%S"))) * 10**9
    except (ValueError, OverflowError):
        pass
    return None


def backup_is_pinned(entry):
    """Whether this backup is the installer's original, pre-Badi snapshot."""
    marker = entry / "pinned"
    if marker.is_file() and not marker.is_symlink():
        return True
    try:
        document = json.loads((entry / BACKUP_MAP).read_text())
    except (OSError, ValueError):
        return False
    return isinstance(document, dict) and document.get("pinned") is True


def prune_backups(home, installer, keep=KEEP_BACKUPS):
    """Delete all but the newest `keep` backups of one installer; returns what was deleted.

    Only this installer's backup directory is searched, and only real,
    user-owned directories whose names are a backup time are candidates, so
    unrelated or unexpected entries are never removed. Prior-setting notes
    are moved to notes/ first.
    """
    root = Path(home) / STATE_DIRECTORY / BACKUP_DIRECTORIES[installer]
    if root.is_symlink() or not root.is_dir():
        return []
    backups = []
    for entry in root.iterdir():
        created = backup_time(entry.name)
        if created is None:
            continue
        info = entry.lstat()
        if stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid():
            backups.append((created, entry.name, entry))
    backups.sort()
    # The first pre-Badi backup is pinned, so keeping the newest three never
    # deletes the original .bashrc or Chromium flags.
    ordinary = [item for item in backups if not backup_is_pinned(item[2])]
    doomed = [entry for _created, _name, entry in ordinary[:max(0, len(ordinary) - keep)]]
    for entry in doomed:
        notes = [path for path in entry.glob(KEPT_NOTES) if path.is_file() and not path.is_symlink()]
        if notes:
            kept = root / "notes" / entry.name
            kept.mkdir(parents=True, exist_ok=True, mode=0o700)
            for path in notes:
                shutil.copy2(path, kept / path.name)
        shutil.rmtree(entry)
    return doomed


class Installation:
    """The file changes of one installer run, each backed up before it happens.

    A file whose bytes and mode (or link) already match is neither backed up
    nor rewritten. The backup directory appears with the first real change, as
    ~/.local/state/badi/{install,editor,ui}-backups/<UTC time>/ holding changes.json
    and each replaced or removed original under home/ (root/ for a path
    outside home). changes.json lists every change in order; `restore` undoes
    them. `current` lists every file that now holds this run's content.
    """

    def __init__(self, home, installer):
        self.home = Path(home)
        self.installer = installer
        self.directory = None
        self.changes = []
        self.current = []
        self._created_at = None
        self._pinned = False
        self._announced = False

    def copy(self, source, target, mode=None):
        """Install a file's bytes, with its own permissions unless `mode` is given."""
        source = Path(source)
        return self.write(target, source.read_bytes(), stat.S_IMODE(source.stat().st_mode) if mode is None else mode)

    def write(self, target, data, mode=0o644, *, replace_link=False):
        """Install `data` at `target`; True when the file changed."""
        target = Path(target)
        target.parent.mkdir(parents=True, exist_ok=True)
        info = self._existing(target, replace_link)
        self.current.append(target)
        if info is not None and stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == mode \
                and info.st_size == len(data) and target.read_bytes() == data:
            return False
        self._record(target, "replace" if info else "create", info, {"sha256": hashlib.sha256(data).hexdigest()})
        atomic_write(target, data, mode)
        return True

    def link(self, target, destination, *, replace_link=False):
        """Make `target` a symlink to `destination`; True when it changed."""
        target = Path(target)
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.is_symlink() and os.readlink(target) == destination:
            return False
        info = self._existing(target, replace_link)
        self._record(target, "replace" if info else "create", info, {"symlink": destination})
        _replace_with_link(target, destination)
        return True

    def remove(self, path):
        """Back up and delete one file or symlink (never what a symlink names)."""
        path = Path(path)
        info = path.lstat()
        if not (stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode)) or info.st_uid != os.getuid():
            raise RuntimeError(f"Unexpected file ownership/type: {path}")
        self._record(path, "remove", info, {})
        path.unlink()

    def note(self, name, document):
        """Keep a private JSON record, such as a prior setting.

        A note alone is not a backup: it lives under notes/ until this run has
        replaced a file, and then it is stored beside that backup's changes.
        """
        if self.changes:
            directory = self._backup_directory()
        else:
            directory = private_directory(self.home / STATE_DIRECTORY / BACKUP_DIRECTORIES[self.installer] / "notes",
                                          self.home / STATE_DIRECTORY)
        path = directory / name
        atomic_write(path, (json.dumps(document) + "\n").encode(), 0o600)
        return path

    def finish(self, keep=KEEP_BACKUPS):
        """Name the backup and prune older ones of this installer once its files are in place."""
        if self.directory:
            print(f"Replaced files and their rollback map: {self.directory}", flush=True)
        else:
            print("Every installed file was already current; no backup was needed.", flush=True)
        pruned = prune_backups(self.home, self.installer, keep)
        if pruned:
            print(f"Removed {len(pruned)} older {self.installer} backup(s); the newest {keep} are kept.", flush=True)
        return pruned

    def _existing(self, target, replace_link):
        try:
            info = target.lstat()
        except FileNotFoundError:
            return None
        if stat.S_ISLNK(info.st_mode):
            if not replace_link:
                raise RuntimeError(f"Inspect existing symlink before replacing {target}")
        elif not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise RuntimeError(f"Unexpected file ownership/type: {target}")
        return info

    def _record(self, target, action, info, installed):
        # The original is saved and the map written before the target changes,
        # so recovery stays discoverable even if this or a later step fails.
        directory = self._backup_directory()
        entry = {"path": home_relative(self.home, target), "action": action}
        if info is not None:
            key = entry["path"]
            saved = directory / ("root" + key if key.startswith("/") else "home/" + key)
            saved.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(target, saved, follow_symlinks=False)
            entry["saved"] = str(saved.relative_to(directory))
        self.changes.append({**entry, **installed})
        self._store_map()

    def _backup_directory(self):
        if self.directory is None:
            root = self.home / STATE_DIRECTORY / BACKUP_DIRECTORIES[self.installer]
            if root.is_symlink():
                raise RuntimeError(f"Inspect the symlinked backup directory {root}")
            private_directory(root, self.home / STATE_DIRECTORY)
            if root.is_symlink():
                raise RuntimeError(f"Inspect the symlinked backup directory {root}")
            first = not any(backup_time(entry.name) is not None and entry.is_dir() and not entry.is_symlink()
                            for entry in root.iterdir()) if root.is_dir() else True
            while self.directory is None:
                moment = time.time_ns()
                candidate = root / (time.strftime("%Y%m%dT%H%M%S", time.gmtime(moment // 10**9))
                                    + f".{moment % 10**9:09d}Z")
                try:
                    candidate.mkdir(mode=0o700)
                except FileExistsError:
                    continue
                self.directory = candidate
                self._pinned = first
                self._created_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(moment // 10**9))
                if first:
                    atomic_write(candidate / "pinned", b"")
            self._store_map()
        return self.directory

    def _store_map(self):
        atomic_write(self.directory / BACKUP_MAP, (json.dumps(
            {"schema": BACKUP_SCHEMA, "installer": self.installer, "home": str(self.home),
             "created_at": self._created_at, "pinned": self._pinned, "changes": self.changes}, indent=2) + "\n").encode())


def _replace_with_link(target, destination):
    staged = target.with_name(f".{target.name}.{os.getpid()}.link")
    staged.unlink(missing_ok=True)
    staged.symlink_to(destination)
    try:
        staged.replace(target)
    finally:
        staged.unlink(missing_ok=True)


def _left_by_installation(target, entry):
    """Whether `target` is still exactly what this installation left there."""
    if entry["action"] == "remove":
        return not os.path.lexists(target)
    if "symlink" in entry:
        return target.is_symlink() and os.readlink(target) == entry["symlink"]
    return not target.is_symlink() and target.is_file() and file_sha256(target) == entry.get("sha256")


def _holds_original(target, saved):
    """Whether `target` already is the original a backup saved (or, with none, absent)."""
    if saved is None:
        return not os.path.lexists(target)
    if saved.is_symlink():
        return target.is_symlink() and os.readlink(target) == os.readlink(saved)
    return not target.is_symlink() and target.is_file() and file_sha256(target) == file_sha256(saved)


def restore(backup, only=()):
    """Undo one backup's changes, newest first; returns the paths left unrestored.

    A path changed again since that installation (by a later installation or
    by hand) is reported instead of overwritten; one already holding its
    original is skipped, so a restore can be repeated. `only` limits the
    restore to these paths and everything below them.
    """
    backup = Path(backup)
    document = json.loads((backup / BACKUP_MAP).read_text())
    if not isinstance(document, dict) or document.get("schema") != BACKUP_SCHEMA:
        raise RuntimeError(f"{backup / BACKUP_MAP} is not a Badi installation backup map")
    home = Path(document["home"])
    prefixes = [item.rstrip("/") for item in only]
    changed = []
    restored = []
    reload_units = False
    for entry in reversed(document["changes"]):
        path = entry["path"]
        if prefixes and not any(path == prefix or path.startswith(prefix + "/") for prefix in prefixes):
            continue
        target = home / path
        saved = backup / entry["saved"] if entry.get("saved") else None
        if _holds_original(target, saved):
            continue
        if not _left_by_installation(target, entry):
            changed.append(path)
            print(f"Changed since that installation, left as it is: {target}", flush=True)
            continue
        if saved is None:
            target.unlink()
            print(f"Removed {target}", flush=True)
            restored.append(path)
            reload_units = reload_units or path.startswith(".config/systemd/user/")
        elif saved.is_symlink():
            target.parent.mkdir(parents=True, exist_ok=True)
            _replace_with_link(target, os.readlink(saved))
            print(f"Restored {target}", flush=True)
            restored.append(path)
            reload_units = reload_units or path.startswith(".config/systemd/user/")
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(dir=target.parent, prefix=f".{target.name}.", delete=False) as stream:
                staged = Path(stream.name)
            try:
                shutil.copy2(saved, staged)
                staged.replace(target)
            finally:
                staged.unlink(missing_ok=True)
            print(f"Restored {target}", flush=True)
            restored.append(path)
            reload_units = reload_units or path.startswith(".config/systemd/user/")
    refresh_restored_receipts(home, restored)
    if reload_units:
        subprocess.run(["systemctl", "--user", "daemon-reload"], check=True, timeout=15)
        print("Reloaded user units so restored service files are what systemd reads.", flush=True)
    return changed


def main(arguments=None):
    parser = argparse.ArgumentParser(description="Restore the files one Badi installation replaced.")
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("restore", help="Undo one backup's file changes; restore the newest backup first")
    command.add_argument("backup", type=Path, help="The backup directory an installer printed")
    command.add_argument("--only", action="append", default=[], metavar="PATH",
                         help="Restore only this home-relative path and what lies below it (repeatable)")
    args = parser.parse_args(arguments)
    changed = restore(args.backup, args.only)
    if changed:
        print(f"{len(changed)} path(s) changed after that installation; inspect them against {args.backup}.",
              file=sys.stderr)
        raise SystemExit(1)


if __name__ == "__main__":
    main()


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


# Omarchy's own commands replace these files wholesale (omarchy-refresh-hyprland,
# omarchy-refresh-chromium, omarchy-install-browser and its migrations), so its
# hooks restore Badi's lines in them; other files are the user's to change.
OMARCHY_REWRITTEN = ("hypr/hyprland.lua", "chromium-flags.conf", "brave-origin-flags.conf")


def dropped_lines(home):
    """Badi's lines missing from files the desktop install changed, as
    (name, path, repaired text): each observed app's accessibility flag and
    the require in hyprland.lua. A file that is gone, a symlink, or now
    conflicts with the flag is left alone."""
    home = Path(home)
    try:
        files = set(json.loads(receipt_path(home, "desktop").read_text()).get("files", []))
    except (OSError, ValueError, AttributeError):
        return []
    config = config_home(home)
    found = []
    for name, app in [*((filename, app) for app, (filename, _) in OBSERVED_APP_FLAGS.items()),
                      ("hypr/hyprland.lua", None)]:
        path = config / name
        if home_relative(home, path) not in files or not path.is_file() or path.is_symlink():
            continue
        try:
            text = path.read_text()
            if app is None:
                updated = hyprland_with_badi(text) if (config / "hypr/badi.lua").is_file() else None
            else:
                updated = observed_flags(text, app)
        except (OSError, UnicodeDecodeError, RuntimeError):
            continue
        if updated is not None:
            found.append((name, path, updated))
    return found


def repair(home, omarchy_only=False):
    """Restore the dropped lines, appending only Badi's; the names restored."""
    restored = []
    for name, path, updated in dropped_lines(home):
        if omarchy_only and name not in OMARCHY_REWRITTEN:
            continue
        atomic_write(path, updated.encode(), stat.S_IMODE(path.stat().st_mode))
        restored.append(name)
    return restored


# Writing models that are not Apache-2.0 come with terms a person accepts
# before the download; the catalog's licence names them (badi.model-advice.v3).
MODEL_TERMS = {
    "Gemma": ("the Gemma Terms of Use", "https://ai.google.dev/gemma/terms"),
    "Llama-3.2-Community": ("the Llama 3.2 Community License", "https://www.llama.com/llama3_2/license/"),
}
MODEL_TERMS_RECORD = STATE_DIRECTORY / "model-terms.json"


def model_terms(model):
    """'<name>, <url>' of the terms that come with a catalog model, or None."""
    terms = MODEL_TERMS.get(model.get("license"))
    return f"{terms[0]}, {terms[1]}" if terms else None


def record_model_terms(home, model, now=None):
    """Note that the user accepted a model's terms: the model, its licence,
    the terms' address and when. Nothing is recorded for an Apache-2.0 model."""
    terms = MODEL_TERMS.get(model.get("license"))
    if terms is None:
        return None
    path = Path(home) / MODEL_TERMS_RECORD
    try:
        document = json.loads(path.read_text())
        accepted = document["accepted"]
        if not isinstance(accepted, list):
            raise ValueError
    except (OSError, ValueError, KeyError, TypeError):
        accepted = []
    moment = (now or datetime.datetime.now(datetime.timezone.utc)).astimezone(datetime.timezone.utc)
    accepted.append({"model": model["id"], "license": model["license"], "terms": terms[1],
                     "accepted_at": moment.strftime("%Y-%m-%dT%H:%M:%SZ")})
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    atomic_write(path, (json.dumps({"schema": "badi.model-terms.v1", "accepted": accepted}, indent=2) + "\n").encode())
    return path


def download_model(model, directory):
    """The pinned file at its pinned revision, checked by size and SHA-256
    before it becomes visible to the broker."""
    import urllib.request
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    target = directory / model["filename"]
    partial = target.with_name(target.name + ".part")
    url = f"https://huggingface.co/{model['repository']}/resolve/{model['revision']}/{model['filename']}"
    digest = hashlib.sha256()
    received = 0
    with urllib.request.urlopen(url, timeout=60) as response, open(partial, "wb") as out:
        os.fchmod(out.fileno(), 0o600)
        while chunk := response.read(1 << 20):
            out.write(chunk)
            digest.update(chunk)
            received += len(chunk)
            if received > model["download_bytes"]:
                break
            if sys.stdout.isatty():
                print(f"\rDownloading {model['name']}: {100 * received // model['download_bytes']}%", end="", flush=True)
    if sys.stdout.isatty():
        print()
    if received != model["download_bytes"] or digest.hexdigest() != model["sha256"]:
        partial.unlink(missing_ok=True)
        raise RuntimeError(f"{model['filename']} did not match its pinned size and hash; nothing was installed.")
    partial.rename(target)
    return target
