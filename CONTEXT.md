# Ask-Opencode

在 zsh 里按 Tab 把 `#` 请求交给 opencode 生成可执行命令、经选择器挑选后回填到命令行的插件。

术语成对定义为「中文 (English)」：中文文档和对话用中文术语，代码标识符用英文术语。中文没有对应自然口语时，不硬直译，保持英文。

## Language

**请求 (Request)**:
用户在命令行输入的、以行首 `#` 开头的一整行文字，按 Tab 后交给 opencode 生成候选。
_Avoid_: 提示词、prompt、问句

**候选命令 (Candidate)**:
opencode 针对一个请求返回的一条可执行命令建议，可能是多行指令，进入选择器供挑选。
_Avoid_: 结果、回答、生成内容

**上下文快照 (Context snapshot)**:
按 Tab 时实时采集、喂给 opencode 的 shell 当前状态（环境底盘、过滤后的命令历史，可选 dirstack/工具列表），目的是让生成的候选命令与该 shell 处境相关且可运行。git 状态与其余信息由 agent 自行只读侦查，不再由插件采集。
_Avoid_: 上下文、prompt 前缀、system prompt

**只读侦查 (read-only recon)**:
cmd-gen agent 生成候选前，自己用只读命令（`git status/diff/log`、`docker images/ps`、`ls/cat/find/grep` 等）收集上下文的行为；写、删、提交等有副作用的命令永远只出现在候选里由用户回填执行。
_Avoid_: 探索、探查

**回填 (Fill-in)**:
选中候选命令后把它写入 zsh 命令行 buffer、等待用户回车确认执行的那一步。
_Avoid_: 插入、粘贴

**危险命令 (danger command)**:
校验层判定为高破坏性、回填前需要二次确认的候选命令（`rm -rf`、`sudo`、`dd`、`curl | sh` 等）。
_Avoid_: 危险操作、敏感命令

**选择器 (picker)**:
在校验通过的候选命令里挑一条的交互组件，默认内嵌 skim、配置可切外部 fzf；候选只有一条时跳过选择器。
_Avoid_: 选择框、下拉

**常驻会话 (resident session)**:
ask-opencode 长期复用的那一个 opencode session，所有生成请求（含校验修正轮）都发往它，模型借此记住跨请求的上下文；与常驻服务是两个正交的东西——后者复用进程、前者复用会话。生命周期与常驻服务一致，用户可显式重置。
_Avoid_: 会话复用、session 共享、复用会话

### 发布与安装

**发布资产 (release asset)**:
GitHub Release 上按平台/架构构建的二进制压缩包，命名 `ask-opencode-<os>-<arch>-<version>.tar.gz`，`<os>` 取 `darwin`/`linux`，`<arch>` 取 `aarch64`/`x86_64`，同前缀带 `.sha256` 校验文件。
_Avoid_: 安装包、编译产物

**安装脚本 (install script)**:
仓库根 `install.sh`，把发布资产下载校验后装入 binary 目录、把插件脚本按同一 tag 从 raw 拉取装入插件目录；幂等覆盖、支持 `--uninstall`。
_Avoid_: 安装器、installer

**插件目录 (plugin directory)**:
安装脚本装载 `ask-opencode.plugin.zsh` 的目标目录，oh-my-zsh 下为 `$ZSH_CUSTOM/plugins/ask-opencode/`。
_Avoid_: 插件文件夹、插件位置

**等待动画 (waiting animation)**:
生成期间由后台动画进程驱动、在 zsh 状态行循环展示的 spinner + 轮换文案；非阻塞（ADR-0004），完成/失败时立即停止并被「已回填」/错误提示替换。
_Avoid_: 加载动画、进度条、loading spinner
