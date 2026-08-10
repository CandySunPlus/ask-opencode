use serde::Deserialize;
use std::path::PathBuf;

/// 配置：默认值起步，文件存在则用文件覆盖，再按字段套环境变量覆盖（文件 + env 合并）。
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 调用 opencode 时使用的 agent。
    pub agent: String,
    /// 调用 opencode 时使用的模型（provider/model），空则用 opencode 默认。
    pub model: Option<String>,
    /// 上下文快照注入的命令历史条数上限（默认 20）。
    pub history_limit: usize,
    /// 上下文快照是否注入 dirstack（默认关闭）。
    pub include_dirstack: bool,
    /// 上下文快照是否注入工具列表（默认关闭）。
    pub include_tools: bool,
    /// 敏感信息过滤的扩展规则（正则），叠加在内置黑名单之上。
    pub sensitive_rules: Vec<String>,
    /// 选择器实现：`skim`（内嵌）或 `fzf`（外部）。
    pub picker: String,
    /// 外部 fzf 可执行文件路径；仅在 `picker` 为 `fzf` 时使用。
    pub fzf_bin: String,
    /// 是否启用常驻 opencode serve（ADR-0004）：首次调用自动拉起、后续请求走 serve 的 HTTP API 复用。
    pub resident: bool,
    /// 是否复用同一个 opencode session（ADR-0007）：默认开，关闭时每次请求开全新会话。
    pub reuse_session: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            agent: "cmd-gen".to_string(),
            model: None,
            history_limit: 20,
            include_dirstack: false,
            include_tools: false,
            sensitive_rules: Vec::new(),
            picker: "skim".to_string(),
            fzf_bin: "fzf".to_string(),
            resident: true,
            reuse_session: true,
        }
    }
}

impl Config {
    /// 加载配置：默认值起步，文件存在则用文件覆盖，再按字段套环境变量覆盖。
    pub fn load() -> Self {
        let mut config = Config::default();
        if let Some(from_file) = load_from_file() {
            config = from_file;
        }
        apply_env_overrides(&mut config);
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

/// 常驻 serve 状态文件路径：与配置文件同目录、文件名 server.json（ADR-0004）。
pub fn state_path() -> Option<PathBuf> {
    Some(config_path()?.with_file_name("server.json"))
}

/// 环境变量按字段覆盖配置；解析失败时保留文件里的值。
fn apply_env_overrides(config: &mut Config) {
    if let Some(value) = std::env::var_os("ASK_OPENCODE_HISTORY_LIMIT")
        && let Ok(limit) = value.to_string_lossy().parse::<usize>()
    {
        config.history_limit = limit;
    }
    if let Some(value) = std::env::var_os("ASK_OPENCODE_INCLUDE_DIRSTACK")
        && let Some(on) = parse_bool(&value)
    {
        config.include_dirstack = on;
    }
    if let Some(value) = std::env::var_os("ASK_OPENCODE_INCLUDE_TOOLS")
        && let Some(on) = parse_bool(&value)
    {
        config.include_tools = on;
    }
    if let Some(value) = std::env::var_os("ASK_OPENCODE_SENSITIVE_RULES") {
        // 列表字段按「文件 + env 合并」语义：env 规则追加到文件规则之上，而非替换（见 ADR-0005）。
        config
            .sensitive_rules
            .extend(split_list(&value.to_string_lossy(), ','));
    }
    if let Some(value) = std::env::var_os("ASK_OPENCODE_PICKER")
        && let Some(picker) = parse_picker(&value)
    {
        config.picker = picker;
    }
    if let Some(value) = std::env::var_os("ASK_OPENCODE_FZF_BIN")
        && !value.is_empty()
    {
        config.fzf_bin = value.to_string_lossy().into_owned();
    }
    if let Some(value) = std::env::var_os("ASK_OPENCODE_RESIDENT")
        && let Some(on) = parse_bool(&value)
    {
        config.resident = on;
    }
    if let Some(value) = std::env::var_os("ASK_OPENCODE_REUSE_SESSION")
        && let Some(on) = parse_bool(&value)
    {
        config.reuse_session = on;
    }
}

/// 解析选择器环境变量：只接受已知实现，非法值忽略（沿用配置/默认）。
fn parse_picker(value: &std::ffi::OsStr) -> Option<String> {
    let picker = value.to_string_lossy().trim().to_string();
    matches!(picker.as_str(), "skim" | "fzf").then_some(picker)
}

/// 解析布尔环境变量；解析失败返回 None，沿用文件里的值。
fn parse_bool(value: &std::ffi::OsStr) -> Option<bool> {
    value.to_string_lossy().trim().parse::<bool>().ok()
}

/// 按分隔符拆字符串为去空白、去空项的条目列表。
fn split_list(value: &str, sep: char) -> Vec<String> {
    value
        .split(sep)
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}
