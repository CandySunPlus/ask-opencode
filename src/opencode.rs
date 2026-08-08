use crate::config::Config;
use std::path::PathBuf;
use std::process::Command;

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

/// 按 `opencode run [--format json] [--attach <url>] --agent <agent> [-m <model>] <request>`
/// 调用外部二进制，返回其完整输出。常驻开关打开时先确保 serve 在跑，用 `--attach` 复用（ADR-0004）；
/// `format` 为 Json 时加 `--format json`（首次请求建会话，见 ADR-0007）。
pub fn invoke(
    request: &str,
    agent: &str,
    model: Option<&str>,
    config: &Config,
    format: OutputFormat,
) -> Result<std::process::Output, OpenCodeError> {
    let bin = resolve_bin()?;
    let mut cmd = Command::new(&bin);
    cmd.arg("run");
    if format == OutputFormat::Json {
        cmd.arg("--format").arg("json");
    }
    if config.resident {
        match crate::resident::ensure_server_url(&bin) {
            Ok(url) => {
                cmd.arg("--attach").arg(&url);
            }
            Err(err) => {
                // serve 拉起失败退化为冷启动，保留可诊断的错误提示（ADR-0004）。
                eprintln!("resident: {}", err.message);
            }
        }
    }
    cmd.arg("--agent").arg(agent);
    if let Some(model) = model {
        cmd.arg("-m").arg(model);
    }
    cmd.arg(request);
    cmd.output().map_err(|err| OpenCodeError {
        message: format!("无法启动 {}: {err}", bin.display()),
    })
}
