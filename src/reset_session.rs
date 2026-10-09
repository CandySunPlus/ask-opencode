use crate::config::Config;

/// `reset-session`（ADR-0007）：只清当前后端分区里的会话 id（omp 下只清当前目录的），常驻
/// 服务与其他后端分区不动（ADR-0009），幂等成功退出。
pub fn run() -> i32 {
    let backend = match crate::backend::select(&Config::load(), None, None) {
        Ok(backend) => backend,
        Err(message) => {
            eprintln!("reset-session: {message}");
            return 1;
        }
    };
    match crate::state::clear_session_id(backend.name(), &backend.session_key()) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("reset-session: {}", err.message);
            1
        }
    }
}
