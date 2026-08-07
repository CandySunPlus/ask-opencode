use crate::cli::ValidateArgs;
use crate::io::read_stdin;
use serde::Serialize;
use std::io::Write;
use std::process::{Command, Stdio};

/// 三项静态检查的标识；序列化为机器可读的 snake_case（日志字段名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckName {
    Syntax,
    CommandExists,
    GitContext,
}

impl CheckName {
    /// 面向人的中文说明，修正请求里拼给模型看。
    pub fn label(self) -> &'static str {
        match self {
            CheckName::Syntax => "shell 语法错误",
            CheckName::CommandExists => "命令不存在",
            CheckName::GitContext => "git 命令但当前不在 git 仓库",
        }
    }
}

/// 单项静态检查的结果。
#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub name: CheckName,
    pub passed: bool,
}

/// 一条候选命令的校验结果。passed 只在三项静态检查全过时为 true；dangerous 独立判定，
/// T5 在回填前读它弹二次确认（ADR-0003），不计入 passed。
#[derive(Debug, Serialize)]
pub struct ValidationResult {
    pub candidate: String,
    pub passed: bool,
    pub checks: Vec<CheckResult>,
    pub dangerous: bool,
}

impl ValidationResult {
    /// 未通过检查项的中文说明，供修正请求拼给人看。
    pub fn failed_labels(&self) -> Vec<&'static str> {
        self.checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| check.name.label())
            .collect()
    }
}

/// 前导 shell 关键字：这类候选的「首词」不是可执行命令，跳过存在性检查。
const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "elif", "else", "fi", "while", "until", "do", "done", "for", "case", "esac",
    "function", "select", "time", "repeat", "coproc",
];

/// git 子命令白名单：这些命令不依赖仓库上下文，在任意目录都能跑。
const REPO_INDEPENDENT_GIT: &[&str] = &[
    "clone",
    "init",
    "help",
    "config",
    "credential",
    "daemon",
    "version",
    "instaweb",
];

/// 危险命令清单（ADR-0003）：命中只标记，与三项静态检查互相独立。
const DANGEROUS_PATTERNS: &[&str] = &[
    // rm 带 -r 与 -f（rm -rf、rm -fr、rm -r -f 等）
    r"\brm\b[^\n]*(?:-[^\n]*[rf][^\n]*[rf])",
    // sudo 提权执行
    r"\bsudo\b",
    // dd 裸写磁盘
    r"\bdd\b",
    // 管道直接交给 shell（curl | sh、wget | bash 等）
    r"\|[^\n]*\b(?:ba|k)?sh\b",
    // 格式化分区
    r"\bmkfs(?:\.[a-z0-9]+)?\b",
    // 关机重启类
    r"\b(?:shutdown|reboot|poweroff|halt)\b",
    // 重定向写裸设备
    r">\s*/dev/sd[a-z]+",
];

/// 对一条候选命令做三项静态检查并判定是否命中危险清单。
pub fn validate_candidate(candidate: &str) -> ValidationResult {
    let checks = vec![
        CheckResult {
            name: CheckName::Syntax,
            passed: check_syntax(candidate),
        },
        CheckResult {
            name: CheckName::CommandExists,
            passed: check_command_exists(candidate),
        },
        CheckResult {
            name: CheckName::GitContext,
            passed: check_git_context(candidate),
        },
    ];
    let passed = !candidate.trim().is_empty() && checks.iter().all(|check| check.passed);
    ValidationResult {
        candidate: candidate.trim().to_string(),
        passed,
        checks,
        dangerous: is_dangerous(candidate),
    }
}

/// 危险命令清单判定：命中任一危险模式即返回 true。
pub fn is_dangerous(candidate: &str) -> bool {
    DANGEROUS_PATTERNS
        .iter()
        .any(|pattern| regex::Regex::new(pattern).is_ok_and(|re| re.is_match(candidate)))
}

/// zsh -n 语法检查：把候选喂给 `zsh -n` 解析，退出码 0 视为语法通过；zsh 不可用按失败处理。
fn check_syntax(candidate: &str) -> bool {
    let mut child = match Command::new("zsh")
        .arg("-n")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(candidate.as_bytes());
        let _ = stdin.flush();
    }
    child.wait().is_ok_and(|status| status.success())
}

/// 首词 `command -v` 存在性检查：关键字/特殊符号/纯赋值无可检查命令时跳过。
fn check_command_exists(candidate: &str) -> bool {
    match first_command_word(candidate) {
        Some(word) if needs_command_check(word) => command_exists_in_shell(word),
        _ => true,
    }
}

/// git 上下文匹配：候选里出现仓库相关的 git 调用时，要求当前目录在 git 仓库内。
fn check_git_context(candidate: &str) -> bool {
    if !has_repo_dependent_git(candidate) {
        return true;
    }
    in_git_repo()
}

/// 取候选要执行的首个命令词：跳过前导的 `VAR=value` 赋值（agent 契约允许「变量赋值后跟命令」）。
fn first_command_word(candidate: &str) -> Option<&str> {
    for word in candidate.split_whitespace() {
        if word.contains('=') {
            continue;
        }
        return Some(word);
    }
    None
}

/// 该词是否需要做存在性检查：关键字与 `( { [ !` 开头的结构不查。
fn needs_command_check(word: &str) -> bool {
    !SHELL_KEYWORDS.contains(&word) && !word.starts_with(['(', '{', '[', '!'])
}

/// 以 zsh 执行 `command -v -- <word>`；词经 `$1` 传入避免拼进脚本造成注入。
fn command_exists_in_shell(word: &str) -> bool {
    Command::new("zsh")
        .args(["-c", "command -v -- \"$1\" >/dev/null 2>&1", "sh", word])
        .status()
        .is_ok_and(|status| status.success())
}

/// 候选里是否存在需要仓库上下文的 git 调用：词 `git` 后跟非白名单子命令。
/// 跳过 `-` 开头的标志（git --version、git -C dir …），只认第一个非标志词为子命令。
fn has_repo_dependent_git(candidate: &str) -> bool {
    let words: Vec<&str> = candidate.split_whitespace().collect();
    for (index, word) in words.iter().enumerate() {
        if *word != "git" {
            continue;
        }
        let subcommand = words[index + 1..]
            .iter()
            .find(|word| !word.starts_with('-'));
        match subcommand {
            None => continue, // 只有标志（git --version）→ 不需要仓库
            Some(sub) if REPO_INDEPENDENT_GIT.contains(sub) => continue,
            Some(_) => return true,
        }
    }
    false
}

/// 当前目录是否在 git 工作树内。
fn in_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|text| text.trim() == "true")
}

pub fn run(args: ValidateArgs) -> i32 {
    let candidate = match args.candidate {
        Some(candidate) => candidate.trim().to_string(),
        None => read_stdin().trim().to_string(),
    };
    let result = validate_candidate(&candidate);
    match serde_json::to_string(&result) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(_) => {
            eprintln!("validate: 无法序列化校验结果");
            1
        }
    }
}
