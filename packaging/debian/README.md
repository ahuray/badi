# Debian and Ubuntu

Badi builds, packages and installs on Debian 13 (trixie) and Ubuntu 24.04
LTS (noble), x86-64 only, like its pinned llama.cpp runtime.
[build-deb.sh](build-deb.sh) builds a binary package on the release it runs
on, and [ci.sh](ci.sh) holds the CI steps for both releases. Nothing here is
published.

One part of the desktop contract differs from Omarchy's: the
[field observer](../../adapters/accessibility/README.md) runs only in a
Hyprland session so far. Under GNOME or KDE Plasma, Chromium-based apps,
Zen, VS Code and Writer's grey preview therefore get no suggestions. Fcitx's
manual fields (a Tab request in an app allowed with `badi app ID on`),
Obsidian, Neovim, Bash and the
[Writer spelling extension](../../adapters/libreoffice/README.md) do not
need it.

## Build the package

Build dependencies on Debian 13:

```sh
sudo apt install build-essential cmake ninja-build git libfcitx5core-dev nlohmann-json3-dev \
  python3 dpkg-dev cargo rustc
```

Ubuntu 24.04's own Rust (1.75) is older than Badi's 1.85 minimum. Install
`cargo-1.85 rustc-1.85` from its archive instead of `cargo rustc`, and build
with `PATH=/usr/lib/rust-1.85/bin:$PATH`; that is Debian 13's Rust version,
with no rustup or download. With a rustup `cargo` on `PATH`, the repository's
`rust-toolchain.toml` selects Rust 1.99 instead.

```sh
packaging/debian/build-deb.sh      # writes target/debian/badi_<version>_amd64.deb
```

The script builds the release broker and `badictl` (Cargo fetches the locked
crates), and the Fcitx addon against this system's Fcitx with
`dpkg-buildflags` hardening and without `-Werror`, then runs the addon's
tests. It computes the library dependencies with `dpkg-shlibdeps`. The
version is `0.1.0+r<commits>.g<commit>-1~debian13` (or `~ubuntu24.04`);
`BADI_DEB_VERSION` replaces it, and `DEBFULLNAME` and `DEBEMAIL` name the
maintainer, `Badi local build <badi@localhost>` until one is chosen for
publication. Build on each release: Debian's package needs Fcitx 5.1.12's
library, which Ubuntu 24.04 does not have.

The package installs:

| Path | Contents |
| --- | --- |
| `/usr/lib/badi` | `badi-broker`, `badictl`, the `badi` CLI (`badi-desktop.py`, `badi_install.py`), the observer in `accessibility/` and the session helper in `session/` |
| `/usr/bin` | `badi`, `badi-desktop` and `badictl`, as links |
| `/usr/lib/x86_64-linux-gnu/fcitx5`, `/usr/share/fcitx5/addon` | `libbadi-fcitx5.so` and `badi.conf`, in Fcitx's own directories |
| `/usr/lib/systemd/user` | `badi-broker`, `badi-accessibility` and `badi-session` units |
| `/usr/share/icons/hicolor/scalable/apps` | The icon of the session helper's notifications |
| `/usr/share/doc/badi` | This runbook; `copyright` with Badi's MIT licence, the notices of every component compiled into the binaries ([THIRD-PARTY-NOTICES.txt](../../THIRD-PARTY-NOTICES.txt)) and the SCOWL notices |

It is the [Arch package](../aur/README.md)'s layout without the Omarchy
panel, its repair hook, the Hyprland preview rule and the launcher, which
opens that panel. The addon compiles in the release's own nlohmann-json
(3.11.3), which has the same MIT licence as the 3.12.0 the notices name.

