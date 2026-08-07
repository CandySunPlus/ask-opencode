use crate::cli::GenerateArgs;
use crate::config::Config;
use crate::context::ContextSnapshot;
use std::io::Write;

pub fn run(args: GenerateArgs) -> i32 {
    let config = Config::load();
    let agent = args.agent.as_deref().unwrap_or(&config.agent);
    let model = args.model.as_deref().or(config.model.as_deref());
    let snapshot = ContextSnapshot::collect();
    let request = format!("{}\n\n请求：{}", snapshot.render(), args.request);
    match crate::opencode::invoke(&request, agent, model) {
        Ok(output) => {
            if !output.status.success() {
                if !output.stderr.is_empty() {
                    std::io::stderr()
                        .write_all(&output.stderr)
                        .expect("写 stderr 失败");
                }
                return output.status.code().unwrap_or(1);
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let candidates = crate::parse::split_candidates(&stdout);
            crate::parse::emit_candidates(&candidates, "generate")
        }
        Err(err) => {
            eprintln!("generate: {}", err.message);
            1
        }
    }
}
