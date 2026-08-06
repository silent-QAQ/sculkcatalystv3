use crate::{
    AppState, PersistedState, ServerInfo, TaskInfo, broadcast_line, catalog, internal, persist,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{
        StatusCode,
        header::{CONTENT_RANGE, RANGE},
    },
    routing::{get, post},
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};
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
    MslApi {
        project: String,
    },
    ResourceCenter {
        identifier: String,
        artifact_version: Option<String>,
    },
    PaperApi {
        project: &'static str,
    },
    PurpurApi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProvisionCoreOutcome {
    AlreadyReady,
    Installed,
    Cancelled,
}

fn collect_sources(
    data: &PersistedState,
    server: &ServerInfo,
    mirror_ids: &[String],
) -> Vec<Source> {
    let mut sources = Vec::new();
    let mut core_identifiers = Vec::new();
    if let Some(resource_id) = server
        .core_resource_id
        .as_deref()
        .map(str::trim)
        .filter(|resource_id| !resource_id.is_empty())
    {
        core_identifiers.push(resource_id.to_string());
    }
    if !core_identifiers
        .iter()
        .any(|identifier| identifier.eq_ignore_ascii_case(&server.core))
    {
        core_identifiers.push(server.core.clone());
    }
    for identifier in &core_identifiers {
        let version = if let Some(artifact_version) = server
            .core_resource_version
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            catalog::resolve_core_download_exact(
                &data.catalog,
                identifier,
                &server.version,
                "stable",
                artifact_version,
            )
        } else {
            catalog::resolve_core_download(&data.catalog, identifier, &server.version, "stable")
        };
        let Some(version) = version else {
            continue;
        };
        sources.push(Source::Catalog {
            version_id: version.id,
            version: version.version,
            url: version.download_url,
            expected_size: version.size,
            expected_sha256: version.sha256,
        });
        break;
    }
    for identifier in &core_identifiers {
        sources.push(Source::ResourceCenter {
            identifier: identifier.clone(),
            artifact_version: server.core_resource_version.clone(),
        });
    }
    // A user-selected artifact must never silently fall back to an unpinned
    // mirror or an "latest" official build. If the pinned catalog source is
    // unavailable, surface that failure so the user can choose another build.
    if server
        .core_resource_version
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return sources;
    }
    for identifier in &core_identifiers {
        sources.push(Source::MslApi {
            project: identifier.to_ascii_lowercase(),
        });
    }
    let mut mirrors: Vec<_> = data
        .mirrors
        .iter()
        .filter(|mirror| {
            mirror.enabled
                && !placeholder_url(&mirror.base_url)
                && (mirror_ids.is_empty() || mirror_ids.contains(&mirror.id))
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
    sources
}

struct Resolved {
    label: String,
    url: String,
    expected_size: Option<u64>,
    expected_sha256: Option<String>,
    catalog_version_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Transport,
    Timeout,
    Http(StatusCode),
    Protocol,
    Integrity,
    LocalIo,
}

#[derive(Debug)]
enum AttemptError {
    Cancelled,
    Failed { message: String, kind: FailureKind },
}

impl AttemptError {
    fn failed(message: impl Into<String>, kind: FailureKind) -> Self {
        Self::Failed {
            message: message.into(),
            kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedContentRange {
    Satisfied {
        start: u64,
        end: u64,
        total: Option<u64>,
    },
    Unsatisfied {
        total: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseDecision {
    Restart {
        total: Option<u64>,
        maximum_size: Option<u64>,
        range_ignored: bool,
    },
    Resume {
        offset: u64,
        total: Option<u64>,
        maximum_size: Option<u64>,
        response_end: u64,
        response_complete: bool,
    },
    AlreadyComplete {
        total: u64,
    },
}

const ACTIVE_PHASES: [&str; 3] = ["resolving", "downloading", "verifying"];
const MAX_TRANSFER_ATTEMPTS: u8 = 3;
const RETRY_BASE_DELAY: Duration = Duration::from_millis(250);

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
    {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == id)
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        crate::require_server_kind(server, "core download")?;
    }
    let status = state.downloads.read().await.get(&id).cloned();
    let active = status.as_ref().is_some_and(is_active);
    Ok(Json(StatusResponse { active, status }))
}

async fn cancel_download(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<StatusResponse> {
    {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == id)
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        crate::require_server_kind(server, "core download")?;
    }
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
    if state.shutting_down.load(Ordering::Acquire) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "后端正在关闭，不再接受新的核心下载任务".into(),
        ));
    }
    {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == id)
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        crate::require_server_kind(server, "core download")?;
        if server.workspace_path.is_some() {
            return Err((
                StatusCode::CONFLICT,
                "已有服务器目录不会被核心下载覆盖；请直接管理目录中的现有核心".into(),
            ));
        }
        if server.core_source == "local_upload" {
            return Err((
                StatusCode::CONFLICT,
                "该服务器使用本地上传核心，不能改用镜像下载".into(),
            ));
        }
    }
    let operation = crate::server_operation_lock(&state, &id).await;
    let _guard = operation.lock().await;
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
        crate::require_server_kind(&server, "core download")?;
        let sources = collect_sources(&data, &server, &request.mirror_ids);
        if sources.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("核心 {} 暂无可用下载源", server.core),
            ));
        }
        let task = crate::new_task_record(
            id.clone(),
            format!("下载 {} {} 核心", server.core, server.version),
            "download".into(),
            "running".into(),
            0,
            "low".into(),
            None,
        );
        data.tasks.insert(0, task.clone());
        data.tasks.truncate(50);
        if let Some(item) = data.servers.iter_mut().find(|server| server.id == id) {
            item.task = "核心下载中".into();
        }
        if let Some(item) = data.servers.iter_mut().find(|server| server.id == id) {
            item.operation_state = "provisioning".into();
            item.last_error = None;
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

pub(crate) async fn provision_core(
    state: &AppState,
    server_id: &str,
    task_id: Uuid,
    cancel: Arc<AtomicBool>,
) -> Result<ProvisionCoreOutcome, String> {
    let directory = crate::runtime::server_directory(server_id);
    let target = directory.join("server.jar");
    let part = directory.join("server.jar.part");
    if fs::metadata(&target)
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
    {
        clear_part(&part).await?;
        return Ok(ProvisionCoreOutcome::AlreadyReady);
    }
    let (core, version, sources) = {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == server_id)
            .cloned()
            .ok_or_else(|| "server not found".to_string())?;
        if server.workspace_path.is_some() {
            return Err("已有服务器目录不支持核心下载或初始化".into());
        }
        if server.core_source == "local_upload" {
            return Err("该服务器等待本地核心上传，不能从资源库下载核心".into());
        }
        let sources = collect_sources(&data, &server, &[]);
        (server.core, server.version, sources)
    };
    if sources.is_empty() {
        return Err(format!(
            "no download source is available for {core} {version}"
        ));
    }
    {
        let mut downloads = state.downloads.write().await;
        if downloads
            .get(server_id)
            .is_some_and(|status| is_active(status) && status.task_id != task_id)
        {
            return Err("another core download is already active".into());
        }
        downloads.insert(
            server_id.to_string(),
            DownloadStatus {
                task_id,
                phase: "resolving".into(),
                source: String::new(),
                received: 0,
                total: None,
                percent: 0,
                message: String::new(),
                cancel: cancel.clone(),
            },
        );
    }
    run_download(
        state.clone(),
        server_id.to_string(),
        core,
        version,
        sources,
        task_id,
        cancel,
    )
    .await;
    let status = state
        .downloads
        .read()
        .await
        .get(server_id)
        .cloned()
        .ok_or_else(|| "core download status disappeared".to_string())?;
    match status.phase.as_str() {
        "completed" => Ok(ProvisionCoreOutcome::Installed),
        "cancelled" => Ok(ProvisionCoreOutcome::Cancelled),
        _ => Err(if status.message.is_empty() {
            "core download failed".into()
        } else {
            status.message
        }),
    }
}

