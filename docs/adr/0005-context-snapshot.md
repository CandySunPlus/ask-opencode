# 0005 上下文快照的采集口径

按 Tab 时把 shell 当前状态喂给 opencode 的上下文快照（见 `CONTEXT.md`「上下文快照」），采集口径如下。

命令历史：读 `HISTFILE`（缺省 `~/.zsh_history`），按 zsh 历史格式解析（`<ts>:<elapsed>;` 头 + 续行，命令内换行以反斜杠换行编码，解析时还原；无 extended 头时逐行裸命令兜底），过滤（剔除行首 `#` 的请求行与命中敏感规则的行）、去重保留最新、截取最近 `history_limit` 条（默认 20）。

敏感过滤：内置黑名单（token/password/secret/api_key 等键值对、任意协议 URL 内嵌凭据、超长行）命中任一条即整行剔除；配置 `sensitive_rules` 提供扩展正则，叠加在内置规则之上，env 提供的规则再追加到文件规则之上。

git 状态：不采集。agent 生成候选前自行只读侦查（`git status/diff/log` 等），插件不再注入；只读侦查的权限边界见 `CONTEXT.md`「只读侦查」与 cmd-gen agent 配置。

dirstack 与工具列表：默认关闭，开关 `include_dirstack`/`include_tools` 来自配置（文件 + env 覆盖）；内容由 zsh 侧经 `ASK_OPENCODE_DIRSTACK`（冒号分隔）与 `ASK_OPENCODE_TOOLS`（逗号分隔）注入。

采集只在本机拼进请求文本，对用户历史不做任何持久化写入。
