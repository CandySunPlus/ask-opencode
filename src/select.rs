use crate::cli::SelectArgs;
use crate::config::Config;
use crate::validate::is_dangerous;
use std::io::Write;
use std::path::Path;

/// 在候选里挑一条：单候选跳过选择器直接进入危险确认；多条弹选择器（ADR-0006）；
/// 选中的命令命中危险清单时先弹 [y/N] 确认，确认后把选定命令打到 stdout。
pub fn run(args: SelectArgs) -> i32 {
    let config = Config::load();
    let picker = args.picker.as_deref().unwrap_or(&config.picker);
    let fzf_bin = args.fzf_bin.as_deref().unwrap_or(&config.fzf_bin);

    let candidates = match resolve_candidates(args.file.as_deref(), args.candidates) {
        Ok(candidates) => candidates,
        Err(message) => {
            eprintln!("select: {message}");
            return 1;
        }
    };
    if candidates.is_empty() {
        eprintln!("select: 没有候选命令");
        return 1;
    }

    let selected = if candidates.len() == 1 {
        candidates[0].clone()
    } else {
        match crate::picker::select(&candidates, picker, fzf_bin) {
            Ok(Some(selected)) => selected,
            Ok(None) => return 0,
            Err(message) => {
                eprintln!("select: {message}");
                return 1;
            }
        }
    };

    if is_dangerous(&selected) && !confirm_dangerous() {
        return 0;
    }

    println!("{selected}");
    0
}

/// 候选来源：CLI 参数用于人工/测试直接调用；zsh 壳用 --file 交出 generate 的 JSON 数组。
fn resolve_candidates(file: Option<&Path>, candidates: Vec<String>) -> Result<Vec<String>, String> {
    match file {
        Some(path) if !candidates.is_empty() => Err(format!(
            "--file 已提供候选来源，不能再同时传候选参数：{}",
            path.display()
        )),
        Some(path) => load_candidates_file(path),
        None => Ok(candidates),
    }
}

fn load_candidates_file(path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("无法读取候选文件 {}：{err}", path.display()))?;
    serde_json::from_str::<Vec<String>>(&text)
        .map_err(|err| format!("候选文件不是 JSON 字符串数组 {}：{err}", path.display()))
}

/// 危险命令二次确认：提示打到 stderr（stdout 留给选定命令），读一行 stdin，
/// 仅显式回答 y/Y 放行；EOF、空白或其它输入一律拒绝。
fn confirm_dangerous() -> bool {
    eprint!("⚠ 危险命令，确认? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    match std::io::stdin().read_line(&mut answer) {
        Ok(_) => answer.trim().eq_ignore_ascii_case("y"),
        Err(_) => false,
    }
}
