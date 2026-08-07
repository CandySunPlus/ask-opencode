# 0004 异步与常驻服务

Tab 按下即返回控制权，任务在后台跑 `opencode run`，就绪后自动弹选择器；首次自动拉起常驻 `opencode serve`，之后 `run --attach` 复用（可配置关闭）。冷启动 10-30s 阻塞 shell 不可接受，常驻服务把二次调用降到 1-3s，代价是自管理一个后台进程，作为配置项可关。

就绪通知用 `zle -F` 事件而非定时轮询：后台任务把结果写文件、把退出码写 FIFO，`zle -F` 在 FIFO 可读时触发 handler。两条落地约束（见 zsh/ask-opencode.plugin.zsh）：

- 状态走 FIFO 而非结果文件——`zle -F` 的 handler 里读 FIFO fd 可靠，读普通文件反而会挂。
- `zle -F` 的 handler 上下文不能做终端 I/O，就绪后的选择器由 handler 转调一个 zsh widget 在前台跑；该 widget 上下文里进程 stdin/stderr 都不是终端，显式从控制终端 `$TTY` 重定向 stdin（危险确认）与 stderr（确认提示）。
