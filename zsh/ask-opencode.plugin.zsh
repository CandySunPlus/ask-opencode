# ask-opencode zsh widget —— ADR-0001：zsh 侧只做 zle widget、Tab 绑定、buffer 回填。
# 加载：`source zsh/ask-opencode.plugin.zsh`（或放进插件目录）。
# 用法：行首以 `#` 开头输入请求（整行式触发），按 Tab；后台生成候选，
# 就绪后弹选择器（多候选）或直接进入危险确认（单候选），选中后回填替换请求行、
# 光标停在末尾，回车执行。生成中 shell 不冻结、重复 Tab 被忽略；失败时提示错误、
# 请求行原样保留可重试（ADR-0004）。

# ask-opencode 可执行文件：默认按 PATH 解析，可用该变量覆盖
: ${_ask_opencode_cmd:=ask-opencode}

# ---- 初始化：捕获 Tab 原绑定并绑定本 widget（只做一次） ----
_ask_opencode_init() {
  (( _ask_opencode_loaded )) && return
  _ask_opencode_loaded=1

  zle -N _ask_opencode_expand
  # 回填 widget：ADR-0004——zle -F 的 handler 不能做终端 I/O，转调它在前台跑选择器
  zle -N _ask_opencode_fill

  # 捕获原 Tab 绑定：非请求行回落给它；没有捕获到按 expand-or-complete 兜底
  local orig
  orig="${$(bindkey '^I')##* }"
  _ask_opencode_orig_tab="${orig:-expand-or-complete}"

  bindkey '^I' _ask_opencode_expand
  # viins 键位若存在，一并绑定（vi 模式按 Tab 同样可用）；回落沿用主键位的原绑定
  if bindkey -M viins '^I' >/dev/null 2>&1; then
    bindkey -M viins '^I' _ask_opencode_expand
  fi
}

_ask_opencode_init

# ---- Tab 触发：整行式触发，生成中忽略重复 Tab ----
_ask_opencode_expand() {
  if (( _ask_opencode_busy )); then
    zle -M "正在生成…（ask-opencode）"
    return
  fi
  # 整行式触发：仅当整行以 '#' 开头（与真实命令无歧义）
  if [[ $BUFFER == '#'* ]]; then
    _ask_opencode_start
  else
    zle "${_ask_opencode_orig_tab:-expand-or-complete}"
  fi
}

# ---- 起后台任务：请求行保持不动，FIFO + zle -F 通知完成 ----
_ask_opencode_start() {
  # 去行首 '#' 与紧随的空格（'# 请求' → '请求'）
  local request="${BUFFER#\#}"
  request="${request## }"
  if [[ -z $request ]]; then
    zle -M "请求为空：在 '#' 后输入描述"
    return
  fi

  _ask_opencode_busy=1
  local tmpdir
  tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/ask-opencode.XXXXXX")
  _ask_opencode_tmpdir=$tmpdir
  _ask_opencode_result="$tmpdir/result.json"
  _ask_opencode_fifo="$tmpdir/fifo"
  mkfifo "$_ask_opencode_fifo"

  # 后台：候选写结果文件，退出码经 FIFO 回传（ADR-0004）。
  # 错误信息折行成单行——FIFO 信号按行读取，多行错误只留首行会截断提示
  (
    "${_ask_opencode_cmd}" generate "$request" >"$_ask_opencode_result" 2>"$tmpdir/err.log"
    local code=$?
    if (( code == 0 )); then
      print -r -- "OK" >"$_ask_opencode_fifo"
    else
      local err
      err=$(<"$tmpdir/err.log")
      print -r -- "ERR ${err//$'\n'/ }" >"$_ask_opencode_fifo"
    fi
  ) &!
  local bg_pid=$!

  # 非阻塞打开 FIFO 读端并注册 handler；打不开时杀掉后台任务并复位（ADR-0004 的状态通道依赖它）
  zmodload zsh/system 2>/dev/null
  local fd
  if sysopen -r -o nonblock -u fd "$_ask_opencode_fifo" 2>/dev/null; then
    zle -F "$fd" _ask_opencode_ready
  else
    kill "$bg_pid" 2>/dev/null
    _ask_opencode_busy=0
    zle -M "ask-opencode：无法建立通知管道"
    return
  fi
  zle -M "正在生成…（ask-opencode）"
  zle -R
}

# ---- 完成通知（zle -F handler）：不碰终端，只备好状态并转调回填 widget ----
_ask_opencode_ready() {
  local fd=$1
  # 先摘除 handler 再排空，避免 EOF 状态下 fd 持续可读导致重入
  zle -F "$fd" 2>/dev/null
  # 重复触发（第一次已处理完）直接收尾返回
  if (( ! _ask_opencode_busy )); then
    exec {fd}<&- 2>/dev/null
    return
  fi
  local sig
  # 排空 FIFO 并取信号：可能整行「OK」或「ERR …」
  if read -r sig <&$fd 2>/dev/null; then
    :
  else
    sig="ERR 未收到完成信号"
  fi
  exec {fd}<&- 2>/dev/null

  if [[ $sig == OK ]]; then
    # ADR-0004：handler 不能做终端 I/O，转调回填 widget 在前台跑选择器
    _ask_opencode_ready=1
    zle _ask_opencode_fill
  else
    _ask_opencode_busy=0
    zle -M "${sig#ERR }（请求行保留，可再按 Tab 重试）"
  fi
}

# ---- 就绪回填（widget）：前台跑选择器，回填或保留请求行 ----
_ask_opencode_fill() {
  if (( ! _ask_opencode_ready )); then
    return
  fi
  _ask_opencode_ready=0
  _ask_opencode_busy=0
  local tmpdir=$_ask_opencode_tmpdir

  # 候选全被校验丢弃时 generate 输出空数组，select 会以「没有候选命令」报错退出；
  # 提前识别，给用户「无可用候选」而非系统故障的提示
  if [[ $(<"$_ask_opencode_result") == '[]' ]]; then
    zle -M "没有可运行的候选，请求行保留，可换一种说法再试"
    rm -rf "$tmpdir"
    return
  fi

  # 前台跑 select——选择器（默认内嵌 skim）接管终端，危险确认读的是终端 stdin。
  # ADR-0004：widget 上下文里进程 stdin/stderr 都不是终端，从 $TTY 重定向两者
  zle -I
  local selected rc
  selected=$("${_ask_opencode_cmd}" select --file "$_ask_opencode_result" <"$TTY" 2>"$TTY")
  rc=$?
  if (( rc == 0 )) && [[ -n $selected ]]; then
    BUFFER=$selected
    CURSOR=${#BUFFER}
    zle -M "已回填，回车执行"
  elif (( rc != 0 )); then
    zle -M "选择器出错（退出码 $rc），请求行保留"
  else
    zle -M "未选中，请求行保留"
  fi
  zle -R

  rm -rf "$tmpdir"
}
