use std::path::{Path, PathBuf};
use std::process::Command;

fn run_zsh(script: &str, envs: &[(&str, &Path)]) -> std::process::Output {
    if !Path::new("/bin/zsh").exists() {
        return Command::new("true").output().unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let script_path = dir.path().join("case.zsh");
    std::fs::write(&script_path, script).unwrap();
    let mut cmd = Command::new("/bin/zsh");
    cmd.arg("-f").arg(&script_path);
    for (name, value) in envs {
        cmd.env(name, value);
    }
    cmd.output().unwrap()
}

fn plugin_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("zsh")
        .join("ask-opencode.plugin.zsh")
}

fn assert_zsh_success(out: std::process::Output) {
    assert!(
        out.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write_fake_ask_opencode(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("ask-opencode");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }
    path
}

fn harness(body: &str) -> String {
    format!(
        r#"
zle() {{
  case "$1" in
    -N) return 0 ;;
    -M) ZLE_MESSAGE="$2"; return 0 ;;
    -R|-I) return 0 ;;
    _ask_opencode_fill) _ask_opencode_fill; return $? ;;
    _ask_opencode_poll) _ask_opencode_poll; return $? ;;
    *) ZLE_CALLED="$1"; return 0 ;;
  esac
}}
bindkey() {{ return 1 }}
sched() {{ SCHED_CALLED="$*"; return 0 }}

wait_ready() {{
  local i
  for i in {{1..100}}; do
    [[ -r "$_ask_opencode_status" ]] && return 0
    sleep 0.02
  done
  print -r -- "timeout waiting for ask-opencode" >&2
  return 1
}}

_ask_opencode_cmd="$ASK_OPENCODE_FAKE"
source "$ASK_OPENCODE_PLUGIN"

{body}
"#
    )
}

#[test]
fn zsh_widget_generates_selects_and_fills_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let request_log = dir.path().join("request.log");
    let select_log = dir.path().join("select.log");
    let select_file_log = dir.path().join("select-file.log");
    let fake = write_fake_ask_opencode(
        dir.path(),
        r#"
case "$1" in
  generate)
    printf '%s\n' "$2" > "$REQUEST_LOG"
    printf '["echo ignored","echo filled"]\n'
    ;;
  select)
    printf '%s\n' "$*" > "$SELECT_LOG"
    cat "$3" > "$SELECT_FILE_LOG"
    printf 'echo filled\n'
    ;;
  *) exit 99 ;;
esac
"#,
    );
    let plugin = plugin_path();
    let script = harness(
        r##"
BUFFER="# list files"
CURSOR=${#BUFFER}
_ask_opencode_expand
[[ "$BUFFER" == "# list files" ]] || exit 10
wait_ready
_ask_opencode_poll
[[ "$(cat "$REQUEST_LOG")" == "list files" ]] || exit 11
[[ "$(cat "$SELECT_LOG")" == *"--file"* ]] || exit 12
[[ "$(cat "$SELECT_FILE_LOG")" == '["echo ignored","echo filled"]' ]] || exit 13
[[ "$BUFFER" == "echo filled" ]] || exit 14
[[ "$CURSOR" -eq ${#BUFFER} ]] || exit 15
"##,
    );
    let out = run_zsh(
        &script,
        &[
            ("ASK_OPENCODE_PLUGIN", &plugin),
            ("ASK_OPENCODE_FAKE", &fake),
            ("REQUEST_LOG", &request_log),
            ("SELECT_LOG", &select_log),
            ("SELECT_FILE_LOG", &select_file_log),
        ],
    );
    assert_zsh_success(out);
}

#[test]
fn zsh_widget_ignores_repeated_tab_while_generating() {
    let dir = tempfile::tempdir().unwrap();
    let calls = dir.path().join("calls.log");
    let fake = write_fake_ask_opencode(
        dir.path(),
        r#"
case "$1" in
  generate)
    printf 'generate\n' >> "$CALLS_LOG"
    sleep 0.1
    printf '["echo later"]\n'
    ;;
  select)
    printf 'echo later\n'
    ;;
  *) exit 99 ;;
esac
"#,
    );
    let plugin = plugin_path();
    let script = harness(
        r##"
BUFFER="# show status"
_ask_opencode_expand
_ask_opencode_expand
[[ "$ZLE_MESSAGE" == 正在生成* ]] || exit 20
wait_ready
[[ "$(wc -l < "$CALLS_LOG" | tr -d ' ')" == "1" ]] || exit 21
_ask_opencode_poll
"##,
    );
    let out = run_zsh(
        &script,
        &[
            ("ASK_OPENCODE_PLUGIN", &plugin),
            ("ASK_OPENCODE_FAKE", &fake),
            ("CALLS_LOG", &calls),
        ],
    );
    assert_zsh_success(out);
}

#[test]
fn zsh_widget_keeps_request_on_generate_failure() {
    let dir = tempfile::tempdir().unwrap();
    let fake = write_fake_ask_opencode(
        dir.path(),
        r#"
case "$1" in
  generate)
    printf 'boom\n' >&2
    exit 42
    ;;
  select)
    exit 99
    ;;
  *) exit 99 ;;
esac
"#,
    );
    let plugin = plugin_path();
    let script = harness(
        r##"
BUFFER="# broken request"
_ask_opencode_expand
wait_ready
_ask_opencode_poll
[[ "$BUFFER" == "# broken request" ]] || exit 30
[[ "$ZLE_MESSAGE" == *boom* ]] || exit 31
"##,
    );
    let out = run_zsh(
        &script,
        &[
            ("ASK_OPENCODE_PLUGIN", &plugin),
            ("ASK_OPENCODE_FAKE", &fake),
        ],
    );
    assert_zsh_success(out);
}

#[test]
fn zsh_sched_event_reschedules_when_zle_is_inactive() {
    let dir = tempfile::tempdir().unwrap();
    let fake = write_fake_ask_opencode(
        dir.path(),
        r#"
case "$1" in
  generate)
    sleep 0.1
    printf '["echo later"]\n'
    ;;
  select)
    printf 'echo later\n'
    ;;
  *) exit 99 ;;
esac
"#,
    );
    let plugin = plugin_path();
    let script = format!(
        r##"
zle() {{
  case "$1" in
    -N|-M|-R|-I) return 0 ;;
    *) return 1 ;;
  esac
}}
bindkey() {{ return 1 }}
sched() {{
  print -r -- "$*" >> "$SCHED_LOG"
  return 0
}}

_ask_opencode_cmd="$ASK_OPENCODE_FAKE"
source "$ASK_OPENCODE_PLUGIN"
BUFFER="# slow request"
_ask_opencode_expand
_ask_opencode_poll_event
[[ "$(wc -l < "$SCHED_LOG" | tr -d ' ')" == "2" ]] || exit 40
"##
    );
    let sched_log = dir.path().join("sched.log");
    let out = run_zsh(
        &script,
        &[
            ("ASK_OPENCODE_PLUGIN", &plugin),
            ("ASK_OPENCODE_FAKE", &fake),
            ("SCHED_LOG", &sched_log),
        ],
    );
    assert_zsh_success(out);
}
