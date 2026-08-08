mod common;
use common::*;
use std::path::Path;
use std::path::PathBuf;

/// 建 tempdir + fake opencode（argv 写入 SHIM_ARGS_LOG，stdout 回一个可通过校验的候选）。
fn setup() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let shim = write_shim_echo_args(dir.path(), &log);
    (dir, shim)
}

/// 注入受控历史与额外 env，跑 generate，返回请求文本（最后一个 argv）。
fn generate_with(dir: &Path, shim: &Path, history: &str, extra_env: &[(&str, &str)]) -> String {
    let hist = write_history(dir, history);
    let log = dir.join("args.log");
    let mut envs: Vec<(String, String)> = vec![
        (
            "ASK_OPENCODE_BIN".to_string(),
            shim.to_str().unwrap().to_string(),
        ),
        ("HISTFILE".to_string(), hist.to_str().unwrap().to_string()),
        (
            "SHIM_ARGS_LOG".to_string(),
            log.to_str().unwrap().to_string(),
        ),
        (
            // 常驻 serve 单独在 tests/resident.rs 覆盖；这里关掉，保证上下文快照测试走冷启动。
            "ASK_OPENCODE_RESIDENT".to_string(),
            "false".to_string(),
        ),
        (
            // 指向不存在的配置路径，隔离开发者真实 ~/.config/ask-opencode/config.json。
            "ASK_OPENCODE_CONFIG".to_string(),
            dir.join("no-config.json").to_str().unwrap().to_string(),
        ),
    ];
    for (k, v) in extra_env {
        envs.push((k.to_string(), v.to_string()));
    }
    let out = run_in_dir_owned(dir, &["generate", "list files"], &envs);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let log_text = std::fs::read_to_string(&log).unwrap();
    request_from_log(&log_text)
}

#[test]
fn snapshot_injects_deduplicated_filtered_history_without_request_lines() {
    let (dir, shim) = setup();
    let long = format!("cat {}", "a".repeat(600));
    let history = [
        ": 1700000001:0;ls",
        ": 1700000002:0;# 怎么压缩这个目录",
        ": 1700000003:0;ls",
        ": 1700000004:0;git status",
        ": 1700000005:0;echo hello",
        ": 1700000006:0;echo hello",
        &format!(": 1700000007:0;{long}"),
    ]
    .join("\n");
    let text = generate_with(dir.path(), &shim, &history, &[]);
    assert!(text.contains("- ls"), "缺去重后 ls: {text}");
    assert!(text.contains("- git status"), "缺 git status: {text}");
    assert!(text.contains("- echo hello"), "缺 echo hello: {text}");
    assert!(!text.contains("怎么压缩"), "不应含 # 请求行: {text}");
    assert!(!text.contains(&"a".repeat(600)), "不应含超长行: {text}");
    assert_eq!(text.matches("- ls").count(), 1, "应去重: {text}");
    assert_eq!(text.matches("- echo hello").count(), 1, "应去重: {text}");
}

#[test]
fn snapshot_filters_history_lines_containing_credentials() {
    let (dir, shim) = setup();
    let history = [
        ": 1700000001:0;export API_TOKEN=abc123",
        ": 1700000002:0;curl https://user:p4ss@example.com/data",
        ": 1700000003:0;git clone https://github.com/foo/bar.git",
        ": 1700000004:0;echo done",
    ]
    .join("\n");
    let text = generate_with(dir.path(), &shim, &history, &[]);
    assert!(!text.contains("API_TOKEN"), "不应泄露 token: {text}");
    assert!(!text.contains("abc123"), "不应泄露 token 值: {text}");
    assert!(!text.contains("user:p4ss"), "不应泄露 URL 凭据: {text}");
    assert!(!text.contains("example.com"), "凭据 URL 整行应剔除: {text}");
    assert!(
        text.contains("git clone https://github.com/foo/bar.git"),
        "无凭据 URL 应保留: {text}"
    );
    assert!(text.contains("echo done"), "普通行应保留: {text}");
}

#[test]
fn snapshot_reconstructs_multiline_history_entries() {
    let (dir, shim) = setup();
    let history = [
        ": 1700000001:0;git clone https://github.com/foo/bar.git \\\\",
        "  ${ZSH_CUSTOM:-~/.oh-my-zsh}/custom/plugins/foo",
        ": 1700000002:0;echo done",
    ]
    .join("\n");
    let text = generate_with(dir.path(), &shim, &history, &[]);
    assert!(
        text.contains("git clone https://github.com/foo/bar.git"),
        "缺多行历史首行: {text}"
    );
    assert!(text.contains("ZSH_CUSTOM"), "缺多行历史续行: {text}");
}

#[test]
fn dirstack_and_tools_default_off() {
    let (dir, shim) = setup();
    let text = generate_with(
        dir.path(),
        &shim,
        "",
        &[
            ("ASK_OPENCODE_DIRSTACK", "/tmp/one:/tmp/two"),
            ("ASK_OPENCODE_TOOLS", "git,docker"),
        ],
    );
    assert!(!text.contains("/tmp/one"), "默认不应含 dirstack: {text}");
    assert!(!text.contains("docker"), "默认不应含工具列表: {text}");
}

