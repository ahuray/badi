#!/usr/bin/env bash
# Isolated phrase-broker verification for Badi. Never touches a desktop install.
# Usage, from the repository root, with BADI_VERIFY_RUN_ID set:
#   .cursor/skills/verify-badi/verify-badi.sh launch|doctor|drive FEATURE|cleanup
# FEATURE is word-suggestion, spelling, or pause.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BROKER="$ROOT/target/debug/badi-broker"
CLI="$ROOT/target/debug/badictl"
BRIDGE="$ROOT/adapters/shell/bridge.mjs"

die() { printf 'verify-badi: %s\n' "$*" >&2; exit 1; }

require_run_id() {
  [[ "${BADI_VERIFY_RUN_ID:-}" =~ ^[A-Za-z0-9._-]{1,64}$ ]] || die "Set BADI_VERIFY_RUN_ID to a short token without slashes"
  EVIDENCE="/tmp/badi-verify-evidence/${BADI_VERIFY_RUN_ID}"
  INSTANCE="/tmp/badi-verify-${BADI_VERIFY_RUN_ID}"
  ENV_FILE="$EVIDENCE/instance.env"
  RUNTIME="$INSTANCE/runtime"
  CONFIG="$INSTANCE/config"
  DATA="$INSTANCE/data"
  SOCKET="$RUNTIME/badi/broker.sock"
}

load_instance() {
  require_run_id
  [[ -f "$ENV_FILE" ]] || die "No instance env at $ENV_FILE. Run launch first."
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  [[ "$BADI_VERIFY_ROOT" == "$INSTANCE" ]] || die "Instance root in $ENV_FILE is not $INSTANCE"
  [[ "$BADI_SOCKET" == "$SOCKET" ]] || die "Socket in $ENV_FILE is not $SOCKET"
  cmdline="$(tr '\0' ' ' < "/proc/${BADI_BROKER_PID}/cmdline" 2>/dev/null || true)"
  [[ "$cmdline" == *badi-broker* && "$cmdline" == *"--provider phrase"* ]] || die "Recorded pid ${BADI_BROKER_PID} is not this run's phrase broker"
}

broker_env() {
  env -i \
    HOME="${HOME}" \
    PATH="${PATH}" \
    USER="${USER:-}" \
    LANG="${LANG:-C.UTF-8}" \
    BADI_LANGUAGE=en \
    XDG_RUNTIME_DIR="$RUNTIME" \
    XDG_CONFIG_HOME="$CONFIG" \
    XDG_DATA_HOME="$DATA" \
    "$@"
}

status_json() {
  broker_env "$CLI" --socket "$SOCKET" status
}

