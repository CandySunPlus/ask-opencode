mod common;
use common::*;
use serde_json::Value;

const OMP_OK: &str = "echo from-omp\n---CANDIDATE---\nls -la";

fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn generate_uses_omp_when_backend_env_is_omp() {
    let dir = tempfile::tempdir().unwrap();
    let omp = write_fake_omp(dir.path(), &[OMP_OK]);
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BACKEND", "omp"),
            ("ASK_OPENCODE_OMP_BIN", omp.bin.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo from-omp", "ls -la"])
    );
    assert_eq!(omp.calls(), 1);
}

/// 仓库 cmd-gen agent 文件去掉 frontmatter 后的正文（独立按文件格式切，不复用实现）。
fn agent_body() -> String {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".opencode/agents/cmd-gen.md"),
    )
    .unwrap();
    let mut parts = text.splitn(3, "---\n");
    assert_eq!(parts.next(), Some(""), "agent 文件应以 frontmatter 开头");
    parts.next().unwrap();
    parts.next().unwrap().trim_start_matches('\n').to_string()
}

/// argv 里 `flag` 后紧跟的值。
fn value_after(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).cloned()
}

fn omp_env(omp: &FakeOmp) -> Vec<(&'static str, String)> {
    vec![
        ("ASK_OPENCODE_BACKEND", "omp".to_string()),
        ("ASK_OPENCODE_OMP_BIN", omp.bin.display().to_string()),
    ]
}

fn run_omp(omp: &FakeOmp, args: &[&str], extra: &[(&str, &str)]) -> std::process::Output {
    let env = omp_env(omp);
    let mut envs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    envs.extend_from_slice(extra);
    run_with_env(args, &envs)
}

#[test]
fn generate_passes_json_mode_no_session_system_prompt_and_narrowing_flags_to_omp() {
    let dir = tempfile::tempdir().unwrap();
    let omp = write_fake_omp(dir.path(), &[OMP_OK]);
    let out = run_omp(&omp, &["generate", "list files by size"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let args = omp.args(1);
    assert_eq!(value_after(&args, "--mode").as_deref(), Some("json"));
    for flag in [
        "--no-session",
        "--no-skills",
        "--no-rules",
        "--no-extensions",
    ] {
        assert!(args.iter().any(|arg| arg == flag), "缺 {flag}: {args:?}");
    }
    let prompt = value_after(&args, "--system-prompt").expect("缺 --system-prompt");
    assert_eq!(prompt.trim_end(), agent_body().trim_end());
    assert!(prompt.starts_with("你是命令生成 agent"), "{prompt}");
    assert!(
        !args.iter().any(|arg| arg == "--model"),
        "model 空不应带 --model: {args:?}"
    );
    assert!(args.last().unwrap().contains("list files by size"));
}

#[test]
fn generate_uses_omp_when_config_file_backend_is_omp() {
    let dir = tempfile::tempdir().unwrap();
    let omp = write_fake_omp(dir.path(), &[OMP_OK]);
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"backend":"omp","model":""}"#).unwrap();
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
            ("ASK_OPENCODE_OMP_BIN", omp.bin.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo from-omp", "ls -la"])
    );
    assert!(!omp.args(1).iter().any(|arg| arg == "--model"));
}

#[test]
fn generate_passes_configured_model_to_omp() {
    let dir = tempfile::tempdir().unwrap();
    let omp = write_fake_omp(dir.path(), &[OMP_OK]);
    let out = run_omp(
        &omp,
        &[
            "generate",
            "list files",
            "--model",
            "anthropic/claude-haiku-4",
        ],
        &[],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        value_after(&omp.args(1), "--model").as_deref(),
        Some("anthropic/claude-haiku-4")
    );
}

#[test]
fn generate_ignores_agent_for_omp() {
    let dir = tempfile::tempdir().unwrap();
    let omp = write_fake_omp(dir.path(), &[OMP_OK]);
    let out = run_omp(&omp, &["generate", "list files", "--agent", "custom"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let args = omp.args(1);
    assert!(
        !args.iter().any(|arg| arg == "--agent" || arg == "custom"),
        "{args:?}"
    );
}

#[test]
fn generate_runs_correction_round_through_omp() {
    let dir = tempfile::tempdir().unwrap();
    let omp = write_fake_omp(
        dir.path(),
        &[
            "echo hello\n---CANDIDATE---\nfoobar_nonexistent_xyz",
            "echo fixed",
        ],
    );
    let out = run_omp(&omp, &["generate", "list files"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "echo fixed"])
    );
    assert_eq!(omp.calls(), 2);
    let fix_request = omp.args(2).last().unwrap().clone();
    assert!(
        fix_request.contains("foobar_nonexistent_xyz"),
        "{fix_request}"
    );
    assert!(omp.args(2).iter().any(|arg| arg == "--no-session"));
}

/// 会记录自己是否被调用的 fake opencode：用来断言 omp 失败时没有回退到 opencode。
fn opencode_tripwire(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let marker = dir.join("opencode-called");
    let bin = write_fake_opencode(
        dir,
        &format!(
            "touch \"{}\"\nprintf 'echo from-opencode\\n'",
            marker.display()
        ),
    );
    (bin, marker)
}

#[test]
fn generate_reports_missing_omp_without_falling_back_to_opencode() {
    let dir = tempfile::tempdir().unwrap();
    let (opencode, marker) = opencode_tripwire(dir.path());
    let missing = dir.path().join("no-omp");
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BACKEND", "omp"),
            ("ASK_OPENCODE_OMP_BIN", missing.to_str().unwrap()),
            ("ASK_OPENCODE_BIN", opencode.to_str().unwrap()),
        ],
    );
    assert!(!out.status.success());
    assert!(
        stderr_str(&out).contains("ASK_OPENCODE_OMP_BIN"),
        "{}",
        stderr_str(&out)
    );
    assert!(!marker.exists(), "不应回退到 opencode");
}