fn is_retryable_failure(kind: FailureKind) -> bool {
    match kind {
        FailureKind::Transport | FailureKind::Timeout => true,
        FailureKind::Http(status) => {
            status == StatusCode::REQUEST_TIMEOUT
                || status == StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
        }
        FailureKind::Protocol | FailureKind::Integrity | FailureKind::LocalIo => false,
    }
}

fn reqwest_failure(error: reqwest::Error) -> AttemptError {
    let kind = if error.is_timeout() {
        FailureKind::Timeout
    } else if error.is_builder() || error.is_redirect() {
        FailureKind::Protocol
    } else if error.is_connect() || error.is_request() || error.is_body() || error.is_decode() {
        FailureKind::Transport
    } else if let Some(status) = error.status() {
        FailureKind::Http(status)
    } else {
        FailureKind::Transport
    };
    AttemptError::failed(error.to_string(), kind)
}

fn parse_content_range(value: &str) -> Result<ParsedContentRange, String> {
    let (unit, value) = value
        .trim()
        .split_once(' ')
        .ok_or_else(|| "Content-Range 格式无效".to_string())?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return Err("Content-Range 单位不是 bytes".into());
    }
    let (range, total) = value
        .split_once('/')
        .ok_or_else(|| "Content-Range 缺少总长度".to_string())?;
    let total = if total == "*" {
        None
    } else {
        Some(
            total
                .parse::<u64>()
                .map_err(|_| "Content-Range 总长度无效".to_string())?,
        )
    };
    if range == "*" {
        return Ok(ParsedContentRange::Unsatisfied { total });
    }
    let (start, end) = range
        .split_once('-')
        .ok_or_else(|| "Content-Range 字节范围无效".to_string())?;
    let start = start
        .parse::<u64>()
        .map_err(|_| "Content-Range 起点无效".to_string())?;
    let end = end
        .parse::<u64>()
        .map_err(|_| "Content-Range 终点无效".to_string())?;
    if start > end || total.is_some_and(|total| total == 0 || end >= total) {
        return Err("Content-Range 字节范围越界".into());
    }
    Ok(ParsedContentRange::Satisfied { start, end, total })
}

fn minimum_limit(first: Option<u64>, second: Option<u64>) -> Option<u64> {
    match (first, second) {
        (Some(first), Some(second)) => Some(first.min(second)),
        (Some(limit), None) | (None, Some(limit)) => Some(limit),
        (None, None) => None,
    }
}