launch() {
  require_run_id
  if [[ -f "$ENV_FILE" ]]; then
    pid="$(sed -n 's/^BADI_BROKER_PID=//p' "$ENV_FILE" | head -1)"
    if [[ -n "$pid" ]] && tr '\0' ' ' < "/proc/${pid}/cmdline" 2>/dev/null | grep -q badi-broker; then
      die "Run ${BADI_VERIFY_RUN_ID} already has a live broker (pid ${pid})"
    fi
  fi
  if [[ ! -x "$BROKER" || ! -x "$CLI" ]]; then
    (cd "$ROOT" && cargo build -p badi-broker --bins --locked)
  fi
  [[ -x "$BROKER" && -x "$CLI" ]] || die "Broker binaries missing after build"
  rm -rf "$INSTANCE"
  mkdir -p "$EVIDENCE" "$RUNTIME" "$CONFIG/badi" "$DATA/badi"
  chmod 700 "$EVIDENCE" "$INSTANCE" "$RUNTIME" "$CONFIG" "$CONFIG/badi" "$DATA" "$DATA/badi"
  cat > "$CONFIG/badi/settings.json" <<'JSON'
{"schema":"badi.settings.v2","revision":1,"paused":false,"subjects":[{"identity":{"kind":"linux_app","adapter":"shell","app_id":"bash"},"permissions":{"context_read":"allow","display":"allow","suggest":"allow","learn":"block","retention":{"mode":"none"}}}]}
JSON
  chmod 600 "$CONFIG/badi/settings.json"
  : > "$EVIDENCE/broker.log"
  # Clean environment, then setsid in this process before exec, so $! stays
  # the broker and the launcher's exit does not deliver SIGHUP. Cleanup
  # signals this pid only, after checking its command line.
  (
    exec env -i \
      HOME="${HOME}" \
      PATH="${PATH}" \
      USER="${USER:-}" \
      LANG="${LANG:-C.UTF-8}" \
      BADI_LANGUAGE=en \
      XDG_RUNTIME_DIR="$RUNTIME" \
      XDG_CONFIG_HOME="$CONFIG" \
      XDG_DATA_HOME="$DATA" \
      python3 -c 'import os, sys; os.setsid(); os.execvpe(sys.argv[1], sys.argv[1:], os.environ)' \
      "$BROKER" --provider phrase
  ) >>"$EVIDENCE/broker.log" 2>&1 < /dev/null &
  pid=$!
  cat > "$ENV_FILE" <<EOF
BADI_VERIFY_RUN_ID=${BADI_VERIFY_RUN_ID}
BADI_VERIFY_ROOT=${INSTANCE}
BADI_VERIFY_EVIDENCE=${EVIDENCE}
XDG_RUNTIME_DIR=${RUNTIME}
XDG_CONFIG_HOME=${CONFIG}
XDG_DATA_HOME=${DATA}
BADI_SOCKET=${SOCKET}
BADI_BROKER_PID=${pid}
EOF
  chmod 600 "$ENV_FILE"
  (cd "$ROOT" && git rev-parse HEAD) > "$EVIDENCE/commit.txt"
  "$BROKER" --version > "$EVIDENCE/broker-version.txt"
  "$CLI" --version > "$EVIDENCE/badictl-version.txt"
  deadline=$((SECONDS + 30))
  until [[ -S "$SOCKET" ]]; do
    if ! kill -0 "$pid" 2>/dev/null; then
      die "Broker exited before the socket appeared. See $EVIDENCE/broker.log"
    fi
    if (( SECONDS >= deadline )); then
      die "Broker socket was not ready within 30s. See $EVIDENCE/broker.log"
    fi
    sleep 0.05
  done
  printf 'verify-badi ready run=%s socket=%s pid=%s\n' "$BADI_VERIFY_RUN_ID" "$SOCKET" "$pid"
}

doctor() {
  load_instance
  set +e
  status_json > "$EVIDENCE/doctor.json" 2> "$EVIDENCE/doctor.err"
  code=$?
  set -e
  printf '%s\n' "$code" > "$EVIDENCE/doctor.exit"
  [[ "$code" -eq 0 ]] || die "badictl status exited ${code}. See $EVIDENCE/doctor.err"
  python3 - "$EVIDENCE/doctor.json" "$EVIDENCE/broker.log" <<'PY'
import json, pathlib, sys
status = json.loads(pathlib.Path(sys.argv[1]).read_text())
log = pathlib.Path(sys.argv[2]).read_text()
problems = []
if status.get("provider") != "phrase_v1":
    problems.append(f"provider={status.get('provider')!r}")
if status.get("paused") is not False:
    problems.append("paused")
if status.get("control_plane_degraded") is not False:
    problems.append("control plane degraded")
if status.get("socket_mode") != "0600":
    problems.append(f"socket_mode={status.get('socket_mode')!r}")
if status.get("sessions") != 0:
    problems.append(f"sessions={status.get('sessions')!r}")
if log.strip():
    problems.append("phrase broker wrote diagnostics")
if problems:
    raise SystemExit("doctor: " + "; ".join(problems))
PY
  printf 'verify-badi doctor ok run=%s\n' "$BADI_VERIFY_RUN_ID"
}