#[test]
fn generate_reports_omp_not_on_path_without_falling_back() {
    let dir = tempfile::tempdir().unwrap();
    let (opencode, marker) = opencode_tripwire(dir.path());
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BACKEND", "omp"),
            ("PATH", "/nonexistent"),
            ("ASK_OPENCODE_BIN", opencode.to_str().unwrap()),
        ],
    );
    assert!(!out.status.success());
    assert!(stderr_str(&out).contains("omp"), "{}", stderr_str(&out));
    assert!(!marker.exists(), "不应回退到 opencode");
}

#[test]
fn generate_propagates_omp_failure_without_falling_back() {
    let dir = tempfile::tempdir().unwrap();
    let (opencode, marker) = opencode_tripwire(dir.path());
    let omp = write_fake_bin(dir.path(), "omp", "printf 'model not found\\n' >&2\nexit 3");
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BACKEND", "omp"),
            ("ASK_OPENCODE_OMP_BIN", omp.to_str().unwrap()),
            ("ASK_OPENCODE_BIN", opencode.to_str().unwrap()),
        ],
    );
    assert_eq!(out.status.code(), Some(3));
    assert!(stderr_str(&out).contains("model not found"));
    assert!(!marker.exists(), "不应回退到 opencode");
}

#[test]
fn generate_with_omp_ignores_resident_and_never_starts_opencode_serve() {
    let dir = tempfile::tempdir().unwrap();
    let omp = write_fake_omp(dir.path(), &[OMP_OK]);
    let (opencode, marker) = opencode_tripwire(dir.path());
    let cfg = dir.path().join("config.json");
    let out = run_omp(
        &omp,
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_RESIDENT", "true"),
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
            ("ASK_OPENCODE_BIN", opencode.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo from-omp", "ls -la"])
    );
    assert!(!marker.exists(), "omp 后端不应拉起 opencode serve");
    assert!(
        !dir.path().join("server.json").exists(),
        "不应写常驻服务状态"
    );
    assert!(
        !stderr_str(&out).contains("resident"),
        "{}",
        stderr_str(&out)
    );
}

#[test]
fn generate_reports_omp_model_error_even_when_exit_code_is_zero() {
    let dir = tempfile::tempdir().unwrap();
    let ndjson = serde_json::json!({
        "type": "agent_end",
        "messages": [{"role": "assistant", "content": [], "stopReason": "error", "errorMessage": "rate limited"}]
    });
    let resp = dir.path().join("resp.ndjson");
    std::fs::write(&resp, format!("{ndjson}\n")).unwrap();
    let omp = write_fake_bin(dir.path(), "omp", &format!("cat \"{}\"", resp.display()));
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BACKEND", "omp"),
            ("ASK_OPENCODE_OMP_BIN", omp.to_str().unwrap()),
        ],
    );
    assert!(!out.status.success());
    assert!(
        stderr_str(&out).contains("rate limited"),
        "{}",
        stderr_str(&out)
    );
}
