use crate::config;
use crate::opencode::{OPENCODE, OpenCodeError};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// 状态文件里一个后端的分区：字段由该后端自己解释（ADR-0009）。
pub type Partition = Map<String, Value>;

/// 状态文件 `server.json` 顶层按后端分区，`{后端名 → 分区}`（ADR-0009）。
type StateFile = Map<String, Value>;

/// 旧格式顶层的字段，全属 opencode。
const LEGACY_FIELDS: [&str; 3] = ["url", "pid", "session_id"];

/// 解析状态文件路径。
fn state_path() -> Result<PathBuf, OpenCodeError> {
    config::state_path().ok_or_else(|| OpenCodeError {
        message: "无法确定常驻服务状态文件路径".to_string(),
    })
}

/// 解析状态文件路径并确保父目录存在；serve 日志也放在同一目录。
pub fn prepare_state_path() -> Result<PathBuf, OpenCodeError> {
    let state_path = state_path()?;
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| OpenCodeError {
            message: format!("无法创建配置目录 {}: {err}", parent.display()),
        })?;
    }
    Ok(state_path)
}

/// 读整个状态文件；缺失或解析不了返回 None。
fn load(path: &Path) -> Option<StateFile> {
    let text = std::fs::read_to_string(path).ok()?;
    let state: StateFile = serde_json::from_str(&text).ok()?;
    Some(migrate_legacy(state))
}

/// 旧格式就地读作 opencode 分区，下次写入即转成新格式（ADR-0009）：有顶层旧字段、又没有
/// opencode 分区的，就是旧格式。
fn migrate_legacy(mut state: StateFile) -> StateFile {
    if state.contains_key(OPENCODE) {
        return state;
    }
    let legacy: Partition = LEGACY_FIELDS
        .iter()
        .filter_map(|key| state.remove_entry(*key))
        .collect();
    if !legacy.is_empty() {
        state.insert(OPENCODE.to_string(), Value::Object(legacy));
    }
    state
}

fn save(path: &Path, state: &StateFile) -> Result<(), OpenCodeError> {
    let text = serde_json::to_string(state).map_err(|err| OpenCodeError {
        message: format!("无法序列化状态文件: {err}"),
    })?;
    std::fs::write(path, text).map_err(|err| OpenCodeError {
        message: format!("无法写入状态文件 {}: {err}", path.display()),
    })
}

/// 读一个后端的分区；文件或分区缺失返回空分区。
pub fn load_partition(backend: &str) -> Partition {
    let Ok(path) = state_path() else {
        return Partition::new();
    };
    take_partition(&mut load(&path).unwrap_or_default(), backend)
}

/// 从状态里取出一个分区；缺失或不是对象都当空分区。
fn take_partition(state: &mut StateFile, backend: &str) -> Partition {
    match state.remove(backend) {
        Some(Value::Object(partition)) => partition,
        _ => Partition::new(),
    }
}

/// 读改写一个后端的分区，其他分区原样保留；改完为空的分区整个去掉。
pub fn update_partition(
    backend: &str,
    edit: impl FnOnce(&mut Partition),
) -> Result<(), OpenCodeError> {
    let path = prepare_state_path()?;
    let mut state = load(&path).unwrap_or_default();
    write_partition(&path, &mut state, backend, edit)
}

fn write_partition(
    path: &Path,
    state: &mut StateFile,
    backend: &str,
    edit: impl FnOnce(&mut Partition),
) -> Result<(), OpenCodeError> {
    let mut partition = take_partition(state, backend);
    edit(&mut partition);
    if !partition.is_empty() {
        state.insert(backend.to_string(), Value::Object(partition));
    }
    save(path, state)
}

/// 读后端分区里的常驻会话 id；没落盘返回 None（ADR-0007）。
pub fn load_session_id(backend: &str) -> Option<String> {
    load_partition(backend)
        .get("session_id")?
        .as_str()
        .map(str::to_string)
}

/// 把新建会话的 id 落进后端分区，分区里其余字段不动（ADR-0007）。
pub fn save_session_id(backend: &str, session_id: &str) -> Result<(), OpenCodeError> {
    update_partition(backend, |partition| {
        partition.insert("session_id".to_string(), session_id.into());
    })
}

/// 清掉后端分区里的会话 id，常驻服务信息与其他分区不动；文件缺失或无 id 时同样成功
/// （幂等，`reset-session` 子命令，ADR-0007）。文件缺失时不创建。
pub fn clear_session_id(backend: &str) -> Result<(), OpenCodeError> {
    let path = state_path()?;
    let Some(mut state) = load(&path) else {
        return Ok(());
    };
    write_partition(&path, &mut state, backend, |partition| {
        partition.remove("session_id");
    })
}
