mod common;
use common::*;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TAG: &str = "v0.1.0";
const RAW_MAIN: &str =
    "https://raw.githubusercontent.com/CandySunPlus/ask-opencode/main/install.sh";
/// 固定 darwin/arm64 平台信号的 uname 覆盖（macOS arm64 归一为 aarch64 的输入）。
const DARWIN_ARM64: &[(&str, &str)] = &[("FAKE_UNAME_S", "Darwin"), ("FAKE_UNAME_M", "arm64")];

/// stub curl：按 URL 分发 fixture。`-o` 写文件，否则写 stdout；每次请求的 URL 追加到
/// `$CURL_LOG`，供断言下载顺序与资产命名契约。
const CURL_STUB: &str = r#"
out=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
echo "$url" >> "$CURL_LOG"
emit() {
  if [ -n "$out" ]; then cat "$1" > "$out"; else cat "$1"; fi
}
case "$url" in
  */releases/latest) emit "$RELEASES_JSON" ;;
  *install.sh) emit "$FAKE_INSTALL_SCRIPT" ;;
  *ask-opencode.plugin.zsh) emit "$FIXTURE_PLUGIN" ;;
  *.tar.gz) emit "$FIXTURE_ASSET" ;;
  *.sha256) emit "$FIXTURE_SHA" ;;
  *) echo "unexpected url: $url" >&2; exit 1 ;;
esac
"#;

/// stub uname：`$FAKE_UNAME_S`/`$FAKE_UNAME_M` 未设时回落到真实 uname（经 /usr/bin/uname 避免递归）。
const UNAME_STUB: &str = r#"
case "$1" in
  -s) printf '%s\n' "${FAKE_UNAME_S:-$(/usr/bin/uname -s)}" ;;
  -m) printf '%s\n' "${FAKE_UNAME_M:-$(/usr/bin/uname -m)}" ;;
  *) exit 1 ;;
esac
"#;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn install_sh() -> PathBuf {
    repo_root().join("install.sh")
}

/// 沙箱：隔离 HOME / TMPDIR / PATH，stub curl/uname 与 fixture 资产都落在沙箱内。
struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    tmp: PathBuf,
    fake_bin: PathBuf,
    curl_log: PathBuf,
    fixture_asset: PathBuf,
    fixture_sha: PathBuf,
    fixture_plugin: PathBuf,
    releases_json: PathBuf,
}

fn setup_sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let home = root.join("home");
    let tmp = root.join("tmp");
    let fake_bin = root.join("fake-bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&tmp).unwrap();
    fs::create_dir_all(&fake_bin).unwrap();
    Sandbox {
        home,
        tmp,
        fake_bin,
        curl_log: root.join("curl.log"),
        fixture_asset: root.join("fixture.tar.gz"),
        fixture_sha: root.join("fixture.sha256"),
        fixture_plugin: root.join("fixture.plugin.zsh"),
        releases_json: root.join("releases.json"),
        _dir: dir,
    }
}

/// 放上 stub curl/uname；linux 分支校验走 sha256sum，macOS 宿主没有，用委托 shasum 的 stub。
fn install_fakes(s: &Sandbox) {
    write_fake_bin(&s.fake_bin, "curl", CURL_STUB);
    write_fake_bin(&s.fake_bin, "uname", UNAME_STUB);
    write_fake_bin(&s.fake_bin, "sha256sum", "exec shasum -a 256 \"$@\"");
}

/// 造 fixture：顶层含 `ask-opencode` 的 tar.gz + 真实 sha256 校验文件 + releases/latest JSON。
fn make_fixtures(s: &Sandbox) {
    let content = s._dir.path().join("fixture-content");
    fs::create_dir_all(&content).unwrap();
    fs::write(
        content.join("ask-opencode"),
        "#!/bin/sh\necho fake-ask-opencode\n",
    )
    .unwrap();
    let status = Command::new("tar")
        .args(["-czf"])
        .arg(&s.fixture_asset)
        .arg("-C")
        .arg(&content)
        .arg("ask-opencode")
        .status()
        .unwrap();
    assert!(status.success(), "打包 fixture 失败");
    let hash = sha256_of(&s.fixture_asset);
    fs::write(
        &s.fixture_sha,
        format!("{hash}  ask-opencode-darwin-aarch64-{TAG}.tar.gz\n"),
    )
    .unwrap();
    fs::write(
        &s.releases_json,
        format!("{{\"tag_name\":\"{TAG}\",\"prerelease\":false}}\n"),
    )
    .unwrap();
    fs::write(
        &s.fixture_plugin,
        "#!/bin/zsh\n# fake ask-opencode zsh plugin\n",
    )
    .unwrap();
}