#[test]
fn dirstack_and_tools_enabled_from_config() {
    let (dir, shim) = setup();
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"include_dirstack":true,"include_tools":true}"#).unwrap();
    let text = generate_with(
        dir.path(),
        &shim,
        "",
        &[
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
            ("ASK_OPENCODE_DIRSTACK", "/tmp/one:/tmp/two"),
            ("ASK_OPENCODE_TOOLS", "git,docker"),
        ],
    );
    assert!(text.contains("/tmp/one"), "缺 dirstack: {text}");
    assert!(text.contains("/tmp/two"), "缺 dirstack 第二项: {text}");
    assert!(text.contains("git, docker"), "缺工具列表: {text}");
}

#[test]
fn dirstack_toggle_overridden_by_env() {
    let (dir, shim) = setup();
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"include_dirstack":false}"#).unwrap();
    let text = generate_with(
        dir.path(),
        &shim,
        "",
        &[
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
            ("ASK_OPENCODE_INCLUDE_DIRSTACK", "true"),
            ("ASK_OPENCODE_DIRSTACK", "/tmp/one"),
        ],
    );
    assert!(
        text.contains("/tmp/one"),
        "env 应覆盖文件的关闭开关: {text}"
    );
}

#[test]
fn sensitive_rules_extendable_from_config() {
    let (dir, shim) = setup();
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"sensitive_rules":["MYCRED[0-9]+"]}"#).unwrap();
    let history = [": 1700000001:0;echo MYCRED42", ": 1700000002:0;echo plain"].join("\n");
    let text = generate_with(
        dir.path(),
        &shim,
        &history,
        &[("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap())],
    );
    assert!(!text.contains("MYCRED42"), "配置规则应过滤: {text}");
    assert!(text.contains("echo plain"), "普通行应保留: {text}");
}

#[test]
fn sensitive_rules_merged_from_env() {
    let (dir, shim) = setup();
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"sensitive_rules":["FROMFILE"]}"#).unwrap();
    let history = [
        ": 1700000001:0;echo FROMFILE hit",
        ": 1700000002:0;echo FROMENV hit",
        ": 1700000003:0;echo keep",
    ]
    .join("\n");
    let text = generate_with(
        dir.path(),
        &shim,
        &history,
        &[
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
            ("ASK_OPENCODE_SENSITIVE_RULES", "FROMENV"),
        ],
    );
    assert!(!text.contains("FROMFILE hit"), "文件规则应生效: {text}");
    assert!(!text.contains("FROMENV hit"), "env 规则应追加生效: {text}");
    assert!(text.contains("echo keep"), "普通行应保留: {text}");
}

#[test]
fn snapshot_excludes_git_sections() {
    let (dir, shim) = setup();
    let history = [": 1700000001:0;git status", ": 1700000002:0;echo done"].join("\n");
    let text = generate_with(dir.path(), &shim, &history, &[]);
    // 这些标签来自旧版 git 状态小节（ADR-0005 改为不采集，实现随 5d0a7ed 移除），防其回归。
    for marker in ["分支：", "status --short", "最近提交", "diff --stat", "git 状态"] {
        assert!(!text.contains(marker), "快照不应含 git 小节 {marker}: {text}");
    }
    assert!(text.contains("- git status"), "git 命令历史应保留: {text}");
    assert!(text.contains("- echo done"), "普通命令历史应保留: {text}");
}

#[test]
fn snapshot_reads_bare_format_history() {
    let (dir, shim) = setup();
    let history = ["ls", "# 怎么压缩这个目录", "git status", "echo done"].join("\n");
    let text = generate_with(dir.path(), &shim, &history, &[]);
    assert!(text.contains("- ls"), "裸命令历史应注入: {text}");
    assert!(text.contains("- git status"), "缺 git status: {text}");
    assert!(text.contains("- echo done"), "缺 echo done: {text}");
    assert!(!text.contains("怎么压缩"), "仍应剔除 # 请求行: {text}");
}

#[test]
fn history_limit_read_from_config() {
    let (dir, shim) = setup();
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"history_limit":2}"#).unwrap();
    let history = [
        ": 1700000001:0;one",
        ": 1700000002:0;two",
        ": 1700000003:0;three",
        ": 1700000004:0;four",
    ]
    .join("\n");
    let text = generate_with(
        dir.path(),
        &shim,
        &history,
        &[("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap())],
    );
    assert!(
        text.contains("- three") && text.contains("- four"),
        "应保留最近 2 条: {text}"
    );
    assert!(
        !text.contains("- one") && !text.contains("- two"),
        "超限应截断: {text}"
    );
}

#[test]
fn history_limit_overridden_by_env() {
    let (dir, shim) = setup();
    let cfg = dir.path().join("config.json");
    std::fs::write(&cfg, r#"{"history_limit":10}"#).unwrap();
    let history = [
        ": 1700000001:0;one",
        ": 1700000002:0;two",
        ": 1700000003:0;three",
    ]
    .join("\n");
    let text = generate_with(
        dir.path(),
        &shim,
        &history,
        &[
            ("ASK_OPENCODE_CONFIG", cfg.to_str().unwrap()),
            ("ASK_OPENCODE_HISTORY_LIMIT", "1"),
        ],
    );
    assert!(text.contains("- three"), "env 应覆盖为 1 条: {text}");
    assert!(!text.contains("- one") && !text.contains("- two"));
}
