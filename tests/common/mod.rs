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
    let mut cmd = Command::cargo_bin("ask-opencode").unwrap();
    cmd.args(args).envs(envs.iter().copied()).write_stdin(stdin);
    cmd.output().unwrap()
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

pub fn stdout_str(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr_str(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
