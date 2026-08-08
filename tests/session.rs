mod common;
use common::*;
use serde_json::Value;
use std::net::TcpListener;
use std::path::Path;

/// 支持常驻会话的 fake opencode shim（ADR-0007）：
/// `serve` 时打印监听行、exec nc 占端口保持存活；`run` 时把 argv 逐行写入 $FAKE_ARGS_LOG，
/// 按是否带 `--format json` 分支：json 输出带 `sessionID` 的事件流（`text` 事件含分隔符候选），
/// default 输出候选文本。
const SHIM_SESSION: &str = r#"
if [ "$1" = "serve" ]; then
  echo serve >> "$FAKE_SERVE_COUNT"
  echo "opencode server listening on http://127.0.0.1:$FAKE_SERVE_PORT"
  exec nc -lk 127.0.0.1 "$FAKE_SERVE_PORT" >/dev/null 2>&1
fi
echo run >> "$FAKE_RUN_COUNT"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
if printf '%s' "$*" | grep -q -- '--format json'; then
  printf '%s\n' '{"type":"step_start","sessionID":"sess-abc123","timestamp":1,"part":{}}'
  printf '%s\n' '{"type":"text","sessionID":"sess-abc123","timestamp":2,"part":{"id":"p1","type":"text","text":"echo hello\n---CANDIDATE---\nls -la\n","time":{"start":1,"end":2}}}'
else
  printf 'echo hello\n---CANDIDATE---\nls -la\n'
fi
"#;

fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap()
}

fn read_state(dir: &Path) -> Value {
    let text = std::fs::read_to_string(dir.join("server.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn read_serve_pid(dir: &Path) -> Option<u32> {
    read_state(dir)["pid"].as_u64().map(|pid| pid as u32)
}

fn kill_serve(dir: &Path) {
    if let Some(pid) = read_serve_pid(dir) {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
    }
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// argv 日志里 `flag` 与 `value` 是否相邻成对（日志格式：每条参数独占一行、以 `@@@` 分隔）。
fn has_pair(log: &str, flag: &str, value: &str) -> bool {
    log.contains(&format!("{flag}\n@@@\n{value}"))
}

/// 拼驱动 generate 的完整 env：fake bin、隔离配置路径与 shim 日志路径，外加 extra 覆盖。
fn session_envs(dir: &Path, shim: &Path, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let hist = write_history(dir, "");
    let mut envs: Vec<(String, String)> = vec![
        (
            "ASK_OPENCODE_BIN".to_string(),
            shim.to_str().unwrap().to_string(),
        ),
        (
            "ASK_OPENCODE_CONFIG".to_string(),
            dir.join("config.json").to_str().unwrap().to_string(),
        ),
        ("HISTFILE".to_string(), hist.to_str().unwrap().to_string()),
        (
            "FAKE_ARGS_LOG".to_string(),
            dir.join("args.log").to_str().unwrap().to_string(),
        ),
        (
            "FAKE_RUN_COUNT".to_string(),
            dir.join("run-count").to_str().unwrap().to_string(),
        ),
    ];
    for (k, v) in extra {
        envs.push((k.to_string(), v.to_string()));
    }
    envs
}

/// 无落盘 session id（reuse_session 默认开）时走 json 首次路径：
/// argv 带 `--format json`、不带 `--session`，候选从 `text` 事件重组后照常出，
/// 状态文件落盘 `session_id`（冷启动只有该字段）。
#[test]
fn first_request_uses_json_and_persists_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert!(
        has_pair(&args, "--format", "json"),
        "首次请求应带 --format json: {args}"
    );
    assert!(
        !args.contains("--session"),
        "首次请求不应带 --session: {args}"
    );

    let state = read_state(dir.path());
    assert_eq!(
        state["session_id"], "sess-abc123",
        "状态文件应落盘 session_id: {state}"
    );
    assert!(
        state.get("url").is_none() && state.get("pid").is_none(),
        "冷启动不应写 url/pid: {state}"
    );
}

/// 候选分隔符跨多条 `text` 事件时按序拼接仍能正确解析（ADR-0002 契约不变）。
#[test]
fn text_events_across_lines_are_joined_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(
        dir.path(),
        r#"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
if printf '%s' "$*" | grep -q -- '--format json'; then
  printf '%s\n' '{"type":"text","sessionID":"sess-multi","timestamp":1,"part":{"id":"a","type":"text","text":"echo hello\n---CANDIDATE---"}}'
  printf '%s\n' '{"type":"text","sessionID":"sess-multi","timestamp":2,"part":{"id":"b","type":"text","text":"\nls -la\n","time":{"start":1,"end":2}}}'
else
  printf 'echo hello\n'
fi
"#,
    );
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );
}

