# 0007 常驻会话

之前每次 Tab 都开一个全新 opencode session（常驻服务只复用进程、不复用会话），决定改为所有生成请求（含校验修正轮）都发往同一个「常驻会话」（见 `CONTEXT.md`），目的是让模型记住跨请求的上下文——这是共享会话唯一的实质收益，性能问题已有 ADR-0004 的常驻服务解决。会话全局一个、跨目录共享；单 shell 内由 busy 守卫串行（zsh 侧已有），跨 shell 并发竞态接受并写进文档，不设文件锁。

生命周期与常驻服务一致，新增 `reset-session` 子命令只清状态文件里的 `session_id`、不动 serve，下次请求自动开新会话；不做自动重置（何时换话题只有用户知道）。上下文快照照旧每次注入，但请求文本里显式声明「忽略本会话历史中的旧上下文快照，以本条为准」，把快照从会话记忆里剥离。会话由独立开关 `reuse_session`（默认开）控制，与 `resident` 正交：serve 路径 `run --attach <url> --session <id>`，冷启动路径同样用持久化 id 的 `--session <id>`，全程不用 `--continue`——它按目录取上一个会话，与「全局同一个」冲突。

session id 只能从 `--format json` 事件里抓，因此 `--format json` 只走「会话尚未建立」的首次路径：抓 id 落盘、从 text 事件重组候选文本再走 ADR-0002 解析；常规请求保持 default 格式（`--format default --session <id>`），解析契约不动，JSON schema 变更风险被隔离在窄路径。状态文件 `server.json` 扩为 `{url, pid, session_id?}`。`--session <id>` 对不存在的 id 会硬失败（`Session not found`），检测到后清 id、以新会话自动重试一次、重新落盘，第二次仍失败才报错。
