use std::io::Write;
use std::process::{Command, Stdio};

/// 弹选择器在候选里挑一条，返回选中的候选；取消（无选中）返回 None。
/// 选择器实现由 `picker` 决定（ADR-0006）：`skim` 内嵌、`fzf` 外部。
pub fn select(
    candidates: &[String],
    picker: &str,
    fzf_bin: &str,
) -> Result<Option<String>, String> {
    match picker {
        "skim" => select_skim(candidates),
        "fzf" => select_fzf(candidates, fzf_bin),
        other => Err(format!("未知选择器: {other}（可选 skim、fzf）")),
    }
}

/// 内嵌 skim：直接以候选为条目跑 TUI，返回被接受条目的完整文本。
fn select_skim(candidates: &[String]) -> Result<Option<String>, String> {
    use skim::prelude::*;
    let options = SkimOptionsBuilder::default()
        .height("50%")
        .build()
        .map_err(|e| format!("skim 初始化失败: {e}"))?;
    let output = Skim::run_items(options, candidates.to_vec())
        .map_err(|e| format!("skim 运行失败: {e}"))?;
    if output.is_abort {
        return Ok(None);
    }
    Ok(output
        .selected_items
        .first()
        .map(|item| item.output().into_owned()))
}

/// 外部 fzf：候选以 NUL 分隔喂给 `fzf --read0 --print0`，stdout 取回选中条目；
/// fzf 非零退出视为取消。NUL 分隔的原因见 ADR-0006。
fn select_fzf(candidates: &[String], fzf_bin: &str) -> Result<Option<String>, String> {
    let mut child = Command::new(fzf_bin)
        .args(["--read0", "--print0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动 fzf（{}）: {e}", fzf_bin))?;

    let mut input = String::new();
    for candidate in candidates {
        input.push_str(candidate);
        input.push('\0');
    }
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("写入 fzf 输入失败: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 fzf 退出失败: {e}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let selected = output
        .stdout
        .split(|&byte| byte == 0)
        .next()
        .unwrap_or_default();
    if selected.is_empty() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(selected).into_owned()))
}
