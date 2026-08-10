mod common;
use common::*;
use serde_json::Value;
use std::net::TcpListener;
use std::path::Path;

/// 支持常驻会话的 fake opencode shim（ADR-0007）：
/// `serve` 时打印监听行、exec fake HTTP 后端占用端口；`run` 时把 argv 逐行写入 $FAKE_ARGS_LOG，
/// 按是否带 `--format json` 分支：json 输出带 `sessionID` 的事件流（`text` 事件含分隔符候选），
/// default 输出候选文本。常驻模式只走 serve 的 HTTP API，run 分支不应被触发。
const SHIM_SESSION: &str = r#"
if [ "$1" = "serve" ]; then
  echo serve >> "$FAKE_SERVE_COUNT"
  echo "opencode server listening on http://127.0.0.1:$FAKE_SERVE_PORT"
  exec python3 "$FAKE_SERVE_SCRIPT"
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

/// 常驻模式的完整 env：在 session_envs 之上追加 fake HTTP 后端所需的 serve 环境变量。
fn serve_envs(
    dir: &Path,
    shim: &Path,
    serve: &Path,
    port: u16,
    extra: &[(&str, &str)],
) -> Vec<(String, String)> {
    let mut envs = session_envs(dir, shim, extra);
    envs.push(("FAKE_SERVE_PORT".to_string(), port.to_string()));
    envs.push((
        "FAKE_SERVE_SCRIPT".to_string(),
        serve.to_str().unwrap().to_string(),
    ));
    envs.push((
        "FAKE_SERVE_COUNT".to_string(),
        dir.join("serve-count").to_str().unwrap().to_string(),
    ));
    envs.push((
        "FAKE_MSG_LOG".to_string(),
        dir.join("msg.log").to_str().unwrap().to_string(),
    ));
    envs.push((
        "FAKE_SESSION_LOG".to_string(),
        dir.join("session.log").to_str().unwrap().to_string(),
    ));
    envs.push((
        "FAKE_RESPONSE".to_string(),
        "echo hello\n---CANDIDATE---\nls -la\n".to_string(),
    ));
    envs
}

/// 常驻路径不该走 CLI `opencode run`：断言 run-count 文件不存在（shim 的 run 分支从未触发）。
fn assert_no_cli_runs(dir: &Path) {
    assert!(
        !dir.join("run-count").exists(),
        "常驻模式不应调用 CLI run"
    );
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

/// 断言冷启动 argv 日志符合「一次 json 首次 + 一次 default 复用」的会话形态：
/// json 恰一次、default 恰一次、`--session <sess-abc123>` 恰一次、不带 `--attach`。
fn assert_cold_session_reuse_args(args: &str) {
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
        "二次请求应复用同一会话：{args}"
    );
    assert!(!args.contains("--attach"), "冷启动不应带 --attach：{args}");
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
    assert_cold_session_reuse_args(&args);
}

