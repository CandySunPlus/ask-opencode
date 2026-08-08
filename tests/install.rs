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

/// stub curl：按 URL 分发 fixture。`-o` 写文件，否则写 stdout；`-w` 时把 HTTP 状态码打到
/// stdout（真实 curl 配合 -o 的行为）。每次请求的 URL 追加到 `$CURL_LOG`，供断言下载顺序与
/// 资产命名契约。`API_STATUS`/`ASSET_STATUS`/`PLUGIN_STATUS`/`AGENT_STATUS` 可覆盖对应请求的
/// 状态码，模拟 API 不可用、平台无资产、插件脚本下载失败等降级路径。
const CURL_STUB: &str = r#"
out=""
url=""
print_code=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) print_code=1; shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
echo "$url" >> "$CURL_LOG"
payload=""
status=200
case "$url" in
  */releases/latest) payload="$RELEASES_JSON"; status="${API_STATUS:-200}" ;;
  *install.sh) payload="$FAKE_INSTALL_SCRIPT" ;;
  *ask-opencode.plugin.zsh) payload="$FIXTURE_PLUGIN"; status="${PLUGIN_STATUS:-200}" ;;
  *cmd-gen.md) payload="$FIXTURE_AGENT"; status="${AGENT_STATUS:-200}" ;;
  *.tar.gz) payload="$FIXTURE_ASSET"; status="${ASSET_STATUS:-200}" ;;
  *.sha256) payload="$FIXTURE_SHA" ;;
  *) echo "unexpected url: $url" >&2; exit 1 ;;
esac
if [ "$status" = 200 ] && [ -n "$payload" ]; then
  if [ -n "$out" ]; then cat "$payload" > "$out"; else cat "$payload"; fi
elif [ -n "$out" ]; then
  : > "$out"
fi
if [ "$print_code" = 1 ]; then printf '%s\n' "$status"; fi
[ "$status" = 200 ] || exit 22
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
    fixture_agent: PathBuf,
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
        fixture_agent: root.join("fixture.cmd-gen.md"),
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
    fs::write(
        &s.fixture_agent,
        "---\ndescription: fake cmd-gen agent\n---\nfake agent body\n",
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
        ("FIXTURE_AGENT".into(), s.fixture_agent.display().to_string()),
        ("FAKE_INSTALL_SCRIPT".into(), install_sh().display().to_string()),
        // 隔离 $ZSH（curl|sh 下 zsh 的 omz 变量只导出 $ZSH，不导出 $ZSH_CUSTOM）：
        // 默认空串当作无 omz，需回退路径的用例在 extra 里覆盖。
        ("ZSH".into(), "".into()),
        // stub curl 的降级路径状态码默认全 200，需要故障路径的用例在 extra 里覆盖。
        ("API_STATUS".into(), "200".into()),
        ("ASSET_STATUS".into(), "200".into()),
        ("PLUGIN_STATUS".into(), "200".into()),
        ("AGENT_STATUS".into(), "200".into()),
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

    let agent = s.home.join(".config/opencode/agents/cmd-gen.md");
    assert!(agent.exists(), "应装入 agent: {}", agent.display());
    assert_eq!(
        fs::read_to_string(&agent).unwrap(),
        fs::read_to_string(&s.fixture_agent).unwrap(),
        "agent 内容应与 fixture 一致"
    );

    let log = curl_log(&s);
    assert_eq!(log.len(), 4, "应有 API/资产/校验/agent 四次请求: {log:?}");
    assert!(log[0].ends_with("/releases/latest"), "{log:?}");
    assert!(
        log[1].ends_with(&format!("ask-opencode-darwin-aarch64-{TAG}.tar.gz")),
        "资产 URL 应带归一化的 aarch64: {log:?}"
    );
    assert!(log[2].ends_with(".sha256"), "{log:?}");
    assert!(
        log[3].contains(&format!("/{TAG}/.opencode/agents/cmd-gen.md")),
        "agent 应按同一 tag 从 raw 拉取: {log:?}"
    );
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
        assert_eq!(log.len(), 4, "{uname_s}/{uname_m}: {log:?}");
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
    assert_eq!(log.len(), 5, "应有 API/资产/校验/插件/agent 五次请求: {log:?}");
    assert!(
        log[3].contains(&format!("/{TAG}/zsh/ask-opencode.plugin.zsh")),
        "插件应按同一 tag 从 raw 拉取: {log:?}"
    );
    assert!(
        log[4].contains(&format!("/{TAG}/.opencode/agents/cmd-gen.md")),
        "agent 应按同一 tag 从 raw 拉取: {log:?}"
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
    let log = curl_log(&s);
    assert_eq!(log.len(), 4, "无 ZSH_CUSTOM 应 API/资产/校验/agent，不拉插件: {log:?}");
    assert!(
        !log.iter().any(|u| u.contains("ask-opencode.plugin.zsh")),
        "无 omz 不应拉插件: {log:?}"
    );
    assert!(
        !s.home.join(".local/bin").join("ask-opencode.plugin.zsh").exists(),
        "插件不应落盘"
    );
    let stdout = stdout_str(&out);
    assert!(stdout.contains("source"), "应打印 source 提示: {stdout}");
    assert!(stdout.contains("$ZSH_CUSTOM"), "应说明未检测到 oh-my-zsh: {stdout}");
}

/// 无 `$ZSH_CUSTOM` 但 `$ZSH/custom` 存在：curl|sh 场景——zsh 只导出 `$ZSH` 不导出
/// `$ZSH_CUSTOM`，应回退到 omz 惯例目录 `$ZSH/custom/plugins/ask-opencode/` 装插件。
#[test]
fn no_zsh_custom_but_zsh_dir_falls_back_to_omz_convention() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let zsh_root = s._dir.path().join("oh-my-zsh");
    let zsh_custom = zsh_root.join("custom");
    fs::create_dir_all(&zsh_custom).unwrap();

    let mut extra: Vec<(&str, &str)> = vec![("ZSH", zsh_root.to_str().unwrap())];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &[], &extra);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let plugin = zsh_custom.join("plugins/ask-opencode/ask-opencode.plugin.zsh");
    assert!(plugin.exists(), "应装入 $ZSH/custom 惯例目录: {}", plugin.display());
    assert_eq!(plugin_content(&plugin), plugin_content(&s.fixture_plugin));
    assert_tmp_empty(&s);

    let log = curl_log(&s);
    assert_eq!(log.len(), 5, "应有 API/资产/校验/agent/插件 五次请求: {log:?}");
    assert!(
        stdout_str(&out).contains("plugins=(...)"),
        "回退到惯例目录应打印 omz 启用提示: {}",
        stdout_str(&out)
    );
}

