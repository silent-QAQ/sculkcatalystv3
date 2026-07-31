// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

const MAX_FILE_BYTES: u64 = 1_048_576;
const MAX_OUTPUT_BYTES: usize = 262_144;
const ALLOWED_PROPERTIES: [&str; 8] = [
    "motd",
    "max-players",
    "difficulty",
    "gamemode",
    "pvp",
    "view-distance",
    "simulation-distance",
    "white-list",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskArtifact {
    pub name: String,
    pub path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub output: Value,
    pub rollback_available: bool,
    #[serde(default)]
    pub artifacts: Vec<TaskArtifact>,
}

pub struct ExecutionContext<'a> {
    pub workspace_root: &'a Path,
    pub state_dir: &'a Path,
    pub workspace_label: &'a str,
    pub permissions: &'a [String],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceListInput {
    #[serde(default)]
    path: String,
    #[serde(default = "default_max_entries")]
    max_entries: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogTailInput {
    #[serde(default = "default_log_path")]
    path: String,
    #[serde(default = "default_lines")]
    lines: usize,
    #[serde(default = "default_max_output")]
    max_bytes: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathInput {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PropertiesUpdateInput {
    path: String,
    changes: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RollbackInput {
    source_task_id: String,
}

#[derive(Serialize, Deserialize)]
struct RollbackRecord {
    kind: String,
    relative_path: String,
    existed: bool,
    expected_sha256: Option<String>,
}

fn default_max_entries() -> usize {
    200
}

fn default_log_path() -> String {
    "logs/latest.log".into()
}

fn default_lines() -> usize {
    200
}

fn default_max_output() -> usize {
    65_536
}

pub fn execute(
    context: &ExecutionContext<'_>,
    task_id: &str,
    operation: &str,
    input: Value,
) -> Result<ExecutionResult, String> {
    let required = required_permission(operation)?;
    if !context
        .permissions
        .iter()
        .any(|item| item == required || item == "full")
    {
        return Err(format!("Agent 未获得 {required} 权限"));
    }
    let root = canonical_workspace(context.workspace_root)?;
    let state_dir = prepare_state_dir(context.state_dir)?;
    match operation {
        "host.inspect" => host_inspect(context, &root),
        "workspace.list" => workspace_list(&root, input),
        "log.tail" => log_tail(&root, input),
        "workspace.create_directory" => create_directory(&root, &state_dir, task_id, input),
        "server.properties.update" => update_server_properties(&root, &state_dir, task_id, input),
        "task.rollback" => rollback(&root, &state_dir, input),
        _ => Err("Agent 拒绝了未知任务操作".into()),
    }
}

fn required_permission(operation: &str) -> Result<&'static str, String> {
    match operation {
        "host.inspect" | "workspace.list" | "log.tail" => Ok("read"),
        "workspace.create_directory" | "server.properties.update" | "task.rollback" => Ok("write"),
        _ => Err("任务操作不在 Agent 白名单中".into()),
    }
}

fn canonical_workspace(root: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(root)
        .map_err(|error| format!("无法访问 Agent 工作区 {}：{error}", root.display()))?;
    if !canonical.is_dir() {
        return Err("Agent 工作区不是目录".into());
    }
    Ok(canonical)
}

fn prepare_state_dir(path: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|error| format!("无法创建 Agent 状态目录：{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("无法保护 Agent 状态目录：{error}"))?;
    }
    fs::canonicalize(path).map_err(|error| format!("无法解析 Agent 状态目录：{error}"))
}

fn clean_relative(value: &str, allow_root: bool) -> Result<PathBuf, String> {
    if value.chars().any(char::is_control) || value.len() > 512 {
        return Err("路径包含控制字符或长度超过限制".into());
    }
    let path = Path::new(value.trim());
    if path.is_absolute() {
        return Err("任务路径必须相对于 Agent 工作区".into());
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("任务路径不能离开 Agent 工作区".into());
            }
        }
    }
    if clean.as_os_str().is_empty() && !allow_root {
        return Err("任务路径不能为空".into());
    }
    Ok(clean)
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            current.push(part);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err("任务路径不能经过符号链接".into());
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("无法检查任务路径：{error}")),
            }
        }
    }
    Ok(())
}

