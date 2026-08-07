# 0001 Rust 二进制 + 薄 zsh 壳

插件的核心逻辑全部放在 Rust 二进制 `ask-opencode` 里（采集上下文快照、调用 `opencode run`、解析候选、校验、内嵌选择器），zsh 侧只做 zle widget、Tab 绑定、buffer 回填这三件必须原生 zsh 的事。仓库本身就是 Rust crate，核心逻辑在 Rust 里可测、JSON 解析和校验可靠，而命令行 buffer 操作只有 zsh 能做，两边各取所长。
