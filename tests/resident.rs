mod common;
use common::*;
use serde_json::Value;
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

/// 支持常驻 serve 的 fake opencode shim（ADR-0004）：
/// `serve` 时打印监听行、把启动次数写入 $FAKE_SERVE_COUNT、exec fake HTTP 后端占用端口；
/// `run` 时把运行次数写入 $FAKE_RUN_COUNT、stdout 回可校验候选——常驻模式下不应走到该分支，
/// 请求走 serve 的 HTTP API。
const SHIM_RESIDENT: &str = r#"
if [ "$1" = "serve" ]; then
  echo serve >> "$FAKE_SERVE_COUNT"
  echo "opencode server listening on http://127.0.0.1:$FAKE_SERVE_PORT"
  exec python3 "$FAKE_SERVE_SCRIPT"
fi
echo run >> "$FAKE_RUN_COUNT"
printf 'echo hello\n---CANDIDATE---\nls -la\n'
"#;

fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap()
}

/// 找一个当前空闲的端口（用于 fake serve 监听与健康检查）。
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// 拼 fake opencode 驱动 generate 的完整 env 列表；`resident` 传 None 时不设开关（走默认）。
fn resident_envs(
    dir: &Path,
    shim: &Path,
    serve: &Path,
    port: u16,
    resident: Option<&str>,
) -> Vec<(String, String)> {
    resident_envs_with_config(dir, shim, serve, port, resident, &dir.join("config.json"))
}

/// 同 resident_envs，但可指定 ASK_OPENCODE_CONFIG 路径（用于配置目录缺失等场景）。
fn resident_envs_with_config(
    dir: &Path,
    shim: &Path,
    serve: &Path,
    port: u16,
    resident: Option<&str>,
    config: &Path,
) -> Vec<(String, String)> {
    let hist = write_history(dir, "");
    let mut envs: Vec<(String, String)> = vec![
        (
            "ASK_OPENCODE_BIN".to_string(),
            shim.to_str().unwrap().to_string(),
        ),
        (
            "ASK_OPENCODE_CONFIG".to_string(),
            config.to_str().unwrap().to_string(),
        ),
        ("HISTFILE".to_string(), hist.to_str().unwrap().to_string()),
        ("FAKE_SERVE_PORT".to_string(), port.to_string()),
        (
            "FAKE_SERVE_SCRIPT".to_string(),
            serve.to_str().unwrap().to_string(),
        ),
        (
            "FAKE_SERVE_COUNT".to_string(),
            dir.join("serve-count").to_str().unwrap().to_string(),
        ),
        (
            "FAKE_RUN_COUNT".to_string(),
            dir.join("run-count").to_str().unwrap().to_string(),
        ),
        (
            "FAKE_MSG_LOG".to_string(),
            dir.join("msg.log").to_str().unwrap().to_string(),
        ),
        (
            "FAKE_SESSION_LOG".to_string(),
            dir.join("session.log").to_str().unwrap().to_string(),
        ),
        ("FAKE_RESPONSE".to_string(), "echo hello\n---CANDIDATE---\nls -la\n".to_string()),
    ];
    if let Some(value) = resident {
        envs.push(("ASK_OPENCODE_RESIDENT".to_string(), value.to_string()));
    }
    // 常驻会话路径单独在 tests/session.rs 覆盖；这里默认关，保持 serve 生命周期用例聚焦进程复用。
    envs.push((
        "ASK_OPENCODE_REUSE_SESSION".to_string(),
        "false".to_string(),
    ));
    envs
}

/// 在共享目录里驱动 generate（不设 resident 开关，走默认开启）。
fn run_generate(dir: &Path, shim: &Path, serve: &Path, port: u16) -> std::process::Output {
    let envs = resident_envs(dir, shim, serve, port, None);
    run_in_dir_owned(dir, &["generate", "list files"], &envs)
}