fn decide_response(
    status: StatusCode,
    requested_offset: u64,
    content_range: Option<&str>,
    content_length: Option<u64>,
    expected_size: Option<u64>,
) -> Result<ResponseDecision, AttemptError> {
    if status == StatusCode::RANGE_NOT_SATISFIABLE && requested_offset > 0 {
        let parsed = content_range
            .ok_or_else(|| {
                AttemptError::failed("416 响应缺少 Content-Range", FailureKind::Http(status))
            })
            .and_then(|value| {
                parse_content_range(value)
                    .map_err(|error| AttemptError::failed(error, FailureKind::Protocol))
            })?;
        if let ParsedContentRange::Unsatisfied { total: Some(total) } = parsed
            && total == requested_offset
            && expected_size.is_none_or(|expected| total == expected)
        {
            return Ok(ResponseDecision::AlreadyComplete {
                total: expected_size.unwrap_or(total),
            });
        }
    }

    if status == StatusCode::PARTIAL_CONTENT {
        let parsed = content_range
            .ok_or_else(|| {
                AttemptError::failed("206 响应缺少 Content-Range", FailureKind::Protocol)
            })
            .and_then(|value| {
                parse_content_range(value)
                    .map_err(|error| AttemptError::failed(error, FailureKind::Protocol))
            })?;
        let ParsedContentRange::Satisfied { start, end, total } = parsed else {
            return Err(AttemptError::failed(
                "206 响应的 Content-Range 不包含字节范围",
                FailureKind::Protocol,
            ));
        };
        if start != requested_offset {
            return Err(AttemptError::failed(
                format!("Content-Range 起点错位：请求 {requested_offset}，响应 {start}"),
                FailureKind::Protocol,
            ));
        }
        let response_length = end - start + 1;
        if content_length.is_some_and(|length| length != response_length) {
            return Err(AttemptError::failed(
                "Content-Length 与 Content-Range 不一致",
                FailureKind::Protocol,
            ));
        }
        let response_end = end
            .checked_add(1)
            .ok_or_else(|| AttemptError::failed("响应范围溢出", FailureKind::Protocol))?;
        if expected_size.is_some_and(|expected| {
            response_end > expected || total.is_some_and(|total| total > expected)
        }) {
            return Err(AttemptError::failed(
                "响应声明的文件大小超过可信预期大小",
                FailureKind::Integrity,
            ));
        }
        return Ok(ResponseDecision::Resume {
            offset: requested_offset,
            total: expected_size.or(total).or(Some(response_end)),
            maximum_size: minimum_limit(expected_size, Some(response_end)),
            response_end,
            response_complete: total.is_none_or(|total| response_end == total),
        });
    }

    if status == StatusCode::OK {
        if expected_size
            .is_some_and(|expected| content_length.is_some_and(|length| length > expected))
        {
            return Err(AttemptError::failed(
                "响应声明的文件大小超过可信预期大小",
                FailureKind::Integrity,
            ));
        }
        return Ok(ResponseDecision::Restart {
            total: expected_size.or(content_length),
            maximum_size: minimum_limit(expected_size, content_length),
            range_ignored: requested_offset > 0,
        });
    }

    Err(AttemptError::failed(
        format!("HTTP 响应状态 {status}"),
        FailureKind::Http(status),
    ))
}

async fn wait_for_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn clear_part(part: &std::path::Path) -> Result<bool, String> {
    match fs::remove_file(part).await {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("清理临时文件失败：{error}")),
    }
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
    let directory = crate::runtime::server_directory(&server_id);
    let _ = fs::create_dir_all(&directory).await;
    let part = directory.join("server.jar.part");
    let target = directory.join("server.jar");
    let mut last_error = String::from("没有可用的下载源");

    // A part file has no persisted source identity. Only parts created during this
    // running task are safe to resume; restart recovery belongs to a later phase.
    if let Err(error) = clear_part(&part).await {
        finish_failed(&state, &server_id, task_id, error).await;
        return;
    }

    for (source_index, source) in sources.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            finish_cancelled(&state, &server_id, task_id, &part).await;
            return;
        }
        if source_index > 0 {
            let removed_part = match clear_part(&part).await {
                Ok(removed_part) => removed_part,
                Err(error) => {
                    finish_failed(&state, &server_id, task_id, error).await;
                    return;
                }
            };
            log(
                &state,
                &server_id,
                if removed_part {
                    "切换来源，已清除上一来源的临时文件".into()
                } else {
                    "切换来源".into()
                },
            )
            .await;
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

        let resolved_result = tokio::select! {
            resolved = resolve_source(&client, source, &version) => resolved,
            _ = wait_for_cancel(&cancel) => {
                finish_cancelled(&state, &server_id, task_id, &part).await;
                return;
            }
        };
        let resolved = match resolved_result {
            Ok(resolved) => resolved,
            Err(error) => {
                log(&state, &server_id, format!("{label} 不可用：{error}")).await;
                last_error = error;
                continue;
            }
        };
        for attempt in 1..=MAX_TRANSFER_ATTEMPTS {
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
                        break;
                    }
                    if let Err(error) = install_download(&part, &target).await {
                        let _ = fs::remove_file(&part).await;
                        log(&state, &server_id, error.clone()).await;
                        last_error = error;
                        break;
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
                Err(AttemptError::Failed { message, kind }) => {
                    last_error = format!("{label}：{message}");
                    if !is_retryable_failure(kind) || attempt == MAX_TRANSFER_ATTEMPTS {
                        log(&state, &server_id, format!("{label} 下载失败：{message}")).await;
                        break;
                    }
                    let delay = RETRY_BASE_DELAY * 2_u32.pow(u32::from(attempt - 1));
                    log(
                        &state,
                        &server_id,
                        format!(
                            "{label} 传输中断：{message}；{}ms 后进行第 {attempt} 次重试",
                            delay.as_millis()
                        ),
                    )
                    .await;
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = wait_for_cancel(&cancel) => {
                            finish_cancelled(&state, &server_id, task_id, &part).await;
                            return;
                        }
                    }
                }
            }
        }
    }
    let _ = fs::remove_file(&part).await;
    finish_failed(&state, &server_id, task_id, last_error).await;
}

