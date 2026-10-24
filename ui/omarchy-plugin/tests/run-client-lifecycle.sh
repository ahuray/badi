#!/usr/bin/env bash

set -euo pipefail

tests_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck disable=SC1090,SC1091
source "$tests_dir/process-group.sh"
artifact_dir=$(cd -- "$tests_dir/.." && pwd)
harness_source="$tests_dir/client-harness/shell.qml"
schema="$artifact_dir/../../broker/schemas/badi.settings.v2.schema.json"
test_root=$(mktemp -d "${TMPDIR:-/tmp}/badi-client-lifecycle.XXXXXX")
harness_dir="$test_root/harness"
panel_dir="$test_root/panel"
settings_extra="$test_root/settings-extra.json"
runtime_dir="$test_root/runtime"
call_log="$test_root/badictl.calls"
pid_log="$test_root/badictl.pids"
shell_log="$test_root/quickshell.log"
shell_pid=""

cleanup() {
  if [[ $shell_pid =~ ^[1-9][0-9]*$ ]]; then
    badi_stop_private_process_group "$shell_pid" "$test_root" || true
    shell_pid=""
  fi
  if [[ -f $pid_log ]]; then
    while IFS= read -r pid; do
      [[ $pid =~ ^[1-9][0-9]*$ ]] || continue
      [[ -r /proc/$pid/environ ]] || continue
      if tr '\0' '\n' <"/proc/$pid/environ" 2>/dev/null |
          grep -Fx "BADI_FAKE_PID_LOG=$pid_log" >/dev/null; then
        kill -KILL "$pid" 2>/dev/null || true
      fi
    done <"$pid_log"
  fi
  rm -rf -- "$test_root"
}
trap cleanup EXIT

command -v qs >/dev/null 2>&1 || {
  echo "client lifecycle check: Quickshell is required" >&2
  exit 1
}

mkdir -p "$harness_dir" "$panel_dir" "$runtime_dir" "$test_root/config" "$test_root/cache" \
  "$test_root/data" "$test_root/state"
chmod 700 "$runtime_dir" "$test_root/config"
cp -- "$harness_source" "$harness_dir/shell.qml"
cp -- "$artifact_dir/BadiClient.qml" "$harness_dir/BadiClient.qml"
cp -- "$tests_dir/panel-harness/shell.qml" "$panel_dir/shell.qml"
cp -- "$artifact_dir/BadiClient.qml" "$artifact_dir/DesktopPanel.qml" \
  "$artifact_dir/KeyModel.js" "$artifact_dir/StatsModel.js" "$panel_dir/"

runtime_env=(
  env
  # A desktop GTK theme plugin may require a live display even with offscreen Qt.
  -u QT_QPA_PLATFORMTHEME
  -u QT_STYLE_OVERRIDE
  "HOME=$test_root"
  "XDG_CONFIG_HOME=$test_root/config"
  "XDG_DATA_HOME=$test_root/data"
  "XDG_CACHE_HOME=$test_root/cache"
  "XDG_STATE_HOME=$test_root/state"
  "XDG_RUNTIME_DIR=$runtime_dir"
  "PATH=$tests_dir/fake-bin:$PATH"
  # `badi site all on` state: the panel must accept and preserve it.
  "BADI_FAKE_ALL_WEB_ORIGINS=1"
  "BADI_FAKE_CALL_LOG=$call_log"
  "BADI_FAKE_PID_LOG=$pid_log"
  "BADI_FAKE_HOLD_OVERVIEW=$test_root/hold-overview"
  "BADI_FAKE_HOLD_MUTATION=$test_root/hold-mutation"
  "BADI_FAKE_FAIL_OVERVIEW=$test_root/fail-overview"
  "BADI_FAKE_SETTINGS=$settings_extra"
  "QT_QPA_PLATFORM=offscreen"
  "NO_COLOR=1"
)

ipc_dir=$harness_dir
ipc_target=badi-client-lifecycle
ipc() {
  "${runtime_env[@]}" qs --no-color -p "$ipc_dir" ipc call "$ipc_target" "$@"
}

