#!/usr/bin/env python3
"""Install the tested, user-local Obsidian and Bash integrations."""

import argparse
import json
from pathlib import Path
import subprocess

import badi_install

ROOT = Path(__file__).resolve().parents[1]


def build_shell_preview():
    subprocess.run(["python3", "adapters/shell/build-preview.py"], cwd=ROOT, check=True)
    return (ROOT / "adapters/shell/build/badi-preview.so").read_bytes()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vault", type=Path, help="Existing Obsidian vault to install into")
    parser.add_argument("--bash", action="store_true", help="Add the Badi hook to this user's Bash rc")
    args = parser.parse_args()
    if not args.vault and not args.bash:
        parser.error("Select --vault PATH and/or --bash")
    checkout = badi_install.source_identity(ROOT)
    # Finish the native build before touching a user's shell or installation.
    shell_preview = build_shell_preview() if args.bash else None
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
            installation.write(community, (json.dumps([*enabled, "badi"], indent=2) + "\n").encode(), 0o644)
        print("Obsidian installed. Reload Obsidian; enable community plugins normally if restricted mode is on.")
    if args.bash:
        installation.write(home / ".local/lib/badi/editors/shell/badi-preview.so", shell_preview, 0o755)
        for path in ("shared/broker-client.mjs", "shared/activity.mjs", "shared/writing-language.mjs", "shared/text-safety.mjs",
                     "shell/bridge.mjs", "shell/badi.bash"):
            installation.copy(ROOT / "adapters" / path, home / ".local/lib/badi/editors" / path, 0o644)
        bashrc = home / ".bashrc"
        text = bashrc.read_text() if bashrc.exists() else ""
        source = '[[ -r "$HOME/.local/lib/badi/editors/shell/badi.bash" ]] && source "$HOME/.local/lib/badi/editors/shell/badi.bash"'
        if source not in text:
            installation.write(bashrc, (text.rstrip() + "\n\n" + source + "\n").encode(), 0o644)
        subprocess.run(["bash", "-n", str(bashrc)], check=True)
        print("Bash installed with grey inline previews. New interactive shells: Ctrl-X then Tab requests/accepts; ordinary Tab is unchanged.")
    installation.finish()
    print(f"Install receipt: {badi_install.write_receipt(home, 'editors', checkout, installation.current)}")


if __name__ == "__main__":
    main()