/// 冷启动（resident=false）会话复用回归（ADR-0007）：连续请求全程不拉起 serve、
/// 不写 serve 日志，只复用同一个持久化 session id。
#[test]
fn cold_start_reuse_never_touches_serve() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let envs = session_envs(
        dir.path(),
        &shim,
        &[
            ("ASK_OPENCODE_RESIDENT", "false"),
            (
                "FAKE_SERVE_COUNT",
                dir.path().join("serve-count").to_str().unwrap(),
            ),
        ],
    );

    let first = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(first.status.success(), "stderr: {}", stderr_str(&first));
    let second = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(second.status.success(), "stderr: {}", stderr_str(&second));
    assert_eq!(
        json_stdout(&second),
        serde_json::json!(["echo hello", "ls -la"])
    );

    assert!(
        !dir.path().join("serve-count").exists(),
        "冷启动不应拉起 serve"
    );
    assert!(
        !dir.path().join("serve.log").exists(),
        "冷启动不应写 serve 日志"
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_cold_session_reuse_args(&args);
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
            ("FAKE_CALL_N", dir.path().join("call-n").to_str().unwrap()),
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

/// serve 模式首次请求：无落盘会话 id 时由 HTTP API 建新会话（POST /session）再发消息，
/// 状态文件 `{url, pid}` 不受影响、只追加新建的 `session_id`；不再调用 CLI run。
#[test]
fn first_request_in_serve_mode_keeps_url_and_pid() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let serve = write_fake_serve(dir.path());
    let port = free_port();
    let envs = serve_envs(
        dir.path(),
        &shim,
        &serve,
        port,
        &[("ASK_OPENCODE_RESIDENT", "true")],
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
    assert_no_cli_runs(dir.path());

    let sessions = std::fs::read_to_string(dir.path().join("session.log")).unwrap();
    assert_eq!(
        sessions.lines().count(),
        1,
        "首次请求应经 POST /session 建会话: {sessions}"
    );

    let state = read_state(dir.path());
    let url = format!("http://127.0.0.1:{port}");
    assert_eq!(state["url"], url, "状态应保留 url: {state}");
    assert!(state["pid"].as_u64().is_some(), "状态应保留 pid: {state}");
    assert_eq!(
        state["session_id"], "sess-http-1",
        "状态应落盘新建会话 id: {state}"
    );
    kill_serve(dir.path());
}

/// serve 模式二次请求：复用已落盘会话，消息仍走 HTTP API、不再新建会话、serve 仍只拉起一次。
#[test]
fn second_request_in_serve_mode_reuses_session() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let serve = write_fake_serve(dir.path());
    let port = free_port();
    let envs = serve_envs(
        dir.path(),
        &shim,
        &serve,
        port,
        &[("ASK_OPENCODE_RESIDENT", "true")],
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
    assert_no_cli_runs(dir.path());

    let sessions = std::fs::read_to_string(dir.path().join("session.log")).unwrap();
    assert_eq!(
        sessions.lines().count(),
        1,
        "二次请求应复用已落盘会话、不再新建: {sessions}"
    );
    let msgs = std::fs::read_to_string(dir.path().join("msg.log")).unwrap();
    assert_eq!(msgs.lines().count(), 2, "两次请求都应发消息: {msgs}");

    let state = read_state(dir.path());
    assert_eq!(state["session_id"], "sess-http-1");
    kill_serve(dir.path());
}

/// 常驻模式下校验修正轮复用主请求刚落盘的同一会话（ADR-0007）：主请求走 HTTP 首次路径
/// 建会话，触发修正轮后第二次 POST /message 不发 POST /session、仍落盘同一 id。
#[test]
fn correction_round_in_serve_mode_reuses_session() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let serve = write_fake_serve(dir.path());
    let port = free_port();
    let envs = serve_envs(
        dir.path(),
        &shim,
        &serve,
        port,
        &[
            ("ASK_OPENCODE_RESIDENT", "true"),
            (
                "FAKE_RESPONSE_1",
                "echo ok\n---CANDIDATE---\nfoobar_nonexistent_xyz\n",
            ),
            ("FAKE_RESPONSE_2", "echo fixed\n"),
        ],
    );

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo ok", "echo fixed"])
    );

    let sessions = std::fs::read_to_string(dir.path().join("session.log")).unwrap();
    assert_eq!(
        sessions.lines().count(),
        1,
        "主请求与修正轮不应各自建会话: {sessions}"
    );
    let msgs = std::fs::read_to_string(dir.path().join("msg.log")).unwrap();
    assert_eq!(msgs.lines().count(), 2, "主请求与修正轮各发一次消息: {msgs}");
    assert_no_cli_runs(dir.path());

    let state = read_state(dir.path());
    assert_eq!(state["session_id"], "sess-http-1");
    kill_serve(dir.path());
}

