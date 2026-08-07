: ${_ask_opencode_cmd:=ask-opencode}

_ask_opencode_init() {
  (( ${_ask_opencode_loaded:-0} )) && return
  _ask_opencode_loaded=1

  zle -N _ask_opencode_expand
  zle -N _ask_opencode_fill
  zle -N _ask_opencode_poll

  local orig
  orig="${${(z)$(bindkey '^I' 2>/dev/null)}[-1]}"
  _ask_opencode_orig_tab="${orig:-expand-or-complete}"
  bindkey '^I' _ask_opencode_expand

  if bindkey -M viins '^I' >/dev/null 2>&1; then
    orig="${${(z)$(bindkey -M viins '^I' 2>/dev/null)}[-1]}"
    _ask_opencode_orig_viins="${orig:-expand-or-complete}"
    bindkey -M viins '^I' _ask_opencode_expand
  fi
}

_ask_opencode_expand() {
  if (( ${_ask_opencode_busy:-0} )); then
    zle -M "正在生成...（ask-opencode）"
    return
  fi

  if [[ $BUFFER == \#* ]]; then
    _ask_opencode_start
  else
    zle "${_ask_opencode_orig_tab:-expand-or-complete}"
  fi
}

_ask_opencode_start() {
  emulate -L zsh
  setopt extendedglob
  unsetopt bg_nice

  local request="${BUFFER#\#}"
  request="${request##[[:space:]]#}"
  if [[ -z $request ]]; then
    zle -M "请求为空：在 # 后输入描述"
    return
  fi

  local tmpdir
  tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/ask-opencode.XXXXXX") || {
    zle -M "ask-opencode：无法创建临时目录"
    return
  }

  _ask_opencode_busy=1
  _ask_opencode_tmpdir="$tmpdir"
  _ask_opencode_result="$tmpdir/candidates.json"
  _ask_opencode_error="$tmpdir/error.log"
  _ask_opencode_status="$tmpdir/status"

  (
    "$_ask_opencode_cmd" generate "$request" >"$_ask_opencode_result" 2>"$_ask_opencode_error"
    print -r -- "$?" >"$_ask_opencode_status"
  ) &

  zle -M "正在生成...（ask-opencode）"
  if ! _ask_opencode_schedule_poll; then
    _ask_opencode_busy=0
    _ask_opencode_cleanup
    zle -M "ask-opencode：无法安排结果轮询"
    return
  fi
  zle -R
}

_ask_opencode_schedule_poll() {
  zmodload -F zsh/sched b:sched 2>/dev/null || return
  sched +1 _ask_opencode_poll_event
}

_ask_opencode_poll_event() {
  if zle 2>/dev/null; then
    zle _ask_opencode_poll
  elif (( ${_ask_opencode_busy:-0} )); then
    _ask_opencode_schedule_poll
  fi
}

_ask_opencode_poll() {
  (( ${_ask_opencode_busy:-0} )) || return
  if [[ ! -r "$_ask_opencode_status" ]]; then
    zle -M "正在生成...（ask-opencode）"
    _ask_opencode_schedule_poll
    zle -R
    return
  fi

  local exit_code=1
  exit_code="$(<"$_ask_opencode_status")"

  if [[ "$exit_code" == 0 ]]; then
    _ask_opencode_ready_to_fill=1
    zle _ask_opencode_fill
    return
  fi

  local message="生成失败"
  [[ -s "$_ask_opencode_error" ]] && message="$(<"$_ask_opencode_error")"
  _ask_opencode_busy=0
  _ask_opencode_cleanup
  zle -M "${message}（请求行保留，可再按 Tab 重试）"
  zle -R
}

_ask_opencode_fill() {
  (( ${_ask_opencode_ready_to_fill:-0} )) || return
  _ask_opencode_ready_to_fill=0
  _ask_opencode_busy=0

  zle -I
  local selected rc
  if [[ -n ${TTY:-} && -r "$TTY" && -w "$TTY" ]]; then
    selected=$("$_ask_opencode_cmd" select --file "$_ask_opencode_result" <"$TTY" 2>"$TTY")
    rc=$?
  else
    selected=$("$_ask_opencode_cmd" select --file "$_ask_opencode_result")
    rc=$?
  fi

  if (( rc == 0 )) && [[ -n $selected ]]; then
    BUFFER="$selected"
    CURSOR=${#BUFFER}
    zle -M "已回填，回车执行"
  elif (( rc == 0 )); then
    zle -M "未选中，请求行保留"
  else
    zle -M "选择器出错（退出码 $rc），请求行保留"
  fi

  _ask_opencode_cleanup
  zle -R
}

_ask_opencode_cleanup() {
  [[ -n ${_ask_opencode_tmpdir:-} ]] && rm -rf "$_ask_opencode_tmpdir"
  unset _ask_opencode_tmpdir _ask_opencode_result _ask_opencode_error _ask_opencode_status
}

_ask_opencode_init
