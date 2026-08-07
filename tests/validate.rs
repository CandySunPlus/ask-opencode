mod common;
use common::*;
use serde_json::Value;

#[test]
fn validate_emits_structured_result_from_stdin() {
    let out = run_with_input(&["validate"], "echo hi\n");
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["candidate"], "echo hi");
    assert!(value["passed"].as_bool().is_some());
}

#[test]
fn validate_emits_structured_result_from_arg() {
    let out = run(&["validate", "echo hi"]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["candidate"], "echo hi");
    assert!(value["passed"].as_bool().is_some());
}
