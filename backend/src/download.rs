use crate::{AppState, TaskInfo, broadcast_line, catalog, internal, persist};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{fs, io::AsyncWriteExt};
use url::Url;
use uuid::Uuid;

type ApiError = (StatusCode, String);
type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Clone, Serialize)]
pub(crate) struct DownloadStatus {
    pub(crate) task_id: Uuid,
    pub(crate) phase: String,
    pub(crate) source: String,
    pub(crate) received: u64,
    pub(crate) total: Option<u64>,
    pub(crate) percent: u8,
    pub(crate) message: String,
    #[serde(skip)]
    pub(crate) cancel: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct StatusResponse {
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<DownloadStatus>,
}

#[derive(Deserialize, Default)]
struct StartDownloadRequest {
    #[serde(default)]
    mirror_ids: Vec<String>,
}

enum Source {
    Catalog {
        version_id: String,
        version: String,
        url: String,
        expected_size: u64,
        expected_sha256: String,
    },
    Mirror {
        name: String,
        url: String,
    },
    PaperApi {
        project: &'static str,
    },
    PurpurApi,
}

struct Resolved {
    label: String,
    url: String,
    expected_size: Option<u64>,
    expected_sha256: Option<String>,
    catalog_version_id: Option<String>,
}

enum AttemptError {
    Cancelled,
    Failed(String),
}

const ACTIVE_PHASES: [&str; 3] = ["resolving", "downloading", "verifying"];

pub(crate) fn is_active(status: &DownloadStatus) -> bool {
    ACTIVE_PHASES.contains(&status.phase.as_str())
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/servers/{id}/download/core", post(start_download))
        .route("/api/servers/{id}/download/status", get(download_status))
        .route("/api/servers/{id}/download/cancel", post(cancel_download))
}

async fn download_status(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<StatusResponse> {
    if !state
        .inner
        .read()
        .await
        .servers
        .iter()
        .any(|server| server.id == id)
    {
        return Err((StatusCode::NOT_FOUND, "server not found".into()));
    }
    let status = state.downloads.read().await.get(&id).cloned();
    let active = status.as_ref().is_some_and(is_active);
    Ok(Json(StatusResponse { active, status }))
}

async fn cancel_download(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<StatusResponse> {
    let downloads = state.downloads.read().await;
    let status = downloads
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, "no download for this server".into()))?;
    if !is_active(status) {
        return Err((StatusCode::CONFLICT, "download is not running".into()));
    }
    status.cancel.store(true, Ordering::Relaxed);
    Ok(Json(StatusResponse {
        active: true,
        status: Some(status.clone()),
    }))
}

async fn start_download(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<StartDownloadRequest>,
) -> ApiResult<TaskInfo> {
    let mut downloads = state.downloads.write().await;
    if downloads.get(&id).is_some_and(is_active) {
        return Err((StatusCode::CONFLICT, "已有下载任务在进行中".into()));
    }
    if state.processes.read().await.contains_key(&id) {
        return Err((
            StatusCode::CONFLICT,
            "服务器正在运行，请先停止后再更新核心".into(),
        ));
    }

    let (core, version, sources, task) = {
        let mut data = state.inner.write().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == id)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        let mut sources = Vec::new();
        if let Some(version) =
            catalog::resolve_core_download(&data.catalog, &server.core, &server.version, "stable")
        {
            sources.push(Source::Catalog {
                version_id: version.id,
                version: version.version,
                url: version.download_url,
                expected_size: version.size,
                expected_sha256: version.sha256,
            });
        }
        let mut mirrors: Vec<_> = data
            .mirrors
            .iter()
            .filter(|mirror| {
                mirror.enabled
                    && !placeholder_url(&mirror.base_url)
                    && (request.mirror_ids.is_empty() || request.mirror_ids.contains(&mirror.id))
                    && mirror
                        .cores
                        .iter()
                        .any(|item| item == "*" || item.eq_ignore_ascii_case(&server.core))
            })
            .collect();
        mirrors.sort_by_key(|mirror| mirror.priority);
        sources.extend(mirrors.into_iter().map(|mirror| {
            Source::Mirror {
                name: mirror.name.clone(),
                url: mirror
                    .base_url
                    .replace("{core}", &server.core.to_lowercase())
                    .replace("{version}", &server.version)
                    .replace("{filename}", "server.jar"),
            }
        }));
        match server.core.to_ascii_lowercase().as_str() {
            "paper" => sources.push(Source::PaperApi { project: "paper" }),
            "velocity" => sources.push(Source::PaperApi {
                project: "velocity",
            }),
            "purpur" => sources.push(Source::PurpurApi),
            _ => {}
        }
        if sources.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("核心 {} 暂无可用下载源", server.core),
            ));
        }
        let task = TaskInfo {
            id: Uuid::new_v4(),
            server_id: id.clone(),
            title: format!("下载 {} {} 核心", server.core, server.version),
            kind: "download".into(),
            status: "running".into(),
            progress: 0,
            created_at: Local::now().to_rfc3339(),
            risk: "low".into(),
            approved_by: None,
        };
        data.tasks.insert(0, task.clone());
        data.tasks.truncate(50);
        if let Some(item) = data.servers.iter_mut().find(|server| server.id == id) {
            item.task = "核心下载中".into();
        }
        persist(&state, &data).await.map_err(internal)?;
        (server.core, server.version, sources, task)
    };

    let cancel = Arc::new(AtomicBool::new(false));
    downloads.insert(
        id.clone(),
        DownloadStatus {
            task_id: task.id,
            phase: "resolving".into(),
            source: String::new(),
            received: 0,
            total: None,
            percent: 0,
            message: String::new(),
            cancel: cancel.clone(),
        },
    );
    drop(downloads);
    let job_state = state.clone();
    let job_id = id.clone();
    let task_id = task.id;
    tokio::spawn(async move {
        run_download(job_state, job_id, core, version, sources, task_id, cancel).await;
    });
    Ok(Json(task))
}