fn read_serve_pid(dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(dir.join("server.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value["pid"].as_u64().map(|pid| pid as u32)
}

fn kill_serve(dir: &Path) {
    if let Some(pid) = read_serve_pid(dir) {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
    }
}

fn wait_port_closed(port: u16) {
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 常驻路径不该走 CLI `opencode run`：断言 run-count 文件不存在（shim 的 run 分支从未触发）。
fn assert_no_cli_runs(dir: &Path) {
    assert!(
        !dir.join("run-count").exists(),
        "常驻模式不应调用 CLI run"
    );
}

/// 首次调用自动拉起 serve，二次调用复用同一 URL 且不重新拉起；请求都走 serve 的 HTTP API，
/// 不再调用 CLI run。
#[test]
fn resident_starts_serve_once_and_reuses_url_across_calls() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_RESIDENT);
    let serve = write_fake_serve(dir.path());
    let port = free_port();

    let first = run_generate(dir.path(), &shim, &serve, port);
    assert!(first.status.success(), "stderr: {}", stderr_str(&first));
    assert_eq!(
        json_stdout(&first),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let second = run_generate(dir.path(), &shim, &serve, port);
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

    let msgs = std::fs::read_to_string(dir.path().join("msg.log")).unwrap();
    assert_eq!(msgs.lines().count(), 2, "两次请求都应走 HTTP API: {msgs}");
    kill_serve(dir.path());
}

/// 常驻 serve 死亡后再次调用应自动重新拉起。
#[test]
fn resident_restarts_serve_after_server_dies() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_RESIDENT);
    let serve = write_fake_serve(dir.path());
    let port = free_port();

    let first = run_generate(dir.path(), &shim, &serve, port);
    assert!(first.status.success(), "stderr: {}", stderr_str(&first));
    let pid = read_serve_pid(dir.path()).expect("首次调用应落盘 serve PID");
    std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    wait_port_closed(port);

    let second = run_generate(dir.path(), &shim, &serve, port);
    assert!(second.status.success(), "stderr: {}", stderr_str(&second));

    let serves = std::fs::read_to_string(dir.path().join("serve-count")).unwrap();
    assert_eq!(
        serves.matches("serve").count(),
        2,
        "死后应重新拉起: {serves}"
    );
    assert_no_cli_runs(dir.path());
    kill_serve(dir.path());
}

/// 配置文件 `"resident": false` 关闭常驻：每次冷启动，不拉起 serve、走 CLI run。
#[test]
fn resident_disabled_via_config_uses_cold_start() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), r#"{"resident":false}"#).unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_RESIDENT);
    let serve = write_fake_serve(dir.path());
    let port = free_port();

    let out = run_generate(dir.path(), &shim, &serve, port);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );

    let serves = std::fs::read_to_string(dir.path().join("serve-count")).unwrap_or_default();
    assert!(serves.is_empty(), "关闭常驻不应拉起 serve: {serves}");
    let runs = std::fs::read_to_string(dir.path().join("run-count")).unwrap_or_default();
    assert_eq!(runs.matches("run").count(), 1, "应走 CLI run: {runs}");
}

/// 环境变量关闭常驻：env 覆盖配置，冷启动路径同样生效。
#[test]
fn resident_disabled_via_env_uses_cold_start() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_RESIDENT);
    let serve = write_fake_serve(dir.path());
    let port = free_port();
    let envs = resident_envs(dir.path(), &shim, &serve, port, Some("false"));

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    let serves = std::fs::read_to_string(dir.path().join("serve-count")).unwrap_or_default();
    assert!(serves.is_empty(), "关闭常驻不应拉起 serve: {serves}");
    let runs = std::fs::read_to_string(dir.path().join("run-count")).unwrap_or_default();
    assert_eq!(runs.matches("run").count(), 1, "应走 CLI run: {runs}");
}

/// 首次调用即默认开启常驻：serve 自动拉起、请求走 HTTP API（不设任何 resident 开关）。
#[test]
fn resident_defaults_to_on() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_RESIDENT);
    let serve = write_fake_serve(dir.path());
    let port = free_port();

    let out = run_generate(dir.path(), &shim, &serve, port);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    let serves = std::fs::read_to_string(dir.path().join("serve-count")).unwrap_or_default();
    assert_eq!(
        serves.matches("serve").count(),
        1,
        "默认应开启常驻: {serves}"
    );
    assert_no_cli_runs(dir.path());
    let msgs = std::fs::read_to_string(dir.path().join("msg.log")).unwrap();
    assert_eq!(msgs.lines().count(), 1, "请求应走 HTTP API: {msgs}");
    kill_serve(dir.path());
}

