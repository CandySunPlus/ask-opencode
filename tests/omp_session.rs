//! omp 后端的常驻会话（ADR-0007、ADR-0009）：按目录各一个，放在 ask-opencode 自有的会话目录。
mod common;
use common::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Output;

const OMP_OK: &str = "echo from-omp\n---CANDIDATE---\nls -la";

/// 一套隔离环境：`state/` 放配置与 server.json，`a/`、`b/` 是两个工作目录，`bin/` 放 shim。
struct Env {
    root: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let env = Env {
            root: tempfile::tempdir().unwrap(),
        };
        for sub in ["state", "a", "b", "bin"] {
            std::fs::create_dir(env.path(sub)).unwrap();
        }
        env
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.root.path().join(sub)
    }

    fn omp(&self, responses: &[&str]) -> FakeOmp {
        write_fake_omp(&self.path("bin"), responses)
    }

    fn state_file(&self) -> PathBuf {
        self.path("state/server.json")
    }

    fn write_state(&self, state: &Value) {
        std::fs::write(self.state_file(), state.to_string()).unwrap();
    }

    fn read_state(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.state_file()).unwrap()).unwrap()
    }

    /// omp 分区里按目录记的会话。
    fn omp_sessions(&self) -> Value {
        self.read_state()["omp"]["sessions"].clone()
    }

    /// 像 shell 一样在 `dir` 里跑：`PWD` 是 `dir` 本身（逻辑路径），常驻会话开、常驻服务关。
    fn run_in(&self, dir: &Path, args: &[&str], omp: &FakeOmp, extra: &[(&str, &str)]) -> Output {
        let hist = write_history(self.root.path(), "");
        let mut envs: Vec<(String, String)> = vec![
            ("HISTFILE".into(), path_str(&hist)),
            ("PWD".into(), path_str(dir)),
            (
                "ASK_OPENCODE_CONFIG".into(),
                path_str(&self.path("state/config.json")),
            ),
            ("ASK_OPENCODE_BACKEND".into(), "omp".into()),
            ("ASK_OPENCODE_OMP_BIN".into(), path_str(&omp.bin)),
            ("ASK_OPENCODE_RESIDENT".into(), "false".into()),
            ("ASK_OPENCODE_REUSE_SESSION".into(), "true".into()),
            ("PATH".into(), std::env::var("PATH").unwrap()),
        ];
        envs.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        run_in_dir_owned(dir, args, &envs)
    }

    fn generate_in(&self, sub: &str, omp: &FakeOmp) -> Output {
        let out = self.run_in(&self.path(sub), &["generate", "list files"], omp, &[]);
        assert!(out.status.success(), "stderr: {}", stderr_str(&out));
        out
    }
}

fn path_str(path: &Path) -> String {
    path.to_str().unwrap().to_string()
}

/// argv 里 `flag` 后紧跟的值。
fn value_after(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).cloned()
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

#[test]
fn second_request_in_same_dir_resumes_first_session() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);

    env.generate_in("a", &omp);
    let first = omp.args(1);
    assert!(!has_flag(&first, "--no-session"), "{first:?}");
    assert!(!has_flag(&first, "--resume"), "{first:?}");
    assert_eq!(
        env.omp_sessions(),
        json!({path_str(&env.path("a")): "omp-sess-1"})
    );

    env.generate_in("a", &omp);
    let second = omp.args(2);
    assert_eq!(
        value_after(&second, "--resume").as_deref(),
        Some("omp-sess-1")
    );
    assert!(!has_flag(&second, "--no-session"), "{second:?}");
}

#[test]
fn correction_round_resumes_the_same_session() {
    let env = Env::new();
    let omp = env.omp(&[
        "echo hello\n---CANDIDATE---\nfoobar_nonexistent_xyz",
        "echo fixed",
    ]);

    let out = env.generate_in("a", &omp);
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        json!(["echo hello", "echo fixed"])
    );
    assert_eq!(omp.calls(), 2);
    assert_eq!(
        value_after(&omp.args(2), "--resume").as_deref(),
        Some("omp-sess-1")
    );
}

#[test]
fn each_dir_has_its_own_session_and_returning_resumes_it() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);

    env.generate_in("a", &omp);
    env.generate_in("b", &omp);
    let in_b = omp.args(2);
    assert!(!has_flag(&in_b, "--resume"), "换目录应开新会话: {in_b:?}");
    env.generate_in("a", &omp);
    assert_eq!(
        value_after(&omp.args(3), "--resume").as_deref(),
        Some("omp-sess-1")
    );

    assert_eq!(
        env.omp_sessions(),
        json!({
            path_str(&env.path("a")): "omp-sess-1",
            path_str(&env.path("b")): "omp-sess-2",
        })
    );
}

/// 按 `$PWD` 逻辑路径记，不解析符号链接：从链接进入的目录与链接目标各算各的。
#[cfg(unix)]
#[test]
fn session_is_keyed_by_logical_pwd_not_realpath() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);
    let link = env.path("link-to-a");
    std::os::unix::fs::symlink(env.path("a"), &link).unwrap();

    let out = env.run_in(&link, &["generate", "list files"], &omp, &[]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_eq!(env.omp_sessions(), json!({path_str(&link): "omp-sess-1"}));
}

#[test]
fn every_omp_call_uses_own_session_dir_under_config_dir() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);
    let session_dir = path_str(&env.path("state/omp-sessions"));

    env.generate_in("a", &omp);
    env.generate_in("a", &omp);
    let out = env.run_in(
        &env.path("a"),
        &["generate", "list files"],
        &omp,
        &[("ASK_OPENCODE_REUSE_SESSION", "false")],
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    for n in 1..=3 {
        assert_eq!(
            value_after(&omp.args(n), "--session-dir").as_deref(),
            Some(session_dir.as_str()),
            "第 {n} 次调用: {:?}",
            omp.args(n)
        );
    }
    assert!(has_flag(&omp.args(3), "--no-session"), "{:?}", omp.args(3));
    assert!(!has_flag(&omp.args(3), "--resume"), "{:?}", omp.args(3));
}

