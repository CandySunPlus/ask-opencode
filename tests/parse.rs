mod common;
use common::*;
use serde_json::Value;

fn parse_stdin(input: &str) -> Value {
    let out = run_with_input(&["parse"], input);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn parse_splits_candidates_on_delimiter_lines() {
    let value = parse_stdin("echo one\n---CANDIDATE---\necho two\n");
    assert_eq!(value, serde_json::json!(["echo one", "echo two"]));
}

#[test]
fn parse_preserves_multiline_candidates() {
    let value = parse_stdin("cat x | grep foo\nsort -u\n---CANDIDATE---\necho done\n");
    assert_eq!(
        value,
        serde_json::json!(["cat x | grep foo\nsort -u", "echo done"])
    );
}

#[test]
fn parse_strips_markdown_fences() {
    let value = parse_stdin("```bash\necho hi\n```\n---CANDIDATE---\nls\n");
    assert_eq!(value, serde_json::json!(["echo hi", "ls"]));
}

#[test]
fn parse_trims_surrounding_blank_lines() {
    let value = parse_stdin("\n\necho hi\n\n---CANDIDATE---\n\nls\n\n");
    assert_eq!(value, serde_json::json!(["echo hi", "ls"]));
}

#[test]
fn parse_skips_empty_blocks() {
    let value = parse_stdin("echo one\n---CANDIDATE---\n---CANDIDATE---\necho two\n");
    assert_eq!(value, serde_json::json!(["echo one", "echo two"]));
}

#[test]
fn parse_empty_input_yields_empty_list() {
    let value = parse_stdin("");
    assert_eq!(value, serde_json::json!([]));
}

#[test]
fn parse_without_delimiter_yields_single_candidate() {
    let value = parse_stdin("echo hi\nls\n");
    assert_eq!(value, serde_json::json!(["echo hi\nls"]));
}

#[test]
fn parse_reads_args_when_given() {
    let out = run(&["parse", "echo one", "---CANDIDATE---", "echo two"]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value, serde_json::json!(["echo one", "echo two"]));
}
