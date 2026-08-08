// 发布资产打包契约（T1 #38）的生产侧钉桩：`scripts/package-release.sh` 打出的资产必须与
// install.rs 消费的 fixture 同形——tar.gz 顶层即 `ask-opencode`、`.sha256` 单行
// `<hash>  <name>`、命名 `ask-opencode-<os>-<arch>-<version>.tar.gz`。脚本被 CI release
// workflow 与这里的测试共用，跑在沙箱里、只依赖宿主自带工具。
mod common;
use common::*;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const VERSION: &str = "v1.2.3";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn package_script() -> PathBuf {
    repo_root().join("scripts/package-release.sh")
}

/// 沙箱：输出目录 + 可放 stub 的 fake-bin 目录。
struct Sandbox {
    _dir: tempfile::TempDir,
    out: PathBuf,
    fake_bin: PathBuf,
}

fn setup_sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dist");
    let fake_bin = dir.path().join("fake-bin");
    fs::create_dir_all(&out).unwrap();
    fs::create_dir_all(&fake_bin).unwrap();
    Sandbox {
        _dir: dir,
        out,
        fake_bin,
    }
}

/// 宿主没有 sha256sum（macOS）时放一个委托 shasum 的 stub；linux 宿主用真实 sha256sum。
fn install_fakes(s: &Sandbox) {
    if Command::new("sha256sum").arg("--version").output().is_err() {
        write_fake_bin(&s.fake_bin, "sha256sum", "exec shasum -a 256 \"$@\"");
    }
}

/// 黑盒驱动打包脚本：伪造一个可执行二进制，输出目录里应只出现契约命名的两个文件。
fn run_package(s: &Sandbox, os: &str, arch: &str) -> Output {
    let bin = write_fake_bin(s._dir.path(), "ask-opencode", "echo fake-ask-opencode");
    let path = format!("{}:{}", s.fake_bin.display(), std::env::var("PATH").unwrap());
    Command::new(package_script())
        .args([os, arch, VERSION])
        .arg(&bin)
        .arg(&s.out)
        .env("PATH", path)
        .output()
        .unwrap()
}

/// 全契约：命名、tar 顶层布局、二进制可执行、.sha256 单行 `<hash>  <name>`（两空格）与真实值一致。
#[test]
fn packages_asset_with_contract_naming_and_layout() {
    let s = setup_sandbox();
    install_fakes(&s);

    let out = run_package(&s, "darwin", "aarch64");
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    let asset = s.out.join(format!("ask-opencode-darwin-aarch64-{VERSION}.tar.gz"));
    let sha = s.out.join(format!("ask-opencode-darwin-aarch64-{VERSION}.tar.gz.sha256"));
    assert!(asset.exists(), "应产出命名契约的 tar.gz");
    assert!(sha.exists(), "应产出同名 .sha256");

    let list = Command::new("tar").args(["-tzf"]).arg(&asset).output().unwrap();
    assert!(list.status.success());
    let listing = String::from_utf8_lossy(&list.stdout);
    let entries: Vec<&str> = listing.lines().collect();
    assert_eq!(entries, vec!["ask-opencode"], "tar 顶层应只有 ask-opencode: {entries:?}");

    let extract = s._dir.path().join("extract");
    fs::create_dir_all(&extract).unwrap();
    let status = Command::new("tar")
        .args(["-xzf"])
        .arg(&asset)
        .current_dir(&extract)
        .status()
        .unwrap();
    assert!(status.success(), "解压失败");
    let extracted = extract.join("ask-opencode");
    let mode = fs::metadata(&extracted).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "解压出的二进制应可执行: mode={mode:o}");
    let run = Command::new(&extracted).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "fake-ask-opencode",
        "解压出的二进制应可运行"
    );

    let hash = sha256_of(&asset);
    let name = asset.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        fs::read_to_string(&sha).unwrap(),
        format!("{hash}  {name}\n"),
        ".sha256 应为单行 `<hash>  <name>`（两空格）且与资产实际值一致"
    );
}

/// 命名矩阵钉契约：三个发布目标各产出对应命名，`.sha256` 同步同名。
#[test]
fn naming_matrix_matches_contract() {
    for (os, arch) in [("darwin", "aarch64"), ("linux", "aarch64"), ("linux", "x86_64")] {
        let s = setup_sandbox();
        install_fakes(&s);

        let out = run_package(&s, os, arch);
        assert!(out.status.success(), "{os}/{arch} stderr: {}", stderr_str(&out));

        let asset = s.out.join(format!("ask-opencode-{os}-{arch}-{VERSION}.tar.gz"));
        let sha = s.out.join(format!("ask-opencode-{os}-{arch}-{VERSION}.tar.gz.sha256"));
        assert!(asset.exists(), "{os}/{arch} 应产出资产");
        assert!(sha.exists(), "{os}/{arch} 应产出校验文件");
    }
}

/// 未知 os/arch（不在矩阵内）应报错而非打出错命名的资产。
#[test]
fn rejects_unknown_platform() {
    let s = setup_sandbox();
    install_fakes(&s);

    let out = run_package(&s, "darwin", "x86_64");
    assert!(!out.status.success(), "矩阵外的平台应非零退出");
    assert!(stderr_str(&out).contains("x86_64"), "stderr: {}", stderr_str(&out));
    let leftover: Vec<_> = fs::read_dir(&s.out).unwrap().collect();
    assert!(leftover.is_empty(), "失败不应留下任何文件: {leftover:?}");
}
