mod common;
use common::*;
use serde_json::Value;

const SHIM_OK: &str = "printf 'echo hello\\n---CANDIDATE---\\nls -la\\n'";
const SHIM_GARBAGE: &str =
    "printf 'Here is your command:\\n```bash\\nls -la\\n```\\n---CANDIDATE---\\necho done\\n'";
const SHIM_ECHO_ARGS: &str = "printf '%s\\n' \"$@\"";
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
    let shim = write_fake_opencode(dir.path(), SHIM_ECHO_ARGS);
    let out = run_with_env(
        &["generate", "list files by size", "--agent", "cmd-gen"],
        &[("ASK_OPENCODE_BIN", shim.to_str().unwrap())],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let text = stdout_str(&out);
    assert!(text.contains("run"));
    assert!(text.contains("--agent"));
    assert!(text.contains("cmd-gen"));
    assert!(text.contains("list files by size"));
}

#[test]
fn generate_uses_agent_from_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_ECHO_ARGS);
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"agent":"custom-agent"}"#).unwrap();
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert!(stdout_str(&out).contains("custom-agent"));
}

#[test]
fn generate_embeds_env_context_in_request() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_ECHO_ARGS);
    let out = run_in_dir_with_env(
        dir.path(),
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("SHELL", "/bin/zsh"),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let text = stdout_str(&out);
    assert!(
        text.contains(dir.path().to_str().unwrap()),
        "缺 cwd: {text}"
    );
    assert!(text.contains("macos"), "缺 OS: {text}");
    assert!(text.contains("/bin/zsh"), "缺 shell: {text}");
    assert!(text.contains("list files"), "缺请求: {text}");
}

#[test]
fn generate_passes_model_flag_when_given() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_ECHO_ARGS);
    let out = run_with_env(
        &[
            "generate",
            "list files",
            "--model",
            "anthropic/claude-haiku-4",
        ],
        &[("ASK_OPENCODE_BIN", shim.to_str().unwrap())],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let text = stdout_str(&out);
    assert!(text.contains("-m"), "缺 -m: {text}");
    assert!(text.contains("anthropic/claude-haiku-4"));
}

#[test]
fn generate_uses_model_from_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_ECHO_ARGS);
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"model":"anthropic/claude-haiku-4"}"#).unwrap();
    let out = run_with_env(
        &["generate", "list files"],
        &[
            ("ASK_OPENCODE_BIN", shim.to_str().unwrap()),
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let text = stdout_str(&out);
    assert!(text.contains("-m"), "缺 -m: {text}");
    assert!(text.contains("anthropic/claude-haiku-4"));
}

#[test]
fn generate_omits_model_flag_when_unset() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_ECHO_ARGS);
    let out = run_with_env(
        &["generate", "list files"],
        &[("ASK_OPENCODE_BIN", shim.to_str().unwrap())],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert!(!stdout_str(&out).contains("-m"));
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