# Starts Quickshell on the harness in $ipc_dir and waits for its IPC.
start_shell() {
  setsid "${runtime_env[@]}" qs --no-color -p "$ipc_dir" >"$shell_log" 2>&1 &
  shell_pid=$!
  badi_assert_private_process_group "$shell_pid"
  for _ in $(seq 1 80); do
    if ! kill -0 "$shell_pid" 2>/dev/null; then
      cat "$shell_log" >&2
      echo "client lifecycle check: Quickshell exited before IPC was ready" >&2
      exit 1
    fi
    [[ $(ipc ping 2>/dev/null || true) == ok ]] && return 0
    sleep 0.05
  done
  cat "$shell_log" >&2
  echo "client lifecycle check: IPC did not become ready" >&2
  exit 1
}

fail() {
  cat "$shell_log" >&2
  echo "client lifecycle check: $*" >&2
  exit 1
}

start_shell

wait_for_call_count() {
  local expected=$1
  local observed=0
  for _ in $(seq 1 120); do
    [[ -f $call_log ]] && observed=$(wc -l <"$call_log")
    ((observed == expected)) && return 0
    ((observed > expected)) && break
    sleep 0.05
  done
  cat "$shell_log" >&2
  echo "client lifecycle check: expected $expected calls, observed $observed" >&2
  return 1
}

wait_until_idle() {
  local state=""
  for _ in $(seq 1 120); do
    state=$(ipc state)
    [[ $(jq -r '.busy' <<<"$state") == false ]] && return 0
    sleep 0.05
  done
  cat "$shell_log" >&2
  echo "client lifecycle check: client stayed busy: $state" >&2
  return 1
}

wait_for_pid_exit() {
  local pid=$1
  for _ in $(seq 1 80); do
    ! kill -0 "$pid" 2>/dev/null && return 0
    sleep 0.05
  done
  echo "client lifecycle check: TERM-ignoring PID $pid survived teardown" >&2
  return 1
}

ipc activate >/dev/null
wait_for_call_count 1
wait_until_idle
state=$(ipc state)
jq -e '
  .overviewSchema == "badi.overview.v2"
  and .settingsSchema == "badi.settings.v2"
  and .settingsDocumentValid == true
  and .subjectCount == 2
' <<<"$state" >/dev/null
legacy_settings='{"schema":"badi.settings.v1","revision":0,"paused":true,"subjects":[]}'
[[ $(ipc validateSettings "$legacy_settings") == false ]]
invalid_linux_settings=$(jq -cn '{
  schema: "badi.settings.v2",
  revision: 1,
  paused: false,
  subjects: [{
    identity: {kind: "linux_app", adapter: "fcitx", app_id: "Omawrite Window"},
    permissions: {
      suggest: "block", display: "block", context_read: "block", learn: "block",
      retention: {mode: "none"}
    }
  }]
}')
[[ $(ipc validateSettings "$invalid_linux_settings") == false ]]
linux_learning_settings=$(jq -cn '{
  schema: "badi.settings.v2",
  revision: 1,
  paused: false,
  subjects: [{
    identity: {kind: "linux_app", adapter: "fcitx", app_id: "omawrite"},
    permissions: {
      suggest: "allow", display: "allow", context_read: "allow", learn: "allow",
      retention: {mode: "none"}
    }
  }]
}')
[[ $(ipc validateSettings "$linux_learning_settings") == false ]]
for all_web_origins in true false; do
  document=$(jq -cn --argjson value "$all_web_origins" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, all_web_origins: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == true ]]
done
for all_web_origins in '"true"' null 1; do
  document=$(jq -cn --argjson value "$all_web_origins" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, all_web_origins: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == false ]]
done
# The app list mode is the same optional boolean, alone or with the site mode.
for value in true false; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, all_linux_apps: $value, all_web_origins: true, subjects: []}')
  [[ $(ipc validateSettings "$document") == true ]]
done
for value in '"true"' null 1; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, all_linux_apps: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == false ]]
done
# The timed Escape quiet is an optional whole number of seconds, 1 to 3600.
for value in 1 90 3600; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, escape_quiet_seconds: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == true ]]
done
for value in 0 3601 1.5 '"90"' null; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, escape_quiet_seconds: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == false ]]
done
# Battery scheduling and opt-in style learning are optional settings keys.
for value in '"normal"' '"quiet"' '"explicit"'; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, battery_mode: $value, style_learning: true, subjects: []}')
  [[ $(ipc validateSettings "$document") == true ]]