/// 有落盘 id 时二次请求复用同一会话（ADR-0007）：argv 带 `--format default --session <id>`、
/// 不带 `--format json`，不重新抓 id。
#[test]
fn second_request_skips_json_first_path() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    let first = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(first.status.success(), "stderr: {}", stderr_str(&first));
    assert_eq!(
        json_stdout(&first),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let second = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(second.status.success(), "stderr: {}", stderr_str(&second));
    assert_eq!(
        json_stdout(&second),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_eq!(
        args.matches("--format\n@@@\njson").count(),
        1,
        "只有首次请求走 json：{args}"
    );
    assert_eq!(
        args.matches("--format\n@@@\ndefault").count(),
        1,
        "二次请求应走 default 格式：{args}"
    );
    assert_eq!(
        args.matches("--session\n@@@\nsess-abc123").count(),
        1,
        "二次请求应复用已落盘会话：{args}"
    );
}

/// 校验修正轮复用主请求同一会话（ADR-0007）：主请求已落盘 id 时，修正请求带相同
/// `--session <id>`、default 格式，不抓新 id。用调用序号 shim 区分首次/二次/修正轮。
#[test]
fn correction_round_reuses_same_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(
        dir.path(),
        r#"
n=$(cat "$FAKE_CALL_N" 2>/dev/null || printf '0')
n=$((n+1))
echo "$n" > "$FAKE_CALL_N"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
case "$n" in
  1)
    printf '%s\n' '{"type":"step_start","sessionID":"sess-abc123","timestamp":1,"part":{}}'
    printf '%s\n' '{"type":"text","sessionID":"sess-abc123","timestamp":2,"part":{"id":"p1","type":"text","text":"echo ok\n---CANDIDATE---\nls -la\n","time":{"start":1,"end":2}}}'
    ;;
  2)
    printf 'echo ok\n---CANDIDATE---\nfoobar_nonexistent_xyz\n'
    ;;
  3)
    printf 'echo fixed\n'
    ;;
esac
"#,
    );
    let envs = session_envs(
        dir.path(),
        &shim,
        &[
            ("ASK_OPENCODE_RESIDENT", "false"),
            (
                "FAKE_CALL_N",
                dir.path().join("call-n").to_str().unwrap(),
            ),
        ],
    );

    let first = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(first.status.success(), "stderr: {}", stderr_str(&first));
    assert_eq!(
        json_stdout(&first),
        serde_json::json!(["echo ok", "ls -la"])
    );

    let second = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(second.status.success(), "stderr: {}", stderr_str(&second));
    assert_eq!(
        json_stdout(&second),
        serde_json::json!(["echo ok", "echo fixed"])
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_eq!(
        args.matches("--format\n@@@\njson").count(),
        1,
        "只有首次请求走 json：{args}"
    );
    assert_eq!(
        args.matches("--session\n@@@\nsess-abc123").count(),
        2,
        "二次主请求与修正轮都应复用同一会话：{args}"
    );
}

