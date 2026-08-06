use crate::{PersistedState, ServerInfo, ServiceSettings, TaskInfo, conversations, runtime};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::{
    fs,
    io::{AsyncWriteExt, BufWriter},
};
use uuid::Uuid;

pub(crate) const FILE_NAME: &str = "sculk.yml";
const BACKUP_FILE_NAME: &str = "sculk.yml.bak";
const SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct SculkManifest {
    schema_version: u32,
    server: ManifestServer,
    plan: ManifestPlan,
    #[serde(default)]
    lifecycle: ManifestLifecycle,
    #[serde(default)]
    services: ServiceSettings,
    executor: ManifestExecutor,
    updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct ManifestServer {
    name: String,
    id: String,
    /// pending / creating / stopped / starting / running / stopping / startup_error
    status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    core: String,
    #[serde(default = "default_core_source")]
    core_source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    core_resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    core_resource_version: Option<String>,
    #[serde(default)]
    port: u16,
    #[serde(default = "default_memory_gb")]
    memory_gb: u8,
    #[serde(default)]
    core_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct ManifestPlan {
    /// pending / completed
    status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct ManifestLifecycle {
    /// create / build / operate
    #[serde(default = "default_lifecycle_phase")]
    phase: String,
}

impl Default for ManifestLifecycle {
    fn default() -> Self {
        Self {
            phase: default_lifecycle_phase(),
        }
    }
}

fn default_lifecycle_phase() -> String {
    "create".into()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct ManifestExecutor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_task_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    progress: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

fn default_memory_gb() -> u8 {
    8
}

fn default_core_source() -> String {
    "catalog".into()
}

fn normalize_core_source(value: &str) -> String {
    if value.trim().eq_ignore_ascii_case("local_upload") {
        "local_upload".into()
    } else {
        "catalog".into()
    }
}

fn latest_task<'a>(data: &'a PersistedState, server_id: &str) -> Option<&'a TaskInfo> {
    data.tasks.iter().find(|task| task.server_id == server_id)
}

fn manifest_status(server: &ServerInfo, latest_task: Option<&TaskInfo>) -> &'static str {
    if server.status == "planning" {
        return "pending";
    }
    if server.operation_state == "provisioning"
        || latest_task.is_some_and(|task| {
            matches!(
                task.kind.as_str(),
                "server_bootstrap" | "server_provision" | "bootstrap"
            ) && matches!(
                task.status.as_str(),
                "awaiting_approval" | "queued" | "running" | "cancelling"
            )
        })
    {
        return "creating";
    }
    match server.operation_state.as_str() {
        "starting" => "starting",
        "stopping" => "stopping",
        _ if server.status == "online" => "running",
        _ if server.status == "warning" || server.status == "error" => "startup_error",
        _ => "stopped",
    }
}

fn plan_status(server: &ServerInfo) -> &'static str {
    if server.status != "planning"
        && !server.core.trim().is_empty()
        && !server.version.trim().is_empty()
    {
        "completed"
    } else {
        "pending"
    }
}

fn from_state(data: &PersistedState, server: &ServerInfo) -> SculkManifest {
    let task = latest_task(data, &server.id);
    SculkManifest {
        schema_version: SCHEMA_VERSION,
        server: ManifestServer {
            name: server.name.clone(),
            id: server.id.clone(),
            status: manifest_status(server, task).into(),
            core: server.core.clone(),
            core_source: server.core_source.clone(),
            version: server.version.clone(),
            core_resource_id: server.core_resource_id.clone(),
            core_resource_version: server.core_resource_version.clone(),
            port: server.port,
            memory_gb: server.memory_gb,
            core_ready: server.core_ready,
            last_error: server.last_error.clone(),
        },
        plan: ManifestPlan {
            status: plan_status(server).into(),
        },
        lifecycle: ManifestLifecycle {
            phase: server.lifecycle_phase.clone(),
        },
        services: server.service_settings.clone(),
        executor: task
            .map(|task| ManifestExecutor {
                latest_task_id: Some(task.id),
                kind: Some(task.kind.clone()),
                status: Some(task.status.clone()),
                progress: Some(task.progress),
                summary: task.summary.clone().or_else(|| task.error.clone()),
            })
            .unwrap_or_default(),
        updated_at: Local::now().to_rfc3339(),
    }
}

