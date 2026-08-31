#!/usr/bin/env bash

set -euo pipefail

artifact_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
qml_files=("$artifact_dir/BadiClient.qml" "$artifact_dir/Panel.qml")
compatibility="$artifact_dir/compatibility.json"
omarchy_root=${BADI_OMARCHY_ROOT:-/usr/share/omarchy}
require_host_checks=${BADI_OMARCHY_REQUIRE_HOST_CHECKS:-0}
[[ $require_host_checks == 0 || $require_host_checks == 1 ]] || {
  echo "plugin source gate: BADI_OMARCHY_REQUIRE_HOST_CHECKS must be 0 or 1" >&2
  exit 64
}

jq -e '
  .schemaVersion == 1
  and .id == "io.github.ahuray.badi"
  and .kinds == ["panel"]
  and .entryPoints == {"panel": "Panel.qml"}
' "$artifact_dir/manifest.json" >/dev/null

jq -e '
  .status == "disabled_repo_local_feasibility"
  and .omarchy.package == "4.0.2-1"
  and .omarchy.source_repository == "https://github.com/omacom/omarchy.git"
  and .omarchy.source_tag == "v4.0.2"
  and .omarchy.source_commit == "346e69e1cec6c4e8924531874af6ba010a1bc99e"
' "$compatibility" >/dev/null

if grep -En 'ShellRoot|execDetached|Qt\.openUrlExternally|/bin/(ba)?sh|(^|[[:space:]])(ba)?sh[[:space:]]+-c|console\.(log|warn|error)' "${qml_files[@]}"; then
  echo "plugin source gate: forbidden shell root, execution, or logging surface" >&2
  exit 1
fi

if grep -En 'command[[:space:]]*:' "${qml_files[@]}"; then
  echo "plugin source gate: Process.command is forbidden; keep reviewed fixed exec arrays" >&2
  exit 1
fi

if grep -En 'Qt\.callLater' "$artifact_dir/BadiClient.qml"; then
  echo "plugin source gate: client lifecycle callbacks must not defer unguarded work" >&2
  exit 1
fi

if grep -En 'localhost|4173|fixture' "${qml_files[@]}"; then
  echo "plugin source gate: development target leaked into the Dillinger product panel" >&2
  exit 1
fi

process_count=$(grep -Ec '^[[:space:]]*Process[[:space:]]*\{' "$artifact_dir/BadiClient.qml")
[[ $process_count == 2 ]] || {
  echo "plugin source gate: expected exactly two bounded Process objects, found $process_count" >&2
  exit 1
}

grep -F 'overviewProcess.exec(["badictl", "overview", "--json"])' \
  "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'mutationProcess.exec(["badictl", "memory", "clear"])' \
  "$artifact_dir/BadiClient.qml" >/dev/null
grep -F '"badictl", "settings", "replace",' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'overviewProcess.signal(15)' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'mutationProcess.signal(15)' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'overviewKillTimeout.restart()' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'mutationKillTimeout.restart()' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'if (overviewProcess.running) overviewProcess.signal(9)' \
  "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'if (mutationProcess.running) mutationProcess.signal(9)' \
  "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'property int lifecycleGeneration: 0' "$artifact_dir/BadiClient.qml" >/dev/null
generation_guard_count=$(grep -Ec 'exitedGeneration !== root\.lifecycleGeneration' \
  "$artifact_dir/BadiClient.qml")
[[ $generation_guard_count == 2 ]] || {
  echo "plugin source gate: both Process exits must reject stale lifecycle generations" >&2
  exit 1
}
grep -F 'if (disposed || !active) return' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'Component.onDestruction: root.dispose()' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'Component.onDestruction: client.dispose()' "$artifact_dir/Panel.qml" >/dev/null
grep -F 'client.deactivate()' "$artifact_dir/Panel.qml" >/dev/null
grep -F 'client.activate()' "$artifact_dir/Panel.qml" >/dev/null
grep -F 'root.shell.hide(root.pluginId)' "$artifact_dir/Panel.qml" >/dev/null
grep -F 'readonly property bool canRevokeSubjects: canMutateSettings' \
  "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'readonly property bool canGrantSubjects: canMutateSettings' \
  "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'enabled: client.canRevokeSubjects && client.targetAnyAuthority' \
  "$artifact_dir/Panel.qml" >/dev/null
grep -F 'enabled: client.canGrantSubjects' "$artifact_dir/Panel.qml" >/dev/null
grep -F 'identity.scheme === "https"' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'identity.host === "dillinger.io"' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'identity.port === 443' "$artifact_dir/BadiClient.qml" >/dev/null
grep -F 'text: "https://dillinger.io/"' "$artifact_dir/Panel.qml" >/dev/null
grep -F 'term-ignoring-mutation)' "$artifact_dir/tests/fake-bin/badictl" >/dev/null

validator="$omarchy_root/bin/omarchy-plugin-validate"
if [[ -x $validator ]]; then
  "$validator" "$artifact_dir"
elif ((require_host_checks)); then
  echo "plugin source gate: pinned official manifest validator is required" >&2
  exit 1
else
  echo "plugin source gate: official manifest validation not run (portable lane)"
fi

qml_linter=${BADI_QMLLINT:-/usr/lib/qt6/bin/qmllint}
if [[ -x $qml_linter && -d $omarchy_root/shell ]]; then
  "$qml_linter" --version 2>&1 | grep -E '^qmllint 6\.' >/dev/null || {
    echo "plugin source gate: Qt 6 qmllint is required" >&2
    exit 1
  }
  lint_root=$(mktemp -d "${TMPDIR:-/tmp}/badi-qmllint.XXXXXX")
  lint_log="$lint_root/qmllint.log"
  cleanup_lint_root() {
    rm -rf -- "$lint_root"
  }
  trap cleanup_lint_root EXIT
  ln -s -- "$omarchy_root/shell" "$lint_root/qs"
  if ! "$qml_linter" --ignore-settings -I "$lint_root" \
      "${qml_files[@]}" >"$lint_log" 2>&1; then
    cat "$lint_log" >&2
    echo "plugin source gate: Qt 6 QML syntax validation failed" >&2
    exit 1
  fi
  cleanup_lint_root
  trap - EXIT
elif ((require_host_checks)); then
  echo "plugin source gate: pinned host and Qt 6 qmllint are required" >&2
  exit 1
else
  echo "plugin source gate: Qt 6 QML syntax validation not run (portable lane)"
fi

if [[ -f $omarchy_root/shell/shell.qml ]]; then
  while IFS=$'\t' read -r relative expected; do
    [[ -f $omarchy_root/$relative ]] || {
      echo "plugin source gate: pinned Omarchy contract file is missing: $relative" >&2
      exit 1
    }
    actual=$(sha256sum "$omarchy_root/$relative" | cut -d' ' -f1)
    [[ $actual == "$expected" ]] || {
      echo "plugin source gate: pinned Omarchy contract drifted at $relative" >&2
      exit 1
    }
  done < <(jq -r '
    .omarchy.contract_files
    | to_entries[]
    | [.key, (.value | sub("^sha256:"; ""))]
    | @tsv
  ' "$compatibility")
elif ((require_host_checks)); then
  echo "plugin source gate: pinned Omarchy host contract is required" >&2
  exit 1
else
  echo "plugin source gate: pinned host hashes not checked (portable lane)"
fi

echo "Badi Omarchy plugin source checks passed"
