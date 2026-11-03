# Arch package

`PKGBUILD` is a local `badi-git` candidate, not an AUR publication. It follows
`main`; the source changes it needs must land before publication. Build with
`makepkg` in this directory. Rust dependencies are fetched with the lockfile
in `prepare()`, then builds and tests run offline. The active Arch stable Rust
must satisfy the repository's 1.85 MSRV. This package targets x86-64 only.

The package installs:
- the broker, controls, observer and session helper in `/usr/lib/badi`, with
  `badi`, `badi-desktop` and `badictl` in `/usr/bin`;
- the Fcitx addon in `/usr/lib/fcitx5` (built without `-Werror`, so a newer
  compiler's warning cannot fail it) and user units in `/usr/lib/systemd/user`;
- in `/usr/share/badi`: the panel files and the Fcitx theme script in
  `omarchy/`, the Omarchy repair hook `omarchy/badi-repair.hook`, and the
  Hyprland preview rule `hypr/badi.lua`;
- the launcher and its icon;
- in `/usr/share/licenses/badi-git`: `LICENSE` (MIT),
  `THIRD-PARTY-NOTICES.txt` (every component compiled into the binaries, with
  its licence text) and the complete modified SCOWL notices.

`license=` names MIT and the licences of code compiled into the binaries:
MPL-2.0 (spellbook, whose source is linked from the notices), Zlib
(foldhash) and SCOWL. It does not contain model weights,
llama.cpp, the Prediction Lab or editor plugins. No install hook starts a
service, changes accessibility, edits a home directory or downloads anything.

The full desktop contract (field observer, grey preview, settings panel)
still requires Omarchy/Hyprland and its lock IPC, so those parts are
optdepends (`hyprland`, `gtk4-layer-shell`, `quickshell`,
`qt6-declarative`): the base package — broker, session helper, Fcitx addon
and commands — installs on any Arch Wayland desktop, but that is not yet
general Linux desktop support. The package requires Fcitx
5.1.23 or later, whose Wayland frontend keeps Chromium's surrounding text
current; it does not replace the system frontend.

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
test ! -e "$badi_data/runtimes/llama-b11840"
test ! -e "$badi_data/runtimes/downloads/llama-b11840-bin-ubuntu-x64.tar.gz"
badi_stage=$(mktemp -d "$badi_data/.download.XXXXXXXX")
trap 'rm -rf -- "$badi_stage"' EXIT
curl --fail --location --connect-timeout 20 --max-time 1800 \
  --output "$badi_stage/Qwen3-1.7B-Q4_K_M.gguf" \
  'https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF/resolve/daeb8e2d528a760970442092f6bf1e55c3b659eb/Qwen3-1.7B-Q4_K_M.gguf'
curl --fail --location --connect-timeout 20 --max-time 300 \
  --output "$badi_stage/llama-b11840-bin-ubuntu-x64.tar.gz" \
  'https://github.com/ggml-org/llama.cpp/releases/download/b11840/llama-b11840-bin-ubuntu-x64.tar.gz'
(
  cd "$badi_stage"
  sha256sum --check <<'SUMS'
d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5  Qwen3-1.7B-Q4_K_M.gguf
5f6e72e772a1264df279d6b7a848570fdc00e5d968b72e7a917d19e07da96400  llama-b11840-bin-ubuntu-x64.tar.gz
SUMS
  tar --extract --gzip --file llama-b11840-bin-ubuntu-x64.tar.gz --no-same-owner
)
mv --no-clobber "$badi_stage/Qwen3-1.7B-Q4_K_M.gguf" "$badi_data/models/"
mv --no-clobber "$badi_stage/llama-b11840" "$badi_data/runtimes/"
mv --no-clobber "$badi_stage/llama-b11840-bin-ubuntu-x64.tar.gz" "$badi_data/runtimes/downloads/"
```

## User activation and removal

Inspect existing user-local installations first: `command -v badi` and
`systemctl --user cat badi-broker badi-accessibility badi-session`. User-local command links,
addon copies and service overrides take precedence over package files. Keep
their installer backups; migrate deliberately instead of mixing two builds.
The package does not delete or rewrite them.

On a fresh unlocked Omarchy session with the assets installed:

```sh
systemctl --user daemon-reload
systemctl --user enable --now badi-broker.service badi-accessibility.service badi-session.service
systemctl --user restart omarchy-fcitx5.service
badi doctor
badi app omawrite on
```

The broker creates private settings with access denied by default. Follow
the [application setup and privacy controls](https://github.com/ahuray/badi/blob/main/README.md)
for other apps and sites. Accessibility flags and desktop-wide
`ScreenReaderEnabled` remain explicit user choices. Model readiness alone
does not prove that an application exposes a supported field.

A packaged install has no model yet: `badi model` shows the recommendation,
and `badi model use ID` shows the terms that come with a Gemma or Llama model
and downloads it only after you accept. The commands above fetch an Apache-2.0
model instead.

The optional panel files are in `/usr/share/badi/omarchy`. For a fresh panel,
create a new `io.github.ahuray.badi` directory under `~/.config/omarchy/plugins`,
copy the eight files `manifest.json`, `BarWidget.qml`, `BadiMark.qml`,
`DesktopPanel.qml`, `SettingsView.qml`, `BadiClient.qml`, `KeyModel.js` and
`StatsModel.js`,
run `omarchy plugin validate` on it, then
`omarchy plugin enable io.github.ahuray.badi`. The panel runs `badi-desktop`
from `PATH`, so it finds `/usr/bin/badi-desktop`.

The grey preview's Hyprland rule needs `/usr/share/badi/hypr/badi.lua` copied
to `~/.config/hypr/badi.lua` and `require("hypr.badi")` at the end of
`~/.config/hypr/hyprland.lua`. Omarchy's refresh commands replace that file;
copying `/usr/share/badi/omarchy/badi-repair.hook` into
`~/.config/omarchy/hooks/post-boot.d/badi-repair` and `post-update.d/badi-repair`
restores the line at login and after `omarchy-update` (`badi repair` does it by
hand). For updates, use the backed-up
[panel installer](https://github.com/ahuray/badi/blob/main/ui/omarchy-plugin/README.md)
from the matching checkout. The package does not overwrite a personal panel.

Before removing the package, disable the panel and stop/disable the three Badi
user units. Remove the package with pacman, then restart Fcitx while unlocked.
Settings and writing assets in the home directory remain available for a
later installation; delete them separately only when desired. Restore any
accessibility choice using its recorded prior values.

## Review before publication

Use Arch's [Rust](https://wiki.archlinux.org/title/Rust_package_guidelines) and
[VCS](https://wiki.archlinux.org/title/VCS_package_guidelines) packaging rules.
CI's "AUR package / pinned Arch" job checks `makepkg --printsrcinfo` against
`.SRCINFO`, builds and tests the commit under review as an unprivileged user,
fails on any namcap error and checks that the notices are packaged. Verify the
real install/update/removal flow on a fresh Omarchy system. Local build evidence does not establish that fresh-system
flow. Naming, maintainer identity and publication require a final review.

Tested with: an isolated Arch container on the 2026-10-12 snapshot (Rust
1.99, Fcitx 5.1.23): build, checks, CLI and phrase-broker control, settings
compare-and-swap, typelib and library resolution, file integrity, reinstall
and removal (private settings kept, no package files left). Not tested: a
graphical session or a running user service manager.

Namcap reported no errors after dependency review. Remaining warnings cover
the unassigned maintainer, bundled Python imports, Omarchy-owned QML modules,
the Python interpreter and indirectly detected system dependencies. The
repository strips release symbols, so this candidate disables the empty debug
package rather than distributing dangling build-id links.
