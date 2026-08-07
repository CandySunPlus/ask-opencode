mod common;
use common::*;

const SHIM_OK: &str = "printf 'echo hello\\n---CANDIDATE---\\nls -la\\n'";
const SHIM_ECHO_ARGS: &str = "printf '%s\\n' \"$@\"";
const SHIM_FAIL: &str = "printf 'boom\\n' >&2\nexit 42";

#[test]
fn generate_echoes_fake_opencode_stdout_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let shim = write_fake_opencode(dir.path(), SHIM_OK);
    let out = run_with_env(
        &["generate", "list files"],
        &[("ASK_OPENCODE_BIN", shim.to_str().unwrap())],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "echo hello\n---CANDIDATE---\nls -la\n");
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
    assert_eq!(
        stdout_str(&out).lines().collect::<Vec<_>>(),
        vec!["run", "--agent", "cmd-gen", "list files by size"],
    );
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
    assert_eq!(
        stdout_str(&out).lines().collect::<Vec<_>>(),
        vec!["run", "--agent", "custom-agent", "list files"],
    );
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