/// `$ZSH` 也未导出（curl|sh 从 zsh 起的子进程可能两个都拿不到）：只剩 omz 标准安装目录
/// `$HOME/.oh-my-zsh/custom` 真实存在这一条信号，也应认定装了 omz 并装进惯例目录。
#[test]
fn home_oh_my_zsh_dir_alone_detects_omz() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let zsh_custom = s.home.join(".oh-my-zsh/custom");
    fs::create_dir_all(&zsh_custom).unwrap();

    let out = run_install(&s, &[], DARWIN_ARM64);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let plugin = zsh_custom.join("plugins/ask-opencode/ask-opencode.plugin.zsh");
    assert!(plugin.exists(), "应装入 ~/.oh-my-zsh/custom 惯例目录: {}", plugin.display());
    assert_eq!(plugin_content(&plugin), plugin_content(&s.fixture_plugin));
    assert!(
        stdout_str(&out).contains("plugins=(...)"),
        "应打印 omz 启用提示: {}",
        stdout_str(&out)
    );
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
    // 插件没落在 omz 惯例目录，plugins 数组加载不到它，提示必须是 source 而不是 omz 启用。
    assert!(
        stdout_str(&out).contains("source"),
        "插件被 --plugin-dir 移走应打印 source 提示: {}",
        stdout_str(&out)
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

/// `-V <版本>` 指定版本：资产与插件都按该 tag 拉取，且不再请求 releases/latest。
#[test]
fn v_flag_pins_version_and_skips_latest_api() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let out = run_install(&s, &["-V", "v9.9.9"], DARWIN_ARM64);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    assert!(
        stdout_str(&out).contains("v9.9.9"),
        "应打印指定版本: {}",
        stdout_str(&out)
    );
    let log = curl_log(&s);
    assert_eq!(log.len(), 3, "-V 指定版本应跳过 releases/latest（资产/校验/agent）: {log:?}");
    assert!(
        log[0].ends_with("ask-opencode-darwin-aarch64-v9.9.9.tar.gz"),
        "资产 URL 应带指定版本: {log:?}"
    );
    assert!(log[1].ends_with(".sha256"), "{log:?}");
    assert!(
        log[2].contains("/v9.9.9/.opencode/agents/cmd-gen.md"),
        "agent 应按同一指定 tag 拉取: {log:?}"
    );
    assert_tmp_empty(&s);
}

