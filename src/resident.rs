use crate::config;
use crate::opencode::OpenCodeError;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// serve 启动日志里监听行的固定前缀（实测 `opencode serve` 的输出，见 ADR-0004）。
const LISTEN_LINE: &str = "opencode server listening on ";
/// serve 启动就绪的最大等待时间。
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// 健康检查的 TCP 连接超时。
const HEALTH_TIMEOUT: Duration = Duration::from_millis(300);
/// 轮询启动日志的间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// 常驻服务状态：URL/PID 与 session_id 按需缺省（字段语义见 ADR-0004/0007）。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ServerState {
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

/// 解析状态文件路径并确保父目录存在，供 serve 与会话状态共用。
fn prepare_state_path() -> Result<PathBuf, OpenCodeError> {
    let state_path = config::state_path().ok_or_else(|| OpenCodeError {
        message: "无法确定常驻服务状态文件路径".to_string(),
    })?;
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| OpenCodeError {
            message: format!("无法创建配置目录 {}: {err}", parent.display()),
        })?;
    }
    Ok(state_path)
}

/// 确保常驻 serve 在跑并返回其 URL，供 `run --attach` 复用（ADR-0004）。
pub fn ensure_server_url(bin: &Path) -> Result<String, OpenCodeError> {
    let state_path = prepare_state_path()?;
    if let Some(state) = load_state(&state_path)
        && let Some(url) = state.url.as_deref()
        && is_alive(url)
    {
        return Ok(url.to_string());
    }
    let started = start_server(bin, &state_path)?;
    // 合并进既有状态：拉起 serve 不得抹掉已落盘的 session_id（ADR-0007）。
    let mut state = load_state(&state_path).unwrap_or_default();
    state.url = started.url;
    state.pid = started.pid;
    save_state(&state_path, &state)?;
    Ok(state.url.unwrap())
}

/// 拉起 `opencode serve`：stdout/stderr 落到 serve.log（每次 truncate，避免读到旧监听行），
/// 轮询日志等监听行、再等端口就绪，拿到 URL 即返回（进程保留为常驻孤儿进程）。
fn start_server(bin: &Path, state_path: &Path) -> Result<ServerState, OpenCodeError> {
    let log_path = state_path.with_file_name("serve.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .map_err(|err| OpenCodeError {
            message: format!("无法打开 serve 日志 {}: {err}", log_path.display()),
        })?;
    let mut child = Command::new(bin)
        .arg("serve")
        .stdout(Stdio::from(log_file.try_clone().map_err(|err| {
            OpenCodeError {
                message: format!("无法复制 serve 日志句柄: {err}"),
            }
        })?))
        .stderr(Stdio::from(log_file))
        .spawn()
        .map_err(|err| OpenCodeError {
            message: format!("无法启动 {} serve: {err}", bin.display()),
        })?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut found_url = None;
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            let tail = read_log_tail(&log_path);
            let _ = child.kill();
            return Err(OpenCodeError {
                message: format!("{} serve 启动失败：{tail}", bin.display()),
            });
        }
        if let Some(url) = found_url.clone().or_else(|| read_listening_url(&log_path)) {
            if is_alive(&url) {
                return Ok(ServerState {
                    url: Some(url),
                    pid: Some(child.id()),
                    session_id: None,
                });
            }
            found_url = Some(url);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(OpenCodeError {
                message: format!(
                    "{} serve 启动超时（{}s），日志见 {}",
                    bin.display(),
                    STARTUP_TIMEOUT.as_secs(),
                    log_path.display()
                ),
            });
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// 端口存活检查：能 TCP 连上监听地址即认为 serve 在跑。
fn is_alive(url: &str) -> bool {
    let Some(addr) = parse_addr(url) else {
        return false;
    };
    TcpStream::connect_timeout(&addr, HEALTH_TIMEOUT).is_ok()
}

/// 从 `http://host:port` 解析 SocketAddr；解析失败返回 None。
fn parse_addr(url: &str) -> Option<SocketAddr> {
    url.strip_prefix("http://")?.parse().ok()
}

/// 读 serve 日志，找监听行并返回其后的 URL；还没出现返回 None。
fn read_listening_url(log_path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(log_path).ok()?;
    text.lines().find_map(|line| {
        line.find(LISTEN_LINE)
            .map(|idx| line[idx + LISTEN_LINE.len()..].trim().to_string())
    })
}

/// 读日志尾部（最多 20 行），用于 serve 启动失败时的诊断。
fn read_log_tail(log_path: &Path) -> String {
    let text = std::fs::read_to_string(log_path).unwrap_or_default();
    text.lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

fn load_state(path: &Path) -> Option<ServerState> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_state(path: &Path, state: &ServerState) -> Result<(), OpenCodeError> {
    let text = serde_json::to_string(state).map_err(|err| OpenCodeError {
        message: format!("无法序列化常驻 serve 状态: {err}"),
    })?;
    std::fs::write(path, text).map_err(|err| OpenCodeError {
        message: format!("无法写入常驻 serve 状态 {}: {err}", path.display()),
    })
}

/// 读状态文件里的常驻会话 id；文件缺失或没有落盘返回 None（ADR-0007）。
pub fn load_session_id() -> Option<String> {
    let state_path = config::state_path()?;
    let state = load_state(&state_path)?;
    state.session_id
}

/// 把首次请求抓到的会话 id 落盘；保留 serve 的 `{url, pid}`（读改写，ADR-0007）。
pub fn save_session_id(session_id: &str) -> Result<(), OpenCodeError> {
    let state_path = prepare_state_path()?;
    let mut state = load_state(&state_path).unwrap_or_default();
    state.session_id = Some(session_id.to_string());
    save_state(&state_path, &state)
}
