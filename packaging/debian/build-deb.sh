#!/usr/bin/env bash
# Build Badi's binary package for the Debian or Ubuntu release this runs on.
#
#   packaging/debian/build-deb.sh [OUTPUT_DIRECTORY]   (default: target/debian)
#
# Builds the release broker and badictl, and the Fcitx addon against this
# system's Fcitx with Debian's hardening flags, runs the addon's tests,
# stages the Arch package's layout with the multiarch addon directory,
# computes the shared-library dependencies with dpkg-shlibdeps and writes
# badi_<version>_amd64.deb. The package has no maintainer scripts. The
# runbook beside this script lists the build dependencies.
set -euo pipefail
umask 022

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
output=${1:-$root/target/debian}

# shellcheck source=/dev/null
. /etc/os-release
# Each release's runtime relationships, beyond the computed library ones.
# The field observer needs gtk4-layer-shell, which Ubuntu 24.04 does not
# package; it runs only in a Hyprland session so far, so it is recommended.
case "${ID:-}:${VERSION_ID:-}" in
    debian:13) release=debian13 layer_shell=", gir1.2-gtk4layershell-1.0" ;;
    ubuntu:24.04) release=ubuntu24.04 layer_shell="" ;;
    *)
        echo "build-deb.sh builds for Debian 13 and Ubuntu 24.04; this is ${PRETTY_NAME:-an unknown system}." >&2
        exit 1
        ;;
esac
architecture=$(dpkg --print-architecture)
if [ "$architecture" != amd64 ]; then
    echo "Badi's pinned llama.cpp runtime is x86-64 only; this system is $architecture." >&2
    exit 1
fi
multiarch=$(dpkg-architecture -qDEB_HOST_MULTIARCH)

for tool in cargo rustc cmake ninja ctest dpkg-shlibdeps dpkg-buildflags python3; do
    command -v "$tool" > /dev/null || { echo "Missing build tool: $tool (see packaging/debian/README.md)." >&2; exit 1; }
done
rust=$(rustc --version | cut -d' ' -f2)
if [ "$(printf '%s\n' 1.85.0 "$rust" | sort -V | head -n1)" != 1.85.0 ]; then
    echo "Rust $rust is older than Badi's 1.85 minimum. Ubuntu 24.04: apt install cargo-1.85 rustc-1.85," \
        "then run with PATH=/usr/lib/rust-1.85/bin:\$PATH." >&2
    exit 1
fi

cd "$root"
upstream=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
if [ "$(git rev-parse --show-toplevel 2> /dev/null)" = "$root" ] && commit=$(git rev-parse --verify 'HEAD^{commit}' 2> /dev/null); then
    source_version="r$(git rev-list --count HEAD).g${commit:0:7}"
else
    commit=unknown source_version=unknown
fi
version="${BADI_DEB_VERSION:-$upstream+$source_version-1~$release}"
maintainer="${DEBFULLNAME:-Badi local build} <${DEBEMAIL:-badi@localhost}>"