async fn hash_existing_part(
    part: &std::path::Path,
    expected_length: u64,
    cancel: &AtomicBool,
) -> Result<Sha256, AttemptError> {
    if expected_length == 0 {
        return Ok(Sha256::new());
    }
    let mut file = fs::File::open(part).await.map_err(|error| {
        AttemptError::failed(format!("读取临时文件失败：{error}"), FailureKind::LocalIo)
    })?;
    let mut hasher = Sha256::new();
    let mut hashed = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AttemptError::Cancelled);
        }
        let read = file.read(&mut buffer).await.map_err(|error| {
            AttemptError::failed(format!("读取临时文件失败：{error}"), FailureKind::LocalIo)
        })?;
        if read == 0 {
            break;
        }
        hashed += read as u64;
        hasher.update(&buffer[..read]);
    }
    if hashed != expected_length {
        return Err(AttemptError::failed(
            "临时文件在续传准备期间发生变化",
            FailureKind::LocalIo,
        ));
    }
    Ok(hasher)
}

async fn sync_part(part: &std::path::Path) -> Result<(), AttemptError> {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(part)
        .await
        .map_err(|error| {
            AttemptError::failed(format!("打开临时文件失败：{error}"), FailureKind::LocalIo)
        })?;
    file.sync_all().await.map_err(|error| {
        AttemptError::failed(format!("同步临时文件失败：{error}"), FailureKind::LocalIo)
    })
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
    let existing = match fs::metadata(part).await {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => {
            return Err(AttemptError::failed(
                format!("读取临时文件信息失败：{error}"),
                FailureKind::LocalIo,
            ));
        }
    };
    if resolved
        .expected_size
        .is_some_and(|expected| existing > expected)
    {
        return Err(AttemptError::failed(
            "临时文件大小超过可信预期大小",
            FailureKind::Integrity,
        ));
    }
    let mut request = client.get(&resolved.url);
    if existing > 0 {
        request = request.header(RANGE, format!("bytes={existing}-"));
    }
    let mut response = tokio::select! {
        response = request.send() => response.map_err(reqwest_failure)?,
        _ = wait_for_cancel(cancel) => return Err(AttemptError::Cancelled),
    };
    let content_range = match response.headers().get(CONTENT_RANGE) {
        Some(value) => Some(value.to_str().map_err(|_| {
            AttemptError::failed("Content-Range 不是有效文本", FailureKind::Protocol)
        })?),
        None => None,
    };
    let decision = decide_response(
        response.status(),
        existing,
        content_range,
        response.content_length(),
        resolved.expected_size,
    )?;
    let (mut hasher, mut received, total, maximum_size, response_end, response_complete, mut file) =
        match decision {
            ResponseDecision::AlreadyComplete { total: _ } => {
                let hasher = hash_existing_part(part, existing, cancel).await?;
                sync_part(part).await?;
                return Ok((existing, hex_string(&hasher.finalize())));
            }
            ResponseDecision::Resume {
                offset,
                total,
                maximum_size,
                response_end,
                response_complete,
            } => {
                if offset > 0 {
                    log(
                        state,
                        server_id,
                        format!("从 {offset} 字节续传 {}", resolved.label),
                    )
                    .await;
                }
                let hasher = hash_existing_part(part, offset, cancel).await?;
                let file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(part)
                    .await
                    .map_err(|error| {
                        AttemptError::failed(
                            format!("打开临时文件失败：{error}"),
                            FailureKind::LocalIo,
                        )
                    })?;
                (
                    hasher,
                    offset,
                    total,
                    maximum_size,
                    Some(response_end),
                    response_complete,
                    file,
                )
            }
            ResponseDecision::Restart {
                total,
                maximum_size,
                range_ignored,
            } => {
                if range_ignored {
                    log(
                        state,
                        server_id,
                        "服务器不支持 Range，已安全截断并从头下载".into(),
                    )
                    .await;
                }
                let file = fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(part)
                    .await
                    .map_err(|error| {
                        AttemptError::failed(
                            format!("创建临时文件失败：{error}"),
                            FailureKind::LocalIo,
                        )
                    })?;
                (Sha256::new(), 0, total, maximum_size, None, true, file)
            }
        };

    let initial_percent = total
        .map(|total| ((received.saturating_mul(100)) / total.max(1)).min(99) as u8)
        .unwrap_or(0);
    update_status(state, server_id, |status| {
        status.phase = "downloading".into();
        status.source = resolved.label.clone();
        status.total = total;
        status.received = received;
        status.percent = initial_percent;
    })
    .await;
    if initial_percent > 0 {
        set_task_progress(state, task_id, initial_percent, None, true).await;
    }
    let mut reported = initial_percent;
    loop {
        let chunk_result = tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(60), response.chunk()) => {
                match result {
                    Ok(Ok(chunk)) => Ok(chunk),
                    Ok(Err(error)) => Err(reqwest_failure(error)),
                    Err(_) => Err(AttemptError::failed("下载超时", FailureKind::Timeout)),
                }
            }
            _ = wait_for_cancel(cancel) => Err(AttemptError::Cancelled),
        };
        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(error) => {
                file.flush().await.map_err(|flush_error| {
                    AttemptError::failed(format!("写入失败：{flush_error}"), FailureKind::LocalIo)
                })?;
                return Err(error);
            }
        };
        let Some(chunk) = chunk else { break };
        let next_received = received
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| AttemptError::failed("下载大小计数溢出", FailureKind::Integrity))?;
        if maximum_size.is_some_and(|maximum| next_received > maximum) {
            return Err(AttemptError::failed(
                "响应内容超过声明或可信预期大小",
                FailureKind::Integrity,
            ));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|error| {
            AttemptError::failed(format!("写入失败：{error}"), FailureKind::LocalIo)
        })?;
        received = next_received;
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
            set_task_progress(state, task_id, percent, None, true).await;
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
    file.flush().await.map_err(|error| {
        AttemptError::failed(format!("写入失败：{error}"), FailureKind::LocalIo)
    })?;
    file.sync_all().await.map_err(|error| {
        AttemptError::failed(format!("同步临时文件失败：{error}"), FailureKind::LocalIo)
    })?;
    drop(file);
    if received == 0 {
        return Err(AttemptError::failed("下载内容为空", FailureKind::Integrity));
    }
    if response_end.is_some_and(|end| received != end) {
        return Err(AttemptError::failed(
            "响应正文长度与 Content-Range 不一致",
            FailureKind::Transport,
        ));
    }
    if !response_complete {
        return Err(AttemptError::failed(
            "服务器仅返回了部分剩余内容",
            FailureKind::Transport,
        ));
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
            expected_size: (*expected_size > 0).then_some(*expected_size),
            expected_sha256: (!expected_sha256.is_empty()).then(|| expected_sha256.clone()),
            catalog_version_id: Some(version_id.clone()),
        }),
        Source::Mirror { name, url } => Ok(Resolved {
            label: name.clone(),
            url: url.clone(),
            expected_size: None,
            expected_sha256: None,
            catalog_version_id: None,
        }),
        Source::ResourceCenter {
            identifier,
            artifact_version,
        } => {
            let base = crate::resource_sync::resource_base_url()
                .ok_or_else(|| "资源中心地址未配置".to_string())?;
            let (project, catalog_version) = resolve_resource_center_version(
                client,
                &base,
                identifier,
                version,
                artifact_version.as_deref(),
            )
            .await?;
            let download_url = resource_center_download_url(
                &base,
                &project.slug,
                &catalog_version.version,
                &catalog_version.download_url,
            )?;
            let expected_sha256 =
                resource_integrity_sha256(&catalog_version.sha256, &catalog_version.version)?;
            Ok(Resolved {
                label: format!(
                    "资源中心（{} {}）",
                    project.display_name(),
                    catalog_version.version
                ),
                url: download_url,
                expected_size: (catalog_version.size > 0).then_some(catalog_version.size),
                expected_sha256,
                catalog_version_id: Some(catalog_version.id),
            })
        }
        Source::MslApi { project } => {
            let payload: Value = client
                .get(format!(
                    "https://api.mslmc.cn/v4/download/server/{project}/{version}"
                ))
                .query(&[("build", "latest")])
                .send()
                .await
                .and_then(|response| response.error_for_status())
                .map_err(|error| format!("MSL 镜像 API 请求失败：{error}"))?
                .json()
                .await
                .map_err(|error| format!("MSL 镜像 API 响应异常：{error}"))?;
            let (url, sha256) = parse_msl_download_payload(&payload)?;
            Ok(Resolved {
                label: format!("MSL 镜像（{project} {version}）"),
                url,
                expected_size: None,
                expected_sha256: Some(sha256),
                catalog_version_id: None,
            })
        }
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

