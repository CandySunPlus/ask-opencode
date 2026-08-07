use std::path::PathBuf;
use std::process::Command;

/// 调用 opencode 失败时带给人看的错误。
#[derive(Debug)]
pub struct OpenCodeError {
    pub message: String,
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

/// 按 `opencode run --agent <agent> <request>` 调用外部二进制，返回其完整输出。
pub fn invoke(request: &str, agent: &str) -> Result<std::process::Output, OpenCodeError> {
    let bin = resolve_bin()?;
    Command::new(&bin)
        .arg("run")
        .arg("--agent")
        .arg(agent)
        .arg(request)
        .output()
        .map_err(|err| OpenCodeError {
            message: format!("无法启动 {}: {err}", bin.display()),
        })
}
