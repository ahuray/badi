#!/usr/bin/env bash
# The Debian 13 and Ubuntu 24.04 CI steps (.github/workflows/ci.yml), run as
# root inside a container of that release, from the checkout's root. The same
# commands reproduce a CI run locally (packaging/debian/README.md).
#
#   packaging/debian/ci.sh deps | addon | broker | python | package
#   packaging/debian/ci.sh smoke PACKAGE.deb    # in a fresh container
set -euo pipefail

# shellcheck source=/dev/null
. /etc/os-release
export DEBIAN_FRONTEND=noninteractive
# Ubuntu 24.04's default Rust is 1.75. Its versioned 1.85 packages are the
# same Rust as Debian 13's and Badi's minimum, from the release's archive.
if [ "$ID" = ubuntu ]; then
    rust=(cargo-1.85 rustc-1.85)
    export PATH="/usr/lib/rust-1.85/bin:$PATH"
else
    rust=(cargo rustc)
fi

case "${1:-}" in
deps)
    apt-get update
    # nodejs runs the Bash and Neovim bridges, which the editor installer's tests start.
    apt-get install -y --no-install-recommends ca-certificates git build-essential cmake ninja-build \
        libfcitx5core-dev nlohmann-json3-dev python3 python3-gi gir1.2-atspi-2.0 gir1.2-secret-1 dbus nodejs \
        dpkg-dev lintian "${rust[@]}"
    dpkg-query -W -f='${Package} ${Version}\n' libfcitx5core-dev nlohmann-json3-dev cmake python3 python3-gi nodejs
    rustc --version
    # The checkout belongs to the runner's user, not root.
    git config --global --add safe.directory "$PWD"
    ;;
addon)
    cmake -S adapters/fcitx5 -B adapters/fcitx5/build -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=ON
    cmake --build adapters/fcitx5/build
    ctest --test-dir adapters/fcitx5/build --output-on-failure
    ;;
broker)
    cargo test --workspace --all-features --locked
    ;;
python)
    # package.json's desktop, accessibility and session checks, without Node:
    # neither release packages a Node.js its engines field accepts.
    export PYTHONDONTWRITEBYTECODE=1
    for test in test-badi-install test-desktop test-install-desktop test-install-omarchy-ui test-install-editors; do
        python3 "scripts/$test.py"
    done
    BADI_DEBUG_TRACE=off BADI_A11Y_REQUIRE_SESSION_BUS=0 python3 -m unittest discover -s adapters/accessibility/tests
    python3 -m unittest discover -s adapters/session/tests -p 'test_*.py'
    ;;
package)
    packaging/debian/build-deb.sh target/debian
    deb=$(ls target/debian/badi_*_amd64.deb)
    # Errors fail the step, except the placeholder maintainer address, which
    # publication replaces; the runbook reviews the warnings.
    lintian --fail-on error --display-level '>=warning' \
        --suppress-tags bogus-mail-host,bogus-mail-host-in-debian-changelog "$deb"
    dpkg-deb --contents "$deb" | grep -F ./usr/share/doc/badi/copyright
    ;;
smoke)
    deb=$(realpath "${2:?usage: ci.sh smoke PACKAGE.deb}")
    set -x
    apt-get update
    # Depends and Recommends must resolve on this release; dbus runs Fcitx below.
    apt-get install -y "$deb" dbus
    dpkg-query -W -f='${Package} ${Version}\nDepends: ${Depends}\nRecommends: ${Recommends}\n' badi
    version='[0-9.]+ commit=([0-9a-f]{40}|unknown) dirty=(true|false|unknown)'
    badictl --version | grep -Ex "badictl $version"
    /usr/lib/badi/badi-broker --version | grep -Ex "badi-broker $version"
    badi --help | grep -F 'badi status [--json]'
    python3 -c 'import ast, sys
for path in sys.argv[1:]:
    ast.parse(open(path, encoding="utf-8").read(), path)' /usr/lib/badi/*.py /usr/lib/badi/*/*.py
    # Without a user service manager the CLI fails cleanly, with no traceback.
    if badi doctor --json > /tmp/doctor.out 2> /tmp/doctor.err; then exit 1; fi
    cat /tmp/doctor.err
    if grep -F Traceback /tmp/doctor.err || [ ! -s /tmp/doctor.err ]; then exit 1; fi
    # Fcitx finds the addon in its own directory, with no environment.
    home=$(mktemp -d) runtime=$(mktemp -d)
    HOME="$home" XDG_RUNTIME_DIR="$runtime" timeout 8 dbus-run-session -- fcitx5 --verbose='*=4' \
        > /tmp/fcitx.log 2>&1 || [ $? = 124 ]
    grep -F 'Loaded addon badi' /tmp/fcitx.log
    apt-get purge -y badi
    if [ -e /usr/lib/badi ] || dpkg-query -W badi 2> /dev/null; then exit 1; fi
    ;;
*)
    echo "usage: $0 deps|addon|broker|python|package|smoke PACKAGE.deb" >&2
    exit 2
    ;;
esac
