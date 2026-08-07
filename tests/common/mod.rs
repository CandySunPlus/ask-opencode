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
        .env("ASK_OPENCODE_CONFIG", dir.path().join("no-config.json"));
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
        .env("ASK_OPENCODE_CONFIG", dir.join("no-config.json"));
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

/// 在指定目录写一个可执行的 fake opencode shim，返回其路径。
pub fn write_fake_opencode(dir: &Path, script: &str) -> PathBuf {
    let path = dir.join("opencode");
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