async fn read_manifest_file(path: &Path) -> Result<SculkManifest, String> {
    let metadata = fs::symlink_metadata(path)
        .await
        .map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("sculk.yml 必须是普通文件，不能是符号链接".into());
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err("sculk.yml 超过 64 KiB 限制".into());
    }
    let bytes = fs::read(path).await.map_err(|error| error.to_string())?;
    let manifest: SculkManifest =
        serde_yaml::from_slice(&bytes).map_err(|error| format!("YAML 无效：{error}"))?;
    validate(&manifest)?;
    Ok(manifest)
}

async fn read_manifest(directory: &Path) -> Result<SculkManifest, String> {
    let primary = directory.join(FILE_NAME);
    match read_manifest_file(&primary).await {
        Ok(manifest) => Ok(manifest),
        Err(primary_error) => {
            let backup = directory.join(BACKUP_FILE_NAME);
            read_manifest_file(&backup).await.map_err(|backup_error| {
                format!("主清单不可用（{primary_error}）；备份也不可用（{backup_error}）")
            })
        }
    }
}

fn validate(manifest: &SculkManifest) -> Result<(), String> {
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "不支持 sculk.yml schema_version {}",
            manifest.schema_version
        ));
    }
    if !valid_server_id(&manifest.server.id) {
        return Err("服务器编号无效".into());
    }
    crate::validate_server_name(&manifest.server.name)
        .map_err(|error| format!("服务器名无效：{error}"))?;
    if !matches!(
        manifest.server.status.as_str(),
        "pending" | "creating" | "stopped" | "starting" | "running" | "stopping" | "startup_error"
    ) {
        return Err("服务器状态不在受支持集合中".into());
    }
    if !matches!(manifest.plan.status.as_str(), "pending" | "completed") {
        return Err("服务器计划状态必须是 pending 或 completed".into());
    }
    if !matches!(
        manifest.lifecycle.phase.as_str(),
        "create" | "build" | "operate"
    ) {
        return Err("服务器生命周期阶段必须是 create、build 或 operate".into());
    }
    Ok(())
}

