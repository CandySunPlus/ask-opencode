/// `reset-session` 子命令（ADR-0007）：清空状态文件里的 `session_id`、不动常驻服务，
/// 幂等（无 id 时也成功退出）。重置后下一次 generate 走首次路径建全新会话。
pub fn run() -> i32 {
    match crate::resident::clear_session_id() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("reset-session: {}", err.message);
            1
        }
    }
}