done
for value in '"off"' null true; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, battery_mode: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == false ]]
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, style_learning: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == $([[ $value == true ]] && echo true || echo false) ]]
done
# The first-run language answer is an optional boolean; producers omit false.
for value in true false; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, languages_chosen: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == true ]]
done
for value in '"yes"' null 1; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, languages_chosen: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == false ]]
done
# The suggestion length is an optional choice; producers omit normal.
for value in '"short"' '"normal"'; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, suggestion_length: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == true ]]
done
for value in '"long"' '"medium"' 4 null true; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, suggestion_length: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == false ]]
done
# Screen sharing: `badi screen-share suggest` writes suggest; producers omit hold.
for value in '"hold"' '"suggest"' '"off"' null true; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, screen_share: $value, subjects: []}')
  expected=$([[ $value == '"hold"' || $value == '"suggest"' ]] && echo true || echo false)
  [[ $(ipc validateSettings "$document") == "$expected" ]] || fail "screen_share $value is not $expected"
done
# An unknown key fails the document: the panel never writes over a setting it cannot see.
[[ $(ipc validateSettings '{"schema":"badi.settings.v2","revision":1,"paused":false,"subjects":[],"later":true}') == false ]]

# Opt-in context: a boolean once chosen, and a canonical list of apps and
# sites where it stays off.
for value in true false; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, context_words: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == true ]]
done
for value in '"on"' null 1; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, context_words: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == false ]]
done
site='{"kind": "browser_origin", "adapter": "chromium", "scheme": "https", "host": "mail.example", "port": 443}'
app='{"kind": "linux_app", "adapter": "fcitx", "app_id": "com.anthropic.claude"}'
for value in "[$site]" "[$site, $app]"; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, context_words: true, context_words_off: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == true ]]
done
for value in '[]' "[$app, $site]" "[$app, $app]" '[{"kind": "linux_app"}]' '"all"'; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, context_words: true, context_words_off: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == false ]]
done

# Spelling languages: known codes in the broker's order, no repeats.
for value in '[]' '["en"]' '["en","de","fr"]' '["de","pl"]'; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, spelling_languages: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == true ]]
done
for value in '["fr","de"]' '["en","en"]' '["fa"]' '"en"' null; do
  document=$(jq -cn --argjson value "$value" \
    '{schema: "badi.settings.v2", revision: 1, paused: false, spelling_languages: $value, subjects: []}')
  [[ $(ipc validateSettings "$document") == false ]]
done

# Every optional key the settings schema defines, each with values the schema
# allows (every value of an enum): the panel accepts them all, and this list is
# the schema's, so a new broker setting cannot lock the panel's writes unseen.
examples=$(jq -cn --argjson site "$site" '{
  all_linux_apps: [true, false],
  all_web_origins: [true, false],
  battery_mode: ["explicit", "normal", "quiet"],
  context_words: [true, false],
  context_words_off: [[$site]],
  escape_quiet_seconds: [1, 3600],
  language_overrides: [{}, {"brave-origin": "pt"}],
  languages_chosen: [true, false],
  mid_line: [true, false],
  screen_share: ["hold", "suggest"],
  spelling_languages: [[], ["en", "de", "fr", "es", "it", "nl", "pl"]],
  style_learning: [true, false],
  suggestion_length: ["normal", "short"],
  trailing_space_healing: [{}, {"en": true, "pt": false}],
  writing_languages: [[], ["en", "de", "fa", "fr", "es", "pt"]],
  writing_model: ["gemma-3-1b"]
}')
schema_keys=$(jq -c '[.properties | keys[]] - .required' "$schema")
[[ $schema_keys == "$(jq -c 'keys' <<<"$examples")" ]] ||
  fail "the schema's optional settings $schema_keys differ from this test's; teach BadiClient.isOptionalSetting the new key"
jq -e --slurpfile schema "$schema" '
  to_entries | all(.key as $key | $schema[0].properties[$key].enum as $enum
    | $enum == null or (.value | sort) == ($enum | sort))
' <<<"$examples" >/dev/null || fail "an enum example list differs from the schema's values"
while IFS= read -r document; do
  [[ $(ipc validateSettings "$document") == true ]] || fail "the panel refuses $document"
