use regex::Regex;

/// 超长行阈值：命令历史里超过该长度的行按噪音剔除，属内置过滤规则之一。
pub const MAX_LINE_LEN: usize = 500;

/// 内置敏感黑名单（ADR-0005）；可扩展规则由配置 `sensitive_rules` 提供，叠加在这套内置规则之上。
const BUILT_IN_RULES: &[&str] = &[
    // token/password/secret/api_key 等敏感键的键值对
    r#"(?i)(token|password|passwd|secret|api[_-]?key|access[_-]?key|private[_-]?key)['\"]?\s*[:=]\s*['\"]?[^\s'\"]+"#,
    // 任意协议 URL 内嵌凭据（scheme://user:pass@host）
    r"\b[a-z][a-z0-9+.-]*://[^\s/@:]+:[^\s/@]+@",
];

pub struct SensitiveFilter {
    rules: Vec<Regex>,
}

impl SensitiveFilter {
    /// 以内置规则为底，叠加配置里的扩展正则构建过滤规则集；非法正则忽略。
    pub fn new(extra_rules: &[String]) -> Self {
        let mut rules = Vec::new();
        for pattern in BUILT_IN_RULES
            .iter()
            .copied()
            .chain(extra_rules.iter().map(String::as_str))
        {
            if let Ok(re) = Regex::new(pattern) {
                rules.push(re);
            }
        }
        SensitiveFilter { rules }
    }

    /// 该行是否应被剔除：超长，或命中任一敏感规则。
    pub fn is_sensitive(&self, line: &str) -> bool {
        line.len() > MAX_LINE_LEN || self.rules.iter().any(|re| re.is_match(line))
    }
}
