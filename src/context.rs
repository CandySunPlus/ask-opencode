/// 环境底盘上下文快照：generate 拼进请求的最小子集（cwd、OS、shell），完整快照见后续票。
pub struct ContextSnapshot {
    pub cwd: String,
    pub os: String,
    pub shell: String,
}

impl ContextSnapshot {
    /// 实时采集当前环境底盘。
    pub fn collect() -> Self {
        ContextSnapshot {
            cwd: std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            os: std::env::consts::OS.to_string(),
            shell: std::env::var("SHELL").unwrap_or_default(),
        }
    }

    /// 渲染成请求文本里的环境底盘小节。
    pub fn render(&self) -> String {
        format!(
            "环境：cwd={}，os={}，shell={}",
            self.cwd, self.os, self.shell
        )
    }
}