/// `ASK_OPENCODE_VERSION` 环境变量指定版本，与 `-V` 行为一致。
#[test]
fn ask_opencode_version_env_pins_version() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let mut extra: Vec<(&str, &str)> = vec![("ASK_OPENCODE_VERSION", "v9.9.9")];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &[], &extra);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let log = curl_log(&s);
    assert_eq!(log.len(), 3, "环境变量指定版本应跳过 releases/latest: {log:?}");
    assert!(
        log[0].ends_with("ask-opencode-darwin-aarch64-v9.9.9.tar.gz"),
        "资产 URL 应带指定版本: {log:?}"
    );
    assert!(log[2].contains("/v9.9.9/.opencode/agents/cmd-gen.md"), "{log:?}");
    assert_tmp_empty(&s);
}

/// `-V` 与 `ASK_OPENCODE_VERSION` 同时给出：参数优先于环境变量。
#[test]
fn v_flag_overrides_ask_opencode_version_env() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let mut extra: Vec<(&str, &str)> = vec![("ASK_OPENCODE_VERSION", "v1.1.1")];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &["-V", "v9.9.9"], &extra);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let log = curl_log(&s);
    assert_eq!(log.len(), 3, "指定版本应跳过 releases/latest: {log:?}");
    assert!(
        log[0].ends_with("ask-opencode-darwin-aarch64-v9.9.9.tar.gz"),
        "-V 应优先于环境变量: {log:?}"
    );
    assert!(log[2].contains("/v9.9.9/.opencode/agents/cmd-gen.md"), "{log:?}");
    assert_tmp_empty(&s);
}

/// `-V` 指定版本下插件脚本与 agent 也按同一 tag 从 raw 拉取（同 tag 契约不因指定版本而断）。
#[test]
fn v_flag_pins_plugin_tag_too() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let zsh_custom = s._dir.path().join("oh-my-zsh/custom");

    let mut extra: Vec<(&str, &str)> = vec![("ZSH_CUSTOM", zsh_custom.to_str().unwrap())];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &["-V", "v9.9.9"], &extra);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let plugin = zsh_custom.join("plugins/ask-opencode/ask-opencode.plugin.zsh");
    assert!(plugin.exists(), "应装入插件: {}", plugin.display());
    let log = curl_log(&s);
    assert_eq!(log.len(), 4, "应为 资产/校验/agent/插件 四次、无 API: {log:?}");
    assert!(
        log[0].ends_with("ask-opencode-darwin-aarch64-v9.9.9.tar.gz"),
        "{log:?}"
    );
    assert!(
        log[2].contains("/v9.9.9/zsh/ask-opencode.plugin.zsh"),
        "插件应按同一指定 tag 拉取: {log:?}"
    );
    assert!(
        log[3].contains("/v9.9.9/.opencode/agents/cmd-gen.md"),
        "agent 应按同一指定 tag 拉取: {log:?}"
    );
    assert_tmp_empty(&s);
}

/// 重复安装幂等覆盖：不报「已安装」，旧二进制被新内容覆盖。
#[test]
fn repeat_install_overwrites_idempotently() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let first = run_install(&s, &[], DARWIN_ARM64);
    assert!(first.status.success(), "stderr: {}", stderr_str(&first));
    let bin = s.home.join(".local/bin/ask-opencode");
    assert_installed_bin(&bin);

    // 换一份内容重新打包 fixture，重跑应覆盖旧二进制而不是报「已安装」。
    let content = s._dir.path().join("fixture-content");
    fs::write(content.join("ask-opencode"), "#!/bin/sh\necho v2-ask-opencode\n").unwrap();
    let status = Command::new("tar")
        .args(["-czf"])
        .arg(&s.fixture_asset)
        .arg("-C")
        .arg(&content)
        .arg("ask-opencode")
        .status()
        .unwrap();
    assert!(status.success(), "重新打包 fixture 失败");
    fs::write(
        &s.fixture_sha,
        format!(
            "{}  ask-opencode-darwin-aarch64-{TAG}.tar.gz\n",
            sha256_of(&s.fixture_asset)
        ),
    )
    .unwrap();

    let second = run_install(&s, &[], DARWIN_ARM64);
    assert!(second.status.success(), "stderr: {}", stderr_str(&second));
    let out = Command::new(&bin).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "v2-ask-opencode",
        "旧二进制应被新内容覆盖"
    );
    assert_eq!(curl_log(&s).len(), 8, "两次安装都应走完整链路（各 4 次请求）");
    assert_tmp_empty(&s);
}