done < <(jq -c 'to_entries[] | .key as $key | .value[]
  | {schema: "badi.settings.v2", revision: 1, paused: false, subjects: []} + {($key): .}' <<<"$examples")

# A settings write changes only its own field at the next revision and carries
# every subject and the `badi site all on` flag through unchanged.
ipc setPaused true >/dev/null
wait_for_call_count 3
wait_until_idle
[[ $(sed -n '2p' "$call_log" | cut -d' ' -f1-5) == 'settings replace --if-revision 7 --json' ]]
[[ $(sed -n '2p' "$call_log") == 'settings replace --if-revision 7 --json -' ]]
replacement=$(cat "${call_log}.settings")
jq -e '
  .schema == "badi.settings.v2"
  and .revision == 8
  and .paused == true
  and .all_web_origins == true
  and .language_overrides == {"brave-origin": "de"}
  and any(.subjects[];
    .identity == {kind: "linux_app", adapter: "fcitx", app_id: "omawrite"})
  and any(.subjects[];
    .identity == {kind: "browser_origin", adapter: "chromium", scheme: "https",
      host: "example.com", port: 443})
' <<<"$replacement" >/dev/null
[[ $(jq -r '.message' <<<"$(ipc state)") == 'Predictions paused.' ]]

# Closed mutation: its stale exit must not refresh the inactive client, and a
# TERM-ignoring child is escalated to SIGKILL.
touch "$test_root/hold-mutation"
ipc setPaused false >/dev/null
wait_for_call_count 4
first_pid=$(head -n 1 "$pid_log")
ipc deactivate >/dev/null
sleep 0.7
wait_for_pid_exit "$first_pid"
wait_for_call_count 4
state=$(ipc state)
[[ $(jq -r '.active' <<<"$state") == false ]]
# The killed child's exit belongs to the closed lifecycle and reports nothing.
[[ $(jq -r '.message' <<<"$state") == '' ]]

# Reopen while the second TERM-ignoring mutation is still tearing down. The
# stale mutation result is discarded and exactly one fresh overview is queued.
ipc activate >/dev/null
wait_for_call_count 5
wait_until_idle
ipc setPaused false >/dev/null
wait_for_call_count 6
second_pid=$(tail -n 1 "$pid_log")
ipc deactivate >/dev/null
ipc activate >/dev/null
wait_for_pid_exit "$second_pid"
wait_for_call_count 7
wait_until_idle
sleep 0.2
wait_for_call_count 7
state=$(ipc state)
[[ $(jq -r '.active' <<<"$state") == true ]]
[[ $(jq -r '.refreshQueued' <<<"$state") == false ]]

# A write invalidates the overview read that began before it, then reports
# its own result over exactly one fresh overview.
rm -- "$test_root/hold-mutation"
touch "$test_root/hold-overview"
ipc refresh >/dev/null
wait_for_call_count 8
state=$(ipc state)
jq -e '.loading and .canMutateSettings and .settingsDocumentValid' <<<"$state" >/dev/null
rm -- "$test_root/hold-overview"
ipc setPaused true >/dev/null
wait_for_call_count 10
wait_until_idle
sleep 0.2
wait_for_call_count 10
state=$(ipc state)
jq -e '.canMutateSettings and .message == "Predictions paused."' <<<"$state" >/dev/null

# A failed read reports itself, and the next good read clears it even on a
# refresh that keeps the last result (the open popout's poll).
touch "$test_root/fail-overview"
ipc refresh >/dev/null
wait_for_call_count 11
wait_until_idle
[[ $(jq -r '.message' <<<"$(ipc state)") == 'badictl: broker unavailable' ]] || fail "a failed read is not reported"
rm -- "$test_root/fail-overview"
ipc refresh >/dev/null
wait_for_call_count 12
wait_until_idle
state=$(ipc state)
jq -e '.canMutateSettings and .message == ""' <<<"$state" >/dev/null || fail "a good read kept the old error: $state"

# A settings key from a newer broker turns every write off and says why;
# a known optional key such as screen_share keeps writes on.
printf '%s\n' '{"screen_share": "suggest"}' >"$settings_extra"
ipc refresh >/dev/null
wait_for_call_count 13
wait_until_idle
state=$(ipc state)
jq -e '.settingsDocumentValid and .canMutateSettings and .message == ""' <<<"$state" >/dev/null ||
  fail "screen_share suggest locked the panel: $state"
printf '%s\n' '{"later_setting": true}' >"$settings_extra"
ipc refresh >/dev/null
wait_for_call_count 14
wait_until_idle
state=$(ipc state)
jq -e '(.settingsDocumentValid or .canMutateSettings | not)
  and (.message | startswith("This panel does not know every Badi setting"))' <<<"$state" >/dev/null ||
  fail "an unknown settings key did not explain the locked panel: $state"
rm -- "$settings_extra"

ipc deactivate >/dev/null

if grep -En 'TypeError|ReferenceError|Cannot assign|failed to load' "$shell_log"; then
  cat "$shell_log" >&2
  echo "client lifecycle check: QML runtime error" >&2
  exit 1
fi

badi_stop_private_process_group "$shell_pid" "$test_root"
shell_pid=""

# The real DesktopPanel's settings changes, through the fake badi-desktop.
: >"$call_log"
ipc_dir=$panel_dir
ipc_target=badi-panel-settings
# Languages chosen: English suggestions off, English and German spelling;
# sharing suggests.
printf '%s\n' '{"languages_chosen": true, "screen_share": "suggest", "writing_languages": ["de"],
  "spelling_languages": ["en", "de"]}' >"$settings_extra"