/// 驱动 install.sh 的完整环境：PATH 前置 stub 目录、隔离 HOME/TMPDIR、fixture 变量。
fn envs(s: &Sandbox, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let path = format!("{}:{}", s.fake_bin.display(), std::env::var("PATH").unwrap());
    let mut envs: Vec<(String, String)> = vec![
        ("PATH".into(), path),
        ("HOME".into(), s.home.display().to_string()),
        ("TMPDIR".into(), s.tmp.display().to_string()),
        ("CURL_LOG".into(), s.curl_log.display().to_string()),
        ("RELEASES_JSON".into(), s.releases_json.display().to_string()),
        ("FIXTURE_ASSET".into(), s.fixture_asset.display().to_string()),
        ("FIXTURE_SHA".into(), s.fixture_sha.display().to_string()),
        ("FIXTURE_PLUGIN".into(), s.fixture_plugin.display().to_string()),
        ("FAKE_INSTALL_SCRIPT".into(), install_sh().display().to_string()),
    ];
    for (k, v) in extra {
        envs.push(((*k).into(), (*v).to_string()));
    }
    envs
}

/// 仓库内直接跑 install.sh（等价 `./install.sh`）。
fn run_install(s: &Sandbox, args: &[&str], extra: &[(&str, &str)]) -> Output {
    Command::new(install_sh())
        .current_dir(s._dir.path())
        .args(args)
        .envs(envs(s, extra))
        .output()
        .unwrap()
}

/// curl|sh 路径：stub curl 从 raw main 拉 install.sh 再交给 sh，真实模拟线上那一条路径。
fn run_curl_pipe_sh(s: &Sandbox, extra: &[(&str, &str)]) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(format!("curl -fsSL {RAW_MAIN} | sh"))
        .current_dir(s._dir.path())
        .envs(envs(s, extra))
        .output()
        .unwrap()
}

fn assert_installed_bin(bin: &Path) {
    assert!(bin.exists(), "应已安装: {}", bin.display());
    let mode = fs::metadata(bin).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "应可执行: mode={mode:o}");
    let out = Command::new(bin).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "fake-ask-opencode",
        "装上的二进制应可运行"
    );
}

/// TMPDIR 沙箱里不应残留临时 tar.gz 或解压目录。
fn assert_tmp_empty(s: &Sandbox) {
    let leftover: Vec<_> = fs::read_dir(&s.tmp).unwrap().collect();
    assert!(leftover.is_empty(), "临时文件应清理干净: {leftover:?}");
}

fn curl_log(s: &Sandbox) -> Vec<String> {
    if !s.curl_log.exists() {
        return Vec::new();
    }
    fs::read_to_string(&s.curl_log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

/// 全链路：下载 → 校验 → 安装 → 清理。用固定 uname 信号保证命名断言与宿主平台无关。
#[test]
fn installs_latest_with_download_verify_install_cleanup() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let out = run_install(&s, &[], DARWIN_ARM64);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    assert_tmp_empty(&s);

    let log = curl_log(&s);
    assert_eq!(log.len(), 3, "应有 API/资产/校验 三次请求: {log:?}");
    assert!(log[0].ends_with("/releases/latest"), "{log:?}");
    assert!(
        log[1].ends_with(&format!("ask-opencode-darwin-aarch64-{TAG}.tar.gz")),
        "资产 URL 应带归一化的 aarch64: {log:?}"
    );
    assert!(log[2].ends_with(".sha256"), "{log:?}");
}

/// 直接跑与 curl|sh 两条路径装出同一结果。
#[test]
fn curl_pipe_sh_and_direct_install_are_consistent() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let direct = run_install(&s, &[], DARWIN_ARM64);
    assert!(direct.status.success(), "stderr: {}", stderr_str(&direct));

    let piped = run_curl_pipe_sh(&s, DARWIN_ARM64);
    assert!(piped.status.success(), "stderr: {}", stderr_str(&piped));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    assert_tmp_empty(&s);
    assert!(
        stdout_str(&piped).contains(&format!("ask-opencode {TAG}")),
        "piped 路径应打印安装完成信息: {}",
        stdout_str(&piped)
    );
}

