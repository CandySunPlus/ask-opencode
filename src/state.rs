use crate::config;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// 读写状态文件失败时带给人看的错误。
#[derive(Debug)]
pub struct StateError {
    pub message: String,
}

/// 状态文件里一个后端的分区：字段由该后端自己解释（ADR-0009）。
pub type Partition = Map<String, Value>;

/// 状态文件 `server.json` 顶层按后端分区，`{后端名 → 分区}`（ADR-0009）。
type StateFile = Map<String, Value>;

/// 旧格式顶层的字段，全属 opencode。
const LEGACY_FIELDS: [&str; 3] = ["url", "pid", "session_id"];
/// 旧格式迁入的分区名。
const LEGACY_BACKEND: &str = "opencode";

/// 磁盘上状态文件的样子。
enum Stored {
    Missing,
    /// 文件读到了但解析不了：损坏或顶层不是对象。
    Unreadable,
    /// 文件本身读失败（没权限等），内容可能完好，不能当损坏处理。
    Failed(std::io::Error),
    Parsed(StateFile),
}

/// 解析状态文件路径。
fn state_path() -> Result<PathBuf, StateError> {
    config::state_path().ok_or_else(|| StateError {
        message: "无法确定常驻服务状态文件路径".to_string(),
    })
}

/// 解析状态文件路径并确保父目录存在；serve 日志也放在同一目录。
pub fn prepare_state_path() -> Result<PathBuf, StateError> {
    let state_path = state_path()?;
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| StateError {
            message: format!("无法创建配置目录 {}: {err}", parent.display()),
        })?;
    }
    Ok(state_path)
}

fn read(path: &Path) -> Stored {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Stored::Missing,
        Err(err) => return Stored::Failed(err),
    };
    match serde_json::from_str::<StateFile>(&text) {
        Ok(state) => Stored::Parsed(migrate_legacy(state)),
        Err(_) => Stored::Unreadable,
    }
}

/// 读整个状态文件；缺失或读不出返回 None。
fn load(path: &Path) -> Option<StateFile> {
    match read(path) {
        Stored::Parsed(state) => Some(state),
        Stored::Missing | Stored::Unreadable | Stored::Failed(_) => None,
    }
}

/// 读出来准备改写的状态：读不出的文件先挪到 `<文件名>.corrupt` 再按空状态重写，免得其他
/// 后端的分区被悄悄覆盖掉；挪不动也要在 stderr 说一声。文件读失败则不写。
fn load_for_write(path: &Path) -> Result<StateFile, StateError> {
    match read(path) {
        Stored::Parsed(state) => return Ok(state),
        Stored::Missing => return Ok(StateFile::new()),
        Stored::Failed(err) => {
            return Err(StateError {
                message: format!("无法读取状态文件 {}: {err}", path.display()),
            });
        }
        Stored::Unreadable => {}
    }
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".corrupt");
    let backup = PathBuf::from(backup);
    match std::fs::rename(path, &backup) {
        Ok(()) => eprintln!(
            "state: 状态文件 {} 读不出，已挪到 {} 后重写，其他后端的会话需重新建立",
            path.display(),
            backup.display()
        ),
        Err(err) => eprintln!(
            "state: 状态文件 {} 读不出、也没能备份（{err}），将被覆盖，其他后端的会话会丢失",
            path.display()
        ),
    }
    Ok(StateFile::new())
}

/// 旧格式就地读作 opencode 分区，下次写入即转成新格式（ADR-0009）：有顶层旧字段、又没有
/// opencode 分区的，就是旧格式。
fn migrate_legacy(mut state: StateFile) -> StateFile {
    if state.contains_key(LEGACY_BACKEND) {
        return state;
    }
    let legacy: Partition = LEGACY_FIELDS
        .iter()
        .filter_map(|key| state.remove_entry(*key))
        .collect();
    if !legacy.is_empty() {
        state.insert(LEGACY_BACKEND.to_string(), Value::Object(legacy));
    }
    state
}

fn save(path: &Path, state: &StateFile) -> Result<(), StateError> {
    let text = serde_json::to_string(state).map_err(|err| StateError {
        message: format!("无法序列化状态文件: {err}"),
    })?;
    std::fs::write(path, text).map_err(|err| StateError {
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
) -> Result<(), StateError> {
    let path = prepare_state_path()?;
    let mut state = load_for_write(&path)?;
    write_partition(&path, &mut state, backend, edit)
}

fn write_partition(
    path: &Path,
    state: &mut StateFile,
    backend: &str,
    edit: impl FnOnce(&mut Partition),
) -> Result<(), StateError> {
    let mut partition = take_partition(state, backend);
    edit(&mut partition);
    if !partition.is_empty() {
        state.insert(backend.to_string(), Value::Object(partition));
    }
    save(path, state)
}

/// 读后端分区里 `key` 处的常驻会话 id；没落盘返回 None（ADR-0007）。`key` 是会话 id 在分区里
/// 的键路径，由后端决定（`Backend::session_key`）。
pub fn load_session_id(backend: &str, key: &[String]) -> Option<String> {
    let (last, parents) = key.split_last()?;
    let mut node = &load_partition(backend);
    for name in parents {
        node = node.get(name)?.as_object()?;
    }
    node.get(last)?.as_str().map(str::to_string)
}

/// 把新建会话的 id 落进后端分区的 `key` 处，分区里其余字段不动（ADR-0007）。
pub fn save_session_id(backend: &str, key: &[String], session_id: &str) -> Result<(), StateError> {
    let Some((last, parents)) = key.split_last() else {
        return Ok(());
    };
    update_partition(backend, |partition| {
        let mut node = partition;
        for name in parents {
            let child = node
                .entry(name.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if !child.is_object() {
                *child = Value::Object(Map::new());
            }
            node = child.as_object_mut().expect("刚确保是对象");
        }
        node.insert(last.clone(), session_id.into());
    })
}

/// 清掉后端分区里 `key` 处的会话 id，清空的上层对象一并去掉，常驻服务信息与其他分区不动；
/// 文件缺失或无 id 时同样成功（幂等，`reset-session` 子命令，ADR-0007）。文件缺失时不创建。
pub fn clear_session_id(backend: &str, key: &[String]) -> Result<(), StateError> {
    let path = state_path()?;
    let Some(mut state) = load(&path) else {
        return Ok(());
    };
    write_partition(&path, &mut state, backend, |partition| {
        remove_path(partition, key);
    })
}

fn remove_path(node: &mut Map<String, Value>, key: &[String]) {
    match key {
        [] => {}
        [last] => {
            node.remove(last);
        }
        [name, rest @ ..] => {
            if let Some(Value::Object(child)) = node.get_mut(name) {
                remove_path(child, rest);
                if child.is_empty() {
                    node.remove(name);
                }
            }
        }
    }
}