start_shell
wait_for_call_count 1
wait_until_idle
state=$(ipc state)
jq -e '.settingsDocumentValid and .canChange and .chosenLanguages == ["en", "de"]' <<<"$state" >/dev/null ||
  fail "the panel cannot change these settings: $state"
rows=$(ipc rows)
jq -e '
  (map(.key) | index("fewer_settings") == 0 and index("languages") == 1)
  and any(.[]; .key == "screen_share" and .value == "suggest")
  and any(.[]; .key == "writing_en" and .label == "English" and .value == "off")
  and any(.[]; .key == "writing_de" and .label == "German" and .value == "auto")
  and any(.[]; .key == "spelling_nl" and .label == "Dutch")
' <<<"$rows" >/dev/null || fail "unexpected rows: $rows"

# A language chip changes only its own language, in each list that offers it.
ipc toggleLanguage fr >/dev/null
wait_for_call_count 3
wait_until_idle
jq -e '.writing_languages == ["de", "fr"] and .spelling_languages == ["en", "de", "fr"]
  and .screen_share == "suggest"' "${call_log}.settings" >/dev/null ||
  fail "adding French changed other languages: $(cat -- "${call_log}.settings")"
ipc toggleLanguage en >/dev/null
wait_for_call_count 5
wait_until_idle
jq -e '.writing_languages == ["de"] and .spelling_languages == ["de"]' "${call_log}.settings" >/dev/null ||
  fail "removing English changed other languages: $(cat -- "${call_log}.settings")"
# Portuguese only suggests, Italian only corrects.
ipc toggleLanguage pt >/dev/null
wait_for_call_count 7
wait_until_idle
jq -e '.writing_languages == ["de", "pt"] and .spelling_languages == ["en", "de"]' "${call_log}.settings" >/dev/null ||
  fail "adding Portuguese touched spelling: $(cat -- "${call_log}.settings")"
ipc toggleLanguage it >/dev/null
wait_for_call_count 9
wait_until_idle
jq -e '.writing_languages == ["de"] and .spelling_languages == ["en", "de", "it"]' "${call_log}.settings" >/dev/null ||
  fail "adding Italian touched suggestions: $(cat -- "${call_log}.settings")"

# The screen-share choice round-trips; hold is the omitted default.
ipc setScreenShare hold >/dev/null
wait_for_call_count 11
wait_until_idle
jq -e 'has("screen_share") | not' "${call_log}.settings" >/dev/null || fail "hold was written explicitly"
rm -- "$settings_extra"
ipc refresh >/dev/null
wait_for_call_count 12
wait_until_idle
ipc setScreenShare suggest >/dev/null
wait_for_call_count 14
wait_until_idle
jq -e '.screen_share == "suggest"' "${call_log}.settings" >/dev/null || fail "suggest was not written"
[[ $(jq -r '.message' <<<"$(ipc state)") == 'Suggestions continue while the screen is shared' ]] ||
  fail "the screen-share write did not succeed: $(ipc state)"

if grep -En 'TypeError|ReferenceError|Cannot assign|failed to load' "$shell_log"; then
  cat "$shell_log" >&2
  echo "client lifecycle check: QML runtime error in the panel" >&2
  exit 1
fi

badi_stop_private_process_group "$shell_pid" "$test_root"
shell_pid=""
echo "Badi Omarchy client mutation lifecycle check passed"
