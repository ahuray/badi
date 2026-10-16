#!/usr/bin/env python3
"""Install the tested, user-local Obsidian, Bash, Neovim and LibreOffice Writer integrations."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tempfile

import badi_install

ROOT = Path(__file__).resolve().parents[1]
LIBREOFFICE_EXTENSION = "io.github.ahuray.badi.libreoffice"
LIBREOFFICE_PACKAGE = ".local/lib/badi/editors/libreoffice/badi-libreoffice.oxt"


def build_shell_preview():
    subprocess.run(["python3", "adapters/shell/build-preview.py"], cwd=ROOT, check=True)
    return (ROOT / "adapters/shell/build/badi-preview.so").read_bytes()


SHARED_CLIENT = ("shared/broker-client.mjs", "shared/activity.mjs", "shared/writing-language.mjs",
                 "shared/text-safety.mjs")
NEOVIM_PLUGIN = ("plugin/badi.lua", "lua/badi/init.lua", "lua/badi/context.lua", "lua/badi/bridge.lua")


def neovim_package(home):
    """Neovim's own start package directory: stdpath('data')/site/pack."""
    data = os.environ.get("XDG_DATA_HOME", "")
    base = Path(data) if os.path.isabs(data) else home / ".local/share"
    appname = os.environ.get("NVIM_APPNAME") or "nvim"
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}", appname) or appname in (".", ".."):
        raise RuntimeError("NVIM_APPNAME must be a plain directory name")
    return base / appname / "site/pack/badi/start/badi"


def build_libreoffice_extension():
    with tempfile.TemporaryDirectory(prefix="badi-libreoffice-") as directory:
        output = Path(directory) / "badi-libreoffice.oxt"
        subprocess.run(["python3", "-B", "adapters/libreoffice/build.py", str(output)], cwd=ROOT,
                       check=True, stdout=subprocess.DEVNULL)
        return output.read_bytes()


def libreoffice_running():
    """unopkg changes the profile that a running LibreOffice also writes."""
    if shutil.which("pgrep") is None:
        raise RuntimeError("Cannot tell whether LibreOffice is running (pgrep is missing)")
    return subprocess.run(["pgrep", "-u", str(os.getuid()), "-x", "soffice.bin"],
                          stdout=subprocess.DEVNULL, check=False).returncode == 0


