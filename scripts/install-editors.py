#!/usr/bin/env python3
"""Install the tested, user-local Obsidian and Bash integrations."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def native_host_directories(config):
    browsers = ("chromium", "google-chrome", "google-chrome-beta", "google-chrome-unstable",
                "BraveSoftware/Brave-Browser", "BraveSoftware/Brave-Browser-Beta",
                "BraveSoftware/Brave-Browser-Nightly", "BraveSoftware/Brave-Origin")
    return [config / browser / "NativeMessagingHosts" for browser in browsers
            if browser == "chromium" or (config / browser).is_dir()]


def build_shell_preview():
    subprocess.run(["python3", "adapters/shell/build-preview.py"], cwd=ROOT, check=True)
    return (ROOT / "adapters/shell/build/badi-preview.so").read_bytes()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vault", type=Path, help="Existing Obsidian vault to install into")
    parser.add_argument("--bash", action="store_true", help="Add the Badi hook to this user's Bash rc")
    parser.add_argument("--chromium", action="store_true", help="Install the ordinary website extension and native host")
    args = parser.parse_args()
    if not args.vault and not args.bash and not args.chromium:
        parser.error("Select --vault PATH, --bash and/or --chromium")
    # Finish the native build before touching a user's shell or installation.
    shell_preview = build_shell_preview() if args.bash else None
    home = Path.home()
    backup = home / ".local/state/badi/editor-backups" / str(time.time_ns())
    backup.mkdir(parents=True, mode=0o700)
    changed = []

    def install_bytes(data, destination, mode=0o644):
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.is_symlink():
            raise RuntimeError(f"Inspect existing symlink before replacing {destination}")
        index = str(len(changed))
        if destination.exists():
            if destination.stat().st_uid != os.getuid() or not destination.is_file():
                raise RuntimeError(f"Unexpected file ownership/type: {destination}")
            shutil.copy2(destination, backup / index)
        else:
            index = None
        changed.append({"path": str(destination), "backup": index})
        (backup / "changes.json").write_text(json.dumps(changed, indent=2) + "\n")
        with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(data)
            os.fchmod(stream.fileno(), mode)
        try:
            temporary.replace(destination)
        finally:
            temporary.unlink(missing_ok=True)

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
            install_bytes((ROOT / "adapters/obsidian/dist" / name).read_bytes(), plugin / name)
        if "badi" not in enabled:
            install_bytes((json.dumps([*enabled, "badi"], indent=2) + "\n").encode(), community)
        print("Obsidian installed. Reload Obsidian; enable community plugins normally if restricted mode is on.")
    if args.bash:
        install_bytes(shell_preview,
                      home / ".local/lib/badi/editors/shell/badi-preview.so", 0o755)
        for path in ("shared/broker-client.mjs", "shared/activity.mjs", "shared/writing-language.mjs", "shared/text-safety.mjs",
                     "shell/bridge.mjs", "shell/badi.bash"):
            install_bytes((ROOT / "adapters" / path).read_bytes(), home / ".local/lib/badi/editors" / path)
        bashrc = home / ".bashrc"
        text = bashrc.read_text() if bashrc.exists() else ""
        source = '[[ -r "$HOME/.local/lib/badi/editors/shell/badi.bash" ]] && source "$HOME/.local/lib/badi/editors/shell/badi.bash"'
        if source not in text:
            install_bytes((text.rstrip() + "\n\n" + source + "\n").encode(), bashrc)
        subprocess.run(["bash", "-n", str(bashrc)], check=True)
        print("Bash installed with grey inline previews. New interactive shells: Ctrl-X then Tab requests/accepts; ordinary Tab is unchanged.")
    if args.chromium:
        host = ROOT / "target/release/badi-native-host"
        renderer = ROOT / "target/release/badi-native-manifest"
        if not host.is_file() or not renderer.is_file():
            raise RuntimeError("Run the desktop installer first to build the native host")
        subprocess.run(["npm", "run", "build:web", "--workspace", "@badi/chromium"], cwd=ROOT, check=True)
        destination = home / ".local/lib/badi/chromium"
        for source in sorted((ROOT / "adapters/chromium/dist-web").iterdir()):
            if source.is_file():
                install_bytes(source.read_bytes(), destination / source.name)
        installed_host = home / ".local/lib/badi/badi-native-host"
        install_bytes(host.read_bytes(), installed_host, 0o755)
        manifest = subprocess.run([str(renderer), "--host-path", str(installed_host)],
                                  check=True, capture_output=True).stdout
        config = Path(os.environ.get("XDG_CONFIG_HOME", home / ".config"))
        for directory in native_host_directories(config):
            install_bytes(manifest, directory / "io.github.ahuray.badi.json", 0o600)
        print(f"Chromium files installed. Load unpacked from {destination} in chrome://extensions.")
        print("Enable the site in the Badi popup and grant its exact origin with badi site ORIGIN on.")
    print(f"Rollback map and original files: {backup}")


if __name__ == "__main__":
    main()