fn resolve_existing(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    reject_symlink_components(root, relative)?;
    let target = fs::canonicalize(root.join(relative))
        .map_err(|error| format!("任务目标不存在或不可访问：{error}"))?;
    if !target.starts_with(root) {
        return Err("任务目标超出 Agent 工作区".into());
    }
    Ok(target)
}

fn resolve_new(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    reject_symlink_components(root, relative)?;
    let target = root.join(relative);
    let parent = target
        .parent()
        .ok_or_else(|| "任务目标缺少父目录".to_string())?;
    let parent = fs::canonicalize(parent)
        .map_err(|error| format!("任务目标父目录不存在或不可访问：{error}"))?;
    if !parent.starts_with(root) {
        return Err("任务目标超出 Agent 工作区".into());
    }
    let name = target
        .file_name()
        .ok_or_else(|| "任务目标名称无效".to_string())?;
    Ok(parent.join(name))
}

fn host_inspect(context: &ExecutionContext<'_>, root: &Path) -> Result<ExecutionResult, String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("无法读取 Agent 工作区：{error}"))?
        .count();
    Ok(ExecutionResult {
        output: json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "agent_version": env!("CARGO_PKG_VERSION"),
            "workspace_label": context.workspace_label,
            "workspace_entries": entries,
            "permissions": context.permissions,
        }),
        rollback_available: false,
        artifacts: vec![],
    })
}

fn workspace_list(root: &Path, input: Value) -> Result<ExecutionResult, String> {
    let input: WorkspaceListInput =
        serde_json::from_value(input).map_err(|error| format!("目录任务参数无效：{error}"))?;
    if input.max_entries == 0 || input.max_entries > 500 {
        return Err("max_entries 需要在 1-500 之间".into());
    }
    let relative = clean_relative(&input.path, true)?;
    let target = resolve_existing(root, &relative)?;
    if !target.is_dir() {
        return Err("目录任务目标不是目录".into());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(&target)
        .map_err(|error| format!("无法列出工作区目录：{error}"))?
        .take(input.max_entries)
    {
        let entry = entry.map_err(|error| format!("无法读取目录条目：{error}"))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("无法读取目录元数据：{error}"))?;
        let kind = if metadata.file_type().is_symlink() {
            "symlink"
        } else if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else {
            "other"
        };
        entries.push(json!({
            "name": entry.file_name().to_string_lossy(),
            "kind": kind,
            "size": metadata.len(),
        }));
    }
    entries.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .cmp(&right.get("name").and_then(Value::as_str))
    });
    Ok(ExecutionResult {
        output: json!({ "path": input.path, "entries": entries }),
        rollback_available: false,
        artifacts: vec![],
    })
}

fn log_tail(root: &Path, input: Value) -> Result<ExecutionResult, String> {
    let input: LogTailInput =
        serde_json::from_value(input).map_err(|error| format!("日志任务参数无效：{error}"))?;
    if input.lines == 0
        || input.lines > 1000
        || input.max_bytes == 0
        || input.max_bytes > MAX_OUTPUT_BYTES
    {
        return Err("日志任务范围超过限制".into());
    }
    let relative = clean_relative(&input.path, false)?;
    let target = resolve_existing(root, &relative)?;
    let metadata = fs::metadata(&target).map_err(|error| format!("无法读取日志：{error}"))?;
    if !metadata.is_file() {
        return Err("日志任务目标不是文件".into());
    }
    let mut file = fs::File::open(&target).map_err(|error| format!("无法打开日志：{error}"))?;
    let start = metadata.len().saturating_sub(input.max_bytes as u64);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| format!("无法定位日志尾部：{error}"))?;
    let mut bytes = Vec::with_capacity(input.max_bytes);
    file.take(input.max_bytes as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("无法读取日志尾部：{error}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() > input.lines {
        lines.drain(..lines.len() - input.lines);
    }
    let content = lines.join("\n");
    let hash = sha256_hex(content.as_bytes());
    Ok(ExecutionResult {
        output: json!({ "path": input.path.clone(), "content": content.clone(), "truncated": start > 0 }),
        rollback_available: false,
        artifacts: vec![TaskArtifact {
            name: "log-tail.txt".into(),
            path: input.path,
            kind: "log".into(),
            size_bytes: Some(content.len() as u64),
            sha256: Some(hash),
        }],
    })
}

