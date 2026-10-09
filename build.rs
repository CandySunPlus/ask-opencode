//! 把 cmd-gen agent 文件剥掉 frontmatter 后的正文写进 OUT_DIR，供 omp 后端经
//! `--system-prompt` 传入（ADR-0009：两个后端共用同一份正文）。

use std::path::Path;

const AGENT_FILE: &str = ".opencode/agents/cmd-gen.md";

fn main() {
    println!("cargo:rerun-if-changed={AGENT_FILE}");
    let text = std::fs::read_to_string(AGENT_FILE).expect("读 cmd-gen agent 文件失败");
    let body = strip_frontmatter(&text).expect("cmd-gen agent 文件缺 frontmatter");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    std::fs::write(Path::new(&out_dir).join("cmd_gen_prompt.md"), body)
        .expect("写 cmd-gen 正文失败");
}

/// 去掉首行 `---` 到下一个 `---` 行之间的 frontmatter 及其后的空行。
fn strip_frontmatter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    Some(rest[end + "\n---\n".len()..].trim_start_matches('\n'))
}