#[test]
fn expired_session_is_rebuilt_once_and_new_id_persisted() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);
    let a = path_str(&env.path("a"));
    env.write_state(&json!({"omp": {"sessions": {&a: "omp-gone"}}}));
    omp.expire_session("omp-gone");

    let out = env.generate_in("a", &omp);
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        json!(["echo from-omp", "ls -la"])
    );
    assert_eq!(
        value_after(&omp.args(1), "--resume").as_deref(),
        Some("omp-gone")
    );
    let retry = omp.args(2);
    assert!(!has_flag(&retry, "--resume"), "{retry:?}");
    assert!(!has_flag(&retry, "--no-session"), "{retry:?}");
    assert_eq!(env.omp_sessions(), json!({&a: "omp-sess-2"}));
}

#[test]
fn rebuild_failing_again_reports_error() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);
    let a = path_str(&env.path("a"));
    env.write_state(&json!({"omp": {"sessions": {&a: "omp-gone"}}}));
    omp.expire_session("omp-gone");
    omp.fail_call(2, "model not found\n");

    let out = env.run_in(&env.path("a"), &["generate", "list files"], &omp, &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr_str(&out).contains("model not found"),
        "{}",
        stderr_str(&out)
    );
    assert_eq!(omp.calls(), 2, "只重建一次");
}

/// 只有退出码 1 + `Session "<id>" not found.` 才算会话失效。
#[test]
fn not_found_text_with_other_exit_code_does_not_rebuild() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);
    let a = path_str(&env.path("a"));
    env.write_state(&json!({"omp": {"sessions": {&a: "omp-keep"}}}));
    omp.fail_call(1, "Error: Session \"omp-keep\" not found.\n");

    let out = env.run_in(&env.path("a"), &["generate", "list files"], &omp, &[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(omp.calls(), 1, "不应重建");
    assert_eq!(env.omp_sessions(), json!({&a: "omp-keep"}));
}

#[test]
fn reset_session_clears_only_current_dir_omp_session() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);
    let a = path_str(&env.path("a"));
    let b = path_str(&env.path("b"));
    let opencode = json!({"url": "http://127.0.0.1:1", "pid": 123, "session_id": "ses-oc"});
    env.write_state(&json!({
        "opencode": opencode,
        "omp": {"sessions": {&a: "omp-sess-a", &b: "omp-sess-b"}},
    }));

    let out = env.run_in(&env.path("a"), &["reset-session"], &omp, &[]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_eq!(
        env.read_state(),
        json!({"opencode": opencode, "omp": {"sessions": {&b: "omp-sess-b"}}})
    );
    assert_eq!(omp.calls(), 0);
}

#[test]
fn reset_session_of_last_dir_drops_empty_omp_partition() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);
    let opencode = json!({"session_id": "ses-oc"});
    env.write_state(&json!({
        "opencode": opencode,
        "omp": {"sessions": {path_str(&env.path("a")): "omp-sess-a"}},
    }));

    let out = env.run_in(&env.path("a"), &["reset-session"], &omp, &[]);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_eq!(env.read_state(), json!({"opencode": opencode}));
}

/// 在 opencode 与 omp 间来回切，各自续接自己的会话。
#[test]
fn switching_backends_keeps_each_backends_session() {
    let env = Env::new();
    let omp = env.omp(&[OMP_OK]);
    let a = env.path("a");
    env.write_state(&json!({"opencode": {"session_id": "ses-oc"}}));
    let log = env.path("bin/opencode-args.log");
    let opencode = write_shim_echo_args(&env.path("bin"), &log);
    let as_opencode = [
        ("ASK_OPENCODE_BACKEND", "opencode"),
        ("ASK_OPENCODE_BIN", opencode.to_str().unwrap()),
    ];

    env.generate_in("a", &omp);
    let out = env.run_in(&a, &["generate", "list files"], &omp, &as_opencode);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));
    let opencode_args = std::fs::read_to_string(&log).unwrap();
    assert!(
        opencode_args.contains("--session\n@@@\nses-oc\n"),
        "{opencode_args}"
    );
    env.generate_in("a", &omp);

    assert_eq!(
        value_after(&omp.args(2), "--resume").as_deref(),
        Some("omp-sess-1")
    );
    assert_eq!(
        env.read_state(),
        json!({
            "opencode": {"session_id": "ses-oc"},
            "omp": {"sessions": {path_str(&a): "omp-sess-1"}},
        })
    );
}

/// 状态文件读不出时写入不能悄悄清掉其他分区：stderr 提示，并把原文件留一份备份。
#[test]
fn unreadable_state_file_is_backed_up_with_warning_before_rewrite() {
    for broken in ["{not json", "[1, 2]"] {
        let env = Env::new();
        let omp = env.omp(&[OMP_OK]);
        std::fs::write(env.state_file(), broken).unwrap();

        let out = env.generate_in("a", &omp);

        let stderr = stderr_str(&out);
        assert!(stderr.contains("server.json"), "{broken}: {stderr}");
        let backup = env.path("state/server.json.corrupt");
        assert!(stderr.contains(&path_str(&backup)), "{broken}: {stderr}");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), broken);
        assert_eq!(
            env.omp_sessions(),
            json!({path_str(&env.path("a")): "omp-sess-1"})
        );
    }
}