fn create_directory(
    root: &Path,
    state_dir: &Path,
    task_id: &str,
    input: Value,
) -> Result<ExecutionResult, String> {
    let input: PathInput =
        serde_json::from_value(input).map_err(|error| format!("创建目录参数无效：{error}"))?;
    let relative = clean_relative(&input.path, false)?;
    let target = resolve_new(root, &relative)?;
    if target.exists() {
        return Err("目标目录已经存在".into());
    }
    fs::create_dir(&target).map_err(|error| format!("无法创建目录：{error}"))?;
    let record = RollbackRecord {
        kind: "remove_directory".into(),
        relative_path: relative.to_string_lossy().into_owned(),
        existed: false,
        expected_sha256: None,
    };
    if let Err(error) = save_rollback_record(state_dir, task_id, &record, None) {
        let _ = fs::remove_dir(&target);
        return Err(error);
    }
    Ok(ExecutionResult {
        output: json!({ "path": input.path, "created": true }),
        rollback_available: true,
        artifacts: vec![],
    })
}

fn update_server_properties(
    root: &Path,
    state_dir: &Path,
    task_id: &str,
    input: Value,
) -> Result<ExecutionResult, String> {
    let input: PropertiesUpdateInput =
        serde_json::from_value(input).map_err(|error| format!("服务器配置参数无效：{error}"))?;
    let changes = validate_property_changes(input.changes)?;
    let relative = clean_relative(&input.path, false)?;
    if relative.file_name().and_then(|value| value.to_str()) != Some("server.properties") {
        return Err("server.properties.update 只能修改名为 server.properties 的文件".into());
    }
    reject_symlink_components(root, &relative)?;
    let target = resolve_new(root, &relative)?;
    let existed = target.exists();
    let original = if existed {
        let metadata = fs::metadata(&target)
            .map_err(|error| format!("无法读取 server.properties：{error}"))?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err("server.properties 不是普通文件或超过 1 MiB".into());
        }
        fs::read(&target).map_err(|error| format!("无法读取 server.properties：{error}"))?
    } else {
        Vec::new()
    };
    let original_text = String::from_utf8(original.clone())
        .map_err(|_| "server.properties 不是 UTF-8 文本".to_string())?;
    let updated = apply_property_changes(&original_text, &changes);
    let expected_sha256 = sha256_hex(updated.as_bytes());
    let record = RollbackRecord {
        kind: "restore_file".into(),
        relative_path: relative.to_string_lossy().into_owned(),
        existed,
        expected_sha256: Some(expected_sha256.clone()),
    };
    save_rollback_record(state_dir, task_id, &record, Some(&original))?;
    if let Err(error) = replace_file(&target, updated.as_bytes(), task_id) {
        delete_rollback_record(state_dir, task_id);
        return Err(error);
    }
    Ok(ExecutionResult {
        output: json!({
            "path": input.path,
            "changed_keys": changes.keys().collect::<Vec<_>>(),
            "sha256": expected_sha256,
        }),
        rollback_available: true,
        artifacts: vec![TaskArtifact {
            name: "server.properties".into(),
            path: relative.to_string_lossy().into_owned(),
            kind: "file".into(),
            size_bytes: Some(updated.len() as u64),
            sha256: Some(sha256_hex(updated.as_bytes())),
        }],
    })
}

fn validate_property_changes(
    values: Map<String, Value>,
) -> Result<BTreeMap<String, String>, String> {
    if values.is_empty() || values.len() > ALLOWED_PROPERTIES.len() {
        return Err("server.properties changes 不能为空或超过限制".into());
    }
    let mut changes = BTreeMap::new();
    for (key, value) in values {
        if !ALLOWED_PROPERTIES.contains(&key.as_str()) {
            return Err(format!("不允许修改 server.properties 键：{key}"));
        }
        let normalized = match key.as_str() {
            "motd" => value
                .as_str()
                .filter(|text| {
                    !text.is_empty()
                        && text.chars().count() <= 200
                        && !text.chars().any(char::is_control)
                })
                .map(str::to_string),
            "max-players" => value
                .as_u64()
                .filter(|number| (1..=1000).contains(number))
                .map(|number| number.to_string()),
            "difficulty" => value
                .as_str()
                .filter(|text| ["peaceful", "easy", "normal", "hard"].contains(text))
                .map(str::to_string),
            "gamemode" => value
                .as_str()
                .filter(|text| ["survival", "creative", "adventure", "spectator"].contains(text))
                .map(str::to_string),
            "pvp" | "white-list" => value.as_bool().map(|flag| flag.to_string()),
            "view-distance" | "simulation-distance" => value
                .as_u64()
                .filter(|number| (2..=32).contains(number))
                .map(|number| number.to_string()),
            _ => None,
        }
        .ok_or_else(|| format!("server.properties 键 {key} 的值无效"))?;
        changes.insert(key, normalized);
    }
    Ok(changes)
}

