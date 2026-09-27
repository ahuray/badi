"""Content-free install receipts binding installed files to their source checkout."""

import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

SCHEMA = "badi.install-receipt.v1"
RECEIPT_DIRECTORY = Path(".local/state/badi/receipts")
VERSIONED = frozenset(("badi-broker", "badictl"))
VERSION_LINE = re.compile(r"[a-z][a-z-]* \S+ commit=([0-9a-f]{40}|unknown) dirty=(true|false|unknown)")
UNKNOWN = {"commit": "unknown", "dirty": None}


def receipt_path(home, installer):
    return Path(home) / RECEIPT_DIRECTORY / f"{installer}.json"


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


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


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
        key = str(target.relative_to(home)) if target.is_relative_to(home) else str(target)
        entry = {"sha256": file_sha256(target), "installed_at": installed_at,
                 "commit": source["commit"], "dirty": source["dirty"]}
        if target.name in VERSIONED:
            entry["version"] = binary_version(target)
            entry.update(version_identity(entry["version"]))
        entries[key] = entry
    receipt = {"schema": SCHEMA, "installer": installer, "installed_at": installed_at,
               "source": {"commit": source["commit"], "dirty": source["dirty"]},
               "files": dict(sorted(entries.items()))}
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with tempfile.NamedTemporaryFile("w", dir=path.parent, delete=False) as stream:
        staged = Path(stream.name)
        os.fchmod(stream.fileno(), 0o600)
        json.dump(receipt, stream, indent=2)
        stream.write("\n")
    try:
        staged.replace(path)
    finally:
        staged.unlink(missing_ok=True)
    return path
