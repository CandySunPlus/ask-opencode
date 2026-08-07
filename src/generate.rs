use crate::cli::GenerateArgs;
use crate::config::Config;
use std::io::Write;

pub fn run(args: GenerateArgs) -> i32 {
    let config = Config::load();
    let agent = args.agent.as_deref().unwrap_or(&config.agent);
    match crate::opencode::invoke(&args.request, agent) {
        Ok(output) => {
            if !output.stdout.is_empty() {
                std::io::stdout()
                    .write_all(&output.stdout)
                    .expect("写 stdout 失败");
            }
            if output.status.success() {
                return 0;
            }
            if !output.stderr.is_empty() {
                std::io::stderr()
                    .write_all(&output.stderr)
                    .expect("写 stderr 失败");
            }
            output.status.code().unwrap_or(1)
        }
        Err(err) => {
            eprintln!("generate: {}", err.message);
            1
        }
    }
}