#[derive(Debug, Clone, Deserialize)]
struct ResourceCenterProject {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: String,
}

impl ResourceCenterProject {
    fn display_name(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.slug
        } else {
            &self.name
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ResourceCenterVersion {
    #[serde(default)]
    id: String,
    version: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    minecraft_versions: Vec<String>,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    download_url: String,
    #[serde(default)]
    status: String,
}

async fn resolve_resource_center_version(
    client: &reqwest::Client,
    base: &str,
    identifier: &str,
    minecraft: &str,
    artifact_version: Option<&str>,
) -> Result<(ResourceCenterProject, ResourceCenterVersion), String> {
    let mut search_url = resource_api_url(
        base,
        &["api", "catalog", "cores"],
        &[
            ("search", identifier),
            ("minecraft", minecraft),
            ("channel", "stable"),
        ],
    )?;
    let response = client
        .get(search_url.clone())
        .send()
        .await
        .map_err(|error| format!("资源中心项目查询失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("资源中心项目查询失败：{error}"))?;
    let payload: Value = response
        .json()
        .await
        .map_err(|error| format!("资源中心项目响应异常：{error}"))?;
    let projects = parse_resource_center_projects(&payload)?;
    let normalized = identifier.trim().to_ascii_lowercase();
    let project = projects
        .iter()
        .find(|project| {
            project.slug.eq_ignore_ascii_case(&normalized)
                || project.name.eq_ignore_ascii_case(&normalized)
        })
        .cloned()
        .ok_or_else(|| format!("资源中心未找到核心 {identifier}"))?;
    if project.slug.trim().is_empty() {
        return Err(format!("资源中心返回的核心 {identifier} 缺少 slug"));
    }

    search_url = resource_api_url(
        base,
        &["api", "catalog", "cores", &project.slug, "versions"],
        &[("minecraft", minecraft), ("channel", "stable")],
    )?;
    let response = client
        .get(search_url)
        .send()
        .await
        .map_err(|error| format!("资源中心版本查询失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("资源中心版本查询失败：{error}"))?;
    let payload: Value = response
        .json()
        .await
        .map_err(|error| format!("资源中心版本响应异常：{error}"))?;
    let versions = parse_resource_center_versions(&payload)?;
    let selected = versions
        .into_iter()
        .filter(|item| item.status.is_empty() || item.status.eq_ignore_ascii_case("published"))
        .filter(|item| item.channel.is_empty() || item.channel.eq_ignore_ascii_case("stable"))
        .filter(|item| {
            item.minecraft_versions.is_empty()
                || item
                    .minecraft_versions
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(minecraft))
        })
        .find(|item| {
            artifact_version
                .is_none_or(|expected| item.version.eq_ignore_ascii_case(expected.trim()))
        })
        .ok_or_else(|| {
            if let Some(artifact_version) = artifact_version {
                format!(
                    "资源中心未找到核心 {} 的构建 {}（Minecraft {}）",
                    project.slug, artifact_version, minecraft
                )
            } else {
                format!(
                    "资源中心未找到核心 {} 兼容 Minecraft {} 的已发布版本",
                    project.slug, minecraft
                )
            }
        })?;
    Ok((project, selected))
}

fn parse_resource_center_projects(payload: &Value) -> Result<Vec<ResourceCenterProject>, String> {
    let items = payload
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| payload.get("value").and_then(Value::as_array))
        .or_else(|| payload.as_array())
        .ok_or_else(|| "资源中心项目响应缺少列表".to_string())?;
    serde_json::from_value(Value::Array(items.clone()))
        .map_err(|error| format!("资源中心项目响应格式异常：{error}"))
}

fn parse_resource_center_versions(payload: &Value) -> Result<Vec<ResourceCenterVersion>, String> {
    let items = payload
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| payload.get("value").and_then(Value::as_array))
        .or_else(|| payload.as_array())
        .ok_or_else(|| "资源中心版本响应缺少列表".to_string())?;
    serde_json::from_value(Value::Array(items.clone()))
        .map_err(|error| format!("资源中心版本响应格式异常：{error}"))
}

fn resource_api_url(base: &str, path: &[&str], query: &[(&str, &str)]) -> Result<Url, String> {
    let mut url = Url::parse(base).map_err(|error| format!("资源中心地址无效：{error}"))?;
    if !allowed_resource_url(&url) {
        return Err("资源中心地址必须使用 HTTPS（本机回环地址可使用 HTTP）".into());
    }
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "资源中心地址不支持路径拼接".to_string())?;
        segments.pop_if_empty();
        for segment in path {
            segments.push(segment);
        }
    }
    url.query_pairs_mut()
        .clear()
        .extend_pairs(query.iter().copied());
    Ok(url)
}

fn resource_center_download_url(
    base: &str,
    project: &str,
    version: &str,
    advertised_url: &str,
) -> Result<String, String> {
    if !advertised_url.trim().is_empty() {
        let url =
            Url::parse(advertised_url).map_err(|error| format!("资源中心下载地址无效：{error}"))?;
        if !allowed_resource_url(&url) {
            return Err("资源中心下载地址必须使用 HTTPS（本机回环地址可使用 HTTP）".into());
        }
        return Ok(url.to_string());
    }
    Ok(resource_api_url(
        base,
        &["api", "v1", "download", "core", project, version],
        &[],
    )?
    .to_string())
}

fn allowed_resource_url(url: &Url) -> bool {
    if url.scheme() == "https" {
        return url.host_str().is_some();
    }
    url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        })
}