/// serve 首次拉起时保留既有的 session_id：先冷启动落盘会话、后开常驻，状态不丢会话。
#[test]
fn starting_serve_preserves_persisted_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_SESSION);
    let serve = write_fake_serve(dir.path());
    let port = free_port();

    let cold = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);
    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &cold);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(read_state(dir.path())["session_id"], "sess-abc123");

    let serve_envs = serve_envs(
        dir.path(),
        &shim,
        &serve,
        port,
        &[("ASK_OPENCODE_RESIDENT", "true")],
    );
    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &serve_envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    let state = read_state(dir.path());
    assert_eq!(
        state["session_id"], "sess-abc123",
        "serve 拉起不应抹掉已落盘的会话: {state}"
    );
    assert_eq!(
        state["url"],
        format!("http://127.0.0.1:{port}"),
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

/// 预写一个带旧会话 id 的状态文件，模拟 serve 重启或会话被清后落盘里的失效 id。
fn write_stale_session(dir: &Path) {
    std::fs::write(dir.join("server.json"), r#"{"session_id":"sess-stale"}"#).unwrap();
}

/// 复用请求因会话失效失败（exit 1、stderr 含 `Session not found`，如 serve 重启后）时
/// 自动重建（ADR-0007）：清旧 id、以 json 首次路径重跑一次本请求、落盘新 id，本次候选正常出。
#[test]
fn expired_session_rebuilds_via_json_first_path() {
    let dir = tempfile::tempdir().unwrap();
    write_stale_session(dir.path());
    // 带旧 id 的 default 请求硬失败；json 首次路径输出含新 session id 的事件流。
    let shim = write_fake_opencode(
        dir.path(),
        r#"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
if printf '%s' "$*" | grep -q -- '--format json'; then
  printf '%s\n' '{"type":"text","sessionID":"sess-new","timestamp":1,"part":{"id":"p1","type":"text","text":"echo hello\n---CANDIDATE---\nls -la\n"}}'
else
  echo 'Session not found' >&2
  exit 1
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

    let state = read_state(dir.path());
    assert_eq!(
        state["session_id"], "sess-new",
        "重建应落盘新会话 id: {state}"
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_eq!(
        args.matches("--session\n@@@\nsess-stale").count(),
        1,
        "旧 id 只应尝试一次：{args}"
    );
    assert_eq!(
        args.matches("--format\n@@@\njson").count(),
        1,
        "重建应走 json 首次路径：{args}"
    );
}

/// 只对 `Session not found` 降级（ADR-0007）：其它失败照常报错、不重试、不动状态文件。
#[test]
fn non_session_error_does_not_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    write_stale_session(dir.path());
    let shim = write_fake_opencode(
        dir.path(),
        r#"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
echo 'boom: something else broke' >&2
exit 1
"#,
    );
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_str(&out).contains("boom: something else broke"),
        "应回显原始错误: {}",
        stderr_str(&out)
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_eq!(
        args.matches("--session\n@@@\nsess-stale").count(),
        1,
        "其它失败不应重试：{args}"
    );
    assert_eq!(
        args.matches("--format\n@@@\njson").count(),
        0,
        "其它失败不应走 json 重建：{args}"
    );

    let state = read_state(dir.path());
    assert_eq!(
        state["session_id"], "sess-stale",
        "失败不应改动状态文件: {state}"
    );
}

/// 触发签名钉死为「退出码 1 + stderr 含该串」（ADR-0007）：退出码不对时即使 stderr 提到
/// `Session not found` 也不降级，照常报错、不重试、不动状态文件。
#[test]
fn session_not_found_with_wrong_exit_code_does_not_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    write_stale_session(dir.path());
    let shim = write_fake_opencode(
        dir.path(),
        r#"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
echo 'Session not found' >&2
exit 3
"#,
    );
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert_eq!(out.status.code(), Some(3), "应原样透传退出码");
    assert!(
        stderr_str(&out).contains("Session not found"),
        "应回显原始错误: {}",
        stderr_str(&out)
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_eq!(
        args.matches("--format\n@@@\njson").count(),
        0,
        "退出码不符不应走 json 重建：{args}"
    );

    let state = read_state(dir.path());
    assert_eq!(
        state["session_id"], "sess-stale",
        "不应改动状态文件: {state}"
    );
}

/// 重建后连续第二次请求复用新落盘的 id（ADR-0007）：不再走 json、带 `--session sess-new`。
#[test]
fn next_request_reuses_rebuilt_session_id() {
    let dir = tempfile::tempdir().unwrap();
    write_stale_session(dir.path());
    // 只认重建后的新 id，旧 id 与 json 都按场景分支。
    let shim = write_fake_opencode(
        dir.path(),
        r#"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
if printf '%s' "$*" | grep -q -- '--format json'; then
  printf '%s\n' '{"type":"text","sessionID":"sess-new","timestamp":1,"part":{"id":"p1","type":"text","text":"echo hello\n---CANDIDATE---\nls -la\n"}}'
elif printf '%s' "$*" | grep -q -- 'sess-new'; then
  printf 'echo hello\n---CANDIDATE---\nls -la\n'
else
  echo 'Session not found' >&2
  exit 1
fi
"#,
    );
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
        "只有重建那一次走 json：{args}"
    );
    assert_eq!(
        args.matches("--session\n@@@\nsess-new").count(),
        1,
        "二次请求应复用重建的新 id：{args}"
    );
    assert_eq!(
        args.matches("--session\n@@@\nsess-stale").count(),
        1,
        "旧 id 只应尝试一次：{args}"
    );
}

/// 重建后第二次仍失败时报错、不无限重试（ADR-0007）：json 首次路径也失败时只重跑一次。
#[test]
fn rebuild_retries_only_once_then_errors() {
    let dir = tempfile::tempdir().unwrap();
    write_stale_session(dir.path());
    let shim = write_fake_opencode(
        dir.path(),
        r#"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
if printf '%s' "$*" | grep -q -- '--format json'; then
  echo 'still broken' >&2
  exit 2
else
  echo 'Session not found' >&2
  exit 1
fi
"#,
    );
    let envs = session_envs(dir.path(), &shim, &[("ASK_OPENCODE_RESIDENT", "false")]);

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(!out.status.success(), "两次仍失败应报错");
    assert!(
        stderr_str(&out).contains("still broken"),
        "应回显重建失败的原始错误: {}",
        stderr_str(&out)
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_eq!(
        args.matches("--format\n@@@\njson").count(),
        1,
        "json 首次路径只应重试一次、不无限重试：{args}"
    );
    assert_eq!(
        args.matches("--session\n@@@\nsess-stale").count(),
        1,
        "旧 id 只应尝试一次：{args}"
    );
}

/// serve 模式下会话失效重建（ADR-0007）：预写一个 session_id 但没有 url 的状态文件，
/// 首请求发现 serve 不在跑而重新拉起——模拟「serve 重启后会话失效」；重建只清 session_id、
/// `{url, pid}` 保留，HTTP API 对失效 id 返回 404「Session not found」，随后以新会话重跑一次。
#[test]
fn expired_session_rebuild_keeps_serve_url_and_pid() {
    let dir = tempfile::tempdir().unwrap();
    write_stale_session(dir.path());
    let shim = write_fake_opencode(
        dir.path(),
        r#"
if [ "$1" = "serve" ]; then
  echo serve >> "$FAKE_SERVE_COUNT"
  echo "opencode server listening on http://127.0.0.1:$FAKE_SERVE_PORT"
  exec python3 "$FAKE_SERVE_SCRIPT"
fi
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
printf 'echo hello\n---CANDIDATE---\nls -la\n'
"#,
    );
    let serve = write_fake_serve(dir.path());
    let port = free_port();
    let envs = serve_envs(
        dir.path(),
        &shim,
        &serve,
        port,
        &[
            ("ASK_OPENCODE_RESIDENT", "true"),
            ("FAKE_404_SESSION", "sess-stale"),
        ],
    );

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let state = read_state(dir.path());
    assert_eq!(
        state["session_id"], "sess-http-1",
        "重建应落盘新会话 id: {state}"
    );
    assert_eq!(
        state["url"],
        format!("http://127.0.0.1:{port}"),
        "重建应保留 url: {state}"
    );
    assert!(state["pid"].as_u64().is_some(), "重建应保留 pid: {state}");

    // 失效 id 的消息 404 一次、重建后新会话的消息成功一次；重建走 POST /session 新建。
    let msgs = std::fs::read_to_string(dir.path().join("msg.log")).unwrap();
    assert_eq!(msgs.lines().count(), 2, "应尝试旧 id 一次、新会话一次: {msgs}");
    let sessions = std::fs::read_to_string(dir.path().join("session.log")).unwrap();
    assert_eq!(sessions.lines().count(), 1, "重建应新建一次会话: {sessions}");
    assert_no_cli_runs(dir.path());
    kill_serve(dir.path());
}

/// reset-session 只读改写状态文件，驱动它只需隔离配置路径（不调用 opencode）。
fn reset_envs(dir: &Path) -> Vec<(String, String)> {
    vec![(
        "ASK_OPENCODE_CONFIG".to_string(),
        dir.join("config.json").to_str().unwrap().to_string(),
    )]
}

/// `reset-session` 清空状态文件里的 `session_id`、保留 `{url, pid}`，退出码 0（ADR-0007）。
#[test]
fn reset_session_clears_session_id_and_keeps_url_pid() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("server.json"),
        r#"{"url":"http://127.0.0.1:1","pid":123,"session_id":"sess-stale"}"#,
    )
    .unwrap();
    let envs = reset_envs(dir.path());

    let out = run_in_dir_owned(dir.path(), &["reset-session"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    let state = read_state(dir.path());
    assert_eq!(state["url"], "http://127.0.0.1:1", "url 应保留: {state}");
    assert_eq!(state["pid"], 123, "pid 应保留: {state}");
    assert!(
        state.get("session_id").is_none(),
        "session_id 应被清空: {state}"
    );
}

/// 幂等：状态文件里本来就没有 `session_id` 时同样成功退出、`{url, pid}` 不动（ADR-0007）。
#[test]
fn reset_session_is_idempotent_when_no_session_id() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("server.json"),
        r#"{"url":"http://127.0.0.1:1","pid":123}"#,
    )
    .unwrap();
    let envs = reset_envs(dir.path());

    let out = run_in_dir_owned(dir.path(), &["reset-session"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    let state = read_state(dir.path());
    assert_eq!(state["url"], "http://127.0.0.1:1", "url 应保留: {state}");
    assert_eq!(state["pid"], 123, "pid 应保留: {state}");
    assert!(
        state.get("session_id").is_none(),
        "不应出现 session_id: {state}"
    );
}

/// 状态文件缺失时同样成功退出、不创建文件（幂等的最空形态）。
#[test]
fn reset_session_without_state_file_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let envs = reset_envs(dir.path());

    let out = run_in_dir_owned(dir.path(), &["reset-session"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert!(
        !dir.path().join("server.json").exists(),
        "无状态文件时不应创建文件"
    );
}

/// 不重拉、不杀常驻服务（ADR-0007）：reset-session 只读改写状态文件、从不调用 opencode。
/// 用「一被调用就落盘记录」的 shim 断言它连 opencode 可执行文件都不会去跑；
/// `{url, pid}` 保留、只清 `session_id`。
#[test]
fn reset_session_does_not_touch_running_serve() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("server.json"),
        r#"{"url":"http://127.0.0.1:1","pid":123,"session_id":"sess-stale"}"#,
    )
    .unwrap();
    let shim = write_fake_opencode(
        dir.path(),
        "echo invoked > \"$FAKE_RESET_INVOKED\"\nprintf 'echo hello\\n'",
    );
    let mut envs = reset_envs(dir.path());
    envs.push((
        "ASK_OPENCODE_BIN".to_string(),
        shim.to_str().unwrap().to_string(),
    ));
    envs.push((
        "FAKE_RESET_INVOKED".to_string(),
        dir.path()
            .join("reset-invoked")
            .to_str()
            .unwrap()
            .to_string(),
    ));

    let out = run_in_dir_owned(dir.path(), &["reset-session"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert!(
        !dir.path().join("reset-invoked").exists(),
        "reset-session 不应调用 opencode（否则会重拉或杀 serve）"
    );

    let state = read_state(dir.path());
    assert_eq!(state["url"], "http://127.0.0.1:1", "url 应保留: {state}");
    assert_eq!(state["pid"], 123, "pid 应保留: {state}");
    assert!(
        state.get("session_id").is_none(),
        "session_id 应被清空: {state}"
    );
}

/// 重置后下一次 generate 走首次路径建立新会话并落盘新 id（ADR-0007）：
/// 冷启动下两次请求都走 json、不带 `--session`，第二次不再复用旧 id、落盘新 id。
/// shim 按调用序号给不同 session id。
#[test]
fn reset_session_next_generate_establishes_new_session() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(
        dir.path(),
        r#"
n=$(cat "$FAKE_CALL_N" 2>/dev/null || printf '0')
n=$((n+1))
echo "$n" > "$FAKE_CALL_N"
for a in "$@"; do printf '%s\n@@@\n' "$a"; done >> "$FAKE_ARGS_LOG"
if printf '%s' "$*" | grep -q -- '--format json'; then
  if [ "$n" = "1" ]; then SID="sess-first"; else SID="sess-second"; fi
  printf '%s\n' "{\"type\":\"text\",\"sessionID\":\"$SID\",\"timestamp\":1,\"part\":{\"id\":\"p1\",\"type\":\"text\",\"text\":\"echo hello\n---CANDIDATE---\nls -la\n\"}}"
else
  printf 'echo hello\n---CANDIDATE---\nls -la\n'
fi
"#,
    );
    let envs = session_envs(
        dir.path(),
        &shim,
        &[
            ("ASK_OPENCODE_RESIDENT", "false"),
            ("FAKE_CALL_N", dir.path().join("call-n").to_str().unwrap()),
        ],
    );

    let first = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(first.status.success(), "stderr: {}", stderr_str(&first));
    assert_eq!(read_state(dir.path())["session_id"], "sess-first");

    let reset = run_in_dir_owned(dir.path(), &["reset-session"], &reset_envs(dir.path()));
    assert!(reset.status.success(), "stderr: {}", stderr_str(&reset));

    let second = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(second.status.success(), "stderr: {}", stderr_str(&second));
    assert_eq!(
        json_stdout(&second),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let state = read_state(dir.path());
    assert_eq!(
        state["session_id"], "sess-second",
        "应落盘新会话 id: {state}"
    );

    let args = std::fs::read_to_string(dir.path().join("args.log")).unwrap();
    assert_eq!(
        args.matches("--format\n@@@\njson").count(),
        2,
        "重置后应再次走 json 首次路径：{args}"
    );
    assert_eq!(
        args.matches("--session").count(),
        0,
        "两次请求都不应带 --session：{args}"
    );
}