assert_doctor_passed() {
  [[ -f "$EVIDENCE/doctor.exit" && "$(cat "$EVIDENCE/doctor.exit")" == "0" ]] || die "Doctor has not passed for this run. Run doctor before drive."
}

wait_idle() {
  local destination="$1"
  local tries=0
  while (( tries < 50 )); do
    status_json > "$destination"
    python3 -c 'import json,sys; raise SystemExit(0 if json.load(open(sys.argv[1]))["sessions"]==0 else 1)' "$destination" && return 0
    tries=$((tries + 1))
    sleep 0.1
  done
  die "Broker still has a session. See $destination"
}

# The bridge drops or never observes lines that arrive while it is still
# connecting, and then waits forever. Send each command only after READY,
# and after each reply, the way Bash does.
run_bridge() {
  local commands="$1" transcript="$2" errors="$3" exit_file="$4"
  set +e
  broker_env python3 - "$BRIDGE" "$commands" "$transcript" "$errors" <<'PY'
import signal, subprocess, sys
bridge, commands, transcript, errors = sys.argv[1:]

def timed_out(_signum, _frame):
    raise TimeoutError("bridge reply timed out")

signal.signal(signal.SIGALRM, timed_out)
signal.alarm(8)
code = 1
with open(errors, "w") as err, open(commands) as src, open(transcript, "w") as out:
    proc = subprocess.Popen(["node", bridge], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err, text=True)
    try:
        def read():
            line = proc.stdout.readline()
            if line == "":
                raise SystemExit("bridge closed stdout")
            out.write(line)
            out.flush()
            return line.rstrip("\n")
        ready = read()
        if not ready.startswith("READY"):
            raise SystemExit(f"bridge not ready: {ready}")
        for command in src:
            if not command.strip():
                continue
            proc.stdin.write(command if command.endswith("\n") else command + "\n")
            proc.stdin.flush()
            read()
        proc.stdin.close()
        code = proc.wait(timeout=5)
    finally:
        signal.alarm(0)
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=2)
sys.exit(code)
PY
  printf '%s\n' "$?" > "$exit_file"
  set -e
}

drive_word_suggestion() {
  load_instance
  assert_doctor_passed
  local name="word-suggestion"
  status_json > "$EVIDENCE/status-before-${name}.json"
  local before
  before="$(printf 'thank you' | base64 -w0)"
  printf '{"operation":"suggest","before":"%s"}\n{"operation":"accept"}\n{"operation":"result","applied":true}\n' "$before" \
    > "$EVIDENCE/drive-${name}.commands.jsonl"
  run_bridge "$EVIDENCE/drive-${name}.commands.jsonl" "$EVIDENCE/drive-${name}.txt" \
    "$EVIDENCE/drive-${name}.err" "$EVIDENCE/drive-${name}.exit"
  wait_idle "$EVIDENCE/status-after-${name}.json"
  python3 - "$EVIDENCE/drive-${name}.txt" "$EVIDENCE/drive-${name}.exit" \
    "$EVIDENCE/status-before-${name}.json" "$EVIDENCE/status-after-${name}.json" <<'PY'
import base64, json, pathlib, sys
transcript, exit_file, before, after = map(pathlib.Path, sys.argv[1:])
if exit_file.read_text().strip() != "0":
    raise SystemExit(f"bridge exit {exit_file.read_text().strip()}")
lines = transcript.read_text().splitlines()
def decode(line, label):
    status, _, rest = line.partition(" ")
    if status != label:
        raise SystemExit(f"expected {label}, saw {line!r}")
    parts = rest.split(" ") if rest else []
    text = base64.b64decode(parts[0]).decode() if parts else ""
    return text, parts[1:]
if len(lines) < 4 or not lines[0].startswith("READY "):
    raise SystemExit(f"unexpected transcript: {lines!r}")
suggestion, extra = decode(lines[1], "SUGGEST")
inserted, _ = decode(lines[2], "INSERT")
if suggestion != " for your time" or inserted != " for your time" or extra:
    raise SystemExit(f"suggestion={suggestion!r} inserted={inserted!r} extra={extra!r}")
if not lines[3].startswith("DONE"):
    raise SystemExit(f"missing DONE: {lines[3]!r}")
earlier = json.loads(before.read_text())["metrics"]["commits_applied"]
later = json.loads(after.read_text())
if later["sessions"] != 0 or later["metrics"]["commits_applied"] != earlier + 1:
    raise SystemExit(f"commits {earlier} -> {later['metrics']['commits_applied']}, sessions={later['sessions']}")
if later["provider"] != "phrase_v1" or later["paused"] is not False:
    raise SystemExit("broker left the phrase fixture or stayed paused")
PY
  printf 'verify-badi drive word-suggestion ok\n'
}

