use crate::config::Config;
use std::path::PathBuf;

/// 一次生成调用的会话用法，为什么是三态而非可选 id 见 ADR-0009。复不复用常驻会话是
/// generate 的共享策略，后端只照做（ADR-0007）。
#[derive(Debug, Clone, Copy)]
pub enum Session<'a> {
    /// 不复用会话：每次新会话，不需要知道它的 id。
    Oneshot,
    /// 新建会话并带回 id 供落盘（常驻会话首次路径）。
    New,
    /// 续接已落盘的会话。
    Resume(&'a str),
}

/// 后端的一次成功回复。
#[derive(Debug)]
pub struct Reply {
    /// 候选原文，交给 ADR-0002 分隔行契约解析。
    pub text: String,
    /// `Session::New` 新建的会话 id；其余用法恒为 None，抓不到 id 也为 None。
    pub session_id: Option<String>,
}

#[derive(Debug)]
pub enum BackendError {
    /// 续接的会话已失效，generate 清掉 id 后新建会话重试一次（ADR-0007）。
    SessionExpired,
    /// 后端跑了但失败：原样回显它的错误输出、透传退出码。
    Failed { exit_code: i32, stderr: String },
    /// 后端没能跑起来或调用中断，带一句给人看的错误。
    Unavailable { message: String },
}

/// 后端（见 `CONTEXT.md`「后端」、ADR-0009）：冷启动还是常驻、会话 id 从哪抓、
/// 何为会话失效，都由实现自己决定。
pub trait Backend {
    /// 后端名，也是它在状态文件里的分区名（ADR-0009）。
    fn name(&self) -> &'static str;

    /// 常驻会话 id 在本后端分区里的键路径（ADR-0009）：默认全局一个 `session_id`。
    fn session_key(&self) -> Vec<String> {
        vec!["session_id".to_string()]
    }

    fn generate(&self, request: &str, session: Session<'_>) -> Result<Reply, BackendError>;
}

/// 按配置选定后端；命令行给的 agent/model 优先于配置。未知后端直接报错、不落到默认后端
/// （ADR-0009）。
pub fn select(
    config: &Config,
    agent: Option<&str>,
    model: Option<&str>,
) -> Result<Box<dyn Backend>, String> {
    let model = model.or(config.model.as_deref()).map(str::to_string);
    match config.backend.as_str() {
        "opencode" => Ok(Box::new(crate::opencode::OpenCode {
            agent: agent.unwrap_or(&config.agent).to_string(),
            model,
            resident: config.resident,
        })),
        "omp" => Ok(Box::new(crate::omp::Omp { model })),
        other => Err(format!("未知后端 {other:?}，可选 opencode / omp")),
    }
}

/// 解析后端可执行文件：`env_var` 显式指定则优先并校验存在，否则取 PATH 中的 `default`。
pub fn resolve_bin(env_var: &str, default: &str) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os(env_var) {
        let path = PathBuf::from(path);
        if !path.exists() {
            return Err(format!("{env_var} 指向的文件不存在: {}", path.display()));
        }
        return Ok(path);
    }
    Ok(PathBuf::from(default))
}
