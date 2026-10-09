# 0009 omp 作为可选后端

在 opencode 之外新增 omp 作为可选后端（见 `CONTEXT.md`「后端」），两者并存、配置 `backend` 选定、默认 opencode；选定后端不可用时直接报错，不自动退到另一个后端——两边 agent 与模型配置不同，悄悄换后端产出的候选会让人困惑。生成调用收进 `Backend` 接口：冷启动/常驻、会话 id 从哪抓、何为会话失效都由后端自己决定，generate.rs 只保留「失效则清会话重试一次」的共享逻辑（ADR-0007）。接口的会话入参分三种用法而非一个可选 id：一次性（`reuse_session=false`）、新建并带回 id、续接已有 id——只给可选 id 分不清前两者，而它们在两个后端里走的路径不同（opencode 的 default/json 格式，omp 的 `--no-session` 与落盘新 id）。状态文件 `server.json` 按后端分开存，切回原后端会话还能续上；旧格式（顶层 `{url, pid, session_id}`）读作 opencode 分区、下次写入转成新格式，升级不丢常驻会话；`reset-session` 只清当前后端分区里的会话；`model` 只留一个字段，由当前后端解释，空则用后端自己的默认。

omp 后端与 opencode 有三处刻意不对称。其一，只走冷启动 `omp --mode json`、`resident` 对它不生效：omp 没有可共享的 HTTP 服务，`--mode rpc` 只活在 stdio 父进程里，要复用得自己写守护进程，而它冷启动自身开销约 1.5s，不值得。其二，常驻会话按目录各存一个（`{cwd → session_id}`，会话放在 ask-opencode 自己的 `--session-dir`）：omp 续接会话会把进程切回会话创建时的目录，全局一个会话会让只读侦查跑在旧目录里。其三，omp 没有主会话 `--agent`，cmd-gen 正文编进二进制、经 `--system-prompt` 传入，与 opencode 的 agent 文件共用同一份正文。

只读侦查在 omp 下必须与 opencode 同样严格：`--no-skills --no-rules --no-extensions` 收窄注入（项目 AGENTS.md 保留，与 opencode 一致），bash 用 `--config` 叠加的 `bash.patterns` 白名单并以 `"*": deny` 兜底（默认审批是全放行）；MCP 工具不受 `--tools` 约束，首选 `--approval-mode always-ask` 让非 read 档工具在非交互下直接失败，若与 bash 白名单不兼容则退回按服务器名写 `disabledExtensions`。
