# Ask-Opencode

在 zsh 里按 Tab 把 `#` 请求交给 opencode 生成可执行命令、经选择器挑选后回填到命令行的插件。

核心逻辑放在 Rust 二进制里（上下文快照采集、调用 opencode、解析候选、校验、选择器），zsh 侧只做 zle widget、Tab 绑定、buffer 回填这三件必须原生 zsh 的事（ADR-0001）。

## 工作方式

1. 在 zsh 命令行里以 `#` 开头输入一句话请求（如 `# 查看最近的 git 提交`），按 Tab。
2. 插件采集**上下文快照**（环境底盘、过滤后的命令历史，可选 dirstack/工具列表），在后台交给 opencode 的 cmd-gen agent 生成候选命令；生成期间 shell 不冻结、可继续输入，状态行循环播放**等待动画**（spinner + 轮换文案），重复 Tab 被忽略。
3. agent 输出 3 条**候选命令**（按分隔行契约），插件解析后先做校验：`zsh -n` 语法、首词命令存在性、git 仓库上下文。不过的自动回喂 opencode 修正一轮，仍不过则静默丢弃。
4. 校验通过的候选进入**选择器**：多条弹内嵌 skim（可切外部 fzf），只有一条时跳过。候选是**危险命令**（`rm -rf`、`sudo`、`dd`、`curl | sh` 等）时，回填前弹 `⚠ 危险命令，确认? [y/N]`。
5. 选中后**回填**到命令行 buffer 替换请求行，光标停在末尾，回车执行。

所有生成请求（含校验修正轮）都发往同一个**常驻会话**，模型借此记住跨请求的上下文；常驻服务把二次调用从冷启动的 10-30s 降到 1-3s。

## 安装

要求：`opencode`、zsh。

推荐用安装脚本（ADR-0008）从 GitHub Release 下载二进制与插件，不需要 Rust 工具链：

```sh
curl -fsSL https://raw.githubusercontent.com/CandySunPlus/ask-opencode/main/install.sh | sh
```

脚本把二进制装进 `~/.local/bin`（`-b <目录>` 可改），检测到 oh-my-zsh（`$ZSH_CUSTOM`，curl|sh 下回退 `$ZSH/custom`）时按同一 tag 把插件装进 `$ZSH_CUSTOM/plugins/ask-opencode/` 并打印启用提示，否则退化为只装二进制并提示手动 source。重复安装幂等覆盖；`--uninstall` 卸载；`-V <版本>` 指定版本（默认最新 release），平台无预编译资产（如 macOS x86_64）时提示本地构建。

手动构建（备选）：要求 Rust 工具链。

```sh
cargo build --release
# 把 target/release/ask-opencode 放进 PATH
```

加载 zsh 插件（放进 `fpath` 的插件目录，或在 `.zshrc` 里 source）：

```zsh
source /path/to/ask-opencode/zsh/ask-opencode.plugin.zsh
```

## 配置

配置文件默认在 `~/.config/ask-opencode/config.json`（可用环境变量 `ASK_OPENCODE_CONFIG` 覆盖路径），全字段可选；仓库里的 [`config.example.json`](config.example.json) 是全字段示例，可直接复制改名使用：

```json
{
  "agent": "cmd-gen",
  "model": "",
  "history_limit": 20,
  "include_dirstack": false,
  "include_tools": false,
  "sensitive_rules": ["AKIA[0-9A-Z]{16}"],
  "picker": "skim",
  "fzf_bin": "fzf",
  "resident": true,
  "reuse_session": true
}
```

- `agent` / `model`：调 opencode 用的 agent 与模型（`model` 空则用 opencode 默认）。
- `history_limit`：上下文快照注入的命令历史条数上限。
- `include_dirstack` / `include_tools`：是否注入 dirstack / 工具列表（由 zsh 侧经 `ASK_OPENCODE_DIRSTACK` / `ASK_OPENCODE_TOOLS` 注入）。
- `sensitive_rules`：敏感信息过滤的扩展正则，叠加在内置黑名单之上。
- `picker`：`skim`（内嵌）或 `fzf`（外部）。
- `resident`：是否启用常驻 opencode serve（ADR-0004）。
- `reuse_session`：是否复用同一个 opencode 会话（ADR-0007）。

环境变量逐个字段覆盖配置文件：`ASK_OPENCODE_HISTORY_LIMIT`、`ASK_OPENCODE_INCLUDE_DIRSTACK`、`ASK_OPENCODE_INCLUDE_TOOLS`、`ASK_OPENCODE_SENSITIVE_RULES`（逗号分隔，追加到文件规则之上）、`ASK_OPENCODE_PICKER`、`ASK_OPENCODE_FZF_BIN`、`ASK_OPENCODE_RESIDENT`、`ASK_OPENCODE_REUSE_SESSION`。

## 子命令

```sh
ask-opencode generate <请求>            # 调 opencode 为一个请求生成候选命令
ask-opencode parse [文本]               # 按分隔行契约把候选文本切块为结构化候选（不提供则读 stdin）
ask-opencode validate [候选]            # 校验一条候选命令，输出 JSON（不提供则读 stdin）
ask-opencode select --file <结果>       # 在候选里挑一条：弹选择器，危险命令需 [y/N] 确认后输出
ask-opencode reset-session              # 清空会话 id，下一次请求开全新会话；不动常驻服务
```

zsh 插件直接调用 `generate` 与 `select`，其余子命令供命令行调试与测试。

## 设计

各模块职责见 `src/`（`context` 快照采集、`parse` 候选解析与事件流、`validate` 校验、`picker` 选择器、`resident` 常驻服务与会话 id 持久化、`generate` 生成主流程与会话复用）。关键决策记录在 `docs/adr/`：

- ADR-0001 Rust 二进制 + 薄 zsh 壳
- ADR-0002 分隔行输出契约
- ADR-0003 校验静默策略
- ADR-0004 异步与常驻服务
- ADR-0005 上下文快照的采集口径
- ADR-0006 选择器与危险确认
- ADR-0007 常驻会话
- ADR-0008 curl 安装与发布资产

领域术语见 `CONTEXT.md`。

## 测试

```sh
cargo test
```

测试用 `assert_cmd` 黑盒驱动二进制，默认关常驻服务与会话复用以保证冷启动路径确定性；常驻与会话路径分别在 `tests/resident.rs`、`tests/session.rs` 单独覆盖，安装脚本与发布打包分别在 `tests/install.rs`、`tests/release_package.rs` 覆盖。zsh 插件侧用 `scripts/test-plugin.py` 起 PTY 回归成功/失败/子进程早亡三径。