async fn run_download(
    state: AppState,
    server_id: String,
    core: String,
    version: String,
    sources: Vec<Source>,
    task_id: Uuid,
    cancel: Arc<AtomicBool>,
) {
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .user_agent("SculkCatalyst/0.1 (server-manager)")
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            finish_failed(
                &state,
                &server_id,
                task_id,
                format!("HTTP 客户端初始化失败：{error}"),
            )
            .await;
            return;
        }
    };
    let directory = PathBuf::from("data/servers").join(&server_id);
    let _ = fs::create_dir_all(&directory).await;
    let part = directory.join("server.jar.part");
    let target = directory.join("server.jar");
    let mut last_error = String::from("没有可用的下载源");

    for source in &sources {
        if cancel.load(Ordering::Relaxed) {
            finish_cancelled(&state, &server_id, task_id, &part).await;
            return;
        }
        let label = source_label(source);
        update_status(&state, &server_id, |status| {
            status.phase = "resolving".into();
            status.source = label.clone();
            status.received = 0;
            status.total = None;
            status.percent = 0;
            status.message = String::new();
        })
        .await;
        log(
            &state,
            &server_id,
            format!("正在从 {label} 获取 {core} {version} 核心…"),
        )
        .await;

        let resolved = match resolve_source(&client, source, &version).await {
            Ok(resolved) => resolved,
            Err(error) => {
                log(&state, &server_id, format!("{label} 不可用：{error}")).await;
                last_error = error;
                continue;
            }
        };
        match attempt_download(
            &state, &client, &server_id, &resolved, &part, task_id, &cancel,
        )
        .await
        {
            Ok((size, sha256)) => {
                if resolved.expected_size.is_some() || resolved.expected_sha256.is_some() {
                    update_status(&state, &server_id, |status| {
                        status.phase = "verifying".into()
                    })
                    .await;
                }
                if let Err(error) = verify_download(&resolved, size, &sha256) {
                    let _ = fs::remove_file(&part).await;
                    let error = format!("{label} {error}");
                    log(&state, &server_id, error.clone()).await;
                    last_error = error;
                    continue;
                }
                if let Err(error) = install_download(&part, &target).await {
                    let _ = fs::remove_file(&part).await;
                    log(&state, &server_id, error.clone()).await;
                    last_error = error;
                    continue;
                }
                finish_completed(
                    &state,
                    &server_id,
                    task_id,
                    &resolved.label,
                    size,
                    &sha256,
                    resolved.catalog_version_id.as_deref(),
                )
                .await;
                return;
            }
            Err(AttemptError::Cancelled) => {
                finish_cancelled(&state, &server_id, task_id, &part).await;
                return;
            }
            Err(AttemptError::Failed(error)) => {
                let _ = fs::remove_file(&part).await;
                log(&state, &server_id, format!("{label} 下载失败：{error}")).await;
                last_error = error;
            }
        }
    }
    finish_failed(&state, &server_id, task_id, last_error).await;
}