/// `--uninstall`：删二进制与插件目录、不碰网络、幂等（目标不存在也成功退出）。
#[test]
fn uninstall_removes_binary_and_plugin_and_is_idempotent() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let zsh_custom = s._dir.path().join("oh-my-zsh/custom");
    let mut extra: Vec<(&str, &str)> = vec![("ZSH_CUSTOM", zsh_custom.to_str().unwrap())];
    extra.extend_from_slice(DARWIN_ARM64);

    let install = run_install(&s, &[], &extra);
    assert!(install.status.success(), "stderr: {}", stderr_str(&install));
    let bin = s.home.join(".local/bin/ask-opencode");
    let plugin_dir = zsh_custom.join("plugins/ask-opencode");
    let agent_file = s.home.join(".config/opencode/agents/cmd-gen.md");
    assert!(bin.exists(), "安装应落盘二进制");
    assert!(plugin_dir.join("ask-opencode.plugin.zsh").exists(), "安装应落盘插件");
    assert!(agent_file.exists(), "安装应落盘 agent");
    let log_len_after_install = curl_log(&s).len();

    let uninstall = run_install(&s, &["--uninstall"], &extra);
    assert!(uninstall.status.success(), "stderr: {}", stderr_str(&uninstall));
    assert!(!bin.exists(), "应删除二进制");
    assert!(!plugin_dir.exists(), "应删除插件目录");
    assert!(!agent_file.exists(), "应删除 agent 文件");
    assert!(stdout_str(&uninstall).contains("已卸载"), "stdout: {}", stdout_str(&uninstall));
    assert_eq!(
        curl_log(&s).len(),
        log_len_after_install,
        "--uninstall 不应发起任何请求"
    );

    let again = run_install(&s, &["--uninstall"], &extra);
    assert!(again.status.success(), "目标不存在也应成功退出: {}", stderr_str(&again));
    assert!(!bin.exists() && !plugin_dir.exists() && !agent_file.exists(), "再次卸载后仍无残留");
}

/// releases/latest API 不可用：报错并提示手动传版本，非零退出、不残留。
#[test]
fn latest_api_failure_hints_manual_version() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let mut extra: Vec<(&str, &str)> = vec![("API_STATUS", "500")];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &[], &extra);
    assert!(!out.status.success(), "API 失败应非零退出");
    let err = stderr_str(&out);
    assert!(err.contains("无法获取最新版本"), "stderr: {err}");
    assert!(err.contains("-V"), "应提示手动传版本: {err}");
    assert!(err.contains("ASK_OPENCODE_VERSION"), "应提示环境变量: {err}");
    assert!(
        !s.home.join(".local/bin/ask-opencode").exists(),
        "API 失败不应残留二进制"
    );
}

/// 平台无匹配资产（macOS x86_64 无 runner）：打印本地构建提示、非零退出、不装错架构。
#[test]
fn platform_without_asset_prints_local_build_hint() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let mut extra: Vec<(&str, &str)> = vec![("ASSET_STATUS", "404")];
    extra.extend_from_slice(&[("FAKE_UNAME_S", "Darwin"), ("FAKE_UNAME_M", "x86_64")]);
    let out = run_install(&s, &[], &extra);
    assert!(!out.status.success(), "无匹配资产应非零退出");
    let err = stderr_str(&out);
    assert!(err.contains("暂无发布资产"), "stderr: {err}");
    assert!(err.contains("cargo build --release"), "应提示本地构建: {err}");
    assert!(
        !s.home.join(".local/bin/ask-opencode").exists(),
        "不应装错架构的二进制"
    );
    assert_eq!(
        curl_log(&s)[1],
        format!("https://github.com/CandySunPlus/ask-opencode/releases/download/v0.1.0/ask-opencode-darwin-x86_64-v0.1.0.tar.gz"),
        "应按 x86_64 命名请求资产"
    );
    assert_tmp_empty(&s);
}

