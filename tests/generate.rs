mod common;
use common::*;
use serde_json::Value;

const SHIM_OK: &str = "printf 'echo hello\\n---CANDIDATE---\\nls -la\\n'";
const SHIM_GARBAGE: &str =
    "printf 'Here is your command:\\n```bash\\nls -la\\n```\\n---CANDIDATE---\\necho done\\n'";
const SHIM_FAIL: &str = "printf 'boom\\n' >&2\nexit 42";

fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn generate_outputs_parsed_candidates_from_contract_output() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_OK);
    let out = run_with_env(
        &["generate", "list files"],
        &[("ASK_OPENCODE_BIN", shim.to_str().unwrap())],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );
}

#[test]
fn generate_strips_fences_and_prose_from_garbage_output() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_GARBAGE);
    let out = run_with_env(
        &["generate", "list files"],
        &[("ASK_OPENCODE_BIN", shim.to_str().unwrap())],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["ls -la", "echo done"])
    );
}

#[test]
fn generate_propagates_fake_opencode_failure() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_FAIL);
    let out = run_with_env(
        &["generate", "list files"],
        &[("ASK_OPENCODE_BIN", shim.to_str().unwrap())],
    );
    assert_eq!(out.status.code(), Some(42));
    assert!(stderr_str(&out).contains("boom"));
}

#[test]
fn generate_passes_request_and_agent_to_opencode() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let shim = write_shim_echo_args(dir.path(), &log);
    let out = run_with_env(
        &["generate", "list files by size", "--agent", "cmd-gen"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("SHIM_ARGS_LOG", log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let args_log = std::fs::read_to_string(&log).unwrap();
    assert!(args_log.contains("run"));
    assert!(args_log.contains("--agent"));
    assert!(args_log.contains("cmd-gen"));
    assert!(request_from_log(&args_log).contains("list files by size"));
}

#[test]
fn generate_uses_agent_from_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let shim = write_shim_echo_args(dir.path(), &log);
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"agent":"custom-agent"}"#).unwrap();
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
            ("SHIM_ARGS_LOG", log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let args_log = std::fs::read_to_string(&log).unwrap();
    assert!(args_log.contains("custom-agent"));
}

#[test]
fn generate_embeds_env_context_in_request() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let shim = write_shim_echo_args(dir.path(), &log);
    let out = run_in_dir_with_env(
        dir.path(),
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("SHELL", "/bin/zsh"),
            ("SHIM_ARGS_LOG", log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let args_log = std::fs::read_to_string(&log).unwrap();
    let request = request_from_log(&args_log);
    assert!(
        request.contains(dir.path().to_str().unwrap()),
        "缺 cwd: {request}"
    );
    assert!(request.contains("macos"), "缺 OS: {request}");
    assert!(request.contains("/bin/zsh"), "缺 shell: {request}");
    assert!(request.contains("list files"), "缺请求: {request}");
}

#[test]
fn generate_passes_model_flag_when_given() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let shim = write_shim_echo_args(dir.path(), &log);
    let out = run_with_env(
        &[
            "generate",
            "list files",
            "--model",
            "anthropic/claude-haiku-4",
        ],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("SHIM_ARGS_LOG", log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let args_log = std::fs::read_to_string(&log).unwrap();
    assert!(args_log.contains("-m"), "缺 -m: {args_log}");
    assert!(args_log.contains("anthropic/claude-haiku-4"));
}

#[test]
fn generate_uses_model_from_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let shim = write_shim_echo_args(dir.path(), &log);
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"model":"anthropic/claude-haiku-4"}"#).unwrap();
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
            ("SHIM_ARGS_LOG", log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let args_log = std::fs::read_to_string(&log).unwrap();
    assert!(args_log.contains("-m"), "缺 -m: {args_log}");
    assert!(args_log.contains("anthropic/claude-haiku-4"));
}

#[test]
fn generate_omits_model_flag_when_unset() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let shim = write_shim_echo_args(dir.path(), &log);
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("SHIM_ARGS_LOG", log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let args_log = std::fs::read_to_string(&log).unwrap();
    assert!(!args_log.contains("-m"));
}

#[test]
fn generate_fails_clearly_when_bin_missing() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist");
    let out = run_with_env(
        &["generate", "x"],
        &[("ASK_OPENCODE_BIN", missing.to_str().unwrap())],
    );
    assert!(!out.status.success());
    assert!(stderr_str(&out).contains("ASK_OPENCODE_BIN"));
}

#[test]
fn generate_requires_a_request() {
    let out = run(&["generate"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn generate_corrects_failing_candidates_once() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(
        dir.path(),
        "if printf '%s' \"$*\" | grep -q 修正; then\n\
         \x20 printf 'echo fixed\\n'\n\
         else\n\
         \x20 printf 'echo hello\\n---CANDIDATE---\\nfoobar_nonexistent_xyz\\n'\n\
         fi",
    );
    let out = run_with_env(
        &["generate", "list files"],
        &[("ASK_OPENCODE_BIN", shim.to_str().unwrap())],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "echo fixed"])
    );
}

#[test]
fn generate_corrects_git_command_outside_repo() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(
        dir.path(),
        "if printf '%s' \"$*\" | grep -q 修正; then\n\
         \x20 printf 'echo corrected\\n'\n\
         else\n\
         \x20 printf 'git add .\\n'\n\
         fi",
    );
    let out = run_with_env(
        &["generate", "list files"],
        &[("ASK_OPENCODE_BIN", shim.to_str().unwrap())],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(json_stdout(&out), serde_json::json!(["echo corrected"]));
}

#[test]
fn generate_silently_drops_candidates_still_failing_after_correction() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("calls.log");
    let shim = write_fake_opencode(
        dir.path(),
        "printf 'call\\n' >> \"$SHIM_CALLS\"\n\
         if printf '%s' \"$*\" | grep -q 修正; then\n\
         \x20 printf 'foobar_still_bad\\n'\n\
         else\n\
         \x20 printf 'foobar_first_bad\\n'\n\
         fi",
    );
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("SHIM_CALLS", log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let text = stdout_str(&out);
    assert_eq!(json_stdout(&out), serde_json::json!([]));
    assert!(!text.contains("校验"), "输出出现校验错误标记: {text}");
    assert!(!text.contains("error"), "输出出现错误标记: {text}");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(calls.matches("call").count(), 2, "应恰好一轮修正：{calls}");
}

#[test]
fn generate_skips_correction_when_all_pass() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("calls.log");
    let shim = write_fake_opencode(
        dir.path(),
        "printf 'call\\n' >> \"$SHIM_CALLS\"\n\
         printf 'echo hello\\n---CANDIDATE---\\nls -la\\n'",
    );
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("SHIM_CALLS", log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(
        json_stdout(&out),
        serde_json::json!(["echo hello", "ls -la"])
    );
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(calls.matches("call").count(), 1, "不应触发修正轮：{calls}");
}
