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
    doomed = [entry for _created, _name, entry in backups[:max(0, len(backups) - keep)]]
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
        """Keep a private JSON record beside the changes, such as a prior setting."""
        path = self._backup_directory() / name
        with path.open("x") as stream:
            os.chmod(path, 0o600)
            json.dump(document, stream)
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
            root.mkdir(parents=True, exist_ok=True, mode=0o700)
            if root.is_symlink():
                raise RuntimeError(f"Inspect the symlinked backup directory {root}")
            while self.directory is None:
                moment = time.time_ns()
                candidate = root / (time.strftime("%Y%m%dT%H%M%S", time.gmtime(moment // 10**9))
                                    + f".{moment % 10**9:09d}Z")
                try:
                    candidate.mkdir(mode=0o700)
                except FileExistsError:
                    continue
                self.directory = candidate
                self._created_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(moment // 10**9))
            self._store_map()
        return self.directory

    def _store_map(self):
        atomic_write(self.directory / BACKUP_MAP, (json.dumps(
            {"schema": BACKUP_SCHEMA, "installer": self.installer, "home": str(self.home),
             "created_at": self._created_at, "changes": self.changes}, indent=2) + "\n").encode())


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
        elif saved.is_symlink():
            target.parent.mkdir(parents=True, exist_ok=True)
            _replace_with_link(target, os.readlink(saved))
            print(f"Restored {target}", flush=True)
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
