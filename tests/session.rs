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

/// 有落盘 id 时二次请求不再走 json 首次路径（复用路径由 T2 接上，这里守格式不回退）。
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