/// 首次请求（json 路径建会话）若在同一次 run 内触发修正轮，修正轮复用刚落盘的同一 id、
/// 不抓新 id：主请求带 `--format json`，修正轮带 `--format default --session <id>`。
#[test]
fn correction_round_reuses_id_persisted_by_first_request() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(
        dir.path(),
        r#"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
if printf '%s' "$*" | grep -q 修正; then
  printf 'echo fixed\n'
else
  printf '%s\n' '{"type":"step_start","sessionID":"sess-abc123","timestamp":1,"part":{}}'
  printf '%s\n' '{"type":"text","sessionID":"sess-abc123","timestamp":2,"part":{"id":"p1","type":"text","text":"foobar_nonexistent_xyz\n","time":{"start":1,"end":2}}}'
fi
"#,
    );
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(json_stdout(&out), serde_json::json!(["echo fixed"]));

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_eq!(
        args.matches("--format\n@@@\njson").count(),
        1,
        "只有主请求走 json：{args}"
    );
    assert_eq!(
        args.matches("--format\n@@@\ndefault").count(),
        1,
        "修正轮应走 default 格式：{args}"
    );
    assert_eq!(
        args.matches("--session\n@@@\nsess-abc123").count(),
        1,
        "修正轮应复用刚落盘的会话：{args}"
    );
}

/// serve 模式首次请求：`--attach <url>` 与 `--format json` 同带，
/// 状态文件 `{url, pid}` 不受影响、只追加 `session_id`。
#[test]
fn first_request_in_serve_mode_keeps_url_and_pid() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let port = free_port();
    let envs = session_envs(
        dir.path(),
        &shim,
        &[
            ("ASK_OPENCODE_RESIDENT", "true"),
            ("FAKE_SERVE_PORT", &port.to_string()),
            (
                "FAKE_SERVE_COUNT",
                dir.path().join("serve-count").to_str().unwrap(),
            ),
        ],
    );

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let serves = std::fs::read_to_string(dir.path().join("serve-count")).unwrap();
    assert_eq!(
        serves.matches("serve").count(),
        1,
        "serve 应只拉起一次: {serves}"
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    let url = format!("http://127.0.0.1:{port}");
    assert!(
        has_pair(&args, "--attach", &url),
        "serve 模式应带 --attach: {args}"
    );
    assert!(
        has_pair(&args, "--format", "json"),
        "首次请求应带 --format json: {args}"
    );

    let state = read_state(dir.path());
    assert_eq!(state["url"], url, "状态应保留 url: {state}");
    assert!(state["pid"].as_u64().is_some(), "状态应保留 pid: {state}");
    assert_eq!(
        state["session_id"], "sess-abc123",
        "状态应落盘 session_id: {state}"
    );
    kill_serve(dir.path());
}

