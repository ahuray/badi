#!/usr/bin/env bash

set -euo pipefail

tests_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
artifact_dir=$(cd -- "$tests_dir/.." && pwd)
compatibility="$artifact_dir/compatibility.json"
omarchy_root=${BADI_OMARCHY_ROOT:?BADI_OMARCHY_ROOT is required}

for command_name in git jq pacman qs shellcheck; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "Badi Omarchy host checks require $command_name" >&2
    exit 1
  }
done
[[ -x /usr/lib/qt6/bin/qmllint ]] || {
  echo "Badi Omarchy host checks require Qt 6 qmllint" >&2
  exit 1
}
[[ -d $omarchy_root/.git ]] || {
  echo "Badi Omarchy host checks require a pinned Omarchy Git checkout" >&2
  exit 1
}

expected_commit=$(jq -r '.omarchy.source_commit' "$compatibility")
observed_commit=$(git -C "$omarchy_root" rev-parse 'HEAD^{commit}')
[[ $observed_commit == "$expected_commit" ]] || {
  echo "Badi Omarchy host checks expected commit $expected_commit, observed $observed_commit" >&2
  exit 1
}

expected_quickshell=$(jq -r '.quickshell' "$compatibility")
expected_qt=$(jq -r '.qt6_declarative' "$compatibility")
observed_quickshell=$(pacman -Q quickshell | cut -d' ' -f2)
observed_qt=$(pacman -Q qt6-declarative | cut -d' ' -f2)
[[ $observed_quickshell == "$expected_quickshell" ]] || {
  echo "Badi Omarchy host checks expected quickshell $expected_quickshell, observed $observed_quickshell" >&2
  exit 1
}
[[ $observed_qt == "$expected_qt" ]] || {
  echo "Badi Omarchy host checks expected qt6-declarative $expected_qt, observed $observed_qt" >&2
  exit 1
}

bash "$tests_dir/check-fast.sh"
BADI_OMARCHY_REQUIRE_HOST_CHECKS=1 \
  BADI_QMLLINT=/usr/lib/qt6/bin/qmllint \
  BADI_OMARCHY_ROOT="$omarchy_root" \
  bash "$tests_dir/check-source.sh"
bash "$tests_dir/run-client-lifecycle.sh"

echo "Badi Omarchy pinned host and lifecycle checks passed"
