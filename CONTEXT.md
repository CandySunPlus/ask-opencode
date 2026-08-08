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
