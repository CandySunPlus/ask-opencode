use crate::backend::{Backend, BackendError, Reply, Session};
use std::path::PathBuf;
use std::process::Command;

/// cmd-gen 正文：build.rs 编译期从 opencode agent 文件剥掉 frontmatter 取得（ADR-0009）。
const SYSTEM_PROMPT: &str = include_str!(concat!(env!("OUT_DIR"), "/cmd_gen_prompt.md"));

/// cmd-gen agent frontmatter 里的 bash 规则，build.rs 抽成一行一条 `模式\t动作`（ADR-0009）。
const BASH_RULES: &str = include_str!(concat!(env!("OUT_DIR"), "/cmd_gen_bash_rules.tsv"));

/// 只读叠加配置的文件名，与状态文件同目录。
const READONLY_OVERLAY_FILE: &str = "omp-readonly.json";

/// omp 各发现来源的用户级上下文文件名；叠加配置按 `context-file:user:<文件名>` 逐个屏蔽，
/// 项目级的不动（ADR-0009）。
const USER_CONTEXT_FILES: [&str; 4] = [
    "AGENTS.md",
    "CLAUDE.md",
    "GEMINI.md",
    "copilot-instructions.md",
];

/// 后端的 omp 实现（ADR-0009）：每次冷启动一条 `omp --mode json`，暂一律 `--no-session`，
/// 会话用法与 `resident` 都不生效。
pub struct Omp {
    pub model: Option<String>,
}

impl Backend for Omp {
    fn name(&self) -> &'static str {
        "omp"
    }

    fn generate(&self, request: &str, _session: Session<'_>) -> Result<Reply, BackendError> {
        let bin = crate::backend::resolve_bin("ASK_OPENCODE_OMP_BIN", "omp")
            .map_err(|message| BackendError::Unavailable { message })?;
        let mut cmd = Command::new(&bin);
        cmd.args(["--mode", "json", "--no-session"])
            .arg("--system-prompt")
            .arg(SYSTEM_PROMPT)
            // ADR-0009 只读侦查：收窄注入（项目 AGENTS.md 保留）；非 read 档工具（MCP、写文件、
            // eval）在 always-ask 下无 UI 可批、直接失败；内置工具只留 bash，白名单见叠加配置。
            .args(["--no-skills", "--no-rules", "--no-extensions"])
            .args(["--approval-mode", "always-ask", "--tools", "bash"])
            .arg("--config")
            .arg(write_readonly_overlay()?);
        if let Some(model) = self.model.as_deref().filter(|model| !model.is_empty()) {
            cmd.arg("--model").arg(model);
        }
        cmd.arg(request);
        let output = cmd.output().map_err(|err| BackendError::Unavailable {
            message: format!("无法启动 {}: {err}", bin.display()),
        })?;
        if !output.status.success() {
            return Err(BackendError::Failed {
                exit_code: output.status.code().unwrap_or(1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let text = final_assistant_text(&String::from_utf8_lossy(&output.stdout))?;
        Ok(Reply {
            text,
            session_id: None,
        })
    }
}

/// 只读叠加配置（ADR-0009）：bash 白名单逐条照搬 cmd-gen agent，末尾 `*` 拒绝兜底——omp 首条
/// 命中生效、未命中默认放行，少了兜底就等于没有白名单。
fn readonly_overlay() -> serde_json::Value {
    let mut patterns: Vec<serde_json::Value> = BASH_RULES
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(pattern, action)| {
            // opencode 的 ask 在 omp 里叫 prompt。
            let approval = if action == "ask" { "prompt" } else { action };
            serde_json::json!({"match": pattern, "approval": approval})
        })
        .collect();
    patterns.push(serde_json::json!({"match": "*", "approval": "deny"}));
    serde_json::json!({
        "bash": {"allowCompoundCommands": false, "patterns": patterns},
        "disabledExtensions": USER_CONTEXT_FILES
            .map(|name| format!("context-file:user:{name}")),
    })
}

/// 把只读叠加配置写到状态文件同目录并返回路径；先写临时文件再改名，多个 shell 并发也不会读到半截。
fn write_readonly_overlay() -> Result<PathBuf, BackendError> {
    let unavailable = |message: String| BackendError::Unavailable { message };
    let path = crate::config::state_path()
        .ok_or_else(|| unavailable("无法确定 omp 只读叠加配置路径".to_string()))?
        .with_file_name(READONLY_OVERLAY_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| unavailable(format!("无法创建目录 {}: {err}", parent.display())))?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, readonly_overlay().to_string())
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|err| unavailable(format!("无法写 omp 只读叠加配置 {}: {err}", path.display())))?;
    Ok(path)
}

/// 从 NDJSON 的 `agent_end` 取最后一条 assistant 消息，把它的 text 内容按序拼成候选原文；
/// 每段补到换行结尾，保证分隔符独占一行（ADR-0002）。该消息以 `error` 收尾时报错。
fn final_assistant_text(stdout: &str) -> Result<String, BackendError> {
    let messages = stdout
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("agent_end"))
        .and_then(|event| event.get("messages").cloned())
        .ok_or_else(|| BackendError::Unavailable {
            message: "omp 输出里没有 agent_end 事件".to_string(),
        })?;
    let assistant = messages
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .find(|message| {
            message.get("role").and_then(serde_json::Value::as_str) == Some("assistant")
        })
        .ok_or_else(|| BackendError::Unavailable {
            message: "omp 没有返回 assistant 消息".to_string(),
        })?;
    if assistant
        .get("stopReason")
        .and_then(serde_json::Value::as_str)
        == Some("error")
    {
        let detail = assistant
            .get("errorMessage")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("未知错误");
        return Err(BackendError::Unavailable {
            message: format!("omp 调用失败: {detail}"),
        });
    }
    let mut text = String::new();
    for part in assistant
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        if part.get("type").and_then(serde_json::Value::as_str) == Some("text")
            && let Some(part_text) = part.get("text").and_then(serde_json::Value::as_str)
        {
            text.push_str(part_text);
            if !part_text.ends_with('\n') {
                text.push('\n');
            }
        }
    }
    Ok(text)
}