async fn attempt_download(
    state: &AppState,
    client: &reqwest::Client,
    server_id: &str,
    resolved: &Resolved,
    part: &std::path::Path,
    task_id: Uuid,
    cancel: &AtomicBool,
) -> Result<(u64, String), AttemptError> {
    let mut response = client
        .get(&resolved.url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|error| AttemptError::Failed(error.to_string()))?;
    let total = response.content_length().filter(|length| *length > 0);
    update_status(state, server_id, |status| {
        status.phase = "downloading".into();
        status.source = resolved.label.clone();
        status.total = total;
    })
    .await;
    let mut file = fs::File::create(part)
        .await
        .map_err(|error| AttemptError::Failed(format!("创建临时文件失败：{error}")))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut reported: u8 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AttemptError::Cancelled);
        }
        let chunk = tokio::time::timeout(Duration::from_secs(60), response.chunk())
            .await
            .map_err(|_| AttemptError::Failed("下载超时".into()))?
            .map_err(|error| AttemptError::Failed(error.to_string()))?;
        let Some(chunk) = chunk else { break };
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| AttemptError::Failed(format!("写入失败：{error}")))?;
        received += chunk.len() as u64;
        let percent = total
            .map(|total| ((received.saturating_mul(100)) / total.max(1)).min(99) as u8)
            .unwrap_or(0);
        update_status(state, server_id, |status| {
            status.received = received;
            status.percent = percent;
        })
        .await;
        if percent / 5 > reported / 5 {
            reported = percent;
            set_task_progress(state, task_id, percent, None, false).await;
            if percent % 25 < 5 {
                log(
                    state,
                    server_id,
                    format!("核心下载进度 {percent}%（{}）", format_size(received)),
                )
                .await;
            }
        }
    }
    file.flush()
        .await
        .map_err(|error| AttemptError::Failed(format!("写入失败：{error}")))?;
    drop(file);
    if received == 0 {
        return Err(AttemptError::Failed("下载内容为空".into()));
    }
    Ok((received, hex_string(&hasher.finalize())))
}

async fn resolve_source(
    client: &reqwest::Client,
    source: &Source,
    version: &str,
) -> Result<Resolved, String> {
    match source {
        Source::Catalog {
            version_id,
            version,
            url,
            expected_size,
            expected_sha256,
        } => Ok(Resolved {
            label: format!("资源目录（{version}）"),
            url: url.clone(),
            expected_size: Some(*expected_size),
            expected_sha256: Some(expected_sha256.clone()),
            catalog_version_id: Some(version_id.clone()),
        }),
        Source::Mirror { name, url } => Ok(Resolved {
            label: name.clone(),
            url: url.clone(),
            expected_size: None,
            expected_sha256: None,
            catalog_version_id: None,
        }),
        Source::PaperApi { project } => {
            // PaperMC v2 API 已停用（410 Gone），使用 Fill v3 API
            let build: Value = client
                .get(format!(
                    "https://fill.papermc.io/v3/projects/{project}/versions/{version}/builds/latest"
                ))
                .send()
                .await
                .and_then(|response| response.error_for_status())
                .map_err(|error| format!("PaperMC API 请求失败：{error}"))?
                .json()
                .await
                .map_err(|error| format!("PaperMC API 响应异常：{error}"))?;
            let number = build["id"].as_u64().ok_or("PaperMC 构建编号缺失")?;
            let download = &build["downloads"]["server:default"];
            let url = download["url"]
                .as_str()
                .ok_or("PaperMC 下载地址缺失")?
                .to_string();
            let sha256 = download["checksums"]["sha256"]
                .as_str()
                .map(|value| value.to_string());
            Ok(Resolved {
                label: format!("PaperMC 官方（构建 {number}）"),
                url,
                expected_size: None,
                expected_sha256: sha256,
                catalog_version_id: None,
            })
        }
        Source::PurpurApi => {
            let info: Value = client
                .get(format!("https://api.purpurmc.org/v2/purpur/{version}"))
                .send()
                .await
                .and_then(|response| response.error_for_status())
                .map_err(|error| format!("PurpurMC API 请求失败：{error}"))?
                .json()
                .await
                .map_err(|error| format!("PurpurMC API 响应异常：{error}"))?;
            let build = info["builds"]["latest"]
                .as_str()
                .ok_or_else(|| format!("PurpurMC 没有 {version} 的可用构建"))?
                .to_string();
            Ok(Resolved {
                label: format!("PurpurMC 官方（构建 {build}）"),
                url: format!("https://api.purpurmc.org/v2/purpur/{version}/{build}/download"),
                expected_size: None,
                expected_sha256: None,
                catalog_version_id: None,
            })
        }
    }
}

