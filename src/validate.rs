use crate::cli::ValidateArgs;
use crate::io::read_stdin;
use serde_json::json;

/// 校验层骨架：真实检查（zsh -n、command -v、git 上下文）见后续票，这里只输出结构化结果。
pub fn run(args: ValidateArgs) -> i32 {
    let candidate = match args.candidate {
        Some(candidate) => candidate.trim().to_string(),
        None => read_stdin().trim().to_string(),
    };
    let result = json!({
        "candidate": candidate,
        "passed": true,
        "checks": [],
    });
    println!("{result}");
    0
}
