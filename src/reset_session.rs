/// `reset-session`（ADR-0007）：清空状态文件里的 `session_id`、不动常驻服务，幂等成功退出。
pub fn run() -> i32 {
    match crate::resident::clear_session_id() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("reset-session: {}", err.message);
            1
        }
    }
}
