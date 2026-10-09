use crate::opencode::{OPENCODE, OpenCodeError};
use crate::state;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
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

/// 刚拉起的 serve 的地址与进程号，落进 opencode 分区（字段语义见 ADR-0004）。
struct Started {
    url: String,
    pid: u32,
}

/// 确保常驻 serve 在跑并返回其 URL，供常驻 HTTP API 复用（ADR-0004）。
pub fn ensure_server_url(bin: &Path) -> Result<String, OpenCodeError> {
    let state = state::load_partition(OPENCODE);
    if let Some(url) = state.get("url").and_then(|url| url.as_str())
        && is_alive(url)
    {
        return Ok(url.to_string());
    }
    let log_path = state::prepare_state_path()?.with_file_name("serve.log");
    let started = start_server(bin, &log_path)?;
    // 只改 url/pid：拉起 serve 不得抹掉已落盘的 session_id（ADR-0007）。
    state::update_partition(OPENCODE, |partition| {
        partition.insert("url".to_string(), started.url.clone().into());
        partition.insert("pid".to_string(), started.pid.into());
    })?;
    Ok(started.url)
}

/// 拉起 `opencode serve`：stdout/stderr 落到 serve.log（每次 truncate，避免读到旧监听行），
/// 轮询日志等监听行、再等端口就绪，拿到 URL 即返回（进程保留为常驻孤儿进程）。
fn start_server(bin: &Path, log_path: &Path) -> Result<Started, OpenCodeError> {
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)
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
            let tail = read_log_tail(log_path);
            let _ = child.kill();
            return Err(OpenCodeError {
                message: format!("{} serve 启动失败：{tail}", bin.display()),
            });
        }
        if let Some(url) = found_url.clone().or_else(|| read_listening_url(log_path)) {
            if is_alive(&url) {
                return Ok(Started {
                    url,
                    pid: child.id(),
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
