use serde::Deserialize;
use std::path::PathBuf;

/// 配置骨架：字段随后续票按需增长，环境变量优先于文件。
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 调用 opencode 时使用的 agent。
    pub agent: String,
    /// 调用 opencode 时使用的模型（provider/model），空则用 opencode 默认。
    pub model: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            agent: "cmd-gen".to_string(),
            model: None,
        }
    }
}

impl Config {
    /// 加载配置：先取默认值，文件存在则用文件覆盖。
    pub fn load() -> Self {
        let mut config = Config::default();
        if let Some(from_file) = load_from_file() {
            config = from_file;
        }
        config
    }
}

fn load_from_file() -> Option<Config> {
    let path = config_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Config>(&text).ok()
}

/// 配置文件路径：ASK_OPENCODE_CONFIG 优先，否则取 ~/.config/ask-opencode/config.json。
fn config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ASK_OPENCODE_CONFIG") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/ask-opencode/config.json"))
}