fn valid_server_id(id: &str) -> bool {
    id.starts_with("server-")
        && id.len() <= 64
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

async fn write_manifest(directory: &Path, desired: &SculkManifest) -> Result<bool, String> {
    fs::create_dir_all(directory)
        .await
        .map_err(|error| error.to_string())?;
    if let Ok(existing) = read_manifest(directory).await {
        let mut comparable = desired.clone();
        comparable.updated_at = existing.updated_at.clone();
        if comparable == existing {
            return Ok(false);
        }
    }

    let target = directory.join(FILE_NAME);
    let backup = directory.join(BACKUP_FILE_NAME);
    let temporary = directory.join(format!(".sculk.{}.tmp", Uuid::new_v4().simple()));
    let bytes = serde_yaml::to_string(desired)
        .map_err(|error| error.to_string())?
        .into_bytes();
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .await
        .map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    if let Err(error) = async {
        writer.write_all(&bytes).await?;
        writer.flush().await?;
        writer.get_ref().sync_all().await
    }
    .await
    {
        let _ = fs::remove_file(&temporary).await;
        return Err(error.to_string());
    }
    drop(writer);

    let had_target = fs::try_exists(&target)
        .await
        .map_err(|error| error.to_string())?;
    if had_target {
        if fs::try_exists(&backup)
            .await
            .map_err(|error| error.to_string())?
        {
            fs::remove_file(&backup)
                .await
                .map_err(|error| error.to_string())?;
        }
        fs::rename(&target, &backup)
            .await
            .map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(&temporary, &target).await {
        if had_target {
            let _ = fs::rename(&backup, &target).await;
        }
        let _ = fs::remove_file(&temporary).await;
        return Err(error.to_string());
    }
    if had_target {
        let _ = fs::remove_file(&backup).await;
    }
    Ok(true)
}

/// 将持久化状态投影到每个 Minecraft 服务器目录。单个清单失败不会阻止其他服务器更新。
pub(crate) async fn sync_all(data: &PersistedState) -> Result<bool, Vec<String>> {
    let mut changed = false;
    let mut errors = Vec::new();
    for server in data.servers.iter().filter(|server| server.kind == "server") {
        let manifest = from_state(data, server);
        // External workspaces are user-owned; never write a Sculk manifest into
        // them as a side effect of a dashboard refresh.
        if server.workspace_path.is_some() {
            continue;
        }
        match write_manifest(&runtime::server_directory(&server.id), &manifest).await {
            Ok(written) => changed |= written,
            Err(error) => errors.push(format!("{}: {error}", server.id)),
        }
    }
    if errors.is_empty() {
        Ok(changed)
    } else {
        Err(errors)
    }
}

fn imported_status(manifest: &SculkManifest) -> (String, Option<String>) {
    match manifest.server.status.as_str() {
        "pending" => ("planning".into(), None),
        "startup_error" => (
            "warning".into(),
            manifest
                .server
                .last_error
                .clone()
                .or_else(|| Some("sculk.yml 记录了上次启动错误".into())),
        ),
        "creating" | "starting" | "running" | "stopping" => (
            "stopped".into(),
            Some(
                "已从 sculk.yml 接手；旧设备运行态不会被直接信任，请重新启动以建立受管进程".into(),
            ),
        ),
        _ => ("stopped".into(), manifest.server.last_error.clone()),
    }
}

async fn import_one(
    data: &mut PersistedState,
    directory: PathBuf,
    manifest: SculkManifest,
) -> Result<(), String> {
    let directory_id = directory
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "服务器目录名不是有效 UTF-8".to_string())?;
    if directory_id != manifest.server.id {
        return Err(format!(
            "目录名 {directory_id} 与清单编号 {} 不一致",
            manifest.server.id
        ));
    }
    if data
        .servers
        .iter()
        .any(|server| server.id == manifest.server.id)
    {
        return Ok(());
    }
    let port_conflict = manifest.server.port != 0
        && data
            .servers
            .iter()
            .any(|server| server.port == manifest.server.port);
    let (status, mut last_error) = imported_status(&manifest);
    if port_conflict {
        last_error = Some(format!(
            "sculk.yml 中端口 {} 已被本机其他服务器占用，请重新分配",
            manifest.server.port
        ));
    }
    let core_ready = fs::metadata(directory.join("server.jar"))
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
    let id = manifest.server.id.clone();
    let name = manifest.server.name.clone();
    let plan_pending = manifest.plan.status == "pending";
    let server = ServerInfo {
        id: id.clone(),
        kind: "server".into(),
        name: name.clone(),
        core: manifest.server.core,
        core_source: normalize_core_source(&manifest.server.core_source),
        core_resource_id: manifest.server.core_resource_id,
        core_resource_version: manifest.server.core_resource_version,
        version: manifest.server.version,
        status: if plan_pending {
            "planning".into()
        } else {
            status
        },
        players: "0 / 60".into(),
        memory: 0,
        memory_gb: manifest.server.memory_gb.clamp(1, 128),
        cpu: 0,
        port: if port_conflict {
            0
        } else {
            manifest.server.port
        },
        task: if plan_pending {
            "已从 sculk.yml 接手 · 服务器计划待完成".into()
        } else {
            "已从 sculk.yml 接手 · 等待状态校验".into()
        },
        location: "local".into(),
        workspace_path: None,
        launch_jar: None,
        pid: None,
        runtime_generation: None,
        started_at: None,
        operation_state: "idle".into(),
        core_ready,
        last_error,
        lifecycle_phase: manifest.lifecycle.phase,
        service_settings: manifest.services,
    };
    let config = fs::read_to_string(directory.join("server.properties"))
        .await
        .unwrap_or_default();
    if !config.is_empty() {
        data.configs.insert(id.clone(), config);
    }
    data.logs.entry(id.clone()).or_default().push(format!(
        "[{} INFO]: 已从 sculk.yml 导入服务器身份；实时进程状态将重新探测。",
        Local::now().format("%H:%M:%S")
    ));
    let mut conversation = conversations::new_conversation(&id, Some("服务器接手".into()), None);
    conversation.messages.push(conversations::assistant_message(
        &format!(
            "已从服务器目录中的 sculk.yml 接手“{name}”（编号 {id}）。身份、计划和上次执行摘要已读取；旧设备的 PID 与在线状态不会直接沿用，启动前会重新校验核心、端口和 Java。"
        ),
        None,
    ));
    data.servers.push(server);
    data.conversations.push(conversation);
    Ok(())
}

/// 扫描 data/servers/*/sculk.yml，将尚未登记的服务器安全接入当前设备。
pub(crate) async fn import_discovered(data: &mut PersistedState) -> bool {
    let root = runtime::data_root().join("servers");
    let mut entries = match fs::read_dir(&root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(error) => {
            eprintln!("failed to scan {}: {error}", root.display());
            return false;
        }
    };
    let mut changed = false;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !entry
            .file_type()
            .await
            .is_ok_and(|file_type| file_type.is_dir() && !file_type.is_symlink())
        {
            continue;
        }
        match read_manifest(&path).await {
            Ok(manifest)
                if data
                    .servers
                    .iter()
                    .any(|server| server.id == manifest.server.id) => {}
            Ok(manifest) => match import_one(data, path.clone(), manifest).await {
                Ok(()) => changed = true,
                Err(error) => eprintln!("ignored {}: {error}", path.display()),
            },
            Err(error) => {
                if fs::try_exists(path.join(FILE_NAME)).await.unwrap_or(false) {
                    eprintln!(
                        "ignored invalid {}: {error}",
                        path.join(FILE_NAME).display()
                    );
                }
            }
        }
    }
    changed
}

