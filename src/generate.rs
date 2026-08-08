use crate::cli::GenerateArgs;
use crate::config::Config;
use crate::context::ContextSnapshot;
use crate::opencode::OutputFormat;
use crate::validate::{ValidationResult, validate_candidate};
use std::io::Write;

/// ADR-0007：请求尾部这条声明是「把快照从会话记忆剥离」决策的落点。
const SNAPSHOT_INVALIDATION: &str = "忽略本会话历史中的旧上下文快照，以本条为准";

/// reuse_session 开启时读落盘的会话 id；关闭复用或没落盘返回 None（ADR-0007）。
fn reuse_session_id(config: &Config) -> Option<String> {
    if config.reuse_session {
        crate::resident::load_session_id()
    } else {
        None
    }
}

pub fn run(args: GenerateArgs) -> i32 {
    let config = Config::load();
    let agent = args.agent.as_deref().unwrap_or(&config.agent);
    let model = args.model.as_deref().or(config.model.as_deref());
    let snapshot = ContextSnapshot::collect(&config);
    let request = format!(
        "{}\n\n请求：{}\n\n{}",
        snapshot.render(),
        args.request,
        SNAPSHOT_INVALIDATION
    );
    // 常驻会话（ADR-0007）：无落盘 id 时走 json 首次路径抓 id，否则 default 格式复用同一会话。
    let session_id = reuse_session_id(&config);
    let format = if config.reuse_session && session_id.is_none() {
        OutputFormat::Json
    } else {
        OutputFormat::Default
    };
    match crate::opencode::invoke(&request, agent, model, &config, format, session_id.as_deref()) {
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
            let candidates = match format {
                OutputFormat::Json => {
                    let events = crate::parse::parse_json_events(&stdout);
                    if let Some(session_id) = &events.session_id
                        && let Err(err) = crate::resident::save_session_id(session_id)
                    {
                        // 落盘失败不中断本次生成，stderr 提示便于诊断（ADR-0007）。
                        eprintln!("resident: {}", err.message);
                    }
                    crate::parse::split_candidates(&events.text)
                }
                OutputFormat::Default => crate::parse::split_candidates(&stdout),
            };
            let (passing, failing) = split_by_validation(&candidates);
            let final_candidates = if failing.is_empty() {
                passing
            } else {
                correction_round(&failing, &passing, agent, model, &config)
            };
            crate::parse::emit_candidates(&final_candidates, "generate")
        }
        Err(err) => {
            eprintln!("generate: {}", err.message);
            1
        }
    }
}

/// 把候选按是否通过三项静态检查拆成两组。
fn split_by_validation(candidates: &[String]) -> (Vec<String>, Vec<ValidationResult>) {
    let mut passing = Vec::new();
    let mut failing = Vec::new();
    for candidate in candidates {
        let result = validate_candidate(candidate);
        if result.passed {
            passing.push(candidate.clone());
        } else {
            failing.push(result);
        }
    }
    (passing, failing)
}

/// 一轮修正回喂：未通过的候选重新交给 opencode，修正后通过校验的并入结果；
/// 修正轮失败或修正后仍不过的候选静默丢弃，错误不回显。轮数由 ADR-0003 钉死为一轮。
fn correction_round(
    failing: &[ValidationResult],
    passing: &[String],
    agent: &str,
    model: Option<&str>,
    config: &Config,
) -> Vec<String> {
    let mut result = passing.to_vec();
    let fix_request = build_fix_request(failing);
    // 修正轮复用主请求同一常驻会话（ADR-0007）：主请求刚走 json 首次路径时 id 已落盘，这里重读。
    let session_id = reuse_session_id(config);
    let Ok(output) = crate::opencode::invoke(
        &fix_request,
        agent,
        model,
        config,
        OutputFormat::Default,
        session_id.as_deref(),
    ) else {
        return result;
    };
    if !output.status.success() {
        return result;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for candidate in crate::parse::split_candidates(&text) {
        if validate_candidate(&candidate).passed {
            result.push(candidate);
        }
    }
    result
}

/// 构造修正请求：列出未通过校验的候选与其失败项，要求模型只输出修正版本。
fn build_fix_request(failing: &[ValidationResult]) -> String {
    let details = failing
        .iter()
        .enumerate()
        .map(|(index, result)| {
            format!(
                "{}. 候选：{}\n   失败项：{}",
                index + 1,
                result.candidate,
                result.failed_labels().join("、")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "以下候选命令未能通过可运行性校验，请给出修正后的版本：\n{details}\n\n候选间仍用独占一行的 {} 分隔，只输出候选本身，不要解释。",
        crate::parse::CANDIDATE_SEPARATOR
    )
}
