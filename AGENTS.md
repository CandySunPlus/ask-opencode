## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues, via the `gh` CLI. 实现票收尾时按**交付流程**走：功能分支提 PR（验收方法进正文）、勾 AC、等维护者合并——见 `docs/agents/issue-tracker.md`。

### Triage labels

Default five-role vocabulary, each label string equal to its name: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `CONTEXT.md` + `docs/adr/`. See `docs/agents/domain.md`.

## 语言

**面向人的输出一律用中文，且说人话**：对话回复、议题、PR 正文、提交信息，以及代码里的注释与文档注释。用户拿英文提问时也回中文。

**代码本身照旧英文**：标识符（含测试函数名）、命令、日志字段名、错误的 `class` 串。仓库现状就是这个形态，两边混着写反而难读。

**领域术语双轨**：中文文档和对话用 `CONTEXT.md` 里的中文术语，代码标识符用对应的英文术语。中文没有对应自然口语时，不硬直译，保持英文。


## 工程约定

### 引 ADR 的注释：指向，别复述

代码里提到 ADR 的注释分三类。**第一类和第三类该删**，另一类是资产：

| | 长什么样 | 怎么办 |
|---|---|---|
| **复述** | ADR 正文已有同样的论证，注释把那一段搬过来了 | 删掉论证，只留一句「这里为什么这样」加编号 |
| **指向** | 说的是「这一行是那条决策的哪一步」「改掉它会发生什么」 | 保持简短，留着 |
| **承载** | 理由只活在注释里，ADR 没写 | **整理到 ADR 后删** |


### markdown 一个段落写一行，别刻意换行

只限 markdown 文档，代码注释中不适用。

### 对接外部服务：先看官方 SDK

优先查找官方 SDK 并评估可用性，即使不直接使用，官方 SDK 也是更可靠的参照物，比文档更贴近现实。 