work=$(mktemp -d "${TMPDIR:-/tmp}/badi-deb.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
# debhelper's layout, which dpkg-shlibdeps expects: debian/control beside debian/<package>.
stage="$work/debian/badi"
target_dir=${CARGO_TARGET_DIR:-$root/target/deb-$release}

# The broker's build script embeds the checkout's commit; the release profile strips symbols.
CARGO_TARGET_DIR="$target_dir" cargo build --release --locked -p badi-broker --bins

# The addon, with Debian's hardening flags. CMake reads CXXFLAGS but not CPPFLAGS.
export DEB_BUILD_MAINT_OPTIONS=hardening=+all
eval "$(dpkg-buildflags --export=sh)"
export CXXFLAGS="$CXXFLAGS ${CPPFLAGS:?dpkg-buildflags sets CPPFLAGS}"
cmake -S adapters/fcitx5 -B "$work/fcitx" -G Ninja -DCMAKE_BUILD_TYPE=None -DCMAKE_INSTALL_PREFIX=/usr \
    -DCMAKE_INSTALL_LIBDIR="lib/$multiarch" -DBUILD_TESTING=ON -DBADI_WERROR=OFF
cmake --build "$work/fcitx"
ctest --test-dir "$work/fcitx" --output-on-failure
DESTDIR="$stage" cmake --install "$work/fcitx"
addon="$stage/usr/lib/$multiarch/fcitx5/libbadi-fcitx5.so"
chmod 0644 "$addon"

# The Arch package's layout (packaging/aur/PKGBUILD), less the Omarchy panel,
# its repair hook and the Hyprland rule, and the launcher, which opens that panel.
library="$stage/usr/lib/badi"
install -d "$library/accessibility" "$library/session" "$stage/usr/bin" "$stage/usr/lib/systemd/user" \
    "$stage/usr/share/doc/badi"
install -m755 "$target_dir/release/badi-broker" "$target_dir/release/badictl" "$library/"
# As dh_strip does; the release profile already strips the Rust symbols.
strip --strip-unneeded --remove-section=.comment --remove-section=.note \
    "$library/badi-broker" "$library/badictl" "$addon"
install -m644 scripts/badi-desktop.py scripts/badi_install.py "$library/"
install -m644 adapters/accessibility/*.py "$library/accessibility/"
install -m644 adapters/session/*.py "$library/session/"
# Scripts run as programs get Debian's interpreter path and the executable
# bit, and write no bytecode: run as root they would leave __pycache__
# directories in /usr/lib/badi that dpkg does not own (the units pass -B too).
for script in "$library"/*.py "$library"/*/*.py; do
    if [ "$(head -c2 "$script")" = '#!' ]; then
        sed -i '1s|^#!/usr/bin/env python3$|#!/usr/bin/python3 -B|' "$script"
        chmod 0755 "$script"
    fi
done
ln -s ../lib/badi/badi-desktop.py "$stage/usr/bin/badi"
ln -s ../lib/badi/badi-desktop.py "$stage/usr/bin/badi-desktop"
ln -s ../lib/badi/badictl "$stage/usr/bin/badictl"
for unit in badi-broker badi-accessibility badi-session; do
    sed 's|%h/.local/lib/badi/|/usr/lib/badi/|g' "packaging/systemd/$unit.service" \
        > "$stage/usr/lib/systemd/user/$unit.service"
done
# Desktop notifications from the session helper name this icon.
install -Dm644 packaging/io.github.ahuray.badi.svg \
    "$stage/usr/share/icons/hicolor/scalable/apps/io.github.ahuray.badi.svg"
install -m644 packaging/debian/README.md "$stage/usr/share/doc/badi/README.md"
python3 -I packaging/debian/copyright.py LICENSE THIRD-PARTY-NOTICES.txt broker/data/writing-lexicon/LICENSE \
    > "$stage/usr/share/doc/badi/copyright"
{
    printf 'badi (%s) %s; urgency=medium\n\n' "$version" "${VERSION_CODENAME:-unstable}"
    printf '  * Built by packaging/debian/build-deb.sh from commit\n    %s.\n\n' "$commit"
    printf ' -- %s  %s\n' "$maintainer" "$(date -R -d "@${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2> /dev/null || date +%s)}")"
} | gzip -9n > "$stage/usr/share/doc/badi/changelog.Debian.gz"

# dpkg-shlibdeps finds the package root by its DEBIAN directory.
install -d "$stage/DEBIAN"
printf 'Source: badi\n\nPackage: badi\nArchitecture: amd64\n' > "$work/debian/control"
shlibs=$(cd "$work" && dpkg-shlibdeps -O debian/badi/usr/lib/badi/badi-broker debian/badi/usr/lib/badi/badictl \
    "debian/badi/usr/lib/$multiarch/fcitx5/libbadi-fcitx5.so" | sed -n 's/^shlibs:Depends=//p')

(cd "$stage" && find usr -type f -print0 | LC_ALL=C sort -z | xargs -0 md5sum) > "$stage/DEBIAN/md5sums"
size=$(du -sk --apparent-size "$stage/usr" | cut -f1)
cat > "$stage/DEBIAN/control" << CONTROL
Package: badi
Version: $version
Architecture: amd64
Maintainer: $maintainer
Installed-Size: $size
Depends: $shlibs, python3 (>= 3.12), python3-gi, fcitx5 (>= 5.1.7), systemd, pipewire-bin
Recommends: dbus-user-session, ca-certificates, curl, gir1.2-secret-1, gnome-keyring, xdg-desktop-portal, python3-gi-cairo, gir1.2-atspi-2.0, gir1.2-gtk-4.0$layer_shell
Suggests: hunspell-de-de, hunspell-fr, hunspell-es, hunspell-it, hunspell-nl, hunspell-pl
Section: utils
Priority: optional
Homepage: https://github.com/ahuray/badi
Description: local writing suggestions for Linux desktops
 Badi suggests the next words while you type, from a small language model
 that runs on this computer; nothing leaves it. A cooperative Fcitx 5 addon
 reaches native fields, and its own plugins reach Obsidian, Neovim, Bash and
 LibreOffice Writer.
 .
 The writing model and its pinned llama.cpp runtime are not in this package:
 each user downloads them, checked against pinned hashes, and enables the
 user services. /usr/share/doc/badi/README.md lists the steps.
CONTROL

mkdir -p "$output"
deb="$output/badi_${version}_amd64.deb"
dpkg-deb --root-owner-group --build "$stage" "$deb"
echo "Built $deb"