| Field | Packages |
| --- | --- |
| Depends | The computed libraries (`libc6`, `libgcc-s1`, `libstdc++6`, `libfcitx5core7`, `libfcitx5utils2`), `python3 (>= 3.12)`, `python3-gi` for the session helper, `fcitx5 (>= 5.1.7)`, `systemd` for the user services |
| Recommends | `dbus-user-session` for the services' session bus, `ca-certificates` and `curl` for the downloads, `gir1.2-secret-1` for the keyring key of context and style learning, `pipewire-bin` for the screen-share hold, `xdg-desktop-portal` for the shortcuts; for the observer `python3-gi-cairo`, `gir1.2-atspi-2.0`, `gir1.2-gtk-4.0` and, on Debian, `gir1.2-gtk4layershell-1.0` (Ubuntu 24.04 does not package gtk4-layer-shell) |
| Suggests | `hunspell-de-de`, `hunspell-fr`, `hunspell-es`, `hunspell-it`, `hunspell-nl`, `hunspell-pl` for `badi spelling` |

The package has no maintainer scripts: installing it starts no service,
changes no accessibility setting, edits no home directory and downloads
nothing. Fcitx itself recommends its GTK and Qt frontends and `im-config`.

## Use it

As each desktop user, never as root:

1. Inspect an earlier user-local installation first, as the Arch runbook's
   [activation](../aur/README.md#user-activation-and-removal) describes:
   files under `~/.local` and `~/.config/systemd/user` take precedence over
   the package's.
2. Download the pinned llama.cpp runtime with the
   [writing-asset commands](../aur/README.md#writing-assets), which work
   unchanged here. They also fetch an Apache-2.0 model; `badi model use ID`
   switches later, showing a model's terms before it downloads.
3. Start the services, which create private settings with every app denied:

   ```sh
   systemctl --user daemon-reload
   systemctl --user enable --now badi-broker.service badi-session.service badi-accessibility.service
   ```

   systemd skips the observer outside a Hyprland session, and `badi doctor`
   says so instead of reporting a problem.
4. Make Fcitx 5 the input method: `im-config -n fcitx5` on Debian and
   Ubuntu, or Fcitx 5 as the Virtual Keyboard in KDE Plasma's settings. Log
   out and back in. The packaged addon sits in Fcitx's own directory, so it
   needs no environment.
5. Run `badi doctor`, then allow an app (`badi app ID on`) and the editors
   ([README](../../README.md#choose-where-badi-writes)).

To remove it, stop and disable the three units, run `sudo apt purge badi`
and log in again. Settings, model and runtime stay in the home directory.

## Install from a checkout

`python3 scripts/install-desktop.py` installs user-locally here as well,
with the build dependencies above and no Node.js. It finds no
`omarchy-shell`, and then:

- checks no lock state and never restarts Fcitx, since the desktop started
  it (im-config's autostart runs `fcitx5 -d`; KDE Plasma's compositor
  launches it) and a replacement started from a terminal would lose what
  that launcher gave it;
- writes `~/.config/environment.d/50-badi-fcitx5.conf`, which sets
  `FCITX_ADDON_DIRS` to `~/.local/lib/fcitx5` and the system Fcitx's own
  addon directory (`/usr/lib/x86_64-linux-gnu/fcitx5`, found beside
  `libFcitx5Core`), and `FCITX_DATA_DIRS` as Omarchy's drop-in does.
  systemd's user environment generator reads it at login and the session
  passes it to whatever starts Fcitx, so the addon loads after the next
  login;
- applies the Fcitx 5.1.23 floor only when the compositor's Wayland
  registry offers `zwp_input_method_manager_v2`, where Fcitx's `wayland_v2`
  frontend serves apps (Hyprland, Sway); GNOME and KDE Plasma do not;
- installs and enables the observer, starting and verifying it only in a
  Hyprland session, and refuses `--observed-app` and
  `--screen-reader-accessibility`, which need Omarchy's lock status;
- leaves out the launcher, the Omarchy hooks and `hyprland.lua`.

Backups and rollback are those of the
[Fcitx runbook](../../adapters/fcitx5/README.md#install-and-roll-back).
The package does not contain the editor integrations:
`scripts/install-editors.py` installs them from the checkout as on Omarchy,
and the Bash and Neovim bridges run on the release's `nodejs` (Node 20 on
Debian 13, 18 on Ubuntu 24.04, both of which pass the installer's bridge
tests). The Obsidian plugin's build (`npm ci`, then esbuild) is untried with
those versions; the repository's `engines` field asks for Node 22.20 or
later.

`scripts/install-omarchy-ui.py` refuses to run without Omarchy, and
`badi settings` points to the `badi` commands. `badi spelling` names the
apt package of a missing dictionary.

## Tested with

All in containers from the pinned images that CI uses, on 2026-10-23,
without a graphical session or user service manager.

| Check | Debian 13 | Ubuntu 24.04 |
| --- | --- | --- |
| Toolchain | Fcitx 5.1.12, CMake 3.31, Rust 1.85.1, Python 3.13 | Fcitx 5.1.7, CMake 3.28, Rust 1.85.1 (`rustc-1.85`), Python 3.12 |
| Addon build and its 6 suites (`ci.sh addon`, and hardened in `build-deb.sh`) | Pass | Pass |
| User-local chain: the installer's addon directory detection and environment.d text, systemd's user environment generator, then a real Fcitx on a private session bus with exactly that environment loads `badi` beside the same 18 system addons; without it, Fcitx loads the 18 alone | Pass | Pass |
| The installer's Wayland registry read: headless Sway 1.10.1 offers input-method-v2 (floor applies), headless Weston 14.0.2 does not; Hyprland 0.56 on the Arch workstation offers it | Pass | — |
| `cargo test --workspace --all-features --locked` (`ci.sh broker`) | 612 pass, 5 ignored, as with Rust 1.99 on Arch | The same |
| Desktop, observer and session unit tests (`ci.sh python`), with the release's Node.js for the editor bridges | 372 pass, 1 skipped | The same |
| Package build and lintian (`ci.sh package`) | Pass, tags below | Pass, tags below |
| Install into a fresh container with Depends and Recommends, `--version` lines, `badi --help`, a clean doctor failure without a user manager, Fcitx loading the packaged addon, purge leaving nothing (`ci.sh smoke`) | Pass | Pass |

Lintian (2.122.0 on Debian 13, 2.117.0 on Ubuntu 24.04, every tag level)
leaves the same tags on both:

| Tag | Level | Why it stays |
| --- | --- | --- |
| `bogus-mail-host`, `bogus-mail-host-in-debian-changelog` | Error | The placeholder maintainer; CI suppresses only these two until publication names a maintainer |
| `no-manual-page` for `badi`, `badi-desktop`, `badictl` | Warning | Each has `--help`; there are no manual pages yet |
| `hardening-no-fortify-functions` for the Rust binaries | Info | Rust code calls no fortifiable C functions; the addon is built with Debian's hardening flags |
| `spelling-error-in-binary` (13, in `badi-broker`) | Info on Ubuntu, experimental on Debian | Words of the English lexicon compiled into the broker, not messages |
| `executable-in-usr-lib` | Experimental | The private programs live in `/usr/lib/badi`, as in the Arch package |

Not verified: any GNOME, KDE Plasma or other graphical session; Fcitx
started by im-config or Plasma with the environment.d directory at a real
login; the user units under a real user manager, including whether GNOME
and Plasma export `WAYLAND_DISPLAY` before `graphical-session.target`
starts them (the broker's `ConditionEnvironment`); the observer, preview and
Tab acceptance outside Hyprland; an install on Sway; an X11 session, where
the broker's unit does not start. [Step 7](<../../future plans.md>) covers
them.

Reproduce a CI run locally, from the checkout:

```sh
docker run --detach --name badi-debian --volume "$PWD:/src" --workdir /src debian:trixie sleep infinity
for step in deps addon broker python package; do docker exec badi-debian packaging/debian/ci.sh "$step"; done
docker run --rm --volume "$PWD:/src:ro" --workdir /src debian:trixie packaging/debian/ci.sh smoke target/debian/badi_*_amd64.deb
docker rm --force badi-debian
```

The containers write `target/` and `adapters/fcitx5/build/` as root; use
`ubuntu:24.04` for the other release.
