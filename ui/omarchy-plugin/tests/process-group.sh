#!/usr/bin/env bash

# Shared process-group ownership and teardown helpers for isolated QML tests.
# Callers must launch the group with setsid and give it a unique HOME.

# Read by callers after teardown to assert that TERM-resistant fixtures reached KILL.
# shellcheck disable=SC2034
BADI_PROCESS_GROUP_ESCALATED=0

badi_live_process_group_members() {
  local group_id=$1

  ps -eo pid=,pgid=,stat= |
    awk -v group_id="$group_id" '$2 == group_id && $3 !~ /^Z/ {print $1}'
}

badi_assert_private_process_group() {
  local leader_pid=$1
  local attempt
  local observed_group
  local observed_session

  [[ $leader_pid =~ ^[1-9][0-9]*$ ]] || {
    echo "isolated process-group check: invalid leader PID" >&2
    return 1
  }
  for ((attempt = 0; attempt < 80; attempt++)); do
    read -r observed_group observed_session < <(
      ps -o pgid=,sid= -p "$leader_pid" 2>/dev/null
    ) || true
    if [[ $observed_group == "$leader_pid" \
      && $observed_session == "$leader_pid" ]]; then
      return 0
    fi
    kill -0 "$leader_pid" 2>/dev/null || break
    sleep 0.01
  done
  echo "isolated process-group check: child did not receive a private process group and session" >&2
  return 1
}

badi_verify_process_group_members() {
  local group_id=$1
  local expected_home=$2
  local attempt
  local environment
  local member
  local members=()
  local observed_group
  local observed_state
  local verified

  [[ $expected_home == /* && $expected_home != *$'\n'* ]] || {
    echo "isolated process-group check: expected HOME must be a safe absolute path" >&2
    return 1
  }
  mapfile -t members < <(badi_live_process_group_members "$group_id")
  for member in "${members[@]}"; do
    verified=0
    for ((attempt = 0; attempt < 20; attempt++)); do
      observed_group=""
      observed_state=""
      read -r observed_group observed_state < <(
        ps -o pgid=,stat= -p "$member" 2>/dev/null
      ) || true
      if [[ $observed_group != "$group_id" || $observed_state == Z* ]]; then
        verified=1
        break
      fi
      if environment=$(tr '\0' '\n' 2>/dev/null <"/proc/$member/environ"); then
        if grep -Fx "HOME=$expected_home" <<<"$environment" >/dev/null; then
          verified=1
          break
        fi
        read -r observed_group observed_state < <(
          ps -o pgid=,stat= -p "$member" 2>/dev/null
        ) || true
        if [[ $observed_group != "$group_id" || $observed_state == Z* ]]; then
          verified=1
          break
        fi
        echo "isolated process-group check: refusing to signal foreign group member $member" >&2
        return 1
      fi
      sleep 0.01
    done
    if ((verified == 0)); then
      echo "isolated process-group check: cannot verify group member $member" >&2
      return 1
    fi
  done
}

badi_stop_private_process_group() {
  local leader_pid=$1
  local expected_home=$2
  local attempt
  local members=()

  [[ $leader_pid =~ ^[1-9][0-9]*$ ]] || return 0
  BADI_PROCESS_GROUP_ESCALATED=0
  mapfile -t members < <(badi_live_process_group_members "$leader_pid")
  if ((${#members[@]} == 0)); then
    wait "$leader_pid" 2>/dev/null || true
    return 0
  fi

  badi_verify_process_group_members "$leader_pid" "$expected_home" || return 1
  kill -TERM -- "-$leader_pid" 2>/dev/null || true
  for ((attempt = 0; attempt < 10; attempt++)); do
    mapfile -t members < <(badi_live_process_group_members "$leader_pid")
    ((${#members[@]} == 0)) && break
    sleep 0.05
  done

  mapfile -t members < <(badi_live_process_group_members "$leader_pid")
  if ((${#members[@]} > 0)); then
    badi_verify_process_group_members "$leader_pid" "$expected_home" || return 1
    BADI_PROCESS_GROUP_ESCALATED=1
    kill -KILL -- "-$leader_pid" 2>/dev/null || true
    wait "$leader_pid" 2>/dev/null || true
  fi
  for ((attempt = 0; attempt < 80; attempt++)); do
    mapfile -t members < <(badi_live_process_group_members "$leader_pid")
    ((${#members[@]} == 0)) && break
    sleep 0.05
  done

  mapfile -t members < <(badi_live_process_group_members "$leader_pid")
  if ((${#members[@]} > 0)); then
    echo "isolated process-group check: process group survived teardown: ${members[*]}" >&2
    return 1
  fi
  wait "$leader_pid" 2>/dev/null || true
}