fn valid_sha256(value: &str) -> Option<String> {
    let value = value.trim();
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn resource_integrity_sha256(value: &str, version: &str) -> Result<Option<String>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    valid_sha256(value)
        .map(Some)
        .ok_or_else(|| format!("资源中心版本 {version} 返回了无效 SHA-256"))
}

fn source_label(source: &Source) -> String {
    match source {
        Source::Catalog { version, .. } => format!("资源目录（{version}）"),
        Source::Mirror { name, .. } => name.clone(),
        Source::ResourceCenter { identifier, .. } => format!("资源中心（{identifier}）"),
        Source::MslApi { project } => format!("MSL 镜像（{project}）"),
        Source::PaperApi { project } => format!("PaperMC 官方源（{project}）"),
        Source::PurpurApi => "PurpurMC 官方源".into(),
    }
}

fn parse_msl_download_payload(payload: &Value) -> Result<(String, String), String> {
    if payload.get("code").and_then(Value::as_i64) != Some(200) {
        return Err(payload
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or("MSL 镜像没有可用构建")
            .into());
    }
    let data = payload
        .get("data")
        .ok_or_else(|| "MSL 镜像响应缺少 data".to_string())?;
    let url = data
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| "MSL 镜像响应缺少下载地址".to_string())?;
    let parsed = Url::parse(url).map_err(|error| format!("MSL 下载地址无效：{error}"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err("MSL 下载地址必须使用 HTTPS".into());
    }
    let sha256 = data
        .get("sha256")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|sha256| sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "MSL 镜像响应缺少有效 SHA-256".to_string())?;
    Ok((url.to_string(), sha256.to_ascii_lowercase()))
}

