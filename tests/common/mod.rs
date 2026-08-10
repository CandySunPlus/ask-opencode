// 共享测试助手：每个测试二进制独立编译本模块，未用到的助手属正常。
#![allow(dead_code)]

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use std::process::Output;

/// 以 argv + env 黑盒驱动 ask-opencode 二进制，返回完整输出。
pub fn run(args: &[&str]) -> Output {
    run_with(args, &[], "")
}

pub fn run_with_env(args: &[&str], envs: &[(&str, &str)]) -> Output {
    run_with(args, envs, "")
}

pub fn run_with_input(args: &[&str], stdin: &str) -> Output {
    run_with(args, &[], stdin)
}

pub fn run_with(args: &[&str], envs: &[(&str, &str)], stdin: &str) -> Output {
    // 在临时目录里跑：避免读到真实 ~/.zsh_history、真实用户配置与所在 git 仓库污染断言。
    let dir = tempfile::tempdir().unwrap();
    let hist = write_history(dir.path(), "");
    let mut cmd = Command::cargo_bin("ask-opencode").unwrap();
    cmd.current_dir(dir.path())
        .args(args)
        .env("HISTFILE", &hist)
        .env("ASK_OPENCODE_CONFIG", dir.path().join("no-config.json"))
        // 常驻 serve 单独在 tests/resident.rs 覆盖；这里默认关，保证冷启动路径确定性。
        .env("ASK_OPENCODE_RESIDENT", "false")
        // 常驻会话路径单独在 tests/session.rs 覆盖；这里默认关，沿用每次新会话的旧行为。
        .env("ASK_OPENCODE_REUSE_SESSION", "false");
    cmd.envs(envs.iter().copied()).write_stdin(stdin);
    cmd.output().unwrap()
}

/// 指定工作目录后以 argv + env 黑盒驱动 ask-opencode 二进制，默认注入空 HISTFILE 与不存在的配置路径。
pub fn run_in_dir_with_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let hist = write_history(dir, "");
    let mut cmd = Command::cargo_bin("ask-opencode").unwrap();
    cmd.current_dir(dir)
        .args(args)
        .env("HISTFILE", &hist)
        .env("ASK_OPENCODE_CONFIG", dir.join("no-config.json"))
        .env("ASK_OPENCODE_RESIDENT", "false")
        .env("ASK_OPENCODE_REUSE_SESSION", "false");
    cmd.envs(envs.iter().copied());
    cmd.output().unwrap()
}

/// 以 owned env（可带任意 String 值）在指定目录驱动二进制，不做任何默认注入。
pub fn run_in_dir_owned(dir: &Path, args: &[&str], envs: &[(String, String)]) -> Output {
    let mut cmd = Command::cargo_bin("ask-opencode").unwrap();
    cmd.current_dir(dir).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

/// 在目录里写 zsh 历史文件，返回其路径。
pub fn write_history(dir: &Path, content: &str) -> PathBuf {
    let path = dir.join("zsh_history");
    std::fs::write(&path, content).unwrap();
    path
}

/// 写一个可执行的 fake opencode shim，返回其路径。
pub fn write_fake_opencode(dir: &Path, script: &str) -> PathBuf {
    write_fake_bin(dir, "opencode", script)
}

/// 写一个可执行的 fake fzf shim：把 stdin 原样写入 `$FZF_INPUT_LOG`，再把 `$FZF_SELECT`
/// 以 NUL 结尾打到 stdout，模拟用户在 fzf 里选中一条候选。返回其路径。
pub fn write_fake_fzf(dir: &Path) -> PathBuf {
    let script = "cat > \"$FZF_INPUT_LOG\"\nprintf '%s\\0' \"$FZF_SELECT\"\n";
    write_fake_bin(dir, "fzf", script)
}

/// 写一个可执行的 stub 脚本到指定目录，返回其路径。
pub fn write_fake_bin(dir: &Path, name: &str, script: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }
    path
}