drive_spelling() {
  load_instance
  assert_doctor_passed
  local name="spelling"
  status_json > "$EVIDENCE/status-before-${name}.json"
  local before
  before="$(printf 'We need an adress ' | base64 -w0)"
  printf '{"operation":"suggest","before":"%s"}\n{"operation":"accept"}\n{"operation":"result","applied":true}\n' "$before" \
    > "$EVIDENCE/drive-${name}.commands.jsonl"
  run_bridge "$EVIDENCE/drive-${name}.commands.jsonl" "$EVIDENCE/drive-${name}.txt" \
    "$EVIDENCE/drive-${name}.err" "$EVIDENCE/drive-${name}.exit"
  wait_idle "$EVIDENCE/status-after-${name}.json"
  python3 - "$EVIDENCE/drive-${name}.txt" "$EVIDENCE/drive-${name}.exit" \
    "$EVIDENCE/status-before-${name}.json" "$EVIDENCE/status-after-${name}.json" <<'PY'
import base64, json, pathlib, sys
transcript, exit_file, before, after = map(pathlib.Path, sys.argv[1:])
if exit_file.read_text().strip() != "0":
    raise SystemExit(f"bridge exit {exit_file.read_text().strip()}")
lines = transcript.read_text().splitlines()
def parts(line, label):
    status, _, rest = line.partition(" ")
    if status != label:
        raise SystemExit(f"expected {label}, saw {line!r}")
    fields = rest.split(" ")
    if len(fields) != 2:
        raise SystemExit(f"{label} missing replacement: {line!r}")
    return tuple(base64.b64decode(field).decode() for field in fields)
if len(lines) < 4 or not lines[0].startswith("READY "):
    raise SystemExit(f"unexpected transcript: {lines!r}")
if parts(lines[1], "SUGGEST") != ("address ", "adress "):
    raise SystemExit(f"suggest {lines[1]!r}")
if parts(lines[2], "INSERT") != ("address ", "adress "):
    raise SystemExit(f"insert {lines[2]!r}")
if not lines[3].startswith("DONE"):
    raise SystemExit(f"missing DONE: {lines[3]!r}")
earlier = json.loads(before.read_text())["metrics"]["commits_applied"]
later = json.loads(after.read_text())
if later["sessions"] != 0 or later["metrics"]["commits_applied"] != earlier + 1:
    raise SystemExit(f"commits {earlier} -> {later['metrics']['commits_applied']}, sessions={later['sessions']}")
PY
  printf 'verify-badi drive spelling ok\n'
}

