# Source this file in interactive Bash. Ctrl-X Tab requests/accepts; Ctrl-X Escape dismisses.
[[ $- == *i* ]] || return 0

_badi_clear_preview() {
  if [[ $(type -t badi_preview) == builtin ]]; then badi_preview clear; fi
  unset BADI_PREFIX BADI_PREVIEW BADI_REPLACE
}

_badi_renderer() {
  [[ $(type -t badi_preview) == builtin ]] && return 0
  local badi_module=${BADI_EDITOR_DIR:-$HOME/.local/lib/badi/editors}/shell/badi-preview.so
  [[ -f $badi_module ]] || badi_module=${badi_module%/*}/build/badi-preview.so
  if [[ ! -f $badi_module ]] || ! enable -f "$badi_module" badi_preview 2>/dev/null; then
    printf '\nBadi: inline renderer unavailable; rebuild or reinstall the Bash integration.\n'
    return 1
  fi
}

_badi_stop() {
  _badi_clear_preview
  # Close the bridge descriptors only while Bash still tracks the coprocess.
  # After reaping an exited bridge Bash closes them itself, and their numbers
  # may since belong to something else.
  if [[ -n ${BADI_WORKER_PID:-} ]]; then
    [[ -z ${BADI_READER_FD:-} ]] || exec {BADI_READER_FD}<&-
    [[ -z ${BADI_WRITER_FD:-} ]] || exec {BADI_WRITER_FD}>&-
  fi
  unset BADI_READER_FD BADI_WRITER_FD BADI_CONNECTION
}

_badi_alive() {
  # The bridge exits after ten idle minutes. Until Bash reaps it, its closed
  # output reads as end of file; afterwards Bash has dropped the coprocess.
  [[ -n ${BADI_READER_FD:-} && -n ${BADI_WORKER_PID:-} ]] && ! read -t 0 -u "$BADI_READER_FD" 2>/dev/null
}

_badi_exchange() {
  BADI_REPLY_STATUS=ERROR
  BADI_REPLY_VALUE=connection_closed
  # A bridge exiting at this instant must not end this shell with SIGPIPE:
  # ignore it for the one write, then restore whatever trap the user has.
  local badi_pipe_trap badi_sent
  badi_pipe_trap=$(trap -p PIPE)
  trap '' PIPE
  printf '%s\n' "$1" >&"${BADI_WRITER_FD}" 2>/dev/null
  badi_sent=$?
  if [[ -n $badi_pipe_trap ]]; then eval "$badi_pipe_trap"; else trap - PIPE; fi
  if (( badi_sent )) ||
      ! IFS=' ' read -r -t 3 -u "$BADI_READER_FD" BADI_REPLY_STATUS BADI_REPLY_VALUE BADI_REPLY_REPLACE; then
    _badi_stop
    printf '\nBadi: bridge connection closed. Press Ctrl-X Tab to reconnect.\n'
    return 1
  fi
}

_badi_start() {
  _badi_alive && return 0
  _badi_stop
  local badi_bridge=${BADI_EDITOR_DIR:-$HOME/.local/lib/badi/editors}/shell/bridge.mjs
  if [[ ! -f $badi_bridge ]]; then
    printf '\nBadi: editor bridge is missing; run the desktop installer.\n'
    return 1
  fi
  { coproc BADI_WORKER { exec node "$badi_bridge" 2>&3 3>&-; }; } 3>&2 2>/dev/null
  BADI_READER_FD=${BADI_WORKER[0]}
  BADI_WRITER_FD=${BADI_WORKER[1]}
  disown "$BADI_WORKER_PID" 2>/dev/null || true
  if ! IFS=' ' read -r -t 3 -u "$BADI_READER_FD" BADI_REPLY_STATUS BADI_REPLY_VALUE ||
      [[ $BADI_REPLY_STATUS != READY ]]; then
    _badi_stop
    printf '\nBadi: local model unavailable. Run badi doctor.\n'
    return 1
  fi
}

_badi_words() {
  # READLINE_POINT counts characters since Bash 5.0 and bytes before it.
  if (( BASH_VERSINFO[0] < 5 )); then local LC_ALL=C; fi
  local badi_encoded badi_text badi_replace badi_display
  [[ -v READLINE_LINE ]] || return 0
  _badi_renderer || return 0
  _badi_start || return 0
  _badi_exchange '{"operation":"status"}' || return 0
  if [[ $BADI_REPLY_STATUS != READY ]]; then
    _badi_clear_preview
    [[ $BADI_REPLY_STATUS == BLOCKED ]] || _badi_stop
    printf '\nBadi: %s.\n' "${BADI_REPLY_VALUE:-model_unavailable}"
    return 0
  fi
  if [[ ${BADI_CONNECTION:-} != "$BADI_REPLY_VALUE" ]]; then
    _badi_clear_preview
    BADI_CONNECTION=$BADI_REPLY_VALUE
  fi
  if ! badi_preview eligible "$READLINE_LINE" || [[ -z ${READLINE_LINE//[[:space:]]/} ]]; then
    _badi_clear_preview
    printf '\nBadi: use the end of a nonempty line within 8192 bytes.\n'
    return
  fi
  if [[ ${BADI_PREFIX:-} == "$READLINE_LINE" && -n ${BADI_PREVIEW:-} ]] && badi_preview active "$READLINE_LINE"; then
    _badi_exchange '{"operation":"accept"}' || return 0
    if [[ $BADI_REPLY_STATUS == INSERT ]]; then
      badi_text=$(printf '%s' "$BADI_REPLY_VALUE" | base64 --decode) || return
      badi_replace=
      if [[ ${BADI_REPLY_REPLACE:--} != - ]]; then
        badi_replace=$(printf '%s' "$BADI_REPLY_REPLACE" | base64 --decode) || return
      fi
      if [[ $badi_text != "$BADI_PREVIEW" || $badi_replace != "${BADI_REPLACE:-}" ||
          ( -n $badi_replace && $READLINE_LINE != *"$badi_replace" ) ]]; then
        _badi_exchange '{"operation":"result","applied":false}' || true
        _badi_clear_preview
        return
      fi
      _badi_clear_preview
      READLINE_LINE=${READLINE_LINE%"$badi_replace"}$badi_text
      READLINE_POINT=${#READLINE_LINE}
      _badi_exchange '{"operation":"result","applied":true}' || return 0
      return
    fi
  else
    _badi_clear_preview
    badi_encoded=$(printf '%s' "$READLINE_LINE" | base64 | tr -d '\n')
    _badi_exchange "{\"operation\":\"suggest\",\"before\":\"$badi_encoded\"}" || return 0
    if [[ $BADI_REPLY_STATUS == SUGGEST ]]; then
      BADI_PREFIX=$READLINE_LINE
      BADI_PREVIEW=$(printf '%s' "$BADI_REPLY_VALUE" | base64 --decode) || return
      BADI_REPLACE=
      if [[ ${BADI_REPLY_REPLACE:--} != - ]]; then
        BADI_REPLACE=$(printf '%s' "$BADI_REPLY_REPLACE" | base64 --decode) || return
      fi
      badi_display=$BADI_PREVIEW
      if [[ -n $BADI_REPLACE ]]; then badi_display="  ${BADI_REPLACE% } → ${BADI_PREVIEW% }"; fi
      if ! badi_preview show "$badi_display" "$BADI_PREFIX"; then
        _badi_dismiss
        printf '\nBadi: the inline preview needs an unwrapped line with room after the caret.\n'
      fi
      return
    fi
  fi
  _badi_clear_preview
  printf '\nBadi: %s. Press Ctrl-X Tab to retry; badi debug watch shows activity.\n' "${BADI_REPLY_VALUE:-no_suggestion}"
}

_badi_dismiss() {
  if _badi_alive; then _badi_exchange '{"operation":"cancel"}' || true; fi
  _badi_clear_preview
}

bind -x '"\C-x\C-i":_badi_words'
bind -x '"\C-x\e":_badi_dismiss'