/// 显式 `-V` 版本号不存在（资产 404）：提示先核对版本，再给本地构建兜底，不装错东西。
#[test]
fn explicit_version_404_hints_version_check() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let mut extra: Vec<(&str, &str)> = vec![("ASSET_STATUS", "404")];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &["-V", "v9.9.9"], &extra);
    assert!(!out.status.success(), "无匹配资产应非零退出");
    let err = stderr_str(&out);
    assert!(err.contains("请先确认版本 v9.9.9 正确"), "应提示核对版本: {err}");
    assert!(err.contains("cargo build --release"), "仍应给本地构建提示: {err}");
    assert!(
        !s.home.join(".local/bin/ask-opencode").exists(),
        "不应装错东西"
    );
    assert_tmp_empty(&s);
}

/// 插件脚本下载失败：整体失败、二进制不落盘、插件目录不创建（T3 引入的拉取路径）。
#[test]
fn plugin_download_failure_leaves_no_binary() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let zsh_custom = s._dir.path().join("oh-my-zsh/custom");

    let mut extra: Vec<(&str, &str)> = vec![
        ("ZSH_CUSTOM", zsh_custom.to_str().unwrap()),
        ("PLUGIN_STATUS", "404"),
    ];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &[], &extra);
    assert!(!out.status.success(), "插件下载失败应非零退出");
    assert!(stderr_str(&out).contains("插件脚本下载失败"), "stderr: {}", stderr_str(&out));
    assert!(
        !s.home.join(".local/bin/ask-opencode").exists(),
        "插件失败不应残留二进制"
    );
    assert!(
        !zsh_custom.join("plugins/ask-opencode").exists(),
        "插件失败不应创建插件目录"
    );
    assert_tmp_empty(&s);
}

/// cmd-gen agent 下载失败：整体失败、二进制不落盘、agent 目录不创建。
#[test]
fn agent_download_failure_leaves_no_binary() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);

    let mut extra: Vec<(&str, &str)> = vec![("AGENT_STATUS", "404")];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &[], &extra);
    assert!(!out.status.success(), "agent 下载失败应非零退出");
    assert!(stderr_str(&out).contains("cmd-gen agent 下载失败"), "stderr: {}", stderr_str(&out));
    assert!(
        !s.home.join(".local/bin/ask-opencode").exists(),
        "agent 失败不应残留二进制"
    );
    assert!(
        !s.home.join(".config/opencode/agents").exists(),
        "agent 失败不应创建 agents 目录"
    );
    assert_tmp_empty(&s);
}

/// `--agent-dir` 覆盖 agent 目录。
#[test]
fn agent_dir_flag_overrides_default_agent_dir() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let custom = s._dir.path().join("custom-agents");

    let out = run_install(&s, &["--agent-dir", custom.to_str().unwrap()], DARWIN_ARM64);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let agent = custom.join("cmd-gen.md");
    assert!(agent.exists(), "应装入 --agent-dir: {}", agent.display());
    assert_eq!(
        fs::read_to_string(&agent).unwrap(),
        fs::read_to_string(&s.fixture_agent).unwrap(),
        "agent 内容应与 fixture 一致"
    );
    assert!(
        !s.home.join(".config/opencode/agents/cmd-gen.md").exists(),
        "--agent-dir 后不应再装进默认目录"
    );
    assert_tmp_empty(&s);
}

/// `ASK_OPENCODE_AGENT_DIR` 环境变量覆盖 agent 目录。
#[test]
fn agent_dir_env_overrides_default_agent_dir() {
    let s = setup_sandbox();
    install_fakes(&s);
    make_fixtures(&s);
    let custom = s._dir.path().join("env-agents");

    let mut extra: Vec<(&str, &str)> = vec![("ASK_OPENCODE_AGENT_DIR", custom.to_str().unwrap())];
    extra.extend_from_slice(DARWIN_ARM64);
    let out = run_install(&s, &[], &extra);
    assert!(out.status.success(), "stderr: {}", stderr_str(&out));

    assert_installed_bin(&s.home.join(".local/bin/ask-opencode"));
    let agent = custom.join("cmd-gen.md");
    assert!(agent.exists(), "应装入 env 指定目录: {}", agent.display());
    assert_eq!(
        fs::read_to_string(&agent).unwrap(),
        fs::read_to_string(&s.fixture_agent).unwrap(),
        "agent 内容应与 fixture 一致"
    );
    assert_tmp_empty(&s);
}
