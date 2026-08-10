use crate::config::Config;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// 调用 opencode 失败时带给人看的错误。
#[derive(Debug)]
pub struct OpenCodeError {
    pub message: String,
}

/// `opencode run` 的输出格式：default 是常规候选文本，json 是事件流（首次建会话用，见 ADR-0007）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Default,
    Json,
}

/// 一次请求的完整结果，统一 CLI 冷启动与常驻 HTTP 两种路径（ADR-0004 修订）。
/// CLI 路径从子进程输出映射，HTTP 路径把 serve 返回重组进同形结构，调用方不再区分来源。
#[derive(Debug)]
pub struct InvokeOutput {
    /// 是否成功（CLI：退出码 0；HTTP：2xx）。
    pub success: bool,
    /// 失败时的退出码：CLI 透传 opencode 退出码，HTTP 透传状态码。
    pub exit_code: i32,
    /// 候选文本：CLI 是 stdout，HTTP 是 serve 返回的助手 text part 拼接。
    pub stdout: String,
    /// 失败时的错误文本：CLI 是 stderr，HTTP 是响应体。
    pub stderr: String,
    /// 常驻 HTTP 首次路径新建的会话 id（需落盘）；CLI 路径恒为 None，会话 id 走 json 事件流。
    pub new_session_id: Option<String>,
}

/// 解析 opencode 可执行文件路径：ASK_OPENCODE_BIN 显式指定则优先并校验存在，否则回落 PATH。
pub fn resolve_bin() -> Result<PathBuf, OpenCodeError> {
    if let Some(path) = std::env::var_os("ASK_OPENCODE_BIN") {
        let path = PathBuf::from(path);
        if !path.exists() {
            return Err(OpenCodeError {
                message: format!("ASK_OPENCODE_BIN 指向的文件不存在: {}", path.display()),
            });
        }
        return Ok(path);
    }
    Ok(PathBuf::from("opencode"))
}

/// 按需挑路径发起一次请求：常驻开关打开且 serve 可用时走 HTTP API，否则回退 `opencode run`。
/// 常驻 serve 拉起失败退化为冷启动，保留可诊断的错误提示（ADR-0004）。
pub fn invoke(
    request: &str,
    agent: &str,
    model: Option<&str>,
    config: &Config,
    format: OutputFormat,
    session_id: Option<&str>,
) -> Result<InvokeOutput, OpenCodeError> {
    let bin = resolve_bin()?;
    if config.resident {
        match crate::resident::ensure_server_url(&bin) {
            Ok(url) => return invoke_http(request, agent, model, session_id, &url),
            Err(err) => {
                eprintln!("resident: {}", err.message);
            }
        }
    }
    invoke_cli(request, agent, model, format, session_id, &bin)
}

/// 冷启动路径：按 `opencode run [--format json|default] [--session <id>] --agent <agent>
/// [-m <model>] <request>` 调用外部二进制，返回其完整输出。Json 只走「会话尚未建立」的
/// 首次路径（ADR-0007）；Default 带落盘 id 时显式 `--format default --session <id>`，
/// 无 id 时回退每次新会话旧行为。
fn invoke_cli(
    request: &str,
    agent: &str,
    model: Option<&str>,
    format: OutputFormat,
    session_id: Option<&str>,
    bin: &PathBuf,
) -> Result<InvokeOutput, OpenCodeError> {
    let mut cmd = Command::new(bin);
    cmd.arg("run");
    match format {
        OutputFormat::Json => {
            cmd.arg("--format").arg("json");
        }
        OutputFormat::Default => {
            if let Some(id) = session_id {
                cmd.arg("--format").arg("default");
                cmd.arg("--session").arg(id);
            }
        }
    }
    cmd.arg("--agent").arg(agent);
    if let Some(model) = model {
        cmd.arg("-m").arg(model);
    }
    cmd.arg(request);
    let output = cmd.output().map_err(|err| OpenCodeError {
        message: format!("无法启动 {}: {err}", bin.display()),
    })?;
    Ok(InvokeOutput {
        success: output.status.success(),
        exit_code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        new_session_id: None,
    })
}

