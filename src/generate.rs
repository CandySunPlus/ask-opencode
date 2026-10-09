use crate::backend::{Backend, BackendError, Session};
use crate::cli::GenerateArgs;
use crate::config::Config;
use crate::context::ContextSnapshot;
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

/// 有落盘 id 就续接；没有时 `may_create` 且开了 reuse_session 才新建常驻会话，否则一次性
/// 会话（ADR-0007）。
fn session_for<'a>(config: &Config, session_id: Option<&'a str>, may_create: bool) -> Session<'a> {
    match session_id {
        Some(id) => Session::Resume(id),
        None if may_create && config.reuse_session => Session::New,
        None => Session::Oneshot,
    }
}

pub fn run(args: GenerateArgs) -> i32 {
    let config = Config::load();
    let backend =
        match crate::backend::select(&config, args.agent.as_deref(), args.model.as_deref()) {
            Ok(backend) => backend,
            Err(message) => {
                eprintln!("generate: {message}");
                return 1;
            }
        };
    let snapshot = ContextSnapshot::collect(&config);
    let request = format!(
        "{}\n\n请求：{}\n\n{}",
        snapshot.render(),
        args.request,
        SNAPSHOT_INVALIDATION
    );
    let session_id = reuse_session_id(&config);
    // 会话失效自动重建（ADR-0007）：清掉旧 id，新建会话重试一次。
    let session = session_for(&config, session_id.as_deref(), true);
    let result = match backend.generate(&request, session) {
        Err(BackendError::SessionExpired) => {
            if let Err(err) = crate::resident::clear_session_id() {
                // 清不掉旧 id 不中断重建，stderr 提示便于诊断。
                eprintln!("resident: {}", err.message);
            }
            backend.generate(&request, Session::New)
        }
        other => other,
    };
    let reply = match result {
        Ok(reply) => reply,
        Err(BackendError::Failed { exit_code, stderr }) => {
            if !stderr.is_empty() {
                std::io::stderr()
                    .write_all(stderr.as_bytes())
                    .expect("写 stderr 失败");
            }
            return exit_code;
        }
        Err(BackendError::Unavailable { message }) => {
            eprintln!("generate: {message}");
            return 1;
        }
        // 只有续接会话才会失效，重试走的是新建会话，到不了这里。
        Err(BackendError::SessionExpired) => {
            eprintln!("generate: 会话失效");
            return 1;
        }
    };
    if let Some(new_session_id) = &reply.session_id
        && let Err(err) = crate::resident::save_session_id(new_session_id)
    {
        eprintln!("resident: {}", err.message);
    }
    let candidates = crate::parse::split_candidates(&reply.text);
    let (passing, failing) = split_by_validation(&candidates);
    let final_candidates = if failing.is_empty() {
        passing
    } else {
        correction_round(&failing, &passing, backend.as_ref(), &config)
    };
    crate::parse::emit_candidates(&final_candidates, "generate")
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

/// 一轮修正回喂：未通过的候选重新交给后端，修正后通过校验的并入结果；
/// 修正轮失败或修正后仍不过的候选静默丢弃，错误不回显。轮数由 ADR-0003 钉死为一轮。
fn correction_round(
    failing: &[ValidationResult],
    passing: &[String],
    backend: &dyn Backend,
    config: &Config,
) -> Vec<String> {
    let mut result = passing.to_vec();
    let fix_request = build_fix_request(failing);
    // 修正轮复用主请求同一常驻会话（ADR-0007）：主请求新建会话时 id 已落盘，这里重读；
    // 修正轮自己不新建会话。
    let session_id = reuse_session_id(config);
    let session = session_for(config, session_id.as_deref(), false);
    let Ok(reply) = backend.generate(&fix_request, session) else {
        return result;
    };
    for candidate in crate::parse::split_candidates(&reply.text) {
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
