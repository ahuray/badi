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
  "$tests_dir/run-isolated.sh"
  "$tests_dir/fake-bin/badictl"
  "$tests_dir/fake-bin/term-ignoring-group"
)

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
if command -v qs >/dev/null 2>&1; then
  bash "$tests_dir/run-client-lifecycle.sh"
else
  echo "Badi Omarchy client lifecycle not run in portable lane (Quickshell unavailable; strict CI runs check-host.sh)"
fi

echo "Badi Omarchy portable fast checks passed"