/// 常驻 serve 的 HTTP 后端（ADR-0004 修订：resident 路径不再走 `opencode run --attach`，
/// 改用 serve 的 HTTP API）。写一个可执行的 python 脚本，由 fake opencode shim 的 serve
/// 分支 exec 起来，返回脚本路径。环境变量契约：
///   FAKE_SERVE_PORT    监听端口
///   FAKE_MSG_LOG       POST /message 的请求体逐行 JSON 追加写入该文件
///   FAKE_SESSION_LOG   POST /session 新建的会话 id 追加写入该文件
///   FAKE_RESPONSE      POST /message 返回的助手 text part 内容
///   FAKE_RESPONSE_N    第 N 次 POST /message 返回该内容（覆盖 FAKE_RESPONSE），供修正轮等
///                      按调用序号给不同响应的场景
///   FAKE_404_SESSION   命中该会话 id 的消息返回 404「Session not found」（模拟会话失效）
pub fn write_fake_serve(dir: &Path) -> PathBuf {
    let script = r#"import json, os, re
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(os.environ["FAKE_SERVE_PORT"])
MSG_LOG = os.environ.get("FAKE_MSG_LOG", "")
SESSION_LOG = os.environ.get("FAKE_SESSION_LOG", "")
RESPONSE = os.environ.get("FAKE_RESPONSE", "echo hello\n---CANDIDATE---\nls -la\n")
NOT_FOUND_SESSION = os.environ.get("FAKE_404_SESSION", "")
counter = {"n": 0, "m": 0}

def nth_response():
    return os.environ.get("FAKE_RESPONSE_%d" % counter["m"]) or RESPONSE

class H(BaseHTTPRequestHandler):
    def _reply(self, code, obj):
        body = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b""
        path = self.path.split("?")[0]
        if path == "/session":
            counter["n"] += 1
            sid = "sess-http-%d" % counter["n"]
            if SESSION_LOG:
                with open(SESSION_LOG, "a") as f:
                    f.write(sid + "\n")
            self._reply(200, {"id": sid})
            return
        m = re.match(r"^/session/(sess-[^/]+)/message$", path)
        if m:
            sid = m.group(1)
            counter["m"] += 1
            if MSG_LOG:
                with open(MSG_LOG, "a") as f:
                    f.write(json.dumps(json.loads(raw or "{}"), ensure_ascii=False) + "\n")
            if NOT_FOUND_SESSION and sid == NOT_FOUND_SESSION:
                self._reply(404, {"name": "NotFoundError", "data": {"message": "Session not found: " + sid}})
                return
            self._reply(200, {"info": {"role": "assistant"}, "parts": [{"type": "text", "text": nth_response()}]})
            return
        self._reply(404, {"name": "NotFoundError", "data": {"message": "no route"}})

    def log_message(self, *args):
        pass

HTTPServer(("127.0.0.1", PORT), H).serve_forever()
"#;
    write_fake_bin(dir, "fake-serve.py", script)
}

/// 写一个把 argv 逐行写入日志（`@@@` 分隔、请求文本可能含换行）、stdout 回 `echo done` 的
/// fake opencode shim，返回其路径。用于断言 opencode 收到的参数与请求内容。
pub fn write_shim_echo_args(dir: &Path, log: &Path) -> PathBuf {
    let script = format!(
        "for a in \"$@\"; do printf '%s\\n@@@\\n' \"$a\"; done > \"{}\"\nprintf 'echo done\\n'",
        log.display()
    );
    write_fake_opencode(dir, &script)
}

pub fn stdout_str(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr_str(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// 解析 fake opencode 的 argv 日志中最后一个参数（请求文本）；日志为空返回空串。
/// 日志格式：每条参数独占一行、以 `@@@` 行分隔（请求文本可能含换行）。
pub fn request_from_log(log: &str) -> String {
    let marker = "\n@@@\n";
    let chunks: Vec<&str> = log
        .split(marker)
        .filter(|chunk| !chunk.is_empty())
        .collect();
    chunks.last().copied().unwrap_or("").trim_end().to_string()
}

/// 计算文件 sha256。darwin/linux 两工具输出同形（`<hash>  <path>`），取首个字段；宿主无
/// shasum（linux）时回退 sha256sum，保证测试在两平台都能跑。
pub fn sha256_of(path: &Path) -> String {
    let use_shasum = Command::new("shasum").arg("--version").output().is_ok();
    let out = if use_shasum {
        Command::new("shasum").args(["-a", "256"]).arg(path).output().unwrap()
    } else {
        Command::new("sha256sum").arg(path).output().unwrap()
    };
    assert!(out.status.success(), "计算 sha256 失败: {}", path.display());
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}
