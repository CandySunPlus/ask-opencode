use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ask-opencode",
    version,
    about = "把 '#' 请求交给 opencode 生成可执行命令"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// 调用 opencode 为一个请求生成候选命令
    Generate(GenerateArgs),
    /// 按分隔行契约把候选文本切块为结构化候选
    Parse(ParseArgs),
    /// 校验一条候选命令
    Validate(ValidateArgs),
    /// 在候选命令里挑一条：多条弹选择器（内嵌 skim 或外部 fzf），危险命令需 [y/N] 确认后输出
    Select(SelectArgs),
}

#[derive(Args)]
pub struct GenerateArgs {
    /// 要生成候选命令的请求
    #[arg(value_name = "REQUEST")]
    pub request: String,

    /// 使用的 opencode agent，覆盖配置里的默认值
    #[arg(long)]
    pub agent: Option<String>,

    /// 使用的模型（provider/model），覆盖配置里的默认值
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Args)]
pub struct ParseArgs {
    /// 候选文本；不提供时从 stdin 读取
    #[arg(value_name = "TEXT", allow_hyphen_values = true)]
    pub text: Vec<String>,
}

#[derive(Args)]
pub struct ValidateArgs {
    /// 候选命令；不提供时从 stdin 读取
    #[arg(value_name = "CANDIDATE")]
    pub candidate: Option<String>,
}

#[derive(Args)]
pub struct SelectArgs {
    /// 从 generate 的 JSON 候选文件读候选（与逐条列出互斥，widget 用它把生成结果直接交给选择器）
    #[arg(long, conflicts_with = "candidates")]
    pub file: Option<String>,

    /// 候选命令，逐条列出（每条一个参数，多行候选也能作单条参数）
    #[arg(value_name = "CANDIDATE", allow_hyphen_values = true)]
    pub candidates: Vec<String>,

    /// 使用的选择器：skim（内嵌）或 fzf（外部），覆盖配置默认值
    #[arg(long)]
    pub picker: Option<String>,

    /// 外部 fzf 可执行文件路径，覆盖配置默认值
    #[arg(long)]
    pub fzf_bin: Option<String>,
}