def register_libreoffice_extension(unopkg, package):
    # --force replaces an earlier Badi version with the same identifier.
    subprocess.run([unopkg, "add", "--force", "--suppress-license", str(package)], check=True,
                   timeout=180)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vault", type=Path, help="Existing Obsidian vault to install into")
    parser.add_argument("--bash", action="store_true", help="Add the Badi hook to this user's Bash rc")
    parser.add_argument("--neovim", action="store_true", help="Install the Neovim plugin as a start package")
    parser.add_argument("--libreoffice", action="store_true",
                        help="Add the Writer spelling extension to this user's LibreOffice profile")
    args = parser.parse_args()
    if not args.vault and not args.bash and not args.neovim and not args.libreoffice:
        parser.error("Select --vault PATH, --bash, --neovim and/or --libreoffice")
    checkout = badi_install.source_identity(ROOT)
    unopkg = None
    if args.libreoffice:
        unopkg = shutil.which("unopkg")
        if unopkg is None:
            raise RuntimeError("unopkg was not found; install LibreOffice first")
        if libreoffice_running():
            raise RuntimeError("Close LibreOffice first; its profile cannot change while it runs")
    # Finish every build before touching a user's shell, profile or installation.
    shell_preview = build_shell_preview() if args.bash else None
    extension = build_libreoffice_extension() if args.libreoffice else None
    home = Path.home()
    installation = badi_install.Installation(home, "editors")

    if args.vault:
        vault = args.vault.resolve(strict=True)
        config = vault / ".obsidian"
        if not config.is_dir() or config.is_symlink():
            raise RuntimeError("Select an existing local Obsidian vault")
        plugin = config / "plugins/badi"
        manifest = plugin / "manifest.json"
        if plugin.exists() and (plugin.is_symlink() or not manifest.is_file() or
                                json.loads(manifest.read_text()).get("id") != "badi"):
            raise RuntimeError("An unexpected plugin occupies the Badi installation path")
        community = config / "community-plugins.json"
        enabled = json.loads(community.read_text()) if community.exists() else []
        if not isinstance(enabled, list) or not all(isinstance(item, str) for item in enabled):
            raise RuntimeError("Invalid Obsidian community plugin registry")
        subprocess.run(["node", "adapters/obsidian/build.mjs"], cwd=ROOT, check=True)
        for name in ("main.js", "manifest.json", "styles.css"):
            installation.copy(ROOT / "adapters/obsidian/dist" / name, plugin / name, 0o644)
        if "badi" not in enabled:
            installation.write(community, (json.dumps([*enabled, "badi"], indent=2) + "\n").encode(),
                               stat.S_IMODE(community.stat().st_mode) if community.exists() else 0o644)
        print("Obsidian installed. Reload Obsidian; enable community plugins normally if restricted mode is on.")
    if args.bash:
        installation.write(home / ".local/lib/badi/editors/shell/badi-preview.so", shell_preview, 0o755)
        for path in (*SHARED_CLIENT, "shell/bridge.mjs", "shell/badi.bash"):
            installation.copy(ROOT / "adapters" / path, home / ".local/lib/badi/editors" / path, 0o644)
        bashrc = home / ".bashrc"
        text = bashrc.read_text() if bashrc.exists() else ""
        source = '[[ -r "$HOME/.local/lib/badi/editors/shell/badi.bash" ]] && source "$HOME/.local/lib/badi/editors/shell/badi.bash"'
        if source not in text:
            mode = stat.S_IMODE(bashrc.stat().st_mode) if bashrc.exists() else 0o644
            installation.write(bashrc, (text.rstrip() + "\n\n" + source + "\n").encode(), mode)
        subprocess.run(["bash", "-n", str(bashrc)], check=True)
        print("Bash installed with grey inline previews. New interactive shells: Ctrl-X then Tab requests/accepts; ordinary Tab is unchanged.")
    if args.neovim:
        package = neovim_package(home)
        # A linked development checkout stays the user's; never write through it.
        for path in (package, package.parent, package.parent.parent):
            if path.is_symlink():
                raise RuntimeError(f"A symlink occupies the Neovim package path: {path}")
        if package.exists() and not (package / "lua/badi/init.lua").is_file():
            raise RuntimeError("An unexpected package occupies the Badi Neovim installation path")
        for path in NEOVIM_PLUGIN:
            installation.copy(ROOT / "adapters/neovim" / path, package / path, 0o644)
        for path in (*SHARED_CLIENT, "shared/recovering-client.mjs", "neovim/bridge.mjs"):
            installation.copy(ROOT / "adapters" / path, home / ".local/lib/badi/editors" / path, 0o644)
        note = "" if shutil.which("nvim") else " Neovim 0.10 or newer was not found on PATH."
        print("Neovim installed as a start package. Restart Neovim; prose buffers suggest after a typing pause, "
              f"Tab accepts a word, Ctrl+Right accepts all. Allow it with: badi app nvim on.{note}")
    if args.libreoffice:
        package = home / LIBREOFFICE_PACKAGE
        installation.write(package, extension, 0o644)
        # unopkg keeps its own copy in the LibreOffice profile; restoring the
        # file above does not unregister it.
        installation.note("libreoffice-extension.json", {
            "identifier": LIBREOFFICE_EXTENSION, "package": LIBREOFFICE_PACKAGE,
            "remove": f"unopkg remove {LIBREOFFICE_EXTENSION}"})
        register_libreoffice_extension(unopkg, package)
        print("LibreOffice Writer extension installed. Allow it with: badi app libreoffice-writer on. "
              "In Writer, Alt+F7 shows a correction for the word before the caret; Alt+F7 again replaces it. "
              f"Remove it with: unopkg remove {LIBREOFFICE_EXTENSION}")
    installation.finish()
    print(f"Install receipt: {badi_install.write_receipt(home, 'editors', checkout, installation.current)}")


if __name__ == "__main__":
    main()