/// 常驻路径：走 serve 的 HTTP API 提交请求并阻塞等完成。`opencode run --attach` 实测不等
/// 模型跑完就返回（stdout 空/截断），改用 `POST /session/{id}/message`（官方文档「Send a
/// message and wait for response」）拿全文。无会话 id 时先 `POST /session` 建会话（首次路径）。
fn invoke_http(
    request: &str,
    agent: &str,
    model: Option<&str>,
    session_id: Option<&str>,
    url: &str,
) -> Result<InvokeOutput, OpenCodeError> {
    let client = http_client();
    let directory = std::env::current_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let sid = match session_id {
        Some(id) => id.to_string(),
        None => create_session(&client, url, &directory, agent, model)?,
    };
    let mut body = serde_json::json!({
        "parts": [{"type": "text", "text": request}],
        "agent": agent,
    });
    if let Some(model) = model_json(model) {
        body["model"] = model;
    }
    match client
        .post(&format!("{url}/session/{sid}/message"))
        .query("directory", &directory)
        .send_json(body)
    {
        Ok(resp) => {
            let text = extract_text_parts(resp)?;
            Ok(InvokeOutput {
                success: true,
                exit_code: 0,
                stdout: text,
                stderr: String::new(),
                // 首次路径新建的会话：本次没有复用已有 id 才算新建。
                new_session_id: session_id.is_none().then_some(sid),
            })
        }
        Err(ureq::Error::Status(code, resp)) => {
            let stderr = resp.into_string().unwrap_or_default();
            Ok(InvokeOutput {
                success: false,
                exit_code: code as i32,
                stdout: String::new(),
                stderr,
                new_session_id: None,
            })
        }
        Err(err) => Err(OpenCodeError {
            message: format!("常驻 API 调用失败: {err}"),
        }),
    }
}

/// 会话不存在时 `POST /session/{id}/message` 返回 404；无会话 id 的首次路径先建会话。
fn create_session(
    client: &ureq::Agent,
    url: &str,
    directory: &str,
    agent: &str,
    model: Option<&str>,
) -> Result<String, OpenCodeError> {
    let mut body = serde_json::json!({ "agent": agent });
    if let Some(model) = session_model_json(model) {
        body["model"] = model;
    }
    match client
        .post(&format!("{url}/session"))
        .query("directory", directory)
        .send_json(body)
    {
        Ok(resp) => {
            let value: serde_json::Value = resp.into_json().map_err(|err| OpenCodeError {
                message: format!("创建会话：响应解析失败: {err}"),
            })?;
            value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(|id| id.to_string())
                .ok_or_else(|| OpenCodeError {
                    message: "创建会话：响应里没有会话 id".to_string(),
                })
        }
        Err(ureq::Error::Status(code, resp)) => Err(OpenCodeError {
            message: format!(
                "创建会话失败（HTTP {code}）: {}",
                resp.into_string().unwrap_or_default()
            ),
        }),
        Err(err) => Err(OpenCodeError {
            message: format!("创建会话失败: {err}"),
        }),
    }
}

/// 从消息响应里按序拼接助手 `text` part（ADR-0002 分隔行契约依赖独占行的分隔符）。
fn extract_text_parts(resp: ureq::Response) -> Result<String, OpenCodeError> {
    let value: serde_json::Value = resp.into_json().map_err(|err| OpenCodeError {
        message: format!("常驻 API：响应解析失败: {err}"),
    })?;
    let mut text = String::new();
    if let Some(parts) = value.get("parts").and_then(serde_json::Value::as_array) {
        for part in parts {
            if part.get("type").and_then(serde_json::Value::as_str) == Some("text")
                && let Some(part_text) = part.get("text").and_then(serde_json::Value::as_str)
            {
                text.push_str(part_text);
                if !part_text.ends_with('\n') {
                    text.push('\n');
                }
            }
        }
    }
    Ok(text)
}

/// 消息体里的模型结构：`provider/model` 拆成 `{providerID, modelID}`；拆不开返回 None（不传）。
fn model_json(model: Option<&str>) -> Option<serde_json::Value> {
    let (provider_id, model_id) = split_model(model)?;
    Some(serde_json::json!({ "providerID": provider_id, "modelID": model_id }))
}

/// 建会话体里的模型结构：`provider/model` 拆成 `{id, providerID}`；拆不开返回 None（不传）。
fn session_model_json(model: Option<&str>) -> Option<serde_json::Value> {
    let (provider_id, model_id) = split_model(model)?;
    Some(serde_json::json!({ "id": model_id, "providerID": provider_id }))
}

/// 把 `provider/model` 拆成 (provider, model)；缺少任一返回 None。
fn split_model(model: Option<&str>) -> Option<(&str, &str)> {
    let (provider, model_id) = model?.split_once('/')?;
    if provider.is_empty() || model_id.is_empty() {
        return None;
    }
    Some((provider, model_id))
}

/// 常驻 HTTP 客户端：超时拉长到 30 分钟，因为 `POST /message` 要阻塞到模型跑完（含多步工具调用）。
fn http_client() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30 * 60))
        .build()
}