fn apply_property_changes(original: &str, changes: &BTreeMap<String, String>) -> String {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for line in original.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('#')
            && let Some((key, _)) = trimmed.split_once('=')
            && let Some(value) = changes.get(key.trim())
        {
            if seen.insert(key.trim().to_string()) {
                output.push(format!("{}={value}", key.trim()));
            }
            continue;
        }
        output.push(line.to_string());
    }
    for (key, value) in changes {
        if !seen.contains(key) {
            output.push(format!("{key}={value}"));
        }
    }
    let mut result = output.join("\n");
    result.push('\n');
    result
}

fn rollback(root: &Path, state_dir: &Path, input: Value) -> Result<ExecutionResult, String> {
    let input: RollbackInput =
        serde_json::from_value(input).map_err(|error| format!("回滚参数无效：{error}"))?;
    let source_id = safe_task_id(&input.source_task_id)?;
    let record_path = rollback_record_path(state_dir, &source_id);
    let record: RollbackRecord = serde_json::from_slice(
        &fs::read(&record_path).map_err(|_| "本机没有可用的任务回滚快照".to_string())?,
    )
    .map_err(|_| "本机任务回滚快照已损坏".to_string())?;
    let relative = clean_relative(&record.relative_path, false)?;
    match record.kind.as_str() {
        "remove_directory" => {
            let target = resolve_existing(root, &relative)?;
            fs::remove_dir(&target)
                .map_err(|error| format!("目录不是空目录，无法安全回滚：{error}"))?;
        }
        "restore_file" => {
            let target = root.join(&relative);
            reject_symlink_components(root, &relative)?;
            let current =
                fs::read(&target).map_err(|error| format!("无法验证待回滚文件：{error}"))?;
            let current_sha256 = sha256_hex(&current);
            if record.expected_sha256.as_deref() != Some(current_sha256.as_str()) {
                return Err("文件在任务完成后又被修改，已拒绝覆盖回滚".into());
            }
            if record.existed {
                let original = fs::read(rollback_data_path(state_dir, &source_id))
                    .map_err(|_| "本机回滚原文件不存在".to_string())?;
                replace_file(&target, &original, &format!("rollback-{source_id}"))?;
            } else {
                fs::remove_file(&target)
                    .map_err(|error| format!("无法移除任务创建的文件：{error}"))?;
            }
        }
        _ => return Err("本机回滚快照类型无效".into()),
    }
    delete_rollback_record(state_dir, &source_id);
    Ok(ExecutionResult {
        output: json!({ "source_task_id": source_id, "rolled_back": true }),
        rollback_available: false,
        artifacts: vec![],
    })
}

fn save_rollback_record(
    state_dir: &Path,
    task_id: &str,
    record: &RollbackRecord,
    data: Option<&[u8]>,
) -> Result<(), String> {
    let task_id = safe_task_id(task_id)?;
    let rollback_dir = state_dir.join("rollback");
    fs::create_dir_all(&rollback_dir).map_err(|error| format!("无法创建回滚目录：{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&rollback_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("无法保护回滚目录：{error}"))?;
    }
    if let Some(bytes) = data {
        write_private(&rollback_data_path(state_dir, &task_id), bytes)?;
    }
    let encoded =
        serde_json::to_vec(record).map_err(|error| format!("无法编码回滚记录：{error}"))?;
    write_private(&rollback_record_path(state_dir, &task_id), &encoded)
}

fn delete_rollback_record(state_dir: &Path, task_id: &str) {
    let _ = fs::remove_file(rollback_record_path(state_dir, task_id));
    let _ = fs::remove_file(rollback_data_path(state_dir, task_id));
}

fn rollback_record_path(state_dir: &Path, task_id: &str) -> PathBuf {
    state_dir.join("rollback").join(format!("{task_id}.json"))
}

fn rollback_data_path(state_dir: &Path, task_id: &str) -> PathBuf {
    state_dir.join("rollback").join(format!("{task_id}.data"))
}

fn safe_task_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() < 8
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("任务 ID 无效".into());
    }
    Ok(value.to_string())
}