/// serve 拉起失败（fake serve 立即非零退出）时退化为冷启动：不报错、正常出候选、stderr 有提示。
#[test]
fn resident_falls_back_to_cold_start_when_serve_fails() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(
        dir.path(),
        "if [ \"$1\" = \"serve\" ]; then\n\
         \x20 echo 'serve boom' >&2\n\
         \x20 exit 1\n\
         fi\n\
         echo run >> \"$FAKE_RUN_COUNT\"\n\
         printf 'echo hello\\n---CANDIDATE---\\nls -la\\n'",
    );
    let serve = write_fake_serve(dir.path());
    let port = free_port();

    let out = run_generate(dir.path(), &shim, &serve, port);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );
    assert!(
        stderr_str(&out).contains("resident:"),
        "serve 失败应提示降级: {}",
        stderr_str(&out)
    );
    let runs = std::fs::read_to_string(dir.path().join("run-count")).unwrap_or_default();
    assert_eq!(runs.matches("run").count(), 1, "应退化为 CLI run: {runs}");
}

/// 二次请求耗时显著低于冷启动：fake serve 启动带 1s 延迟，首次调用吃满该延迟，
/// 二次调用复用不重新拉起，耗时应明显更短（ADR-0004）。
#[test]
fn resident_second_request_is_faster_than_cold_start() {
    let dir = tempfile::tempdir().unwrap();
    // serve 分支先睡 1s 再打印监听行，模拟冷启动代价；run 分支正常出候选。
    let shim = write_fake_opencode(
        dir.path(),
        "if [ \"$1\" = \"serve\" ]; then\n\
         \x20 sleep 1\n\
         \x20 echo serve >> \"$FAKE_SERVE_COUNT\"\n\
         \x20 echo \"opencode server listening on http://127.0.0.1:$FAKE_SERVE_PORT\"\n\
         \x20 exec python3 \"$FAKE_SERVE_SCRIPT\"\n\
         fi\n\
         printf 'echo hello\\n'",
    );
    let serve = write_fake_serve(dir.path());
    let port = free_port();

    let start_first = std::time::Instant::now();
    let first = run_generate(dir.path(), &shim, &serve, port);
    let first_elapsed = start_first.elapsed();
    assert!(first.status.success(), "stderr: {}", stderr_str(&first));

    let start_second = std::time::Instant::now();
    let second = run_generate(dir.path(), &shim, &serve, port);
    let second_elapsed = start_second.elapsed();
    assert!(second.status.success(), "stderr: {}", stderr_str(&second));

    let serves = std::fs::read_to_string(dir.path().join("serve-count")).unwrap();
    assert_eq!(
        serves.matches("serve").count(),
        1,
        "二次调用不应重新拉起 serve: {serves}"
    );
    assert!(
        second_elapsed < first_elapsed / 2,
        "二次调用应显著快于冷启动：first={first_elapsed:?} second={second_elapsed:?}"
    );
    kill_serve(dir.path());
}

/// 配置目录不存在时也应自动创建并正常拉起 serve（回归：serve.log 打开失败曾导致常驻失效）。
#[test]
fn resident_creates_config_dir_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_RESIDENT);
    let serve = write_fake_serve(dir.path());
    let port = free_port();
    // 指向一个不存在的子目录里的 config.json，模拟用户从未建过配置目录。
    let cfg = dir.path().join("nested/deeper/config.json");
    let envs = resident_envs_with_config(dir.path(), &shim, &serve, port, None, &cfg);

    let out = run_in_dir_owned(dir.path(), &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );
    let state = dir.path().join("nested/deeper/server.json");
    assert!(
        state.exists(),
        "状态文件应落在自动创建的配置目录: {}",
        state.display()
    );
    assert!(
        dir.path().join("nested/deeper/serve.log").exists(),
        "serve 日志应落在自动创建的配置目录"
    );
    kill_serve(state.parent().unwrap());
}
