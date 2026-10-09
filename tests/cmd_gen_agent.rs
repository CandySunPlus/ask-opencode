/// 仓库 cmd-gen agent frontmatter 里 `permission.bash` 的规则，按文件顺序取 `(模式, 动作)`
/// （独立按文件格式切，不复用 build.rs）。
fn bash_rules() -> Vec<(String, String)> {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("agents/cmd-gen.md"),
    )
    .unwrap();
    let rules: Vec<(String, String)> = text
        .lines()
        .skip_while(|line| *line != "  bash:")
        .skip(1)
        .take_while(|line| line.starts_with("    "))
        .map(|line| {
            let (pattern, action) = line
                .trim()
                .rsplit_once(": ")
                .unwrap_or_else(|| panic!("bash 规则行格式不对: {line}"));
            (pattern.trim_matches('"').to_string(), action.to_string())
        })
        .collect();
    assert!(!rules.is_empty(), "agent 文件缺 bash 规则");
    rules
}

fn allowed(rules: &[(String, String)]) -> Vec<&str> {
    rules
        .iter()
        .filter(|(_, action)| action == "allow")
        .map(|(pattern, _)| pattern.as_str())
        .collect()
}

#[test]
fn bash_allowlist_has_no_find_and_lists_git_ls_files() {
    let rules = bash_rules();
    let allowed = allowed(&rules);
    assert!(
        !allowed.iter().any(|pattern| pattern.starts_with("find")),
        "{allowed:?}"
    );
    assert!(allowed.contains(&"git ls-files*"), "{allowed:?}");
}

#[test]
fn bash_allowlist_only_admits_exact_read_only_git_branch_forms() {
    let rules = bash_rules();
    let branch: Vec<&str> = allowed(&rules)
        .into_iter()
        .filter(|pattern| pattern.starts_with("git branch"))
        .collect();
    assert_eq!(
        branch,
        [
            "git branch",
            "git branch -a",
            "git branch -r",
            "git branch -v",
            "git branch -vv",
            "git branch -av",
            "git branch --list",
            "git branch --show-current",
        ]
    );
}

#[test]
fn bash_denies_git_output_and_redirection_after_every_allow() {
    let rules = bash_rules();
    let last_allow = rules
        .iter()
        .rposition(|(_, action)| action == "allow")
        .unwrap();
    assert!(
        rules[..last_allow]
            .iter()
            .all(|(_, action)| action == "allow"),
        "deny 只能写在全部 allow 之后：{rules:?}"
    );
    let denies: Vec<&str> = rules[last_allow + 1..]
        .iter()
        .map(|(pattern, action)| {
            assert_eq!(action, "deny", "allow 之后只能是 deny：{pattern}");
            pattern.as_str()
        })
        .collect();
    assert_eq!(denies, ["git *--output*", "*>*"]);
}
