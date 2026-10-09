use crate::config::Config;

/// `reset-session`（ADR-0007）：只清当前后端分区里的会话 id，常驻服务与其他后端分区不动
/// （ADR-0009），幂等成功退出。
pub fn run() -> i32 {
    let backend = crate::backend::select(&Config::load(), None, None);
    match crate::state::clear_session_id(backend.name()) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("reset-session: {}", err.message);
            1
        }
    }
}
