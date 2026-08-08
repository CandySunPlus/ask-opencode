use crate::config::Config;
use crate::filter::SensitiveFilter;
use std::collections::HashSet;
use std::path::PathBuf;

/// 上下文快照：环境底盘 + 过滤后的命令历史 + 可选 dirstack/工具列表。
/// git 状态等其余信息由 agent 自行只读侦查，采集口径见 ADR-0005。
pub struct ContextSnapshot {
    pub cwd: String,
    pub os: String,
    pub shell: String,
    pub history: Vec<String>,
    pub dirstack: Vec<String>,
    pub tools: Vec<String>,
}

impl ContextSnapshot {
    /// 实时采集当前上下文：按配置注入历史条数、敏感过滤规则与 dirstack/工具列表开关。
    pub fn collect(config: &Config) -> Self {
        let filter = SensitiveFilter::new(&config.sensitive_rules);
        ContextSnapshot {
            cwd: std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            os: std::env::consts::OS.to_string(),
            shell: std::env::var("SHELL").unwrap_or_default(),
            history: collect_history(config.history_limit, &filter),
            dirstack: if config.include_dirstack {
                split_env_list("ASK_OPENCODE_DIRSTACK", ':')
            } else {
                Vec::new()
            },
            tools: if config.include_tools {
                split_env_list("ASK_OPENCODE_TOOLS", ',')
            } else {
                Vec::new()
            },
        }
    }

    /// 渲染成请求文本：环境底盘、历史小节，可选 dirstack/工具列表。
    pub fn render(&self) -> String {
        let mut sections = vec![format!(
            "环境：cwd={}，os={}，shell={}",
            self.cwd, self.os, self.shell
        )];
        if !self.history.is_empty() {
            let history = self
                .history
                .iter()
                .map(|entry| format!("- {entry}"))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("最近命令历史：\n{history}"));
        }
        if !self.dirstack.is_empty() {
            sections.push(format!("dirstack：\n{}", self.dirstack.join(", ")));
        }
        if !self.tools.is_empty() {
            sections.push(format!("工具列表：\n{}", self.tools.join(", ")));
        }
        sections.join("\n\n")
    }
}

/// 过滤（剔除 `#` 请求行与敏感行）、去重（保留最新）、截取最近 limit 条。
fn collect_history(limit: usize, filter: &SensitiveFilter) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for entry in read_history().into_iter().rev() {
        if result.len() >= limit {
            break;
        }
        if entry.trim_start().starts_with('#') {
            continue;
        }
        if entry.is_empty() {
            continue;
        }
        if filter.is_sensitive(&entry) {
            continue;
        }
        if !seen.insert(entry.clone()) {
            continue;
        }
        result.push(entry);
    }
    result.reverse();
    result
}

/// 读取 zsh 历史文件（HISTFILE 优先，缺省 ~/.zsh_history），解析为命令列表。
fn read_history() -> Vec<String> {
    let path = history_path();
    let text = std::fs::read_to_string(path).unwrap_or_default();
    parse_history(&text)
}

fn history_path() -> PathBuf {
    if let Some(path) = std::env::var_os("HISTFILE") {
        return PathBuf::from(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".zsh_history");
    }
    PathBuf::from(".zsh_history")
}

/// 解析 zsh 历史文件（ADR-0005）：`<ts>:<elapsed>;` 头起新条目、后续行续接，
/// 命令内的换行以反斜杠换行编码存储，解析时还原；无 extended 头时按裸命令逐行兜底。
fn parse_history(text: &str) -> Vec<String> {
    let entries = parse_extended_history(text);
    if !entries.is_empty() {
        return entries;
    }
    text.lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// 解析带 `<ts>:<elapsed>;` 头的 zsh 历史条目。
fn parse_extended_history(text: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(cmd) = parse_history_command(line) {
            if let Some(prev) = current.take() {
                entries.push(prev);
            }
            current = Some(cmd.to_string());
        } else if let Some(prev) = current.as_mut() {
            prev.push('\n');
            prev.push_str(line);
        }
    }
    if let Some(prev) = current.take() {
        entries.push(prev);
    }
    entries
        .into_iter()
        .map(|entry| entry.replace("\\\n", "\n"))
        .map(|entry| entry.trim().to_string())
        .collect()
}

/// 识别 zsh 历史条目头并返回命令部分；不成头返回 None。
fn parse_history_command(line: &str) -> Option<&str> {
    let body = line.strip_prefix(": ")?;
    let semi = body.find(';')?;
    let header = &body[..semi];
    let (timestamp, _elapsed) = header.rsplit_once(':')?;
    if timestamp.is_empty() || timestamp.parse::<u64>().is_err() {
        return None;
    }
    Some(&body[semi + 1..])
}

/// 读环境变量并按分隔符拆成条目列表；变量缺失或为空返回空列表。
fn split_env_list(name: &str, sep: char) -> Vec<String> {
    match std::env::var_os(name) {
        Some(value) => value
            .to_string_lossy()
            .split(sep)
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .collect(),
        None => Vec::new(),
    }
}
