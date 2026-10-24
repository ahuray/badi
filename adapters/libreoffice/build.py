#!/usr/bin/env python3
"""Build the Writer extension (.oxt) from adapters/libreoffice/extension.

The archive is reproducible: sorted entries, fixed timestamps and modes, and
no bytecode caches. `python3 adapters/libreoffice/build.py [OUTPUT]` prints
the path and SHA-256 of the result.
"""

import hashlib
from pathlib import Path
import sys
import zipfile

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT / "extension"
DEFAULT_OUTPUT = ROOT / "build" / "badi-libreoffice.oxt"
FIXED_TIME = (2026, 1, 1, 0, 0, 0)


def sources():
    files = []
    for path in sorted(SOURCE.rglob("*")):
        relative = path.relative_to(SOURCE)
        if path.is_dir() or "__pycache__" in relative.parts or path.suffix in (".pyc", ".pyo"):
            continue
        if path.is_symlink() or not path.is_file():
            raise RuntimeError(f"Unexpected extension source: {relative}")
        files.append((relative.as_posix(), path))
    required = {"META-INF/manifest.xml", "description.xml", "badi.components", "badi_writer.py"}
    missing = required - {name for name, _ in files}
    if missing:
        raise RuntimeError(f"Extension source is incomplete: {', '.join(sorted(missing))}")
    return files


def build(output=DEFAULT_OUTPUT):
    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(output.name + ".tmp")
    with zipfile.ZipFile(temporary, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, path in sources():
            entry = zipfile.ZipInfo(name, FIXED_TIME)
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = 0o644 << 16
            archive.writestr(entry, path.read_bytes())
    temporary.replace(output)
    return output


def main(arguments):
    if len(arguments) > 1:
        raise SystemExit("Usage: build.py [OUTPUT.oxt]")
    output = build(Path(arguments[0]) if arguments else DEFAULT_OUTPUT)
    print(f"{output} sha256={hashlib.sha256(output.read_bytes()).hexdigest()}")


if __name__ == "__main__":
    main(sys.argv[1:])
