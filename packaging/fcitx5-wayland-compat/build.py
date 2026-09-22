#!/usr/bin/env python3
"""Build a pinned opt-in frontend compatibility module; never install it."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import posixpath
import shutil
import subprocess
import sys
import tarfile
import urllib.request

from launch import RUNTIME_FILES

HERE = Path(__file__).resolve().parent
VERSION = "5.1.21"
SOURCE_COMMIT = "1319952f284eae17a36cba9e800843ca61a163c1"


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def installed_versions() -> dict[str, str]:
    versions = {}
    for package in ("Fcitx5Core", "Fcitx5Config", "Fcitx5Utils"):
        version = subprocess.check_output(
            ["pkg-config", "--modversion", package], text=True
        ).strip()
        if version != VERSION:
            raise RuntimeError(f"{package} must be exactly {VERSION}; found {version}")
        versions[package] = version
    return versions


def verify_file(path: Path, expected_sha256: str, expected_bytes: int | None = None) -> None:
    if expected_bytes is not None and path.stat().st_size != expected_bytes:
        raise RuntimeError(f"Unexpected byte count for {path.name}")
    if digest(path) != expected_sha256:
        raise RuntimeError(f"SHA256 mismatch for {path.name}")


def extract_source(archive: Path, destination: Path) -> None:
    prefix = f"fcitx5-{SOURCE_COMMIT}"
    with tarfile.open(archive, "r:gz") as source:
        for member in source.getmembers():
            path = Path(member.name)
            if path.is_absolute() or ".." in path.parts or not path.parts or path.parts[0] != prefix:
                raise RuntimeError("Source archive has an unexpected path")
            if member.issym() or member.islnk():
                base = posixpath.dirname(member.name) if member.issym() else ""
                target = Path(posixpath.normpath(posixpath.join(base, member.linkname)))
                if target.is_absolute() or not target.parts or target.parts[0] != prefix:
                    raise RuntimeError("Source archive link escapes its source root")
            elif not (member.isfile() or member.isdir()):
                raise RuntimeError("Source archive contains a special file")
        source.extractall(destination, filter="data")
    (destination / prefix).rename(destination / "fcitx5")


def execute(args: list[str], *, cwd: Path | None = None, data: bytes | None = None) -> None:
    subprocess.run(args, cwd=cwd, input=data, check=True)


def build(args: argparse.Namespace) -> Path:
    if sys.version_info < (3, 12):
        raise RuntimeError("Python 3.12 or newer is required for bounded archive extraction")
    manifest = json.loads((HERE / "manifest.json").read_text())
    if manifest["upstream_version"] != VERSION or manifest["upstream_commit"] != SOURCE_COMMIT:
        raise RuntimeError("Unexpected source manifest version")
    verify_file(HERE / "idle-done-ack.patch", manifest["patch_sha256"])
    if any(os.environ.get(key) for key in ("LD_LIBRARY_PATH", "LD_PRELOAD", "LD_AUDIT", "CMAKE_PREFIX_PATH", "PKG_CONFIG_PATH", "PKG_CONFIG_LIBDIR")):
        raise RuntimeError("Build from the standard system libraries without loader or package-path overrides")
    for executable in ("cmake", "ninja", "pkg-config", "patch", "wayland-scanner", "c++"):
        if shutil.which(executable) is None:
            raise RuntimeError(f"Required build command is missing: {executable}")
    if args.check:
        for executable in ("dbus-run-session", "fcitx5"):
            if shutil.which(executable) is None:
                raise RuntimeError(f"Required protocol-test command is missing: {executable}")
    versions = installed_versions()
    if subprocess.check_output(["/usr/bin/fcitx5", "--version"], text=True).strip() != VERSION:
        raise RuntimeError("The system Fcitx executable must be exactly 5.1.21")
    runtime_hashes = {path: digest(Path(path)) for path in RUNTIME_FILES}
    root = args.work_dir.expanduser().resolve()
    if root.exists():
        raise RuntimeError("Choose a new work directory; existing files are never replaced")
    if args.archive:
        verify_file(args.archive, manifest["source_sha256"], manifest["source_bytes"])
    root.mkdir(parents=True, mode=0o700)
    archive = root / "fcitx5-source.tar.gz"
    if args.archive:
        shutil.copyfile(args.archive, archive)
    else:
        with urllib.request.urlopen(manifest["source_url"], timeout=30) as response, archive.open("xb") as output:
            total = 0
            while chunk := response.read(1024 * 1024):
                total += len(chunk)
                if total > manifest["source_bytes"]:
                    raise RuntimeError("Source download exceeds its pinned byte count")
                output.write(chunk)
    verify_file(archive, manifest["source_sha256"], manifest["source_bytes"])
    extract_source(archive, root)
    source = root / "fcitx5"
    build_dir = root / "build"
    execute(["cmake", "-S", str(HERE), "-B", str(build_dir), "-G", "Ninja",
             "-DCMAKE_BUILD_TYPE=Release", f"-DFCITX_UPSTREAM_SOURCE={source}",
             "-DCMAKE_FIND_USE_PACKAGE_REGISTRY=OFF", "-DCMAKE_FIND_USE_SYSTEM_PACKAGE_REGISTRY=OFF",
             f"-DBADI_COMPAT_BUILD_TESTS={'ON' if args.check else 'OFF'}"])
    compile_command = ["cmake", "--build", str(build_dir), "--parallel", str(args.jobs)]
    if args.check:
        execute(compile_command)
        shutil.copyfile(build_dir / "libwaylandim.so", root / "libwaylandim-baseline.so")
    execute(["patch", "--batch", "--forward", "--fuzz=0", "-p1"],
            cwd=source, data=(HERE / "idle-done-ack.patch").read_bytes())
    execute(compile_command)
    artifact = root / "libwaylandim.so"
    shutil.copyfile(build_dir / "libwaylandim.so", artifact)
    if args.check:
        test = [sys.executable, str(HERE / "tests/run-protocol.py"), "--work-dir", str(root)]
        execute([*test, "--baseline"])
        execute([*test, "--disabled"])
        execute(test)
    if installed_versions() != versions or any(digest(Path(path)) != expected for path, expected in runtime_hashes.items()):
        raise RuntimeError("System Fcitx changed during the build; no installable receipt was produced")
    receipt = {
        "schema": "badi.fcitx-wayland-compat-build.v1",
        "source": manifest,
        "installed_build_versions": versions,
        "runtime_files_sha256": runtime_hashes,
        "artifact_sha256": digest(artifact),
        "artifact_bytes": artifact.stat().st_size,
        "protocol_checks_passed": args.check,
        "physical_editor_verified": False,
        "installed": False,
    }
    (root / "build-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return artifact


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-dir", type=Path, required=True, help="new disposable source/build directory")
    parser.add_argument("--archive", type=Path, help="offline copy of the exact pinned source archive")
    parser.add_argument("--jobs", type=int, choices=range(1, 5), default=2)
    parser.add_argument("--check", action="store_true", help="prove baseline, opt-out and guarded protocol behavior")
    args = parser.parse_args()
    try:
        artifact = build(args)
    except (OSError, RuntimeError, subprocess.CalledProcessError, tarfile.TarError) as error:
        parser.exit(1, f"Compatibility build failed: {error}\n")
    print(json.dumps({"artifact": str(artifact), "installed": False}))


if __name__ == "__main__":
    main()
