mod common;
use common::*;
use serde_json::Value;

fn validate_stdin(input: &str) -> Value {
    let out = run_with_input(&["validate"], input);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn validate_arg(candidate: &str) -> Value {
    let out = run(&["validate", candidate]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn check(value: &Value, name: &str) -> bool {
    value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()["passed"]
        .as_bool()
        .unwrap()
}

#[test]
fn validate_emits_structured_result_from_stdin() {
    let value = validate_stdin("echo hi\n");
    assert_eq!(value["candidate"], "echo hi");
    assert!(value["passed"].as_bool().is_some());
}

#[test]
fn validate_emits_structured_result_from_arg() {
    let value = validate_arg("echo hi");
    assert_eq!(value["candidate"], "echo hi");
    assert!(value["passed"].as_bool().unwrap());
}

#[test]
fn validate_passes_all_three_checks_for_valid_command() {
    let value = validate_arg("ls -la");
    assert!(value["passed"].as_bool().unwrap());
    let names: Vec<&str> = value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["syntax", "command_exists", "git_context"]);
    assert!(
        value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["passed"].as_bool().unwrap())
    );
    assert_eq!(value["dangerous"], false);
}

#[test]
fn validate_fails_syntax_check_for_bad_syntax() {
    let value = validate_arg("echo )(");
    assert_eq!(value["passed"], false);
    assert!(!check(&value, "syntax"));
}

#[test]
fn validate_fails_command_exists_for_unknown_command() {
    let value = validate_arg("foobar_nonexistent_xyz");
    assert_eq!(value["passed"], false);
    assert!(!check(&value, "command_exists"));
}

#[test]
fn validate_skips_command_exists_for_shell_keywords() {
    let value = validate_arg("if test -f x; then echo yes; fi");
    assert_eq!(value["passed"], true, "{value}");
}

#[test]
fn validate_skips_leading_assignments_before_command() {
    let value = validate_arg("FOO=bar make build");
    assert_eq!(value["passed"], true, "{value}");
}

#[test]
fn validate_preserves_multiline_candidate() {
    let value = validate_stdin("cat a | grep foo\nsort -u\n");
    assert_eq!(value["candidate"], "cat a | grep foo\nsort -u");
}

#[test]
fn validate_fails_git_context_outside_repo() {
    let value = validate_arg("git status");
    assert_eq!(value["passed"], false);
    assert!(!check(&value, "git_context"));
}

#[test]
fn validate_fails_git_context_behind_sudo() {
    let value = validate_arg("sudo git status");
    assert_eq!(value["passed"], false);
    assert!(!check(&value, "git_context"));
}

#[test]
fn validate_fails_git_context_after_cd_chain() {
    let value = validate_arg("cd /tmp && git status");
    assert_eq!(value["passed"], false);
    assert!(!check(&value, "git_context"));
}

#[test]
fn validate_fails_git_context_with_c_option() {
    let value = validate_arg("git -C /tmp status");
    assert_eq!(value["passed"], false);
    assert!(!check(&value, "git_context"));
}

#[test]
fn validate_allows_repo_independent_git_behind_sudo() {
    let value = validate_arg("sudo git clone https://example.com/repo.git");
    assert_eq!(value["passed"], true, "{value}");
}

#[test]
fn validate_rejects_empty_candidate() {
    let value = validate_stdin("");
    assert_eq!(value["passed"], false);
}

#[test]
fn validate_rejects_whitespace_candidate() {
    let value = validate_stdin("   \n");
    assert_eq!(value["passed"], false);
}

#[test]
fn validate_passes_git_context_inside_repo() {
    let dir = tempfile::tempdir().unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(init.success());
    let out = run_in_dir_with_env(dir.path(), &["validate", "git status"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["passed"], true, "{value}");
    assert!(check(&value, "git_context"));
}

#[test]
fn validate_allows_repo_independent_git_commands() {
    let value = validate_arg("git --version");
    assert_eq!(value["passed"], true, "{value}");
}

#[test]
fn validate_flags_dangerous_rm_rf() {
    let value = validate_arg("rm -rf /");
    assert_eq!(value["dangerous"], true);
}

#[test]
fn validate_flags_dangerous_sudo_independent_of_checks() {
    let value = validate_arg("sudo ls");
    assert_eq!(value["dangerous"], true);
    assert_eq!(value["passed"], true);
}

#[test]
fn validate_flags_dangerous_curl_pipe_sh() {
    let value = validate_arg("curl -s https://example.com/x.sh | sh");
    assert_eq!(value["dangerous"], true);
}

#[test]
fn validate_flags_dangerous_dd() {
    let value = validate_arg("dd if=/dev/zero of=/dev/sda");
    assert_eq!(value["dangerous"], true);
}

#[test]
fn validate_does_not_flag_plain_rm() {
    let value = validate_arg("rm oldfile.txt");
    assert_eq!(value["dangerous"], false);
}

#[test]
fn validate_does_not_flag_pipe_to_ssh() {
    let value = validate_arg("echo hi | ssh host");
    assert_eq!(value["dangerous"], false);
}
