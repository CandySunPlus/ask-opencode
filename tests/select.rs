mod common;
use common::*;
use serde_json::Value;

/// 单候选：跳过选择器，直接输出该候选（不经过任何 picker）。
#[test]
fn select_skips_picker_for_single_candidate() {
    let out = run(&["select", "echo hi"]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "echo hi\n");
}

/// 单候选 + 危险命令 + 确认 y：输出该命令。
#[test]
fn select_confirms_dangerous_single_candidate_with_y() {
    let out = run_with_input(&["select", "rm -rf /"], "y\n");
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "rm -rf /\n");
}

/// 单候选 + 危险命令 + 回答 N：无输出。
#[test]
fn select_declines_dangerous_single_candidate_with_n() {
    let out = run_with_input(&["select", "rm -rf /"], "n\n");
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "");
}

/// 单候选 + 危险命令 + EOF（取消）：无输出。
#[test]
fn select_declines_dangerous_single_candidate_on_eof() {
    let out = run_with_input(&["select", "rm -rf /"], "");
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "");
}

/// 单候选 + 非危险命令：无需确认，直接输出。
#[test]
fn select_outputs_safe_single_candidate_without_confirmation() {
    let out = run_with_input(&["select", "echo hi"], "");
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "echo hi\n");
}

/// 多条候选 + fzf：候选经 fzf 选择后输出被选中的那条。
#[test]
fn select_uses_fzf_picker_for_multiple_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let fake_fzf = write_fake_fzf(dir.path());
    let input_log = dir.path().join("fzf-input.log");
    let out = run_with_env(
        &["select", "echo one", "echo two"],
        &[
            ("ASK_OPENCODE_PICKER", "fzf"),
            ("ASK_OPENCODE_FZF_BIN", fake_fzf.to_str().unwrap()),
            ("FZF_SELECT", "echo two"),
            ("FZF_INPUT_LOG", input_log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "echo two\n");
    let fed = std::fs::read_to_string(&input_log).unwrap();
    assert!(fed.contains("echo one\0echo two\0"), "候选应以 NUL 分隔喂入 fzf: {fed:?}");
}

/// 多条候选 + 危险命令被选中 + 确认 y：输出。
#[test]
fn select_confirms_dangerous_fzf_selection_with_y() {
    let dir = tempfile::tempdir().unwrap();
    let fake_fzf = write_fake_fzf(dir.path());
    let input_log = dir.path().join("fzf-input.log");
    let out = run_with(
        &["select", "echo one", "rm -rf /"],
        &[
            ("ASK_OPENCODE_PICKER", "fzf"),
            ("ASK_OPENCODE_FZF_BIN", fake_fzf.to_str().unwrap()),
            ("FZF_SELECT", "rm -rf /"),
            ("FZF_INPUT_LOG", input_log.to_str().unwrap()),
        ],
        "y\n",
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "rm -rf /\n");
}

/// 多条候选 + 危险命令被选中 + 回答 N：无输出。
#[test]
fn select_declines_dangerous_fzf_selection_with_n() {
    let dir = tempfile::tempdir().unwrap();
    let fake_fzf = write_fake_fzf(dir.path());
    let input_log = dir.path().join("fzf-input.log");
    let out = run_with(
        &["select", "echo one", "rm -rf /"],
        &[
            ("ASK_OPENCODE_PICKER", "fzf"),
            ("ASK_OPENCODE_FZF_BIN", fake_fzf.to_str().unwrap()),
            ("FZF_SELECT", "rm -rf /"),
            ("FZF_INPUT_LOG", input_log.to_str().unwrap()),
        ],
        "n\n",
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "");
}

/// 多条候选 + fzf 空选（fzf 取消）：无输出。
#[test]
fn select_outputs_nothing_when_fzf_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let fake_fzf = write_fake_fzf(dir.path());
    let input_log = dir.path().join("fzf-input.log");
    let out = run_with_env(
        &["select", "echo one", "echo two"],
        &[
            ("ASK_OPENCODE_PICKER", "fzf"),
            ("ASK_OPENCODE_FZF_BIN", fake_fzf.to_str().unwrap()),
            ("FZF_SELECT", ""),
            ("FZF_INPUT_LOG", input_log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "");
}

/// 配置 `picker` 可切 fzf：配置文件里 `"picker": "fzf"`，多候选走 fzf。
#[test]
fn select_switches_to_fzf_via_config() {
    let dir = tempfile::tempdir().unwrap();
    let fake_fzf = write_fake_fzf(dir.path());
    let input_log = dir.path().join("fzf-input.log");
    let cfg = dir.path().join("config.json");
    std::fs::write(
        &cfg,
        format!(r#"{{"picker":"fzf","fzf_bin":"{}"}}"#, fake_fzf.display()),
    )
    .unwrap();
    let out = run_with_env(
        &["select", "echo one", "echo two"],
        &[
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
            ("FZF_SELECT", "echo two"),
            ("FZF_INPUT_LOG", input_log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "echo two\n");
    assert!(
        std::fs::read_to_string(&input_log)
            .unwrap()
            .contains("echo one\0"),
        "应经 fzf 输入"
    );
}

/// 单候选不因 picker 配置而弹选择器：即便配置 fzf 也不调 fzf（fzf 不存在也不报错）。
#[test]
fn select_single_candidate_ignores_picker_config() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"picker":"fzf","fzf_bin":"/nonexistent/fzf"}"#).unwrap();
    let out = run_with_env(
        &["select", "echo hi"],
        &[("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap())],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "echo hi\n");
}

/// 未知 picker 值：报错退出。
#[test]
fn select_rejects_unknown_picker() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"picker":"vim"}"#).unwrap();
    let out = run_with_env(
        &["select", "echo one", "echo two"],
        &[("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap())],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_str(&out).contains("未知选择器"));
}

/// 空候选列表：报错退出。
#[test]
fn select_errors_on_no_candidates() {
    let out = run(&["select"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_str(&out).contains("没有候选命令"));
}

/// `--picker` 与 `--fzf-bin` 命令行开关覆盖配置。
#[test]
fn select_switches_picker_via_cli_flags() {
    let dir = tempfile::tempdir().unwrap();
    let fake_fzf = write_fake_fzf(dir.path());
    let input_log = dir.path().join("fzf-input.log");
    let out = run_with_env(
        &[
            "select",
            "--picker",
            "fzf",
            "--fzf-bin",
            fake_fzf.to_str().unwrap(),
            "echo one",
            "echo two",
        ],
        &[
            ("FZF_SELECT", "echo two"),
            ("FZF_INPUT_LOG", input_log.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    assert_eq!(stdout_str(&out), "echo two\n");
}

/// 校验结果 JSON 依然可用（回归：新增的 select 子命令不应破坏其它子命令）。
#[test]
fn select_does_not_break_validate() {
    let out = run(&["validate", "echo hi"]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["candidate"], "echo hi");
}