fn placeholder_url(value: &str) -> bool {
    const RESERVED_HOST: &str = "example.com";
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == RESERVED_HOST || host.ends_with(".example.com"))
}

pub(crate) async fn install_download(
    part: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), String> {
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
        catalog::record_core_download(&mut data.catalog, version_id, size, sha256);
    }
    let standalone_download = data
        .tasks
        .iter()
        .any(|task| task.id == task_id && task.kind == "download");
    if let Some(task) = data
        .tasks
        .iter_mut()
        .find(|task| task.id == task_id && task.kind == "download")
    {
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
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == server_id)
    {
        server.core_ready = true;
        server.operation_state = if standalone_download {
            "idle"
        } else {
            "provisioning"
        }
        .into();
        server.last_error = None;
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
    let core_ready = fs::metadata(crate::runtime::server_directory(server_id).join("server.jar"))
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
    let mut data = state.inner.write().await;
    let standalone_download = data
        .tasks
        .iter()
        .any(|task| task.id == task_id && task.kind == "download");
    if let Some(task) = data
        .tasks
        .iter_mut()
        .find(|task| task.id == task_id && task.kind == "download")
    {
        task.status = "failed".into();
        task.error = Some(error.clone());
        task.updated_at = Local::now().to_rfc3339();
    }
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == server_id)
    {
        server.task = "核心下载失败".into();
    }
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == server_id)
    {
        server.operation_state = if standalone_download {
            "idle"
        } else {
            "provisioning"
        }
        .into();
        server.core_ready = core_ready;
        server.last_error = Some(error);
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
    let core_ready = fs::metadata(crate::runtime::server_directory(server_id).join("server.jar"))
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
    let mut data = state.inner.write().await;
    let standalone_download = data
        .tasks
        .iter()
        .any(|task| task.id == task_id && task.kind == "download");
    if let Some(task) = data
        .tasks
        .iter_mut()
        .find(|task| task.id == task_id && task.kind == "download")
    {
        task.status = "cancelled".into();
        task.updated_at = Local::now().to_rfc3339();
    }
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == server_id)
    {
        server.task = "核心下载已取消".into();
    }
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == server_id)
    {
        server.operation_state = if standalone_download {
            "idle"
        } else {
            "provisioning"
        }
        .into();
        server.core_ready = core_ready;
        server.last_error = Some("core download cancelled".into());
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
        task.updated_at = Local::now().to_rfc3339();
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

pub(crate) fn hex_string(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push_str(&format!("{byte:02x}"));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_kind(error: AttemptError) -> FailureKind {
        match error {
            AttemptError::Failed { kind, .. } => kind,
            AttemptError::Cancelled => panic!("expected a failed attempt"),
        }
    }

    #[test]
    fn parses_satisfied_and_unsatisfied_content_ranges() {
        assert_eq!(
            parse_content_range("bytes 128-255/1024").unwrap(),
            ParsedContentRange::Satisfied {
                start: 128,
                end: 255,
                total: Some(1024),
            }
        );
        assert_eq!(
            parse_content_range("bytes */1024").unwrap(),
            ParsedContentRange::Unsatisfied { total: Some(1024) }
        );
        assert!(parse_content_range("items 0-1/2").is_err());
        assert!(parse_content_range("bytes 20-10/100").is_err());
        assert!(parse_content_range("bytes 0-100/100").is_err());
    }

    #[test]
    fn response_decision_only_appends_an_aligned_partial_response() {
        assert_eq!(
            decide_response(
                StatusCode::PARTIAL_CONTENT,
                128,
                Some("bytes 128-255/1024"),
                Some(128),
                None,
            )
            .unwrap(),
            ResponseDecision::Resume {
                offset: 128,
                total: Some(1024),
                maximum_size: Some(256),
                response_end: 256,
                response_complete: false,
            }
        );

        let error = decide_response(
            StatusCode::PARTIAL_CONTENT,
            127,
            Some("bytes 128-255/1024"),
            Some(128),
            None,
        )
        .unwrap_err();
        assert_eq!(failed_kind(error), FailureKind::Protocol);

        assert_eq!(
            decide_response(
                StatusCode::RANGE_NOT_SATISFIABLE,
                1024,
                Some("bytes */1024"),
                None,
                Some(1024),
            )
            .unwrap(),
            ResponseDecision::AlreadyComplete { total: 1024 }
        );
    }

    #[test]
    fn response_decision_restarts_when_range_is_ignored_and_rejects_overruns() {
        assert_eq!(
            decide_response(StatusCode::OK, 128, None, Some(1024), None).unwrap(),
            ResponseDecision::Restart {
                total: Some(1024),
                maximum_size: Some(1024),
                range_ignored: true,
            }
        );

        let error = decide_response(StatusCode::OK, 0, None, Some(1025), Some(1024)).unwrap_err();
        assert_eq!(failed_kind(error), FailureKind::Integrity);
    }

    #[test]
    fn retry_classification_is_limited_to_transient_failures() {
        assert!(is_retryable_failure(FailureKind::Transport));
        assert!(is_retryable_failure(FailureKind::Timeout));
        assert!(is_retryable_failure(FailureKind::Http(
            StatusCode::REQUEST_TIMEOUT
        )));
        assert!(is_retryable_failure(FailureKind::Http(
            StatusCode::TOO_MANY_REQUESTS
        )));
        assert!(is_retryable_failure(FailureKind::Http(
            StatusCode::BAD_GATEWAY
        )));
        assert!(!is_retryable_failure(FailureKind::Http(
            StatusCode::NOT_FOUND
        )));
        assert!(!is_retryable_failure(FailureKind::Integrity));
        assert!(!is_retryable_failure(FailureKind::Protocol));
        assert!(!is_retryable_failure(FailureKind::LocalIo));
    }

    #[test]
    fn msl_payload_requires_https_and_sha256() {
        let payload = serde_json::json!({
            "code": 200,
            "data": {
                "url": "https://file.mslmc.cn/servers/paper/paper-26.2.jar",
                "sha256": "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"
            }
        });
        let (url, sha256) = parse_msl_download_payload(&payload).unwrap();
        assert_eq!(url, "https://file.mslmc.cn/servers/paper/paper-26.2.jar");
        assert_eq!(
            sha256,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );

        let mut insecure = payload.clone();
        insecure["data"]["url"] = serde_json::json!("http://file.mslmc.cn/server.jar");
        assert!(parse_msl_download_payload(&insecure).is_err());
    }

    #[test]
    fn resource_center_payloads_keep_branch_slug_and_integrity_metadata() {
        let projects = serde_json::json!([
            {
                "slug": "lsqfk",
                "name": "leavesslientqaqfork"
            }
        ]);
        let projects = parse_resource_center_projects(&projects).unwrap();
        assert_eq!(projects[0].slug, "lsqfk");
        assert_eq!(projects[0].display_name(), "leavesslientqaqfork");

        let versions = serde_json::json!([
            {
                "id": "remote-lsqfk-26.2-r1",
                "version": "26.2-r1",
                "channel": "stable",
                "minecraft_versions": ["26.2"],
                "size": 62847572,
                "sha256": "7373464cda4f004bbb1d12886e0a56467a9416c564b78ba5c749848ead57e185",
                "download_url": "https://res.mcmy.love/objects/cores/lsqfk/26.2-r1/server.jar",
                "status": "published"
            }
        ]);
        let versions = parse_resource_center_versions(&versions).unwrap();
        assert_eq!(versions[0].version, "26.2-r1");
        assert_eq!(valid_sha256(&versions[0].sha256).unwrap().len(), 64);
        assert!(
            resource_integrity_sha256(&versions[0].sha256, "26.2-r1")
                .unwrap()
                .is_some()
        );
        assert!(resource_integrity_sha256("not-a-sha", "26.2-r1").is_err());
        let url = resource_center_download_url(
            "https://res.mcmy.love",
            "lsqfk",
            "26.2-r1",
            &versions[0].download_url,
        )
        .unwrap();
        assert!(url.ends_with("/server.jar"));
    }

    #[test]
    fn resource_center_urls_encode_queries_and_reject_insecure_public_hosts() {
        let url = resource_api_url(
            "https://res.mcmy.love",
            &["api", "catalog", "cores"],
            &[("search", "leaves silent"), ("minecraft", "26.2")],
        )
        .unwrap();
        assert_eq!(url.path(), "/api/catalog/cores");
        assert_eq!(
            url.query_pairs()
                .find(|(key, _)| key == "search")
                .map(|(_, value)| value.into_owned()),
            Some("leaves silent".to_string())
        );
        assert!(resource_api_url("http://resource.example.com", &["api"], &[]).is_err());
        assert!(resource_api_url("http://127.0.0.1:8787", &["api"], &[]).is_ok());
    }

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

    #[tokio::test]
    async fn catalog_source_treats_unknown_integrity_metadata_as_optional() {
        let source = Source::Catalog {
            version_id: "mirror-pending-metadata".into(),
            version: "1.20.4".into(),
            url: "https://downloads.example.test/server.jar".into(),
            expected_size: 0,
            expected_sha256: String::new(),
        };
        let client = reqwest::Client::new();
        let resolved = resolve_source(&client, &source, "1.20.4").await.unwrap();

        assert_eq!(resolved.expected_size, None);
        assert_eq!(resolved.expected_sha256, None);
        assert_eq!(
            resolved.catalog_version_id.as_deref(),
            Some("mirror-pending-metadata")
        );
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
