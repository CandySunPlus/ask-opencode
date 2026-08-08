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

/// 从 `opencode run --format json` 事件流里解析出的内容：会话 id 与候选文本。
/// 事件流每行一个 JSON 事件，见 ADR-0007。
pub struct JsonEvents {
    /// 任意事件顶层都带的 `sessionID`，取首个。
    pub session_id: Option<String>,
    /// `text` 事件的 `part.text` 按序拼接成的候选文本，交给 ADR-0002 解析。
    pub text: String,
}

/// 解析 json 事件流：抓顶层 `sessionID`，把 `text` 事件（`part.text`）按序拼成文本。
/// 拼接按 default 格式的口径补换行：每条 text 事件后跟一个换行（`opencode run` 的
/// `part.text + EOL`），保证分隔符能独占一行。非 json 行与其它事件类型跳过。
pub fn parse_json_events(output: &str) -> JsonEvents {
    let mut session_id = None;
    let mut parts = Vec::new();
    for line in output.lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if session_id.is_none()
            && let Some(id) = event.get("sessionID").and_then(serde_json::Value::as_str)
        {
            session_id = Some(id.to_string());
        }
        if event.get("type").and_then(serde_json::Value::as_str) == Some("text")
            && let Some(text) = event
                .get("part")
                .and_then(|part| part.get("text"))
                .and_then(serde_json::Value::as_str)
        {
            parts.push(text.to_string());
        }
    }
    let mut text = String::new();
    for part in &parts {
        text.push_str(part);
        if !part.ends_with('\n') {
            text.push('\n');
        }
    }
    JsonEvents { session_id, text }
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
