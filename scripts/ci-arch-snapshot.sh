#!/usr/bin/env bash
# Point pacman at one dated archive.archlinux.org snapshot and retry.
#
# That host drops connections and aborts a download at under 1 byte/s for
# 10 seconds. CI runs 38242538936, 38684819408 and 39558825776 failed that
# way. DisableDownloadTimeout removes the abort. The loop covers a refused
# connect. Jobs have no git until this runs, so .github/workflows/ci.yml
# keeps a copy in BADI_ARCH_SNAPSHOT. The agent-rules check requires that
# copy to match this file and rejects a direct pacman -S.
set -euo pipefail

date_path="${1:?usage: ci-arch-snapshot.sh YYYY/MM/DD [package ...]}"
shift

if [[ ! "$date_path" =~ ^[0-9]{4}/[0-9]{2}/[0-9]{2}$ ]]; then
  echo "ci-arch-snapshot: date must be YYYY/MM/DD, got ${date_path}" >&2
  exit 2
fi

conf="${BADI_PACMAN_CONF:-/etc/pacman.conf}"
mirror="${BADI_MIRRORLIST:-/etc/pacman.d/mirrorlist}"
pause="${BADI_ARCH_RETRY_SLEEP:-5}"

if [[ ! -f "$conf" ]]; then
  echo "ci-arch-snapshot: no pacman.conf at ${conf}" >&2
  exit 2
fi

if ! grep -q '^DisableDownloadTimeout$' "$conf"; then
  sed -i '/^\[options\]/a DisableDownloadTimeout' "$conf"
fi

# $repo and $arch are pacman mirror placeholders.
printf 'Server = https://archive.archlinux.org/repos/%s/$repo/os/$arch\n' "$date_path" > "$mirror"

pacman_retry() {
  local attempt
  for attempt in 1 2 3 4 5; do
    if "$@"; then
      return 0
    fi
    if [[ "$attempt" -eq 5 ]]; then
      echo "ci-arch-snapshot: pacman failed after 5 attempts because archive.archlinux.org was too slow or unreachable. Keep scripts/ci-arch-snapshot.sh; do not call pacman -S directly or use a rolling mirror." >&2
      return 1
    fi
    sleep "$pause"
  done
}

pacman_retry pacman -Syyu --noconfirm
if [[ "$#" -gt 0 ]]; then
  pacman_retry pacman -S --needed --noconfirm "$@"
fi