/// serve 模式二次请求：复用同一会话，argv 同时带 `--attach <url>` 与
/// `--format default --session <id>`，serve 仍只拉起一次。
#[test]
fn second_request_in_serve_mode_reuses_session_with_attach() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let port = free_port();
    let envs = session_envs(
        dir.path(),
        &shim,
        &[
            ("ASK_OPENCODE_RESIDENT", "true"),
            ("FAKE_SERVE_PORT", &port.to_string()),
            (
                "FAKE_SERVE_COUNT",
                dir.path().join("serve-count").to_str().unwrap(),
            ),
        ],
    );

    let first = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(first.status.success(), "stderr: {}", stderr_str(&first));
    assert_eq!(
        json_stdout(&first),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let second = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(second.status.success(), "stderr: {}", stderr_str(&second));
    assert_eq!(
        json_stdout(&second),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let serves = std::fs::read_to_string(dir.path().join("serve-count")).unwrap();
    assert_eq!(
        serves.matches("serve").count(),
        1,
        "serve 应只拉起一次: {serves}"
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    let url = format!("http://127.0.0.1:{port}");
    assert_eq!(
        args.matches(&format!("--attach\n@@@\n{url}")).count(),
        2,
        "两次请求都应带 --attach：{args}"
    );
    assert_eq!(
        args.matches("--format\n@@@\njson").count(),
        1,
        "只有首次请求走 json：{args}"
    );
    assert_eq!(
        args.matches("--format\n@@@\ndefault").count(),
        1,
        "二次请求应走 default 格式：{args}"
    );
    assert_eq!(
        args.matches("--session\n@@@\nsess-abc123").count(),
        1,
        "二次请求应复用已落盘会话：{args}"
    );
    kill_serve(dir.path());
}

/// serve 首次拉起时保留既有的 session_id：先冷启动落盘会话、后开常驻，状态不丢会话。
#[test]
fn starting_serve_preserves_persisted_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let port = free_port();

    let cold = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);
    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &cold);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(read_state(dir.path())["session_id"], "sess-abc123");

    let serve_envs = session_envs(
        dir.path(),
        &shim,
        &[
            ("ASK_OPENCODE_RESIDENT", "true"),
            ("FAKE_SERVE_PORT", &port.to_string()),
            (
                "FAKE_SERVE_COUNT",
                dir.path().join("serve-count").to_str().unwrap(),
            ),
        ],
    );
    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &serve_envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    let state = read_state(dir.path());
    assert_eq!(
        state["session_id"], "sess-abc123",
        "serve 拉起不应抹掉已落盘的会话: {state}"
    );
    assert_eq!(
        state["url"], format!("http://127.0.0.1:{port}"),
        "serve 应落盘 url: {state}"
    );
    assert!(state["pid"].as_u64().is_some(), "serve 应落盘 pid: {state}");
    kill_serve(dir.path());
}

/// 配置文件 `"reuse_session": false` 关闭会话复用：从不走 json、不带 --session、不落盘状态。
#[test]
fn reuse_session_disabled_via_config_skips_json_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), r#"{"reuse_session":false}"#).unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert!(!args.contains("--format"), "关闭复用不应走 json: {args}");
    assert!(
        !args.contains("--session"),
        "关闭复用不应带 --session: {args}"
    );
    assert!(
        !dir.path().join("server.json").exists(),
        "关闭复用不应落盘会话状态"
    );
}

/// 环境变量关闭会话复用：env 覆盖配置默认，json 路径与落盘同样全部跳过。
#[test]
fn reuse_session_disabled_via_env_skips_json_path() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let envs = session_envs(
        dir.path(),
        &shim,
        &[
            ("ASK_OPENCODE_RESIDENT", "false"),
            ("ASK_OPENCODE_REUSE_SESSION", "false"),
        ],
    );

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert!(!args.contains("--format"), "关闭复用不应走 json: {args}");
    assert!(
        !args.contains("--session"),
        "关闭复用不应带 --session: {args}"
    );
}

/// 关闭会话复用时即使有落盘 id 也不带 `--session`：预写状态文件模拟既存会话，
/// 连续两次请求都回归每次新会话。
#[test]
fn reuse_session_disabled_ignores_persisted_session_id() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), r#"{"reuse_session":false}"#).unwrap();
    std::fs::write(
        dir.path().join("server.json"),
        r#"{"url":"http://127.0.0.1:1","pid":1,"session_id":"sess-stale"}"#,
    )
    .unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    for _ in 0..2 {
        let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
        assert!(out.status.success(), "stderr: {}", stderr_str(&out));
        assert_eq!(
            json_stdout(&out),
            serde_json::json!(["echo hello", "ls -la"])
        );
    }

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert!(
        !args.contains("--session"),
        "关闭复用时不带 --session：{args}"
    );
    assert!(!args.contains("--format"), "关闭复用不应走 json: {args}");
}

/// 请求文本尾部带「旧上下文快照作废、以本条为准」声明（ADR-0007），把快照从会话记忆剥离。
#[test]
fn request_text_invalidates_old_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    let request = request_from_log(&args);
    assert!(
        request.contains("忽略本会话历史中的旧上下文快照，以本条为准"),
        "请求文本应含快照作废声明：{request}"
    );
}