fn source_label(source: &Source) -> String {
    match source {
        Source::Catalog { version, .. } => format!("资源目录（{version}）"),
        Source::Mirror { name, .. } => name.clone(),
        Source::PaperApi { project } => format!("PaperMC 官方源（{project}）"),
        Source::PurpurApi => "PurpurMC 官方源".into(),
    }
}

fn placeholder_url(value: &str) -> bool {
    const RESERVED_HOST: &str = "example.com";
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == RESERVED_HOST || host.ends_with(".example.com"))
}

async fn install_download(part: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    let backup = target.with_file_name("server.jar.backup");
    match fs::remove_file(&backup).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("清理旧核心备份失败：{error}")),
    }

    let had_target = fs::metadata(target).await.is_ok();
    if had_target {
        fs::rename(target, &backup)
            .await
            .map_err(|error| format!("备份现有 server.jar 失败：{error}"))?;
    }

    if let Err(error) = fs::rename(part, target).await {
        let restore_error = if had_target {
            fs::rename(&backup, target).await.err()
        } else {
            None
        };
        return Err(match restore_error {
            Some(restore_error) => format!(
                "写入 server.jar 失败：{error}；旧核心恢复失败并保留在 {}：{restore_error}",
                backup.display()
            ),
            None => format!("写入 server.jar 失败，旧核心已恢复：{error}"),
        });
    }

    if had_target {
        let _ = fs::remove_file(backup).await;
    }
    Ok(())
}

fn verify_download(resolved: &Resolved, size: u64, sha256: &str) -> Result<(), String> {
    if let Some(expected) = resolved.expected_size
        && expected != size
    {
        return Err(format!(
            "文件大小校验失败：期望 {expected} 字节，实际 {size} 字节"
        ));
    }
    if let Some(expected) = &resolved.expected_sha256
        && !expected.eq_ignore_ascii_case(sha256)
    {
        return Err("SHA-256 校验失败".into());
    }
    Ok(())
}

async fn finish_completed(
    state: &AppState,
    server_id: &str,
    task_id: Uuid,
    source: &str,
    size: u64,
    sha256: &str,
    catalog_version_id: Option<&str>,
) {
    update_status(state, server_id, |status| {
        status.phase = "completed".into();
        status.percent = 100;
        status.message = format!("SHA-256 {}", &sha256[..16.min(sha256.len())]);
    })
    .await;
    log(
        state,
        server_id,
        format!(
            "核心下载完成：{source}，大小 {}，SHA-256 {sha256}。server.jar 已就绪，可以启动服务器。",
            format_size(size)
        ),
    )
    .await;
    let mut data = state.inner.write().await;
    if let Some(version_id) = catalog_version_id {
        catalog::record_core_download(&mut data.catalog, version_id);
    }
    if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
        task.status = "completed".into();
        task.progress = 100;
    }
    if let Some(task) = data.tasks.iter_mut().find(|task| {
        task.server_id == server_id && task.kind == "bootstrap" && task.status != "completed"
    }) {
        task.status = "completed".into();
        task.progress = 100;
    }
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == server_id)
    {
        server.task = "核心已就绪".into();
    }
    let _ = persist(state, &data).await;
}

async fn finish_failed(state: &AppState, server_id: &str, task_id: Uuid, error: String) {
    update_status(state, server_id, |status| {
        status.phase = "failed".into();
        status.message = error.clone();
    })
    .await;
    log(state, server_id, format!("核心下载失败：{error}")).await;
    let mut data = state.inner.write().await;
    if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
        task.status = "failed".into();
    }
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == server_id)
    {
        server.task = "核心下载失败".into();
    }
    let _ = persist(state, &data).await;
}

async fn finish_cancelled(
    state: &AppState,
    server_id: &str,
    task_id: Uuid,
    part: &std::path::Path,
) {
    let _ = fs::remove_file(part).await;
    update_status(state, server_id, |status| {
        status.phase = "cancelled".into();
        status.message = "已取消".into();
    })
    .await;
    log(state, server_id, "核心下载已取消。".into()).await;
    let mut data = state.inner.write().await;
    if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
        task.status = "cancelled".into();
    }
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == server_id)
    {
        server.task = "核心下载已取消".into();
    }
    let _ = persist(state, &data).await;
}

