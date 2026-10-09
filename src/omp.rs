use crate::backend::{Backend, BackendError, Reply, Session};
use std::path::PathBuf;
use std::process::Command;

/// cmd-gen 正文：build.rs 编译期从 opencode agent 文件剥掉 frontmatter 取得（ADR-0009）。
const SYSTEM_PROMPT: &str = include_str!(concat!(env!("OUT_DIR"), "/cmd_gen_prompt.md"));

/// 后端的 omp 实现（ADR-0009）：每次冷启动一条 `omp --mode json`，暂一律 `--no-session`，
/// 会话用法与 `resident` 都不生效。
pub struct Omp {
    pub model: Option<String>,
}

impl Backend for Omp {
    fn generate(&self, request: &str, _session: Session<'_>) -> Result<Reply, BackendError> {
        let bin = resolve_bin()?;
        let mut cmd = Command::new(&bin);
        cmd.args(["--mode", "json", "--no-session"])
            .arg("--system-prompt")
            .arg(SYSTEM_PROMPT)
            // 收窄注入，只读侦查与 opencode 同样严格（ADR-0009）；项目 AGENTS.md 保留。
            .args(["--no-skills", "--no-rules", "--no-extensions"]);
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

/// omp 可执行文件：ASK_OPENCODE_OMP_BIN 显式指定则优先并校验存在，否则取 PATH 中的 `omp`。
fn resolve_bin() -> Result<PathBuf, BackendError> {
    if let Some(path) = std::env::var_os("ASK_OPENCODE_OMP_BIN") {
        let path = PathBuf::from(path);
        if !path.exists() {
            return Err(BackendError::Unavailable {
                message: format!("ASK_OPENCODE_OMP_BIN 指向的文件不存在: {}", path.display()),
            });
        }
        return Ok(path);
    }
    Ok(PathBuf::from("omp"))
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