/// `-b` 覆盖 binary 目录。
#[test]
fn b_flag_overrides_bin_dir() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let custom = s._dir.path().join("custom-bin");
    let out = run_install(&s, &["-b", custom.to_str().unwrap()], DARWIN_ARM64);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&custom.join("ask-opencode"));
    assert!(
        !s.home.join(".local/bin/ask-opencode").exists(),
        "-b 后不应再装进默认目录"
    );
    assert_tmp_empty(&s);
}

/// 环境变量 ASK_OPENCODE_BIN_DIR 覆盖 binary 目录。
#[test]
fn ask_opencode_bin_dir_env_overrides_bin_dir() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let custom = s._dir.path().join("env-bin");
    let mut extra: Vec<(&str, &str)> = vec![("ASK_OPENCODE_BIN_DIR", custom.to_str().unwrap())];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &[], &extra);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&custom.join("ask-opencode"));
    assert_tmp_empty(&s);
}

/// sha256 不匹配：非零退出、不落盘、临时文件清理。
#[test]
fn sha256_mismatch_fails_and_leaves_no_residue() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    fs::write(
        &s.fixture_sha,
        format!("{:064x}  ask-opencode-darwin-aarch64-{TAG}.tar.gz\n", 0),
    )
    .unwrap();

    let out = run_install(&s, &[], DARWIN_ARM64);
    assert!(!out.status.success(), "sha256 不匹配应非零退出");
    assert!(stderr_str(&out).contains("校验失败"), "stderr: {}", stderr_str(&out));
    assert!(
        !s.home.join(".local/bin/ask-opencode").exists(),
        "校验失败不应残留二进制"
    );
    assert_tmp_empty(&s);
}

/// uname 映射矩阵：darwin/linux × aarch64/x86_64 的资产命名契约（macOS arm64 → aarch64，
/// x86_64 保持不变）。
#[test]
fn uname_mapping_arm64_normalizes_aarch64_and_keeps_x86_64() {
    let cases = [
        ("Darwin", "arm64", "darwin", "aarch64"),
        ("Darwin", "x86_64", "darwin", "x86_64"),
        ("Linux", "aarch64", "linux", "aarch64"),
        ("Linux", "x86_64", "linux", "x86_64"),
    ];
    for (uname_s, uname_m, want_os, want_arch) in cases {
        let s = setup_sandbox();
        install_fakes(&s);
        make_fixtures(&s);
        let out = run_install(
            &s,
            &[],
            &[("FAKE_UNAME_S", uname_s), ("FAKE_UNAME_M", uname_m)],
        );
        assert!(
            out.status.success(),
            "{uname_s}/{uname_m} stderr: {}",
            stderr_str(&out)
        );
        let log = curl_log(&s);
        assert_eq!(log.len(), 3, "{uname_s}/{uname_m}: {log:?}");
        assert!(
            log[1].ends_with(&format!("ask-opencode-{want_os}-{want_arch}-{TAG}.tar.gz")),
            "{uname_s}/{uname_m} 应请求 {want_os}/{want_arch} 资产，实际: {log:?}"
        );
        assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
        assert_tmp_empty(&s);
    }
}

/// `-h` 打印用法并以 0 退出，不碰网络。
#[test]
fn help_flag_prints_usage_and_exits_zero() {
    let s = setup_sandbox();
    install_fakes(&s);

    let out = run_install(&s, &["-h"], &[]);
    assert!(out.status.success());
    assert!(stdout_str(&out).contains("用法"), "stdout: {}", stdout_str(&out));
    assert!(curl_log(&s).is_empty(), "-h 不应发起任何请求");
}

