use crate::cli::ParseArgs;
use crate::io::read_stdin;

/// ADR-0002 分隔行：独占一行的候选分隔符。
pub const CANDIDATE_SEPARATOR: &str = "---CANDIDATE---";

/// 按 ADR-0002 分隔行契约把候选文本切块为候选命令列表。
pub fn split_candidates(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line == CANDIDATE_SEPARATOR {
            flush_block(&mut candidates, &mut current);
        } else {
            current.push(line);
        }
    }
    flush_block(&mut candidates, &mut current);
    candidates
}

/// 把候选列表以 JSON 输出到 stdout；序列化失败（理论上不可达）返回 1。
pub fn emit_candidates(candidates: &[String], label: &str) -> i32 {
    match serde_json::to_string(candidates) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(_) => {
            eprintln!("{label}: 无法序列化候选列表");
            1
        }
    }
}

/// 清理单个候选块：剥 markdown 围栏、去首尾空行（ADR-0002 兜底）；空块丢弃。
fn clean_block(lines: &[&str]) -> Option<String> {
    let mut lines = lines.to_vec();
    // ADR-0002：围栏成对时只留围栏之间，围栏外的解释一并剥掉
    if let Some((open, close)) = find_fence_pair(&lines) {
        lines = lines[open + 1..close].to_vec();
    }
    while lines
        .first()
        .is_some_and(|l| l.trim_start().starts_with("```"))
    {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().starts_with("```")) {
        lines.pop();
    }
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

/// 找第一处开围栏到最后一处闭围栏的下标；无成对围栏时返回 None。
fn find_fence_pair(lines: &[&str]) -> Option<(usize, usize)> {
    let open = lines
        .iter()
        .position(|l| l.trim_start().starts_with("```"))?;
    let close = lines.iter().rposition(|l| l.trim().starts_with("```"))?;
    if close <= open {
        return None;
    }
    Some((open, close))
}

/// 收束一个候选块：清理后若非空则入列，然后清空缓冲。
fn flush_block(candidates: &mut Vec<String>, current: &mut Vec<&str>) {
    if let Some(candidate) = clean_block(current) {
        candidates.push(candidate);
    }
    current.clear();
}

pub fn run(args: ParseArgs) -> i32 {
    let input = if args.text.is_empty() {
        read_stdin()
    } else {
        args.text.join("\n")
    };
    emit_candidates(&split_candidates(&input), "parse")
}