pub(crate) async fn remove(directory: &Path) -> Result<(), String> {
    for name in [FILE_NAME, BACKUP_FILE_NAME] {
        let path = directory.join(name);
        if fs::try_exists(&path)
            .await
            .map_err(|error| error.to_string())?
        {
            fs::remove_file(path)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_server() -> ServerInfo {
        ServerInfo {
            id: "server-12345678".into(),
            kind: "server".into(),
            name: "测试服".into(),
            core: "Paper".into(),
            core_source: "catalog".into(),
            core_resource_id: None,
            core_resource_version: None,
            version: "1.21.4".into(),
            status: "stopped".into(),
            players: "0 / 20".into(),
            memory: 0,
            memory_gb: 8,
            cpu: 0,
            port: 25565,
            task: String::new(),
            location: "local".into(),
            workspace_path: None,
            launch_jar: None,
            pid: None,
            runtime_generation: None,
            started_at: None,
            operation_state: "idle".into(),
            core_ready: false,
            last_error: None,
            lifecycle_phase: "create".into(),
            service_settings: ServiceSettings::default(),
        }
    }

    #[test]
    fn lifecycle_mapping_covers_portable_states() {
        let mut server = test_server();
        server.status = "planning".into();
        assert_eq!(manifest_status(&server, None), "pending");
        server.status = "stopped".into();
        server.operation_state = "provisioning".into();
        assert_eq!(manifest_status(&server, None), "creating");
        server.operation_state = "starting".into();
        assert_eq!(manifest_status(&server, None), "starting");
        server.operation_state = "idle".into();
        server.status = "online".into();
        assert_eq!(manifest_status(&server, None), "running");
    }

    #[test]
    fn portable_manifest_keeps_non_secret_service_settings() {
        let mut server = test_server();
        server.service_settings.economy = true;
        server.service_settings.social.enabled = true;
        server.service_settings.social.qq_bot = true;
        let mut data = crate::initial_state();
        data.servers.push(server.clone());

        let manifest = from_state(&data, &server);
        assert!(manifest.services.economy);
        assert!(manifest.services.social.enabled);
        assert_eq!(manifest.services.social.sync_interval_seconds, 240);

        let yaml = serde_yaml::to_string(&manifest).unwrap();
        let restored: SculkManifest = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(restored.services, manifest.services);
    }

    #[test]
    fn imported_running_state_is_never_trusted_as_live() {
        let manifest = SculkManifest {
            schema_version: SCHEMA_VERSION,
            server: ManifestServer {
                name: "迁移服".into(),
                id: "server-12345678".into(),
                status: "running".into(),
                core: "Paper".into(),
                core_source: "catalog".into(),
                version: "1.21.4".into(),
                core_resource_id: None,
                core_resource_version: None,
                port: 25565,
                memory_gb: 8,
                core_ready: true,
                last_error: None,
            },
            plan: ManifestPlan {
                status: "completed".into(),
            },
            lifecycle: ManifestLifecycle {
                phase: "operate".into(),
            },
            services: ServiceSettings::default(),
            executor: ManifestExecutor::default(),
            updated_at: Local::now().to_rfc3339(),
        };
        let (status, warning) = imported_status(&manifest);
        assert_eq!(status, "stopped");
        assert!(warning.unwrap().contains("旧设备运行态"));
    }
}
