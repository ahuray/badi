#!/usr/bin/env bash
# TLC on Withhold.tla, then the broker test of the same withhold promise.
# Exit 0 only when both succeed. Prints the TLC summary and the test result.
# --tlc-only skips the broker test; CI's Rust job already runs it.
set -euo pipefail

tlc_only=false
if [[ "${1:-}" == "--tlc-only" ]]; then
  tlc_only=true
elif [[ $# -gt 0 ]]; then
  echo "usage: check.sh [--tlc-only]" >&2
  exit 2
fi

skill=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$skill/../../.." && pwd)
cd "$root"

tlc_version=1.7.4
tlc_sha256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
tlc_url="https://github.com/tlaplus/tlaplus/releases/download/v${tlc_version}/tla2tools.jar"
cache="${XDG_CACHE_HOME:-$HOME/.cache}/badi/tlc-${tlc_version}"
jar="$cache/tla2tools.jar"

if ! command -v java >/dev/null 2>&1; then
  if command -v apt-get >/dev/null 2>&1 && [[ "$(id -u)" -eq 0 ]]; then
    apt-get update -y
    apt-get install -y --no-install-recommends openjdk-21-jre-headless
  else
    echo "verify-badi-formal-withhold: java 17+ is required (OpenJDK 17 or 21 LTS; Debian/Ubuntu package openjdk-21-jre-headless)." >&2
    exit 1
  fi
fi

java_spec=$(java -XshowSettings:properties -version 2>&1 | sed -n 's/^[[:space:]]*java\.specification\.version = //p')
java_spec="${java_spec%%$'\n'*}"
if [[ "$java_spec" == 1.* ]]; then
  java_major="${java_spec#1.}"
else
  java_major="$java_spec"
fi
java_major="${java_major%%[^0-9]*}"
if [[ -z "$java_major" ]] || (( java_major < 17 )); then
  echo "verify-badi-formal-withhold: java 17+ is required, got '${java_spec:-unknown}'." >&2
  exit 1
fi

mkdir -p "$cache"
if [[ ! -f "$jar" ]] || ! echo "${tlc_sha256}  ${jar}" | sha256sum -c --status; then
  tmp=$(mktemp)
  curl -fsSL "$tlc_url" -o "$tmp"
  echo "${tlc_sha256}  ${tmp}" | sha256sum -c --status
  mv "$tmp" "$jar"
fi

echo "verify-badi-formal-withhold: TLA+ ${tlc_version}, tla2tools.jar sha256 ${tlc_sha256}"
java -version

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

set +e
(
  cd "$work"
  java -XX:+UseParallelGC -Xmx512m -cp "$jar" tlc2.TLC \
    -config "$skill/Withhold.cfg" \
    -workers 1 \
    -cleanup \
    "$skill/Withhold.tla"
)
tlc_status=$?

if [[ "$tlc_only" == true ]]; then
  set -e
  echo "verify-badi-formal-withhold: TLC exit ${tlc_status}; cargo test skipped"
  exit "$((tlc_status == 0 ? 0 : 1))"
fi

cargo test --locked -p badi-broker --lib withhold_ -- --test-threads=1
test_status=$?
set -e

echo "verify-badi-formal-withhold: TLC exit ${tlc_status}; cargo test exit ${test_status}"
if [[ "$tlc_status" -ne 0 || "$test_status" -ne 0 ]]; then
  exit 1
fi