fn replace_file(path: &Path, bytes: &[u8], task_id: &str) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "文件缺少父目录".to_string())?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let safe_id = safe_task_id(task_id)?;
    let temporary = parent.join(format!(".{name}.{safe_id}.tmp"));
    let swap = parent.join(format!(".{name}.{safe_id}.swap"));
    if temporary.exists() || swap.exists() {
        return Err("检测到未清理的任务临时文件".into());
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("无法创建任务临时文件：{error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("无法写入任务临时文件：{error}"))?;
    if path.exists() {
        fs::rename(path, &swap).map_err(|error| format!("无法暂存原文件：{error}"))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if swap.exists() {
            let _ = fs::rename(&swap, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(format!("无法提交配置文件：{error}"));
    }
    if swap.exists() {
        fs::remove_file(&swap).map_err(|error| format!("配置已提交但无法清理交换文件：{error}"))?;
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("无法创建受保护的 Agent 文件：{error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("无法写入受保护的 Agent 文件：{error}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{RngCore, rngs::OsRng};

    fn test_root() -> PathBuf {
        let mut bytes = [0_u8; 8];
        OsRng.fill_bytes(&mut bytes);
        let suffix = u64::from_le_bytes(bytes);
        let root = std::env::temp_dir().join(format!("sculk-agent-test-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn context<'a>(
        root: &'a Path,
        state: &'a Path,
        permissions: &'a [String],
    ) -> ExecutionContext<'a> {
        ExecutionContext {
            workspace_root: root,
            state_dir: state,
            workspace_label: "test",
            permissions,
        }
    }

    #[test]
    fn rejects_paths_that_escape_the_workspace() {
        assert!(clean_relative("../secret", false).is_err());
        assert!(clean_relative("/etc/passwd", false).is_err());
        assert!(clean_relative("logs/latest.log", false).is_ok());
    }

    #[test]
    fn updates_only_allowlisted_server_properties_and_rolls_back() {
        let root = test_root();
        let state = root.join(".agent-state");
        fs::write(
            root.join("server.properties"),
            b"motd=old\nmax-players=20\nsecret=value\n",
        )
        .unwrap();
        let permissions = vec!["write".to_string()];
        let ctx = context(&root, &state, &permissions);
        let result = execute(
            &ctx,
            "task-12345678",
            "server.properties.update",
            json!({ "path": "server.properties", "changes": { "motd": "new", "max-players": 30 } }),
        )
        .unwrap();
        assert!(result.rollback_available);
        let updated = fs::read_to_string(root.join("server.properties")).unwrap();
        assert!(updated.contains("motd=new"));
        assert!(updated.contains("secret=value"));
        execute(
            &ctx,
            "rollback-12345678",
            "task.rollback",
            json!({ "source_task_id": "task-12345678" }),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(root.join("server.properties")).unwrap(),
            "motd=old\nmax-players=20\nsecret=value\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_rollback_after_a_later_file_change() {
        let root = test_root();
        let state = root.join(".agent-state");
        fs::write(root.join("server.properties"), b"motd=old\n").unwrap();
        let permissions = vec!["write".to_string()];
        let ctx = context(&root, &state, &permissions);
        execute(
            &ctx,
            "task-abcdefgh",
            "server.properties.update",
            json!({ "path": "server.properties", "changes": { "motd": "new" } }),
        )
        .unwrap();
        fs::write(root.join("server.properties"), b"motd=manual\n").unwrap();
        assert!(
            execute(
                &ctx,
                "rollback-abcdefgh",
                "task.rollback",
                json!({ "source_task_id": "task-abcdefgh" }),
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