async fn update_status<F: FnOnce(&mut DownloadStatus)>(
    state: &AppState,
    server_id: &str,
    apply: F,
) {
    let mut downloads = state.downloads.write().await;
    if let Some(status) = downloads.get_mut(server_id) {
        apply(status);
    }
}

async fn set_task_progress(
    state: &AppState,
    task_id: Uuid,
    progress: u8,
    status: Option<&str>,
    persist_now: bool,
) {
    let mut data = state.inner.write().await;
    if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
        task.progress = progress;
        if let Some(status) = status {
            task.status = status.into();
        }
    }
    if persist_now {
        let _ = persist(state, &data).await;
    }
}

async fn log(state: &AppState, server_id: &str, message: String) {
    let line = format!("[{} AI]: {}", Local::now().format("%H:%M:%S"), message);
    broadcast_line(state, server_id, &line).await;
    let mut data = state.inner.write().await;
    data.logs
        .entry(server_id.to_string())
        .or_default()
        .push(line);
    let _ = persist(state, &data).await;
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

fn hex_string(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push_str(&format!("{byte:02x}"));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catalog_source_carries_trusted_integrity_metadata() {
        let source = Source::Catalog {
            version_id: "paper-232".into(),
            version: "1.21.4-232".into(),
            url: "https://downloads.example.test/paper.jar".into(),
            expected_size: 42,
            expected_sha256: "a".repeat(64),
        };
        let client = reqwest::Client::new();
        let resolved = resolve_source(&client, &source, "1.21.4").await.unwrap();

        assert_eq!(resolved.expected_size, Some(42));
        assert_eq!(resolved.expected_sha256.as_deref(), Some(&*"a".repeat(64)));
        assert_eq!(resolved.catalog_version_id.as_deref(), Some("paper-232"));
        assert_eq!(source_label(&source), "资源目录（1.21.4-232）");
    }

    #[test]
    fn trusted_integrity_validation_rejects_size_and_hash_mismatches() {
        let resolved = Resolved {
            label: "资源目录".into(),
            url: "https://downloads.example.test/paper.jar".into(),
            expected_size: Some(42),
            expected_sha256: Some("a".repeat(64)),
            catalog_version_id: Some("paper-232".into()),
        };

        assert!(verify_download(&resolved, 41, &"a".repeat(64)).is_err());
        assert!(verify_download(&resolved, 42, &"b".repeat(64)).is_err());
        assert!(verify_download(&resolved, 42, &"a".repeat(64)).is_ok());
    }

    #[test]
    fn reserved_example_mirrors_are_not_executable_sources() {
        assert!(placeholder_url(
            "https://mirror-primary.example.com/paper/server.jar"
        ));
        assert!(!placeholder_url(
            "https://fill-data.papermc.io/v1/objects/paper.jar"
        ));
    }

    #[tokio::test]
    async fn installing_a_verified_download_keeps_a_rollback_path() {
        let directory = std::env::temp_dir().join(format!("sculk-download-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).await.unwrap();
        let target = directory.join("server.jar");
        let part = directory.join("server.jar.part");
        fs::write(&target, b"old core").await.unwrap();
        fs::write(&part, b"new core").await.unwrap();

        install_download(&part, &target).await.unwrap();

        assert_eq!(fs::read(&target).await.unwrap(), b"new core");
        assert!(fs::metadata(&part).await.is_err());
        assert!(
            fs::metadata(directory.join("server.jar.backup"))
                .await
                .is_err()
        );
        fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn install_aborts_without_touching_the_old_core_when_backup_is_unsafe() {
        let directory = std::env::temp_dir().join(format!("sculk-download-{}", Uuid::new_v4()));
        fs::create_dir_all(directory.join("server.jar.backup"))
            .await
            .unwrap();
        let target = directory.join("server.jar");
        let part = directory.join("server.jar.part");
        fs::write(&target, b"old core").await.unwrap();
        fs::write(&part, b"new core").await.unwrap();

        assert!(install_download(&part, &target).await.is_err());
        assert_eq!(fs::read(&target).await.unwrap(), b"old core");
        assert_eq!(fs::read(&part).await.unwrap(), b"new core");
        fs::remove_dir_all(directory).await.unwrap();
    }
}
