"""Shared contracts of Badi's user-local installers and its desktop CLI.

Installed next to the `badi` CLI, which imports it; the installers import it
from the checkout. It holds content-free install receipts, atomic file writes,
the systemd and lock-state queries both sides make, and the grant shape of a
settings subject.
"""

import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

SCHEMA = "badi.install-receipt.v1"
RECEIPT_DIRECTORY = Path(".local/state/badi/receipts")
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
    a partial update never claims untouched files are current.
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
        entry = {"sha256": file_sha256(target), "installed_at": installed_at,
                 "commit": source["commit"], "dirty": source["dirty"]}
        if target.name in VERSIONED:
            entry["version"] = binary_version(target)
            entry.update(version_identity(entry["version"]))
        entries[home_relative(home, target)] = entry
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
