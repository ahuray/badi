#!/usr/bin/env bash

set -euo pipefail

tests_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
scripts=(
  "$tests_dir/check-fast.sh"
  "$tests_dir/check-host.sh"
  "$tests_dir/check-process-group.sh"
  "$tests_dir/check-source.sh"
  "$tests_dir/check-term-ignoring.sh"
  "$tests_dir/process-group.sh"
  "$tests_dir/run-client-lifecycle.sh"
  "$tests_dir/fake-bin/badictl"
  "$tests_dir/fake-bin/badi-desktop"
  "$tests_dir/fake-bin/term-ignoring-group"
)
# key-model.test.mjs and stats-model.test.mjs are Node, run below when Node is present.

for command_name in jq ps setsid shellcheck; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "Badi Omarchy fast checks require $command_name" >&2
    exit 1
  }
done

shellcheck "${scripts[@]}"
bash "$tests_dir/check-source.sh"
bash "$tests_dir/check-term-ignoring.sh"
bash "$tests_dir/check-process-group.sh"
if command -v node >/dev/null 2>&1; then
  node --test "$tests_dir/key-model.test.mjs" "$tests_dir/stats-model.test.mjs"
else
  echo "Badi Omarchy keyboard and statistics models not run (Node unavailable; the repository job runs them)"
fi
if command -v qs >/dev/null 2>&1; then
  bash "$tests_dir/run-client-lifecycle.sh"
else
  echo "Badi Omarchy client lifecycle not run in portable lane (Quickshell unavailable; strict CI runs check-host.sh)"
fi

echo "Badi Omarchy portable fast checks passed"
