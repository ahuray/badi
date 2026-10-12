# Arch package

`PKGBUILD` is a local `badi-git` candidate, not an AUR publication. It follows
`main`; the source changes it needs must land before publication. Build with
`makepkg` in this directory. Rust dependencies are fetched with the lockfile
in `prepare()`, then builds and tests run offline. The active Arch stable Rust
must satisfy the repository's 1.85 MSRV. This package targets x86-64 only.

The package installs the broker, controls and observer in `/usr/lib/badi`,
the Fcitx addon in `/usr/lib/fcitx5`, and user units in
`/usr/lib/systemd/user`. It includes the MIT and complete modified SCOWL
notices. It does not contain model weights, llama.cpp, the Prediction Lab or
editor plugins. No install hook starts a service, changes accessibility,
edits a home directory or downloads anything.

The current desktop contract still requires Omarchy/Hyprland and its lock
IPC. This is not general Linux desktop support. Fcitx 5.1.22 needs the
[verified Wayland frontend backport](https://github.com/ahuray/badi/blob/main/packaging/fcitx5-wayland-compat/README.md)
for reliable Chromium surrounding-text updates; Fcitx 5.1.23 includes that
fix. This package does not replace the system frontend.

## Writing assets

Download separately as the desktop user, never as root. `badictl hardware`
and `badictl models writing` report fit and pinned model advice. The following
first-install commands fetch the balanced Qwen3-1.7B model (Apache-2.0) and
the pinned llama.cpp CPU bundle (MIT). They refuse to overwrite existing
assets. The broker independently verifies the model, archive, executable and
bundle manifest before launching; an arbitrary system `llama-server` is not
interchangeable.

```bash
set -euo pipefail
umask 077
badi_data="$HOME/.local/share/badi"
mkdir -p "$badi_data/models" "$badi_data/runtimes/downloads"
test ! -e "$badi_data/models/Qwen3-1.7B-Q4_K_M.gguf"
test ! -e "$badi_data/runtimes/llama-b10726"
test ! -e "$badi_data/runtimes/downloads/llama-b10726-bin-ubuntu-x64.tar.gz"
badi_stage=$(mktemp -d "$badi_data/.download.XXXXXXXX")
trap 'rm -rf -- "$badi_stage"' EXIT
curl --fail --location --connect-timeout 20 --max-time 1800 \
  --output "$badi_stage/Qwen3-1.7B-Q4_K_M.gguf" \
  'https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF/resolve/daeb8e2d528a760970442092f6bf1e55c3b659eb/Qwen3-1.7B-Q4_K_M.gguf'
curl --fail --location --connect-timeout 20 --max-time 300 \
  --output "$badi_stage/llama-b10726-bin-ubuntu-x64.tar.gz" \
  'https://github.com/ggml-org/llama.cpp/releases/download/b10726/llama-b10726-bin-ubuntu-x64.tar.gz'
(
  cd "$badi_stage"
  sha256sum --check <<'SUMS'
d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5  Qwen3-1.7B-Q4_K_M.gguf
d3c4e406b2911c8c75d2d0858459645960f8f592c1ab372d565cf145b870c901  llama-b10726-bin-ubuntu-x64.tar.gz
SUMS
  tar --extract --gzip --file llama-b10726-bin-ubuntu-x64.tar.gz --no-same-owner
)
mv --no-clobber "$badi_stage/Qwen3-1.7B-Q4_K_M.gguf" "$badi_data/models/"
mv --no-clobber "$badi_stage/llama-b10726" "$badi_data/runtimes/"
mv --no-clobber "$badi_stage/llama-b10726-bin-ubuntu-x64.tar.gz" "$badi_data/runtimes/downloads/"
```

## User activation and removal

Inspect existing user-local installations first: `command -v badi` and
`systemctl --user cat badi-broker badi-accessibility`. User-local command links,
addon copies and service overrides take precedence over package files. Keep
their installer backups; migrate deliberately instead of mixing two builds.
The package does not delete or rewrite them.

On a fresh unlocked Omarchy session with the assets installed:

```sh
systemctl --user daemon-reload
systemctl --user enable --now badi-broker.service badi-accessibility.service
systemctl --user restart omarchy-fcitx5.service
badi doctor
badi app omawrite on
```

The broker creates private settings with access denied by default. Follow
the [application setup and privacy controls](https://github.com/ahuray/badi/blob/main/README.md)
for other apps and sites. Accessibility flags and desktop-wide
`ScreenReaderEnabled` remain explicit user choices. Model readiness alone
does not prove that an application exposes a supported field.

The optional panel files are in `/usr/share/badi/omarchy`. For a fresh panel,
create a new `io.github.ahuray.badi` directory under
`$XDG_CONFIG_HOME/omarchy/plugins` (default `~/.config/omarchy/plugins`), copy
those five files, run `omarchy plugin validate` on it, then
`omarchy plugin enable io.github.ahuray.badi`. For updates, use the backed-up
[panel installer](https://github.com/ahuray/badi/blob/main/ui/omarchy-plugin/README.md)
from the matching checkout. The package does not overwrite a personal panel.

Before removing the package, disable the panel and stop/disable both Badi user
units. Remove the package with pacman, then restart Fcitx while unlocked.
Settings and writing assets in the home directory remain available for a
later installation; delete them separately only when desired. Restore any
accessibility choice using its recorded prior values.

## Review before publication

Use Arch's [Rust](https://wiki.archlinux.org/title/Rust_package_guidelines) and
[VCS](https://wiki.archlinux.org/title/VCS_package_guidelines) packaging rules.
Check `makepkg --printsrcinfo` against `.SRCINFO`, inspect package contents,
and verify a clean build and the real install/update/removal flow on a fresh
Omarchy system. Local build evidence does not establish that fresh-system
flow. Naming, maintainer identity and publication require a final review.
