//! 把 cmd-gen agent 文件剥掉 frontmatter 后的正文写进 OUT_DIR，供 omp 后端经
//! `--system-prompt` 传入（ADR-0009：两个后端共用同一份正文）；frontmatter 里的 bash 规则
//! 也一并抽出（deny 挪到 allow 前面），作为 omp 只读叠加配置的来源，两个后端共用同一份 bash 规则。

use std::path::Path;

const AGENT_FILE: &str = "agents/cmd-gen.md";

fn main() {
    println!("cargo:rerun-if-changed={AGENT_FILE}");
    let text = std::fs::read_to_string(AGENT_FILE).expect("读 cmd-gen agent 文件失败");
    let body = strip_frontmatter(&text).expect("cmd-gen agent 文件缺 frontmatter");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    std::fs::write(Path::new(&out_dir).join("cmd_gen_prompt.md"), body)
        .expect("写 cmd-gen 正文失败");
    let mut rules = bash_rules(&text);
    assert!(!rules.is_empty(), "cmd-gen agent 文件缺 bash 规则");
    // 这一步就是 omp 下 deny 优先的保证；稳定排序不打乱相对顺序（ADR-0009）。
    rules.sort_by_key(|(_, action)| action != "deny");
    let lines: Vec<String> = rules
        .iter()
        .map(|(pattern, action)| format!("{pattern}\t{action}\n"))
        .collect();
    std::fs::write(
        Path::new(&out_dir).join("cmd_gen_bash_rules.tsv"),
        lines.concat(),
    )
    .expect("写 bash 规则失败");
}

/// frontmatter 里 `permission.bash` 下的规则，按文件顺序取 `(模式, 动作)`，模式去掉引号。
fn bash_rules(text: &str) -> Vec<(String, String)> {
    text.lines()
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
        .collect()
}

/// 去掉首行 `---` 到下一个 `---` 行之间的 frontmatter 及其后的空行。
fn strip_frontmatter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    Some(rest[end + "\n---\n".len()..].trim_start_matches('\n'))
}