drive_pause() {
  load_instance
  assert_doctor_passed
  local name="pause"
  status_json > "$EVIDENCE/status-before-${name}.json"
  local updates
  updates="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["metrics"]["context_updates"])' "$EVIDENCE/status-before-${name}.json")"
  broker_env "$CLI" --socket "$SOCKET" pause on > "$EVIDENCE/pause-on.json"
  resume() { broker_env "$CLI" --socket "$SOCKET" pause off > "$EVIDENCE/pause-off.json" || true; }
  trap resume EXIT
  local before
  before="$(printf 'thank you' | base64 -w0)"
  printf '{"operation":"suggest","before":"%s"}\n' "$before" > "$EVIDENCE/drive-${name}.commands.jsonl"
  run_bridge "$EVIDENCE/drive-${name}.commands.jsonl" "$EVIDENCE/drive-${name}.txt" \
    "$EVIDENCE/drive-${name}.err" "$EVIDENCE/drive-${name}.exit"
  status_json > "$EVIDENCE/status-paused-${name}.json"
  python3 - "$EVIDENCE/drive-${name}.txt" "$EVIDENCE/drive-${name}.exit" \
    "$EVIDENCE/status-paused-${name}.json" "$updates" <<'PY'
import json, pathlib, sys
transcript = pathlib.Path(sys.argv[1]).read_text().splitlines()
if pathlib.Path(sys.argv[2]).read_text().strip() != "0":
    raise SystemExit("bridge exit")
if not any(line == "BLOCKED app_disabled_or_model_paused" for line in transcript):
    raise SystemExit(f"pause did not block the suggestion: {transcript!r}")
status = json.loads(pathlib.Path(sys.argv[3]).read_text())
if status.get("paused") is not True:
    raise SystemExit("badictl status did not stay paused")
if status["metrics"]["context_updates"] != int(sys.argv[4]):
    raise SystemExit("paused suggest updated context")
PY
  resume
  trap - EXIT
  status_json > "$EVIDENCE/status-after-${name}.json"
  python3 -c 'import json,sys; status=json.load(open(sys.argv[1])); raise SystemExit(0 if status.get("paused") is False else 1)' \
    "$EVIDENCE/status-after-${name}.json" || die "pause off did not resume"
  printf 'verify-badi drive pause ok\n'
}

cleanup() {
  require_run_id
  mkdir -p "$EVIDENCE"
  local note="no instance env"
  if [[ -f "$ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    note="pid ${BADI_BROKER_PID:-missing} not running"
    if [[ -n "${BADI_BROKER_PID:-}" ]]; then
      cmdline="$(tr '\0' ' ' < "/proc/${BADI_BROKER_PID}/cmdline" 2>/dev/null || true)"
      if [[ "$cmdline" == *badi-broker* && "$cmdline" == *"--provider phrase"* ]]; then
        kill "$BADI_BROKER_PID"
        local tries=0
        while kill -0 "$BADI_BROKER_PID" 2>/dev/null && (( tries < 50 )); do
          tries=$((tries + 1))
          sleep 0.1
        done
        if kill -0 "$BADI_BROKER_PID" 2>/dev/null; then
          die "Broker pid ${BADI_BROKER_PID} did not exit after TERM. Not sending KILL by name; inspect the pid."
        fi
        note="terminated pid ${BADI_BROKER_PID}"
      elif [[ -n "$cmdline" ]]; then
        note="left pid ${BADI_BROKER_PID} because cmdline was not this phrase broker"
      fi
    fi
  fi
  if [[ "$INSTANCE" == /tmp/badi-verify-* && "$INSTANCE" != "/tmp/badi-verify-evidence" ]]; then
    rm -rf "$INSTANCE"
  fi
  printf '%s\nevidence=%s\n' "$note" "$EVIDENCE" | tee "$EVIDENCE/cleanup.txt"
}

require_run_id
case "${1:-}" in
  launch) launch ;;
  doctor) doctor ;;
  drive)
    case "${2:-}" in
      word-suggestion) drive_word_suggestion ;;
      spelling) drive_spelling ;;
      pause) drive_pause ;;
      *) die "Usage: drive word-suggestion|spelling|pause" ;;
    esac
    ;;
  cleanup) cleanup ;;
  *) die "Usage: BADI_VERIFY_RUN_ID=TOKEN $0 launch|doctor|drive FEATURE|cleanup" ;;
esac