fn plugin_content(plugin: &Path) -> String {
    fs::read_to_string(plugin).unwrap()
}

/// 有 `$ZSH_CUSTOM`：插件按与二进制同一 tag 从 raw 拉取，落入
/// `$ZSH_CUSTOM/plugins/ask-opencode/ask-opencode.plugin.zsh`，并打印 omz 启用提示。
#[test]
fn zsh_custom_present_installs_plugin_and_prints_omz_hint() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let zsh_custom = s._dir.path().join("oh-my-zsh/custom");

    let mut extra: Vec<(&str, &str)> = vec![("ZSH_CUSTOM", zsh_custom.to_str().unwrap())];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &[], &extra);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let plugin = zsh_custom.join("plugins/ask-opencode/ask-opencode.plugin.zsh");
    assert!(plugin.exists(), "应装入插件目录: {}", plugin.display());
    assert_eq!(plugin_content(&plugin), plugin_content(&s.fixture_plugin));
    assert_tmp_empty(&s);

    let log = curl_log(&s);
    assert_eq!(log.len(), 4, "应有 API/资产/校验/插件 四次请求: {log:?}");
    assert!(
        log[3].contains(&format!("/{TAG}/zsh/ask-opencode.plugin.zsh")),
        "插件应按同一 tag 从 raw 拉取: {log:?}"
    );
    assert!(
        stdout_str(&out).contains("plugins=(...)"),
        "应打印 omz 启用提示: {}",
        stdout_str(&out)
    );
}

/// 无 `$ZSH_CUSTOM`：只装二进制、打印 source 提示，不落插件文件。
#[test]
fn no_zsh_custom_installs_binary_only_and_prints_source_hint() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let out = run_install(&s, &[], DARWIN_ARM64);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    assert_eq!(curl_log(&s).len(), 3, "无 ZSH_CUSTOM 不应拉插件: {:?}", curl_log(&s));
    assert!(
        !s.home.join(".local/bin").join("ask-opencode.plugin.zsh").exists(),
        "插件不应落盘"
    );
    let stdout = stdout_str(&out);
    assert!(stdout.contains("source"), "应打印 source 提示: {stdout}");
    assert!(stdout.contains("$ZSH_CUSTOM"), "应说明未检测到 oh-my-zsh: {stdout}");
}

/// `--plugin-dir` 覆盖插件目录：有 $ZSH_CUSTOM 时也装进覆盖目录而非默认位置。
#[test]
fn plugin_dir_flag_overrides_default_plugin_dir() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let zsh_custom = s._dir.path().join("oh-my-zsh/custom");
    let custom_plugin = s._dir.path().join("my-plugins");

    let mut extra: Vec<(&str, &str)> = vec![("ZSH_CUSTOM", zsh_custom.to_str().unwrap())];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(
        &s,
        &["--plugin-dir", custom_plugin.to_str().unwrap()],
        &extra,
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let overridden = custom_plugin.join("ask-opencode.plugin.zsh");
    assert!(overridden.exists(), "应装入 --plugin-dir: {}", overridden.display());
    assert_eq!(plugin_content(&overridden), plugin_content(&s.fixture_plugin));
    assert!(
        !zsh_custom.join("plugins/ask-opencode/ask-opencode.plugin.zsh").exists(),
        "不应再装进默认 $ZSH_CUSTOM 插件目录"
    );
}

/// `--plugin-dir` 覆盖在纯 zsh（无 $ZSH_CUSTOM）下同样生效：装插件并打印 source 提示。
#[test]
fn plugin_dir_flag_works_without_zsh_custom() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let custom_plugin = s._dir.path().join("standalone-plugins");

    let out = run_install(
        &s,
        &["--plugin-dir", custom_plugin.to_str().unwrap()],
        DARWIN_ARM64,
    );
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let plugin = custom_plugin.join("ask-opencode.plugin.zsh");
    assert!(plugin.exists(), "应装入 --plugin-dir: {}", plugin.display());
    assert_eq!(plugin_content(&plugin), plugin_content(&s.fixture_plugin));
    assert!(
        stdout_str(&out).contains("source"),
        "无 omz 应打印 source 提示: {}",
        stdout_str(&out)
    );
}
