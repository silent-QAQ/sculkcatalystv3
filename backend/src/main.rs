mod acp;
mod ai;
mod bots;
mod catalog;
mod cli_tools;
mod cloud;
mod conversations;
mod download;
mod msl_sync;
mod prefs;
mod process_platform;
mod resource_catalog;
mod resource_sync;
mod runtime;
mod server_intelligence;
mod skills;
mod task_executor;
mod workspace_fs;

use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Multipart, Path, Query, State,
        rejection::JsonRejection,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderValue, Method, StatusCode, header},
    response::Response,
    routing::{delete, get, post},
};
use cap_std::fs::{Dir as CapDir, OpenOptions as CapOpenOptions};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{File as StdFile, OpenOptions as StdOpenOptions},
    io::{Read as _, Write as _},
    path::{Component, Path as StdPath, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
    sync::{Mutex, RwLock, broadcast, mpsc, oneshot, watch},
    time::{Duration, Instant, timeout},
};
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    inner: Arc<RwLock<PersistedState>>,
    file: PathBuf,
    _file_lock: Arc<StdFile>,
    processes: Arc<RwLock<HashMap<String, ManagedProcess>>>,
    shutting_down: Arc<AtomicBool>,
    operation_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<String>>>>,
    downloads: Arc<RwLock<HashMap<String, download::DownloadStatus>>>,
    runtime_install: Arc<Mutex<()>>,
    task_controls: Arc<RwLock<HashMap<Uuid, Arc<AtomicBool>>>>,
    cloud: cloud::CloudRuntime,
}
#[derive(Clone)]
struct ManagedProcess {
    generation: Uuid,
    pid: u32,
    guard: Arc<process_platform::ProcessGuard>,
    control: mpsc::Sender<ProcessCommand>,
    exit: watch::Receiver<Option<ProcessExit>>,
}
enum ProcessCommand {
    WriteLine {
        line: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    GracefulStop {
        reply: oneshot::Sender<Result<(), String>>,
    },
    ForceKill {
        startup_timeout: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
}
#[derive(Clone, Debug)]
struct ProcessExit {
    success: bool,
    code: Option<i32>,
    forced: bool,
    startup_timeout: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ServerInfo {
    pub(crate) id: String,
    #[serde(default = "default_workspace_kind")]
    pub(crate) kind: String,
    name: String,
    core: String,
    version: String,
    pub(crate) status: String,
    players: String,
    /// Resident set size in MiB while the process is running.
    memory: u64,
    #[serde(default = "default_memory_gb")]
    memory_gb: u8,
    cpu: u8,
    port: u16,
    task: String,
    #[serde(default = "default_location")]
    location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_generation: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(default = "default_operation_state")]
    pub(crate) operation_state: String,
    #[serde(default)]
    pub(crate) core_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskEvent {
    at: String,
    level: String,
    message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskArtifact {
    id: String,
    name: String,
    kind: String,
    size: u64,
    created_at: String,
    relative_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskRollback {
    status: String,
    previous_server_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TaskInfo {
    id: Uuid,
    server_id: String,
    title: String,
    kind: String,
    status: String,
    progress: u8,
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default = "default_risk")]
    risk: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    approved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default)]
    events: Vec<TaskEvent>,
    #[serde(default)]
    artifacts: Vec<TaskArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rollback: Option<TaskRollback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_task_id: Option<Uuid>,
}
#[derive(Clone, Serialize, Deserialize)]
struct MirrorInfo {
    id: String,
    name: String,
    base_url: String,
    enabled: bool,
    priority: u8,
    cores: Vec<String>,
    region: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct PlayerInfo {
    id: Uuid,
    server_id: String,
    name: String,
    status: String,
    role: String,
    balance: i64,
    playtime_hours: u32,
    ping: u16,
    joined_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct FeedbackInfo {
    id: Uuid,
    server_id: String,
    player: String,
    content: String,
    category: String,
    sentiment: String,
    status: String,
    created_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct PollOption {
    id: Uuid,
    label: String,
    votes: u32,
}
#[derive(Clone, Serialize, Deserialize)]
struct PollInfo {
    id: Uuid,
    server_id: String,
    title: String,
    status: String,
    options: Vec<PollOption>,
    closes_at: String,
    created_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct IntegrationInfo {
    id: String,
    name: String,
    kind: String,
    status: String,
    enabled: bool,
    endpoint: String,
    latency_ms: Option<u32>,
    capabilities: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct SkillInfo {
    id: String,
    name: String,
    description: String,
    source: String,
    enabled: bool,
    version: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PersistedState {
    pub(crate) servers: Vec<ServerInfo>,
    tasks: Vec<TaskInfo>,
    configs: HashMap<String, String>,
    logs: HashMap<String, Vec<String>>,
    #[serde(default = "seed_mirrors")]
    mirrors: Vec<MirrorInfo>,
    #[serde(default = "seed_players")]
    players: Vec<PlayerInfo>,
    #[serde(default = "seed_feedback")]
    feedback: Vec<FeedbackInfo>,
    #[serde(default = "seed_polls")]
    polls: Vec<PollInfo>,
    #[serde(default = "seed_integrations")]
    integrations: Vec<IntegrationInfo>,
    #[serde(default = "seed_skills")]
    skills: Vec<SkillInfo>,
    #[serde(default = "catalog::seed_catalog")]
    catalog: catalog::CatalogState,
    #[serde(default)]
    ai: ai::AiSettings,
    #[serde(default)]
    ui: prefs::UiSettings,
    #[serde(default)]
    resource_sync: resource_sync::ResourceSyncState,
    #[serde(default)]
    bots: bots::BotState,
    #[serde(default)]
    pub(crate) conversations: Vec<conversations::Conversation>,
}
#[derive(Serialize)]
struct DashboardResponse {
    servers: Vec<ServerInfo>,
    tasks: Vec<TaskInfo>,
    agent_status: &'static str,
    mcp_connected: bool,
}
#[derive(Deserialize)]
struct ChatRequest {
    server_id: String,
    message: String,
}
#[derive(Serialize)]
struct ChatResponse {
    id: Uuid,
    message: String,
    time: String,
    actions: Vec<String>,
    task: Option<TaskInfo>,
}
#[derive(Deserialize)]
struct ActionRequest {
    action: String,
}
#[derive(Serialize)]
struct ActionResponse {
    server: ServerInfo,
    log: String,
}
#[derive(Deserialize)]
struct CommandRequest {
    command: String,
}
#[derive(Serialize)]
struct CommandResponse {
    lines: Vec<String>,
}
#[derive(Deserialize)]
struct ConfigUpdate {
    content: String,
}
#[derive(Serialize)]
struct ConfigResponse {
    content: String,
    updated_at: String,
}
#[derive(Deserialize)]
struct InstallJavaRequest {
    major: u32,
}
#[derive(Deserialize)]
struct CreateServerRequest {
    name: String,
    core: String,
    version: String,
    memory_gb: u8,
    port: u16,
    eula_accepted: bool,
    #[serde(default)]
    location: Option<String>,
}
#[derive(Deserialize)]
struct CreateProjectRequest {
    name: String,
    #[serde(default)]
    location: Option<String>,
}
#[derive(Serialize)]
struct CreateProjectResponse {
    project: ServerInfo,
    directory: String,
    files: Vec<String>,
}
#[derive(Deserialize)]
struct PlanServerRequest {
    name: String,
    #[serde(default)]
    location: Option<String>,
}
#[derive(Serialize)]
struct PlanServerResponse {
    server: ServerInfo,
    conversation: conversations::Conversation,
}
#[derive(Deserialize)]
struct DeleteServerRequest {
    #[serde(default)]
    delete_files: bool,
    #[serde(default)]
    confirmation: Option<String>,
}
#[derive(Serialize)]
struct DeleteServerResponse {
    id: String,
    removed_files: bool,
}
#[derive(Serialize)]
struct CreateServerResponse {
    server: ServerInfo,
    provision_task: TaskInfo,
    directory: String,
    files: Vec<String>,
}
#[derive(Serialize)]
struct ProvisionServerResponse {
    server: ServerInfo,
    provision_task: TaskInfo,
}
#[derive(Serialize)]
struct LogsResponse {
    lines: Vec<String>,
}
#[derive(Deserialize)]
struct DownloadPreviewRequest {
    core: String,
    version: String,
    mirror_ids: Vec<String>,
}
#[derive(Serialize)]
struct DownloadCandidate {
    mirror_id: String,
    mirror_name: String,
    url: String,
    priority: u8,
    region: String,
    supported: bool,
}
#[derive(Serialize)]
struct DownloadPreviewResponse {
    core: String,
    version: String,
    filename: String,
    candidates: Vec<DownloadCandidate>,
    strategy: &'static str,
}
#[derive(Serialize)]
struct AutomationResponse {
    tasks: Vec<TaskInfo>,
    approvals_required: usize,
    running: usize,
}
#[derive(Deserialize)]
struct CreateAutomationTask {
    server_id: String,
    title: String,
    kind: String,
    risk: String,
}

fn new_task_record(
    server_id: String,
    title: String,
    kind: String,
    status: String,
    progress: u8,
    risk: String,
    approved_by: Option<String>,
) -> TaskInfo {
    let now = Local::now().to_rfc3339();
    TaskInfo {
        id: Uuid::new_v4(),
        server_id,
        title,
        kind,
        status,
        progress,
        created_at: now.clone(),
        updated_at: now,
        risk,
        approved_by,
        started_at: None,
        finished_at: None,
        summary: None,
        error: None,
        events: Vec::new(),
        artifacts: Vec::new(),
        rollback: None,
        parent_task_id: None,
    }
}

fn trim_task_history(tasks: &mut Vec<TaskInfo>, terminal_limit: usize) {
    let mut terminal_seen = 0usize;
    tasks.retain(|task| {
        let terminal = matches!(
            task.status.as_str(),
            "completed" | "failed" | "cancelled" | "interrupted" | "rollback_failed"
        );
        if terminal {
            terminal_seen += 1;
            terminal_seen <= terminal_limit
        } else {
            true
        }
    });
}
#[derive(Serialize)]
struct CommunityResponse {
    players: Vec<PlayerInfo>,
    feedback: Vec<FeedbackInfo>,
    polls: Vec<PollInfo>,
    total_balance: i64,
    online_players: usize,
    inflation_rate: f32,
}
#[derive(Deserialize)]
struct CreatePollRequest {
    server_id: String,
    title: String,
    options: Vec<String>,
}
#[derive(Deserialize)]
struct VoteRequest {
    option_id: Uuid,
}
#[derive(Serialize)]
struct FeedbackCluster {
    categories: HashMap<String, usize>,
    positive: usize,
    neutral: usize,
    negative: usize,
    summary: String,
}
#[derive(Deserialize)]
struct PlayerActionRequest {
    action: String,
    reason: String,
    execute: bool,
}
#[derive(Serialize)]
struct PlayerActionResponse {
    player: PlayerInfo,
    action: String,
    preview: bool,
    message: String,
}
#[derive(Serialize)]
struct IntegrationsResponse {
    integrations: Vec<IntegrationInfo>,
    skills: Vec<SkillInfo>,
}
#[derive(Deserialize)]
struct FileQuery {
    path: Option<String>,
}
#[derive(Serialize)]
struct FileEntry {
    name: String,
    path: String,
    kind: String,
    size: u64,
    modified: Option<u64>,
}
#[derive(Serialize)]
struct FileListResponse {
    path: String,
    parent: Option<String>,
    entries: Vec<FileEntry>,
}
#[derive(Serialize)]
struct FileContentResponse {
    path: String,
    content: String,
    size: u64,
    readonly: bool,
}
#[derive(Deserialize)]
struct WriteFileRequest {
    path: String,
    content: String,
}
#[derive(Deserialize)]
struct CreateDirectoryRequest {
    path: String,
}
#[derive(Deserialize)]
struct RenameFileRequest {
    path: String,
    new_path: String,
}
#[derive(Deserialize)]
struct DeleteFileRequest {
    path: String,
    #[serde(default)]
    recursive: bool,
}
#[derive(Debug, Serialize)]
struct RenamedFileResponse {
    path: String,
    new_path: String,
    kind: String,
}
#[derive(Debug, Serialize)]
struct DeletedFileResponse {
    path: String,
    kind: String,
}
#[derive(Debug, Serialize)]
struct FileTransferResponse {
    path: String,
    size: u64,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

const MAX_FILE_TRANSFER_BYTES: usize = 256 * 1024 * 1024;

#[tokio::main]
async fn main() {
    let file = runtime::state_file();
    let file_lock = acquire_state_lock(&file).unwrap_or_else(|error| {
        panic!(
            "failed to lock state file {}: {error}. Another backend may already be using it; set SCULK_STATE_FILE to use an isolated state.",
            file.display()
        )
    });
    let initial = load_state(&file).await;
    let cloud = cloud::CloudRuntime::from_env().await;
    let state = AppState {
        inner: Arc::new(RwLock::new(initial)),
        file,
        _file_lock: Arc::new(file_lock),
        processes: Arc::new(RwLock::new(HashMap::new())),
        shutting_down: Arc::new(AtomicBool::new(false)),
        operation_locks: Arc::new(Mutex::new(HashMap::new())),
        channels: Arc::new(RwLock::new(HashMap::new())),
        downloads: Arc::new(RwLock::new(HashMap::new())),
        runtime_install: Arc::new(Mutex::new(())),
        task_controls: Arc::new(RwLock::new(HashMap::new())),
        cloud,
    };
    resource_sync::spawn_worker(state.clone());
    msl_sync::spawn_worker(state.clone());
    task_executor::resume_queued(state.clone());
    // 默认仅允许本机前端；云端部署可通过环境变量追加精确来源。
    let configured_origins: Arc<Vec<String>> = Arc::new(
        std::env::var("SCULK_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect(),
    );
    let origins = configured_origins.clone();
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(
            move |origin: &HeaderValue, _| {
                origin
                    .to_str()
                    .map(|value| {
                        value.starts_with("http://127.0.0.1:")
                            || value.starts_with("http://localhost:")
                            || origins.iter().any(|allowed| allowed == value)
                    })
                    .unwrap_or(false)
            },
        ))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any);
    let static_dir = PathBuf::from(
        std::env::var("SCULK_STATIC_DIR").unwrap_or_else(|_| "../frontend/dist".into()),
    );
    let resource_object_dir = catalog::resource_object_dir();
    let app = Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/dashboard", get(get_dashboard))
        .route("/api/chat", post(chat))
        .route("/api/system", get(get_system_info))
        .route("/api/runtime/java/install", post(install_java_runtime))
        .route("/api/servers", post(create_server))
        .route("/api/projects", post(create_project))
        .route("/api/servers/plan", post(plan_server))
        .route("/api/servers/{id}", delete(delete_server))
        .route("/api/servers/{id}/provision", post(provision_server))
        .route("/api/servers/{id}/action", post(server_action))
        .route("/api/servers/{id}/command", post(run_command))
        .route(
            "/api/servers/{id}/config",
            get(get_config).put(update_config),
        )
        .route("/api/servers/{id}/logs", get(get_logs))
        .route("/api/servers/{id}/ws/logs", get(ws_logs))
        .route("/api/servers/{id}/files", get(list_files))
        .route(
            "/api/servers/{id}/file",
            get(read_file).put(write_file).delete(delete_file),
        )
        .route(
            "/api/servers/{id}/file/upload",
            post(upload_file).layer(DefaultBodyLimit::max(MAX_FILE_TRANSFER_BYTES + 1024 * 1024)),
        )
        .route("/api/servers/{id}/file/download", get(download_file))
        .route("/api/servers/{id}/file/rename", post(rename_file))
        .route("/api/servers/{id}/directory", post(create_directory))
        .route("/api/download/mirrors", get(get_mirrors))
        .route("/api/download/preview", post(preview_downloads))
        .route("/api/automation", get(get_automation))
        .route("/api/automation/tasks", post(create_automation_task))
        .route("/api/tasks/{id}/approve", post(approve_task))
        .route("/api/tasks/{id}/cancel", post(cancel_task))
        .route("/api/tasks/{id}/rollback", post(rollback_task))
        .route(
            "/api/tasks/{id}/artifacts/{artifact_id}",
            get(task_executor::get_artifact),
        )
        .route("/api/community", get(get_community))
        .route("/api/polls", post(create_poll))
        .route("/api/polls/{id}/vote", post(vote_poll))
        .route("/api/feedback/cluster", post(cluster_feedback))
        .route("/api/players/{id}/action", post(player_action))
        .route("/api/integrations", get(get_integrations))
        .route("/api/integrations/{id}/toggle", post(toggle_integration))
        .route("/api/integrations/{id}/test", post(test_integration))
        .route("/api/skills/{id}/toggle", post(toggle_skill))
        .merge(bots::router())
        .merge(catalog::router())
        .merge(cloud::router())
        .merge(download::router())
        .merge(ai::router())
        .merge(prefs::router())
        .merge(conversations::router())
        .merge(resource_sync::router())
        .merge(resource_catalog::router())
        .merge(msl_sync::router())
        .with_state(state.clone())
        .nest_service("/objects", ServeDir::new(resource_object_dir))
        .fallback_service(
            ServeDir::new(&static_dir)
                .not_found_service(ServeFile::new(static_dir.join("index.html"))),
        )
        .layer(cors);
    let bind_address =
        std::env::var("SCULK_BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .expect("failed to bind API server");
    println!("Sculk Catalyst backend listening on http://{bind_address}");
    let shutdown_state = state.clone();
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(false);
    let shutdown_task = tokio::spawn(async move {
        shutdown_signal().await;
        shutdown_state.shutting_down.store(true, Ordering::Release);
        let _ = shutdown_sender.send(true);
        shutdown_all_processes(&shutdown_state).await;
    });
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            if !*shutdown_receiver.borrow() {
                let _ = shutdown_receiver.changed().await;
            }
        })
        .await;
    if result.is_err() {
        shutdown_task.abort();
        state.shutting_down.store(true, Ordering::Release);
    }
    if let Err(error) = shutdown_task.await
        && !error.is_cancelled()
    {
        eprintln!("backend shutdown task failed: {error}");
    }
    shutdown_all_processes(&state).await;
    result.expect("API server failed");
}

async fn shutdown_all_processes(state: &AppState) {
    let ids: Vec<String> = state.processes.read().await.keys().cloned().collect();
    let mut tasks = tokio::task::JoinSet::new();
    for id in ids {
        let state = state.clone();
        tasks.spawn(async move {
            let operation = server_operation_lock(&state, &id).await;
            let _guard = operation.lock().await;
            if state.processes.read().await.contains_key(&id)
                && let Err((_, error)) = stop_server(state.clone(), id.clone(), false).await
            {
                eprintln!("failed to stop server {id} during backend shutdown: {error}");
                if let Some(process) = state.processes.read().await.get(&id).cloned() {
                    let _ = request_force_kill(&process, false).await;
                    let mut exit = process.exit.clone();
                    let _ = wait_for_process_exit(&mut exit, Duration::from_secs(8)).await;
                }
            }
        });
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            eprintln!("server shutdown task failed: {error}");
        }
    }
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                eprintln!("failed to listen for SIGINT: {error}");
            }
        }
        _ = terminate.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to listen for Ctrl+C: {error}");
    }
}

fn state_sidecar_path(file: &StdPath, suffix: &str) -> PathBuf {
    let name = file
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("state.json");
    file.with_file_name(format!("{name}{suffix}"))
}

fn state_temp_path(file: &StdPath, purpose: &str) -> PathBuf {
    state_sidecar_path(file, &format!(".{purpose}-{}.tmp", Uuid::new_v4().simple()))
}

fn acquire_state_lock(file: &StdPath) -> std::io::Result<StdFile> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock = StdOpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(state_sidecar_path(file, ".lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)?;
    Ok(lock)
}

async fn read_state(file: &StdPath) -> Result<(PersistedState, String), String> {
    let data = fs::read_to_string(file).await.map_err(|e| e.to_string())?;
    let state = serde_json::from_str::<PersistedState>(&data).map_err(|e| e.to_string())?;
    Ok((state, data))
}

async fn write_synced_temp(file: &StdPath, bytes: &[u8]) -> Result<(), String> {
    let mut handle = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(file)
        .await
        .map_err(|e| e.to_string())?;
    handle.write_all(bytes).await.map_err(|e| e.to_string())?;
    handle.flush().await.map_err(|e| e.to_string())?;
    handle.sync_all().await.map_err(|e| e.to_string())
}

async fn promote_state_temp(
    file: &StdPath,
    temp: &StdPath,
    backup_previous: bool,
) -> Result<(), String> {
    if !fs::try_exists(file).await.map_err(|e| e.to_string())? {
        return fs::rename(temp, file).await.map_err(|e| e.to_string());
    }

    let previous = if backup_previous {
        state_sidecar_path(file, ".bak")
    } else {
        state_temp_path(file, "rollback")
    };
    if backup_previous && fs::try_exists(&previous).await.map_err(|e| e.to_string())? {
        fs::remove_file(&previous)
            .await
            .map_err(|e| e.to_string())?;
    }
    fs::rename(file, &previous)
        .await
        .map_err(|e| e.to_string())?;
    if let Err(commit_error) = fs::rename(temp, file).await {
        let restore_result = fs::rename(&previous, file).await;
        let _ = fs::remove_file(temp).await;
        return match restore_result {
            Ok(()) => Err(format!(
                "state commit failed and was rolled back: {commit_error}"
            )),
            Err(restore_error) => Err(format!(
                "state commit failed: {commit_error}; rollback also failed: {restore_error}"
            )),
        };
    }
    if !backup_previous {
        let _ = fs::remove_file(previous).await;
    }
    Ok(())
}

async fn write_state_file(
    file: &StdPath,
    data: &PersistedState,
    backup_previous: bool,
) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?;
    let temp = state_temp_path(file, "write");
    if let Err(error) = write_synced_temp(&temp, &bytes).await {
        let _ = fs::remove_file(&temp).await;
        return Err(error);
    }
    let result = promote_state_temp(file, &temp, backup_previous).await;
    if result.is_err() {
        let _ = fs::remove_file(&temp).await;
    }
    result
}

async fn load_state(file: &StdPath) -> PersistedState {
    if let Ok((mut state, data)) = read_state(file).await {
        let catalog_migrated = state.catalog.migrate();
        let skills_migrated = skills::ensure_bundled_skill(&mut state.skills)
            || skills::ensure_bundled_server_skill(&mut state.skills);
        let bots_migrated = bots::ensure_defaults(&mut state.bots);
        let runtime_reconciled = reconcile_stale_runtime_state(&mut state);
        let tasks_reconciled = task_executor::reconcile_interrupted_tasks(&mut state);
        let servers_reconciled = reconcile_server_file_state(&mut state).await;
        let needs_persist = catalog_migrated
            || skills_migrated
            || bots_migrated
            || runtime_reconciled
            || tasks_reconciled
            || servers_reconciled
            || legacy_servers_missing_memory(&data);
        if needs_persist && let Err(error) = write_state_file(file, &state, true).await {
            eprintln!("failed to persist migrated state: {error}");
        }
        return state;
    }

    let backup = state_sidecar_path(file, ".bak");
    if let Ok((mut state, _)) = read_state(&backup).await {
        state.catalog.migrate();
        skills::ensure_bundled_skill(&mut state.skills);
        skills::ensure_bundled_server_skill(&mut state.skills);
        bots::ensure_defaults(&mut state.bots);
        reconcile_stale_runtime_state(&mut state);
        task_executor::reconcile_interrupted_tasks(&mut state);
        reconcile_server_file_state(&mut state).await;
        if let Err(error) = write_state_file(file, &state, false).await {
            eprintln!("failed to restore state backup: {error}");
        } else {
            eprintln!("restored state from {}", backup.display());
        }
        return state;
    }

    let state = initial_state();
    if let Err(error) = write_state_file(file, &state, false).await {
        eprintln!("failed to initialize state: {error}");
    }
    state
}

fn reconcile_stale_runtime_state(state: &mut PersistedState) -> bool {
    let mut reconciled = false;
    for server in &mut state.servers {
        if server.kind != "server" {
            continue;
        }
        let transitioning = matches!(server.operation_state.as_str(), "starting" | "stopping");
        if server.status == "online"
            || transitioning
            || server.pid.is_some()
            || server.runtime_generation.is_some()
        {
            let previous_pid = server.pid;
            server.status = "warning".into();
            server.task = "上次后端退出后运行状态待确认".into();
            server.cpu = 0;
            server.memory = 0;
            server.players = "0 / 60".into();
            server.pid = None;
            server.runtime_generation = None;
            server.started_at = None;
            server.operation_state = "idle".into();
            server.last_error =
                Some("Backend restarted before the server runtime state was finalized".into());
            state.logs.entry(server.id.clone()).or_default().push(format!(
                "[{} WARN]: 后端启动时发现上次运行状态未正常收尾{}；已撤销受管状态，重新启动前将检查端口占用。",
                Local::now().format("%H:%M:%S"),
                previous_pid
                    .map(|pid| format!("（原 PID {pid}）"))
                    .unwrap_or_default()
            ));
            reconciled = true;
        }
    }
    reconciled
}

async fn reconcile_server_file_state(state: &mut PersistedState) -> bool {
    let active_provisions: std::collections::HashSet<String> = state
        .tasks
        .iter()
        .filter(|task| {
            task.kind == "server_provision"
                && matches!(task.status.as_str(), "queued" | "running" | "cancelling")
        })
        .map(|task| task.server_id.clone())
        .collect();
    let mut reconciled = false;
    for server in &mut state.servers {
        if server.kind != "server" {
            continue;
        }
        let core_ready = fs::metadata(runtime::server_directory(&server.id).join("server.jar"))
            .await
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
        reconciled |= repair_server_operation_metadata(
            server,
            core_ready,
            active_provisions.contains(&server.id),
        );
    }
    reconciled
}

fn repair_server_operation_metadata(
    server: &mut ServerInfo,
    core_ready: bool,
    active_provision: bool,
) -> bool {
    let previous = (
        server.operation_state.clone(),
        server.core_ready,
        server.last_error.clone(),
    );
    server.core_ready = core_ready;
    if active_provision {
        server.operation_state = "provisioning".into();
        server.last_error = None;
    } else if server.operation_state == "provisioning" {
        server.operation_state = "idle".into();
        if core_ready {
            server.last_error = None;
        } else if server.last_error.is_none() {
            server.last_error =
                Some("Provisioning did not finish; retry the provisioning task".into());
        }
    } else if !matches!(
        server.operation_state.as_str(),
        "idle" | "starting" | "stopping"
    ) {
        server.operation_state = "idle".into();
    }
    previous
        != (
            server.operation_state.clone(),
            server.core_ready,
            server.last_error.clone(),
        )
}
pub(crate) async fn persist(state: &AppState, data: &PersistedState) -> Result<(), String> {
    write_state_file(&state.file, data, true).await
}

pub(crate) async fn server_operation_lock(state: &AppState, id: &str) -> Arc<Mutex<()>> {
    let mut locks = state.operation_locks.lock().await;
    locks
        .entry(id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

async fn get_system_info() -> Json<runtime::SystemInfo> {
    Json(runtime::collect_system_info(&runtime::data_root()).await)
}

async fn install_java_runtime(
    State(state): State<AppState>,
    request: Result<Json<InstallJavaRequest>, JsonRejection>,
) -> ApiResult<runtime::JavaInfo> {
    let Json(request) = request.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "请求体必须是包含 major 整数的 JSON，例如 {\"major\":21}".into(),
        )
    })?;
    if !runtime::is_supported_major(request.major) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "暂不支持安装 Java {}，当前支持 Java 8、17、{}",
                request.major,
                runtime::RECOMMENDED_JAVA
            ),
        ));
    }
    let _install_guard = state.runtime_install.try_lock().map_err(|_| {
        (
            StatusCode::CONFLICT,
            "Java 运行时正在安装，请等待当前安装完成".into(),
        )
    })?;
    runtime::install_java(&runtime::data_root(), request.major)
        .await
        .map(Json)
        .map_err(|error| {
            let status = match error.kind {
                runtime::InstallErrorKind::UnsupportedPlatform => StatusCode::NOT_IMPLEMENTED,
                runtime::InstallErrorKind::Network => StatusCode::BAD_GATEWAY,
                runtime::InstallErrorKind::Integrity
                | runtime::InstallErrorKind::Archive
                | runtime::InstallErrorKind::Validation => StatusCode::UNPROCESSABLE_ENTITY,
                runtime::InstallErrorKind::Filesystem => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, error.to_string())
        })
}
async fn create_server(
    State(state): State<AppState>,
    Json(request): Json<CreateServerRequest>,
) -> ApiResult<CreateServerResponse> {
    validate_server_name(&request.name)
        .map_err(|message| (StatusCode::BAD_REQUEST, message.into()))?;
    let name = request.name.trim().to_string();
    validate_location(request.location.as_deref())?;
    validate_server_port(request.port)
        .map_err(|message| (StatusCode::BAD_REQUEST, message.into()))?;
    if !request.eula_accepted {
        return Err((
            StatusCode::BAD_REQUEST,
            "Minecraft EULA must be accepted".into(),
        ));
    }
    let start_script = render_start_script(request.memory_gb)
        .map_err(|message| (StatusCode::BAD_REQUEST, message.into()))?;
    let shell_start_script = render_shell_start_script(request.memory_gb)
        .map_err(|message| (StatusCode::BAD_REQUEST, message.into()))?;
    let mut data = state.inner.write().await;
    validate_catalog_server_template(&data.catalog, &request.core, &request.version)
        .map_err(|message| (StatusCode::BAD_REQUEST, message.into()))?;
    if data
        .servers
        .iter()
        .any(|server| server.port != 0 && server.port == request.port)
    {
        return Err((StatusCode::CONFLICT, "server port is already in use".into()));
    }
    let id = format!(
        "server-{}",
        Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );
    let directory = runtime::server_directory(&id);
    fs::create_dir_all(directory.join("plugins"))
        .await
        .map_err(|error| internal(error.to_string()))?;
    fs::create_dir_all(directory.join("logs"))
        .await
        .map_err(|error| internal(error.to_string()))?;
    let config = format!(
        "# {}\nserver-port={}\nmax-players=60\nview-distance=10\nsimulation-distance=8\nonline-mode=true\ndifficulty=normal\npvp=true\nmotd=§3{} §8| §fPowered by Sculk Catalyst",
        name, request.port, name
    );
    fs::write(directory.join("server.properties"), &config)
        .await
        .map_err(|error| internal(error.to_string()))?;
    fs::write(directory.join("eula.txt"), "eula=true\n")
        .await
        .map_err(|error| internal(error.to_string()))?;
    fs::write(directory.join("start.ps1"), start_script)
        .await
        .map_err(|error| internal(error.to_string()))?;
    let shell_script_path = directory.join("start.sh");
    fs::write(&shell_script_path, shell_start_script)
        .await
        .map_err(|error| internal(error.to_string()))?;
    make_shell_script_executable(&shell_script_path)
        .await
        .map_err(|error| internal(error.to_string()))?;
    let server = ServerInfo {
        id: id.clone(),
        kind: "server".into(),
        name,
        core: request.core,
        version: request.version,
        status: "stopped".into(),
        players: "0 / 60".into(),
        memory: 0,
        memory_gb: request.memory_gb,
        cpu: 0,
        port: request.port,
        task: "环境初始化".into(),
        location: "local".into(),
        pid: None,
        runtime_generation: None,
        started_at: None,
        operation_state: "provisioning".into(),
        core_ready: false,
        last_error: None,
    };
    data.configs.insert(id.clone(), config);
    data.logs.insert(
        id.clone(),
        vec![format!(
            "[{} AI]: 服务器工作区已创建，等待选择核心镜像源。",
            Local::now().format("%H:%M:%S")
        )],
    );
    data.tasks.insert(
        0,
        new_task_record(
            id.clone(),
            "选择核心镜像并预览下载接口".into(),
            "server_provision".into(),
            "queued".into(),
            0,
            "low".into(),
            None,
        ),
    );
    let provision_task = data.tasks[0].clone();
    data.servers.push(server.clone());
    persist(&state, &data).await.map_err(internal)?;
    drop(data);
    task_executor::spawn(state.clone(), provision_task.id).await;
    Ok(Json(CreateServerResponse {
        server,
        provision_task,
        directory: directory.to_string_lossy().to_string(),
        files: vec![
            "server.properties".into(),
            "eula.txt".into(),
            "start.ps1".into(),
            "start.sh".into(),
            "plugins/".into(),
            "logs/".into(),
        ],
    }))
}

async fn create_project(
    State(state): State<AppState>,
    Json(request): Json<CreateProjectRequest>,
) -> ApiResult<CreateProjectResponse> {
    validate_project_name(&request.name)
        .map_err(|message| (StatusCode::BAD_REQUEST, message.into()))?;
    validate_location(request.location.as_deref())?;
    let id = format!(
        "project-{}",
        Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );
    let directory = runtime::project_directory(&id);
    create_empty_project_directory(&directory)
        .await
        .map_err(internal)?;
    let project = ServerInfo {
        id: id.clone(),
        kind: "project".into(),
        name: request.name.trim().to_string(),
        core: String::new(),
        version: String::new(),
        status: "ready".into(),
        players: "- / -".into(),
        memory: 0,
        memory_gb: DEFAULT_MEMORY_GB,
        cpu: 0,
        port: 0,
        task: "项目已就绪".into(),
        location: request.location.unwrap_or_else(default_location),
        pid: None,
        runtime_generation: None,
        started_at: None,
        operation_state: "idle".into(),
        core_ready: false,
        last_error: None,
    };
    let mut data = state.inner.write().await;
    data.servers.push(project.clone());
    if let Err(error) = persist(&state, &data).await {
        data.servers.retain(|item| item.id != id);
        drop(data);
        let _ = fs::remove_dir(&directory).await;
        return Err(internal(error));
    }
    Ok(Json(CreateProjectResponse {
        project,
        directory: directory.to_string_lossy().to_string(),
        files: Vec::new(),
    }))
}

async fn create_empty_project_directory(directory: &StdPath) -> Result<(), String> {
    fs::create_dir_all(directory)
        .await
        .map_err(|error| error.to_string())
}

fn reusable_provision_task(
    tasks: &[TaskInfo],
    server_id: &str,
    core_ready: bool,
) -> Option<TaskInfo> {
    tasks
        .iter()
        .find(|task| {
            task.server_id == server_id
                && task.kind == "server_provision"
                && matches!(task.status.as_str(), "queued" | "running" | "cancelling")
        })
        .or_else(|| {
            core_ready.then(|| {
                tasks.iter().find(|task| {
                    task.server_id == server_id
                        && task.kind == "server_provision"
                        && task.status == "completed"
                })
            })?
        })
        .cloned()
}

async fn provision_server(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<ProvisionServerResponse> {
    if state.shutting_down.load(Ordering::Acquire) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "后端正在关闭，不再接受新的初始化任务".into(),
        ));
    }

    {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == id)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        require_server_kind(&server, "provision")?;
        if let Some(task) = reusable_provision_task(&data.tasks, &id, false) {
            let should_spawn = task.status == "queued";
            drop(data);
            if should_spawn {
                task_executor::spawn(state.clone(), task.id).await;
            }
            return Ok(Json(ProvisionServerResponse {
                server,
                provision_task: task,
            }));
        }
    }

    let operation = server_operation_lock(&state, &id).await;
    let _guard = operation.lock().await;
    if state.processes.read().await.contains_key(&id) {
        return Err((
            StatusCode::CONFLICT,
            "服务器正在运行，不能重新执行初始化".into(),
        ));
    }
    if state
        .downloads
        .read()
        .await
        .get(&id)
        .is_some_and(download::is_active)
    {
        return Err((StatusCode::CONFLICT, "已有核心下载任务在进行中".into()));
    }
    let core_ready = fs::metadata(runtime::server_directory(&id).join("server.jar"))
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
    let (server, provision_task) = {
        let mut data = state.inner.write().await;
        let server_index = data
            .servers
            .iter()
            .position(|server| server.id == id)
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        data.servers[server_index].core_ready = core_ready;
        if let Some(task) = reusable_provision_task(&data.tasks, &id, core_ready) {
            if matches!(task.status.as_str(), "queued" | "running" | "cancelling") {
                data.servers[server_index].operation_state = "provisioning".into();
            } else {
                data.servers[server_index].operation_state = "idle".into();
            }
            data.servers[server_index].last_error = None;
            let server = data.servers[server_index].clone();
            persist(&state, &data).await.map_err(internal)?;
            return Ok(Json(ProvisionServerResponse {
                server,
                provision_task: task,
            }));
        }
        if matches!(
            data.servers[server_index].operation_state.as_str(),
            "starting" | "stopping"
        ) {
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "服务器操作 {} 尚未完成",
                    data.servers[server_index].operation_state
                ),
            ));
        }
        let mut task = new_task_record(
            id.clone(),
            format!(
                "初始化 {} {} 核心",
                data.servers[server_index].core, data.servers[server_index].version
            ),
            "server_provision".into(),
            "queued".into(),
            0,
            "low".into(),
            None,
        );
        task.events.push(TaskEvent {
            at: task.created_at.clone(),
            level: "info".into(),
            message: "已创建可恢复的服务器初始化任务。".into(),
        });
        data.tasks.insert(0, task.clone());
        trim_task_history(&mut data.tasks, 50);
        data.servers[server_index].operation_state = "provisioning".into();
        data.servers[server_index].last_error = None;
        data.servers[server_index].task = "初始化排队中".into();
        let server = data.servers[server_index].clone();
        persist(&state, &data).await.map_err(internal)?;
        (server, task)
    };
    task_executor::spawn(state.clone(), provision_task.id).await;
    Ok(Json(ProvisionServerResponse {
        server,
        provision_task,
    }))
}

fn validate_location(location: Option<&str>) -> Result<(), (StatusCode, String)> {
    match location {
        None | Some("local") => Ok(()),
        Some(_) => Err((
            StatusCode::BAD_REQUEST,
            "远程位置暂未支持，当前版本仅可在本机创建".into(),
        )),
    }
}
fn validate_delete_confirmation(
    delete_files: bool,
    confirmation: Option<&str>,
) -> Result<(), &'static str> {
    if delete_files && confirmation != Some("delete all") {
        Err("删除磁盘文件需要输入 delete all 确认")
    } else {
        Ok(())
    }
}
/// 智能创建：仅登记"规划中"的服务器与开服规划对话，零文件操作，核心由后续对话决定。
async fn plan_server(
    State(state): State<AppState>,
    Json(request): Json<PlanServerRequest>,
) -> ApiResult<PlanServerResponse> {
    validate_server_name(&request.name)
        .map_err(|message| (StatusCode::BAD_REQUEST, message.into()))?;
    let name = request.name.trim().to_string();
    validate_location(request.location.as_deref())?;
    let id = format!(
        "server-{}",
        Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );
    let server = ServerInfo {
        id: id.clone(),
        kind: "server".into(),
        name: name.clone(),
        core: String::new(),
        version: String::new(),
        status: "planning".into(),
        players: "- / -".into(),
        memory: 0,
        memory_gb: DEFAULT_MEMORY_GB,
        cpu: 0,
        port: 0,
        task: "规划中 · 等待对话确定方案".into(),
        location: "local".into(),
        pid: None,
        runtime_generation: None,
        started_at: None,
        operation_state: "idle".into(),
        core_ready: false,
        last_error: None,
    };
    let mut conversation = conversations::new_conversation(&id, Some("开服规划".into()), None);
    conversation.messages.push(conversations::assistant_message(
        &format!(
            "服务器「{name}」已进入规划模式，目前还没有创建任何文件。告诉我你的目标玩法（生存 / 插件 / 模组 / 小游戏）、预计玩家数量与版本偏好，我会为你推荐合适的服务端核心，并在方案确认后完成创建与配置。"
        ),
        Some(vec![
            "推荐适合的服务端核心".into(),
            "我想要插件生存服".into(),
            "查看主流核心对比".into(),
        ]),
    ));
    let mut data = state.inner.write().await;
    data.servers.push(server.clone());
    data.conversations.push(conversation.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(PlanServerResponse {
        server,
        conversation,
    }))
}
async fn delete_server(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<DeleteServerRequest>,
) -> ApiResult<DeleteServerResponse> {
    validate_delete_confirmation(request.delete_files, request.confirmation.as_deref())
        .map_err(|message| (StatusCode::BAD_REQUEST, message.to_string()))?;
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err((StatusCode::BAD_REQUEST, "invalid server id".into()));
    }
    let operation = server_operation_lock(&state, &id).await;
    let _guard = operation.lock().await;
    let workspace = state
        .inner
        .read()
        .await
        .servers
        .iter()
        .find(|server| server.id == id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "workspace not found".into()))?;
    if state
        .downloads
        .read()
        .await
        .get(&id)
        .is_some_and(download::is_active)
    {
        return Err((
            StatusCode::CONFLICT,
            "核心下载进行中，请先取消并等待下载任务结束".into(),
        ));
    }
    if workspace.kind == "server" && state.processes.read().await.contains_key(&id) {
        let _ = stop_server(state.clone(), id.clone(), false).await?;
    }
    {
        let mut data = state.inner.write().await;
        data.servers.retain(|server| server.id != id);
        data.tasks.retain(|task| task.server_id != id);
        data.configs.remove(&id);
        data.logs.remove(&id);
        data.conversations
            .retain(|conversation| conversation.server_id != id);
        persist(&state, &data).await.map_err(internal)?;
    }
    state.downloads.write().await.remove(&id);
    state.channels.write().await.remove(&id);
    let mut removed_files = false;
    if request.delete_files {
        let base = runtime::data_root().join(if workspace.kind == "project" {
            "projects"
        } else {
            "servers"
        });
        let target = base.join(&id);
        if fs::metadata(&target).await.is_ok() {
            let canonical_base = fs::canonicalize(&base)
                .await
                .map_err(|error| internal(error.to_string()))?;
            let canonical_target = fs::canonicalize(&target)
                .await
                .map_err(|error| internal(error.to_string()))?;
            if !canonical_target.starts_with(&canonical_base) {
                return Err((StatusCode::BAD_REQUEST, "invalid server directory".into()));
            }
            fs::remove_dir_all(&canonical_target)
                .await
                .map_err(|error| {
                    internal(format!(
                        "服务器已从列表删除，但文件删除失败，请手动清理：{error}"
                    ))
                })?;
            removed_files = true;
        }
    }
    state.operation_locks.lock().await.remove(&id);
    Ok(Json(DeleteServerResponse { id, removed_files }))
}
async fn get_dashboard(State(state): State<AppState>) -> Json<DashboardResponse> {
    let data = state.inner.read().await;
    Json(DashboardResponse {
        servers: data.servers.clone(),
        tasks: data.tasks.clone(),
        agent_status: "ready",
        mcp_connected: true,
    })
}
async fn chat(
    State(state): State<AppState>,
    Json(request): Json<ChatRequest>,
) -> ApiResult<ChatResponse> {
    let intent = classify_intent(&request.message);
    let body = rule_reply(intent);
    let task = if task_executor::is_executable_kind(intent) {
        let mut data = state.inner.write().await;
        let risk = intent_risk(intent);
        let (status, progress, approved_by) = effective_task_start(risk, &data.ai.review_mode);
        let mut task = new_task_record(
            request.server_id.clone(),
            task_title(intent).into(),
            intent.into(),
            status.into(),
            progress,
            risk.into(),
            approved_by.map(Into::into),
        );
        task.events.push(TaskEvent {
            at: task.created_at.clone(),
            level: "info".into(),
            message: "结构化服务器操作已从对话创建。".into(),
        });
        data.tasks.insert(0, task.clone());
        trim_task_history(&mut data.tasks, 30);
        persist(&state, &data).await.map_err(internal)?;
        drop(data);
        if task.status == "queued" {
            task_executor::spawn(state.clone(), task.id).await;
        }
        Some(task)
    } else {
        None
    };
    Ok(Json(ChatResponse {
        id: Uuid::new_v4(),
        message: format!("{}\n\n目标服务器：{}", body, request.server_id),
        time: Local::now().format("%H:%M").to_string(),
        actions: task
            .as_ref()
            .map(|_| vec!["查看任务详情".into()])
            .unwrap_or_default(),
        task,
    }))
}
async fn server_action(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<ActionRequest>,
) -> ApiResult<ActionResponse> {
    perform_server_action(state, id, &request.action).await
}

fn server_start_blocker(server: &ServerInfo, tasks: &[TaskInfo]) -> Option<String> {
    let latest_provision = tasks
        .iter()
        .find(|task| task.server_id == server.id && task.kind == "server_provision");
    let provision_active = latest_provision
        .is_some_and(|task| matches!(task.status.as_str(), "queued" | "running" | "cancelling"));
    if server.operation_state != "idle" || provision_active {
        return Some(format!(
            "服务器操作 {} 尚未完成，请等待初始化或当前操作结束",
            server.operation_state
        ));
    }
    if let Some(task) = latest_provision
        && matches!(
            task.status.as_str(),
            "failed" | "cancelled" | "interrupted" | "rollback_failed"
        )
    {
        return Some(task.error.clone().unwrap_or_else(|| {
            format!(
                "最近的初始化任务状态为 {}，请重新执行初始化后再启动",
                task.status
            )
        }));
    }
    if !server.core_ready {
        return Some(
            server
                .last_error
                .clone()
                .unwrap_or_else(|| "server.jar 尚未就绪，请先执行初始化任务".into()),
        );
    }
    None
}

pub(crate) async fn perform_server_action(
    state: AppState,
    id: String,
    action: &str,
) -> ApiResult<ActionResponse> {
    {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == id)
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        require_server_kind(server, "server action")?;
    }
    if state.shutting_down.load(Ordering::Acquire) && matches!(action, "start" | "restart") {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "后端正在关闭，不再接受新的启动或重启操作".into(),
        ));
    }
    let operation = server_operation_lock(&state, &id).await;
    let _guard = operation.lock().await;
    match action {
        "start" => start_server(state, id).await,
        "stop" => stop_server(state, id, false).await,
        "force_stop" => stop_server(state, id, true).await,
        "restart" => {
            let _ = stop_server(state.clone(), id.clone(), false).await?;
            start_server(state, id).await
        }
        _ => Err((StatusCode::BAD_REQUEST, "unsupported action".into())),
    }
}
async fn start_server(state: AppState, id: String) -> ApiResult<ActionResponse> {
    if state
        .downloads
        .read()
        .await
        .get(&id)
        .is_some_and(download::is_active)
    {
        return Err((
            StatusCode::CONFLICT,
            "核心下载进行中，请等待校验和安装完成后再启动服务器".into(),
        ));
    }
    if state.processes.read().await.contains_key(&id) {
        return Err((
            StatusCode::CONFLICT,
            "server process is already running".into(),
        ));
    }
    let (server, java_args) = {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == id)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        if let Some(message) = server_start_blocker(&server, &data.tasks) {
            return Err((StatusCode::CONFLICT, message));
        }
        let java_args = server_java_args(server.memory_gb).map_err(|_| {
            (
                StatusCode::CONFLICT,
                "服务器内存配置无效，必须在 2 到 64 GB 之间".into(),
            )
        })?;
        (server, java_args)
    };
    let directory = runtime::server_directory(&id);
    if !fs::metadata(directory.join("server.jar"))
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
    {
        record_server_operation_error(&state, &id, "server.jar is missing or empty").await;
        return Err((
            StatusCode::CONFLICT,
            "server.jar 尚未就绪，请先执行初始化任务".into(),
        ));
    }
    ensure_server_port_available(server.port).await?;
    let required_java = runtime::required_java_major(&server.version);
    let java = runtime::detect_java_for_major(&runtime::data_root(), required_java).await;
    if !java.java_installed {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "未检测到 Minecraft {} 所需的 Java {}。请先执行初始化任务自动安装，或调用 POST /api/runtime/java/install",
                server.version, required_java
            ),
        ));
    }
    if !java.java_compatible {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "当前 Java {} 不兼容 Minecraft {}，需要精确的 Java {}。请先执行初始化任务安装托管运行时",
                java.java_major
                    .map(|major| major.to_string())
                    .unwrap_or_else(|| "版本未知".into()),
                server.version,
                required_java
            ),
        ));
    }
    let java_executable = java.java_executable.clone().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Java 检测结果缺少可执行文件路径".into(),
    ))?;
    let java_version = java.java_version.as_deref().unwrap_or("版本未知");
    let mut command = tokio::process::Command::new(&java_executable);
    command
        .current_dir(&directory)
        .args(java_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    process_platform::configure_managed_command(&mut command).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("无法配置 Java 进程隔离：{error}"),
        )
    })?;
    let process_guard = process_platform::create_process_guard().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("无法创建 Java 进程托管对象：{error}"),
        )
    })?;
    let mut child = command.spawn().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("无法使用已检测的 Java 启动服务器：{error}"),
        )
    })?;
    let pid = child.id().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Java 进程未返回 PID".into(),
        )
    })?;
    if let Err(error) = process_platform::bind_process_to_guard(&process_guard, pid) {
        terminate_untracked_child_with_guard(&mut child, pid, Some(&process_guard)).await;
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("无法将 Java 进程绑定到托管对象，启动已取消：{error}"),
        ));
    }
    let (stdin, stdout, stderr) =
        match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
            (Some(stdin), Some(stdout), Some(stderr)) => (stdin, stdout, stderr),
            _ => {
                terminate_untracked_child_with_guard(&mut child, pid, Some(&process_guard)).await;
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "无法连接 Java 进程的标准输入输出，已终止该进程".into(),
                ));
            }
        };
    let generation = Uuid::new_v4();
    let (control, commands) = mpsc::channel(32);
    let (exit_sender, exit) = watch::channel(None);
    let managed = ManagedProcess {
        generation,
        pid,
        guard: Arc::new(process_guard),
        control,
        exit,
    };
    let line = format!(
        "[{} INFO]: 使用 Java {}（{}）启动 {}，内存上限 {} GB，PID {}，实例 {}",
        Local::now().format("%H:%M:%S"),
        java_version,
        java_executable,
        server.name,
        server.memory_gb,
        pid,
        generation
    );
    let updated = match update_server_starting(&state, &id, generation, pid, &line).await {
        Ok(server) => server,
        Err(error) => {
            terminate_untracked_child_with_guard(&mut child, pid, Some(&managed.guard)).await;
            reset_failed_start(&state, &id, generation).await;
            return Err(internal(error));
        }
    };
    state
        .processes
        .write()
        .await
        .insert(id.clone(), managed.clone());
    tokio::spawn(run_process_actor(
        state.clone(),
        id.clone(),
        server.core.clone(),
        managed.clone(),
        child,
        stdin,
        stdout,
        stderr,
        commands,
        exit_sender,
    ));
    broadcast_line(&state, &id, &line).await;
    Ok(Json(ActionResponse {
        server: updated,
        log: line,
    }))
}

async fn update_server_starting(
    state: &AppState,
    id: &str,
    generation: Uuid,
    pid: u32,
    line: &str,
) -> Result<ServerInfo, String> {
    let mut data = state.inner.write().await;
    let server = data
        .servers
        .iter_mut()
        .find(|server| server.id == id)
        .ok_or_else(|| "server not found".to_string())?;
    server.status = "warning".into();
    server.task = "启动中".into();
    server.operation_state = "starting".into();
    server.last_error = None;
    server.pid = Some(pid);
    server.runtime_generation = Some(generation);
    server.started_at = Some(Local::now().to_rfc3339());
    let updated = server.clone();
    data.logs
        .entry(id.to_string())
        .or_default()
        .push(line.to_string());
    persist(state, &data).await?;
    Ok(updated)
}

async fn reset_failed_start(state: &AppState, id: &str, generation: Uuid) {
    let mut data = state.inner.write().await;
    if let Some(server) = data.servers.iter_mut().find(|server| server.id == id)
        && server.runtime_generation == Some(generation)
    {
        server.status = "warning".into();
        server.task = "启动状态保存失败".into();
        server.pid = None;
        server.runtime_generation = None;
        server.started_at = None;
        server.operation_state = "idle".into();
        server.last_error = Some("Failed to persist the starting runtime state".into());
    }
    let _ = persist(state, &data).await;
}

async fn record_server_operation_error(state: &AppState, id: &str, error: &str) {
    let core_ready = fs::metadata(runtime::server_directory(id).join("server.jar"))
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
    let mut data = state.inner.write().await;
    if let Some(server) = data.servers.iter_mut().find(|server| server.id == id) {
        server.operation_state = "idle".into();
        server.core_ready = core_ready;
        server.last_error = Some(error.into());
        let _ = persist(state, &data).await;
    }
}

async fn terminate_untracked_child_with_guard(
    child: &mut Child,
    pid: u32,
    guard: Option<&process_platform::ProcessGuard>,
) {
    if child.id().is_some() {
        let _ = process_platform::start_kill_tree(child, pid, guard);
    } else {
        let _ = child.start_kill();
    }
    let _ = timeout(Duration::from_secs(5), child.wait()).await;
}

async fn ensure_server_port_available(port: u16) -> Result<(), (StatusCode, String)> {
    if port == 0 {
        return Err((StatusCode::BAD_REQUEST, "服务器端口无效".into()));
    }
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|error| {
            (
                StatusCode::CONFLICT,
                format!("服务器端口 {port} 已被占用或无法绑定：{error}"),
            )
        })?;
    drop(listener);
    Ok(())
}

async fn stop_server(
    state: AppState,
    id: String,
    force_immediately: bool,
) -> ApiResult<ActionResponse> {
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
    let Some(process) = state.processes.read().await.get(&id).cloned() else {
        let mut data = state.inner.write().await;
        let server = data
            .servers
            .iter_mut()
            .find(|server| server.id == id)
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        server.status = "stopped".into();
        server.task = "已停止".into();
        server.pid = None;
        server.runtime_generation = None;
        server.started_at = None;
        server.operation_state = "idle".into();
        server.last_error = None;
        let updated = server.clone();
        persist(&state, &data).await.map_err(internal)?;
        return Ok(Json(ActionResponse {
            server: updated,
            log: "服务器进程未运行，状态已校正为停止".into(),
        }));
    };
    let line = if force_immediately {
        format!(
            "[{} WARN]: 正在强制终止 PID {}，实例 {}。",
            Local::now().format("%H:%M:%S"),
            process.pid,
            process.generation
        )
    } else {
        format!(
            "[{} INFO]: 已向 PID {} 发送安全停服指令，等待进程真实退出。",
            Local::now().format("%H:%M:%S"),
            process.pid
        )
    };
    broadcast_line(&state, &id, &line).await;
    {
        let mut data = state.inner.write().await;
        let server = data
            .servers
            .iter_mut()
            .find(|server| server.id == id)
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        if server.runtime_generation == Some(process.generation) {
            server.status = "warning".into();
            server.task = if force_immediately {
                "正在强制停止".into()
            } else {
                "正在保存并停止".into()
            };
            server.operation_state = "stopping".into();
            server.last_error = None;
        }
        data.logs.entry(id.clone()).or_default().push(line.clone());
        persist(&state, &data).await.map_err(internal)?;
    }
    let mut exit = process.exit.clone();
    if force_immediately {
        request_force_kill(&process, false).await?;
    } else {
        request_graceful_stop(&process).await?;
        if wait_for_process_exit(&mut exit, Duration::from_secs(30))
            .await
            .is_none()
        {
            let force_line = format!(
                "[{} WARN]: 安全停服等待 30 秒超时，正在强制终止 PID {}。",
                Local::now().format("%H:%M:%S"),
                process.pid
            );
            record_process_line(&state, &id, process.generation, &force_line, false).await;
            request_force_kill(&process, false).await?;
        }
    }
    let report = wait_for_process_exit(&mut exit, Duration::from_secs(8))
        .await
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Java 进程未能在强制终止后退出".into(),
        ))?;
    let updated = state
        .inner
        .read()
        .await
        .servers
        .iter()
        .find(|server| server.id == id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
    let result_line = if report.forced {
        format!("服务器已强制停止，退出码 {:?}", report.code)
    } else {
        format!("服务器已安全停止，退出码 {:?}", report.code)
    };
    Ok(Json(ActionResponse {
        server: updated,
        log: result_line,
    }))
}

async fn request_graceful_stop(process: &ManagedProcess) -> Result<(), (StatusCode, String)> {
    let (reply, response) = oneshot::channel();
    process
        .control
        .send(ProcessCommand::GracefulStop { reply })
        .await
        .map_err(|_| (StatusCode::CONFLICT, "Java 进程控制通道已关闭".into()))?;
    timeout(Duration::from_secs(5), response)
        .await
        .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, "发送停服指令超时".into()))?
        .map_err(|_| (StatusCode::CONFLICT, "Java 进程控制任务已退出".into()))?
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))
}

async fn request_force_kill(
    process: &ManagedProcess,
    startup_timeout: bool,
) -> Result<(), (StatusCode, String)> {
    let (reply, response) = oneshot::channel();
    process
        .control
        .send(ProcessCommand::ForceKill {
            startup_timeout,
            reply,
        })
        .await
        .map_err(|_| (StatusCode::CONFLICT, "Java 进程控制通道已关闭".into()))?;
    timeout(Duration::from_secs(5), response)
        .await
        .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, "强制终止请求超时".into()))?
        .map_err(|_| (StatusCode::CONFLICT, "Java 进程控制任务已退出".into()))?
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))
}

async fn wait_for_process_exit(
    exit: &mut watch::Receiver<Option<ProcessExit>>,
    duration: Duration,
) -> Option<ProcessExit> {
    if let Some(report) = exit.borrow().clone() {
        return Some(report);
    }
    timeout(duration, async {
        loop {
            if exit.changed().await.is_err() {
                return None;
            }
            if let Some(report) = exit.borrow().clone() {
                return Some(report);
            }
        }
    })
    .await
    .ok()
    .flatten()
}

#[allow(clippy::too_many_arguments)]
async fn run_process_actor(
    state: AppState,
    id: String,
    core: String,
    managed: ManagedProcess,
    mut child: Child,
    mut stdin: ChildStdin,
    stdout: ChildStdout,
    stderr: ChildStderr,
    mut commands: mpsc::Receiver<ProcessCommand>,
    exit_sender: watch::Sender<Option<ProcessExit>>,
) {
    let mut stdout = BufReader::new(stdout).lines();
    let mut stderr = BufReader::new(stderr).lines();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut ready = false;
    let mut requested_stop = false;
    let mut forced = false;
    let mut startup_timed_out = false;
    let started = Instant::now();
    let mut system = sysinfo::System::new();
    let mut last_metrics_at = Instant::now() - Duration::from_secs(1);
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut exit_status = None;
    while exit_status.is_none() || stdout_open || stderr_open {
        tokio::select! {
            line = stdout.next_line(), if stdout_open => match line {
                Ok(Some(line)) => {
                    let online = process_ready_line(&core, &line);
                    ready |= online;
                    record_process_line(&state, &id, managed.generation, &line, online).await;
                }
                Ok(None) => stdout_open = false,
                Err(error) => {
                    stdout_open = false;
                    record_process_line(&state, &id, managed.generation, &format!("[进程输出读取失败] {error}"), false).await;
                }
            },
            line = stderr.next_line(), if stderr_open => match line {
                Ok(Some(line)) => {
                    let online = process_ready_line(&core, &line);
                    ready |= online;
                    record_process_line(&state, &id, managed.generation, &line, online).await;
                }
                Ok(None) => stderr_open = false,
                Err(error) => {
                    stderr_open = false;
                    record_process_line(&state, &id, managed.generation, &format!("[进程错误输出读取失败] {error}"), false).await;
                }
            },
            command = commands.recv(), if exit_status.is_none() => match command {
                Some(ProcessCommand::WriteLine { line, reply }) => {
                    let result = async {
                        stdin.write_all(format!("{line}\n").as_bytes()).await.map_err(|error| error.to_string())?;
                        stdin.flush().await.map_err(|error| error.to_string())
                    }.await;
                    let _ = reply.send(result);
                }
                Some(ProcessCommand::GracefulStop { reply }) => {
                    requested_stop = true;
                    let result = async {
                        stdin.write_all(b"stop\n").await.map_err(|error| error.to_string())?;
                        stdin.flush().await.map_err(|error| error.to_string())
                    }.await;
                    let _ = reply.send(result);
                }
                Some(ProcessCommand::ForceKill { startup_timeout, reply }) => {
                    requested_stop = true;
                    forced = true;
                    startup_timed_out |= startup_timeout;
                    let result = process_platform::start_kill_tree(&mut child, managed.pid, Some(&managed.guard))
                        .map_err(|error| error.to_string());
                    let _ = reply.send(result);
                }
                None => {
                    requested_stop = true;
                    forced = true;
                    let _ = process_platform::start_kill_tree(&mut child, managed.pid, Some(&managed.guard));
                }
            },
            _ = poll.tick(), if exit_status.is_none() => {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let _ = process_platform::cleanup_remaining_tree(managed.pid);
                        exit_status = Some(status);
                    }
                    Ok(None) => {
                        if last_metrics_at.elapsed() >= Duration::from_secs(1) {
                            last_metrics_at = Instant::now();
                            if let Some(metrics) =
                                runtime::sample_process_metrics(&mut system, managed.pid)
                            {
                                record_process_metrics(
                                    &state,
                                    &id,
                                    managed.generation,
                                    metrics,
                                )
                                .await;
                            }
                        }
                        if !ready && !requested_stop && started.elapsed() >= Duration::from_secs(120) {
                            requested_stop = true;
                            forced = true;
                            startup_timed_out = true;
                            record_process_line(
                                &state,
                                &id,
                                managed.generation,
                                "[启动超时] 120 秒内未检测到服务端就绪标记，正在终止进程。",
                                false,
                            ).await;
                            let _ = process_platform::start_kill_tree(&mut child, managed.pid, Some(&managed.guard));
                        }
                    }
                    Err(error) => {
                        requested_stop = true;
                        forced = true;
                        record_process_line(&state, &id, managed.generation, &format!("[进程状态检查失败] {error}"), false).await;
                        let _ = process_platform::start_kill_tree(&mut child, managed.pid, Some(&managed.guard));
                    }
                }
            },
        }
        if exit_status.is_none()
            && (!stdout_open && !stderr_open)
            && let Ok(Some(status)) = child.try_wait()
        {
            let _ = process_platform::cleanup_remaining_tree(managed.pid);
            exit_status = Some(status);
        }
    }
    let status = match exit_status {
        Some(status) => status,
        None => match child.wait().await {
            Ok(status) => status,
            Err(error) => {
                record_process_line(
                    &state,
                    &id,
                    managed.generation,
                    &format!("[等待进程退出失败] {error}"),
                    false,
                )
                .await;
                let report = ProcessExit {
                    success: false,
                    code: None,
                    forced,
                    startup_timeout: startup_timed_out,
                };
                finish_process_instance(&state, &id, managed.generation, &report, requested_stop)
                    .await;
                let _ = exit_sender.send(Some(report));
                return;
            }
        },
    };
    let report = ProcessExit {
        success: status.success(),
        code: status.code(),
        forced,
        startup_timeout: startup_timed_out,
    };
    finish_process_instance(&state, &id, managed.generation, &report, requested_stop).await;
    let _ = exit_sender.send(Some(report));
}

fn process_ready_line(_core: &str, line: &str) -> bool {
    line.contains("Done (") || line.contains("Done!") || line.contains("Listening on /")
}

async fn record_process_line(
    state: &AppState,
    id: &str,
    generation: Uuid,
    line: &str,
    online: bool,
) {
    broadcast_line(state, id, line).await;
    let mut data = state.inner.write().await;
    let current = data
        .servers
        .iter()
        .any(|server| server.id == id && server.runtime_generation == Some(generation));
    if !current {
        return;
    }
    data.logs
        .entry(id.to_string())
        .or_default()
        .push(line.to_string());
    if let Some(logs) = data.logs.get_mut(id)
        && logs.len() > 1000
    {
        logs.drain(0..logs.len() - 1000);
    }
    if online
        && let Some(server) = data
            .servers
            .iter_mut()
            .find(|server| server.id == id && server.runtime_generation == Some(generation))
    {
        server.status = "online".into();
        server.task = "运行中".into();
        server.operation_state = "idle".into();
        server.last_error = None;
    }
    let _ = persist(state, &data).await;
}

fn apply_process_metrics(
    server: &mut ServerInfo,
    id: &str,
    generation: Uuid,
    metrics: runtime::ProcessMetrics,
) -> bool {
    if server.id != id || server.runtime_generation != Some(generation) {
        return false;
    }
    server.cpu = metrics.cpu;
    server.memory = metrics.memory;
    true
}

async fn record_process_metrics(
    state: &AppState,
    id: &str,
    generation: Uuid,
    metrics: runtime::ProcessMetrics,
) {
    let mut data = state.inner.write().await;
    if !data
        .servers
        .iter_mut()
        .any(|server| apply_process_metrics(server, id, generation, metrics))
    {
        return;
    }
    let _ = persist(state, &data).await;
}

async fn finish_process_instance(
    state: &AppState,
    id: &str,
    generation: Uuid,
    report: &ProcessExit,
    requested_stop: bool,
) {
    {
        let mut processes = state.processes.write().await;
        if processes
            .get(id)
            .is_some_and(|process| process.generation == generation)
        {
            processes.remove(id);
        } else {
            return;
        }
    }
    let exit_line = format!(
        "[{} {}]: Java 进程已退出，实例 {}，退出码 {:?}{}",
        Local::now().format("%H:%M:%S"),
        if report.success { "INFO" } else { "WARN" },
        generation,
        report.code,
        if report.forced {
            "（强制终止）"
        } else {
            ""
        }
    );
    broadcast_line(state, id, &exit_line).await;
    let mut data = state.inner.write().await;
    if let Some(server) = data
        .servers
        .iter_mut()
        .find(|server| server.id == id && server.runtime_generation == Some(generation))
    {
        server.status = if report.startup_timeout || (!requested_stop && !report.success) {
            "warning".into()
        } else {
            "stopped".into()
        };
        server.cpu = 0;
        server.memory = 0;
        server.players = "0 / 60".into();
        server.task = if report.startup_timeout {
            "启动超时".into()
        } else if !requested_stop && !report.success {
            "异常退出".into()
        } else if report.forced {
            "已强制停止".into()
        } else {
            "已停止".into()
        };
        server.pid = None;
        server.runtime_generation = None;
        server.started_at = None;
        server.operation_state = "idle".into();
        server.last_error = if report.startup_timeout {
            Some("Server startup timed out".into())
        } else if !requested_stop && !report.success {
            Some(format!(
                "Server process exited unexpectedly with code {:?}",
                report.code
            ))
        } else {
            None
        };
        data.logs.entry(id.to_string()).or_default().push(exit_line);
        let _ = persist(state, &data).await;
    }
}
async fn run_command(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<CommandRequest>,
) -> ApiResult<CommandResponse> {
    let command = request.command.trim().to_string();
    if command.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "command is empty".into()));
    }
    {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == id)
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        require_server_kind(server, "server command")?;
    }
    let process = state.processes.read().await.get(&id).cloned().ok_or((
        StatusCode::CONFLICT,
        "服务器进程未运行，无法执行控制台命令".into(),
    ))?;
    let time = Local::now().format("%H:%M:%S");
    let (reply, response) = oneshot::channel();
    process
        .control
        .send(ProcessCommand::WriteLine {
            line: command.clone(),
            reply,
        })
        .await
        .map_err(|_| (StatusCode::CONFLICT, "Java 进程控制通道已关闭".into()))?;
    timeout(Duration::from_secs(5), response)
        .await
        .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, "命令发送超时".into()))?
        .map_err(|_| (StatusCode::CONFLICT, "Java 进程控制任务已退出".into()))?
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("命令写入失败：{error}"),
            )
        })?;
    let lines = vec![
        format!("> {command}"),
        format!(
            "[{time} INFO]: 命令已发送到 PID {}，输出将实时显示。",
            process.pid
        ),
    ];
    for line in &lines {
        broadcast_line(&state, &id, line).await;
    }
    let mut data = state.inner.write().await;
    data.logs.entry(id).or_default().extend(lines.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(CommandResponse { lines }))
}
async fn get_config(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<ConfigResponse> {
    let data = state.inner.read().await;
    let content = data
        .configs
        .get(&id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "config not found".into()))?;
    Ok(Json(ConfigResponse {
        content,
        updated_at: Local::now().to_rfc3339(),
    }))
}
async fn update_config(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<ConfigUpdate>,
) -> ApiResult<ConfigResponse> {
    let mut data = state.inner.write().await;
    if !data.servers.iter().any(|server| server.id == id) {
        return Err((StatusCode::NOT_FOUND, "server not found".into()));
    }
    data.configs.insert(id, request.content.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(ConfigResponse {
        content: request.content,
        updated_at: Local::now().to_rfc3339(),
    }))
}
async fn get_logs(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<LogsResponse> {
    let data = state.inner.read().await;
    if !data.servers.iter().any(|server| server.id == id) {
        return Err((StatusCode::NOT_FOUND, "server not found".into()));
    }
    Ok(Json(LogsResponse {
        lines: data.logs.get(&id).cloned().unwrap_or_default(),
    }))
}
async fn log_channel(state: &AppState, id: &str) -> broadcast::Sender<String> {
    if let Some(sender) = state.channels.read().await.get(id) {
        return sender.clone();
    }
    let mut channels = state.channels.write().await;
    channels
        .entry(id.to_string())
        .or_insert_with(|| broadcast::channel(256).0)
        .clone()
}
async fn broadcast_line(state: &AppState, id: &str, line: &str) {
    let _ = log_channel(state, id).await.send(line.to_string());
}
async fn ws_logs(
    Path(id): Path<String>,
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
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
    let receiver = log_channel(&state, &id).await.subscribe();
    let history = state
        .inner
        .read()
        .await
        .logs
        .get(&id)
        .cloned()
        .unwrap_or_default();
    Ok(upgrade.on_upgrade(move |socket| stream_logs_ws(socket, history, receiver)))
}
async fn stream_logs_ws(
    mut socket: WebSocket,
    history: Vec<String>,
    mut receiver: broadcast::Receiver<String>,
) {
    // 先同步历史日志，再持续推送新行；客户端断开或落后过多时结束会话
    for line in history.iter().rev().take(200).rev() {
        if socket
            .send(WsMessage::Text(line.clone().into()))
            .await
            .is_err()
        {
            return;
        }
    }
    loop {
        tokio::select! {
            line=receiver.recv()=>{
                match line{
                    Ok(line)=>{if socket.send(WsMessage::Text(line.into())).await.is_err(){return}},
                    Err(broadcast::error::RecvError::Lagged(_))=>{let _=socket.send(WsMessage::Text("[WS]: 日志推送滞后，部分行已跳过".into())).await;},
                    Err(broadcast::error::RecvError::Closed)=>return,
                }
            },
            message=socket.recv()=>{match message{Some(Ok(WsMessage::Close(_)))|None=>return,Some(Err(_))=>return,_=>{}}},
        }
    }
}
async fn list_files(
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
    State(state): State<AppState>,
) -> ApiResult<FileListResponse> {
    let root = ensure_workspace(&state, &id).await?;
    let relative = safe_relative(query.path.as_deref().unwrap_or(""))?;
    let mut entries = workspace_fs::within_workspace(root, {
        let relative = relative.clone();
        move |workspace| {
            let metadata = if relative.as_os_str().is_empty() {
                workspace.dir_metadata()?
            } else {
                reject_workspace_symlink(workspace, &relative)?;
                workspace.metadata(&relative)?
            };
            if !metadata.is_dir() {
                return Err(workspace_invalid_path("path is not a directory"));
            }
            let reader = if relative.as_os_str().is_empty() {
                workspace.entries()?
            } else {
                workspace.read_dir(&relative)?
            };
            let mut entries = Vec::new();
            for entry in reader {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if file_type.is_symlink() {
                    continue;
                }
                let metadata = entry.metadata()?;
                let name = entry.file_name().to_string_lossy().to_string();
                let entry_relative = relative.join(&name);
                entries.push(FileEntry {
                    name,
                    path: path_string(&entry_relative),
                    kind: if metadata.is_dir() {
                        "folder".into()
                    } else {
                        "file".into()
                    },
                    size: if metadata.is_file() {
                        metadata.len()
                    } else {
                        0
                    },
                    modified: metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.into_std().duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_secs()),
                });
            }
            Ok(entries)
        }
    })
    .await
    .map_err(workspace_io_error)?;
    entries.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then(left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    let parent = relative
        .parent()
        .map(path_string)
        .filter(|path| !path.is_empty());
    Ok(Json(FileListResponse {
        path: path_string(&relative),
        parent,
        entries,
    }))
}
async fn read_file(
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
    State(state): State<AppState>,
) -> ApiResult<FileContentResponse> {
    let requested = query
        .path
        .ok_or((StatusCode::BAD_REQUEST, "file path is required".into()))?;
    let root = ensure_workspace(&state, &id).await?;
    let relative = safe_relative(&requested)?;
    let bytes = workspace_fs::within_workspace(root, {
        let relative = relative.clone();
        move |workspace| {
            reject_workspace_symlink(workspace, &relative)?;
            let metadata = workspace.metadata(&relative)?;
            if !metadata.is_file() {
                return Err(workspace_invalid_path("path is not a file"));
            }
            let file = workspace.open(&relative)?;
            let mut bytes = Vec::with_capacity(metadata.len().min(2_000_000) as usize);
            file.take(2_000_001).read_to_end(&mut bytes)?;
            Ok(bytes)
        }
    })
    .await
    .map_err(workspace_io_error)?;
    if bytes.len() > 2_000_000 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "text file exceeds 2 MB".into(),
        ));
    }
    let content = String::from_utf8(bytes).map_err(|_| {
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "binary files cannot be opened in the text editor".into(),
        )
    })?;
    Ok(Json(FileContentResponse {
        path: path_string(&relative),
        size: content.len() as u64,
        content,
        readonly: !is_editable(&relative),
    }))
}

async fn download_file(
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
    State(state): State<AppState>,
) -> Result<Response, (StatusCode, String)> {
    let requested = query
        .path
        .ok_or((StatusCode::BAD_REQUEST, "file path is required".into()))?;
    let root = ensure_workspace(&state, &id).await?;
    let relative = safe_relative(&requested)?;
    if relative.as_os_str().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "file path is required".into()));
    }
    let bytes = workspace_fs::within_workspace(root, {
        let relative = relative.clone();
        move |workspace| {
            reject_workspace_symlink(workspace, &relative)?;
            let metadata = workspace.metadata(&relative)?;
            if !metadata.is_file() {
                return Err(workspace_invalid_path("path is not a file"));
            }
            let file = workspace.open(&relative)?;
            let mut bytes =
                Vec::with_capacity(metadata.len().min(MAX_FILE_TRANSFER_BYTES as u64) as usize);
            file.take(MAX_FILE_TRANSFER_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        }
    })
    .await
    .map_err(workspace_io_error)?;
    if bytes.len() > MAX_FILE_TRANSFER_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "file changed while it was being downloaded".into(),
        ));
    }
    let filename = safe_download_filename(&relative);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(Body::from(bytes))
        .map_err(|error| internal(error.to_string()))
}

async fn upload_file(
    Path(id): Path<String>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> ApiResult<FileTransferResponse> {
    let mut directory = None;
    let mut filename = None;
    let mut content = None;
    while let Some(field) = multipart.next_field().await.map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid multipart body: {error}"),
        )
    })? {
        let field_name = field.name().map(str::to_owned);
        match field_name.as_deref() {
            Some("path") => {
                if directory.is_some() {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "path field appears more than once".into(),
                    ));
                }
                let value = field.text().await.map_err(|error| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("invalid upload path: {error}"),
                    )
                })?;
                directory = Some(value);
            }
            Some("file") => {
                if content.is_some() {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "file field appears more than once".into(),
                    ));
                }
                let name = field.file_name().map(str::to_owned).ok_or((
                    StatusCode::BAD_REQUEST,
                    "uploaded file name is required".into(),
                ))?;
                validate_upload_filename(&name)?;
                let bytes = field.bytes().await.map_err(|error| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("unable to read upload: {error}"),
                    )
                })?;
                if bytes.len() > MAX_FILE_TRANSFER_BYTES {
                    return Err((
                        StatusCode::PAYLOAD_TOO_LARGE,
                        format!(
                            "file exceeds the {} MiB transfer limit",
                            MAX_FILE_TRANSFER_BYTES / 1024 / 1024
                        ),
                    ));
                }
                filename = Some(name);
                content = Some(bytes);
            }
            Some(other) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("unsupported multipart field: {other}"),
                ));
            }
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "multipart field name is required".into(),
                ));
            }
        }
    }

    let directory = safe_relative(directory.as_deref().unwrap_or(""))?;
    let filename = filename.ok_or((StatusCode::BAD_REQUEST, "file field is required".into()))?;
    let content = content.ok_or((StatusCode::BAD_REQUEST, "file field is required".into()))?;
    let root = ensure_workspace(&state, &id).await?;
    let relative = directory.join(&filename);
    reject_protected_server_artifact(&relative)?;

    let operation = server_operation_lock(&state, &format!("files:{id}")).await;
    let _guard = operation.lock().await;
    let content_size = content.len() as u64;
    workspace_fs::within_workspace(root, move |workspace| {
        let directory = if directory.as_os_str().is_empty() {
            workspace.try_clone()?
        } else {
            reject_workspace_symlink(workspace, &directory)?;
            let metadata = workspace.metadata(&directory)?;
            if !metadata.is_dir() {
                return Err(workspace_invalid_path("upload path is not a directory"));
            }
            workspace.open_dir(&directory)?
        };
        let temporary = PathBuf::from(format!(".sculk-upload-{}.part", Uuid::new_v4()));
        let publish = (|| {
            let mut options = CapOpenOptions::new();
            options.write(true).create_new(true);
            let mut file = directory.open_with(&temporary, &options)?;
            file.write_all(content.as_ref())?;
            file.sync_all()?;
            drop(file);
            directory.hard_link(&temporary, &directory, &filename)
        })();
        // Always remove the private staging file. A cleanup failure must not
        // turn a successfully published upload into an apparent API failure.
        let _ = directory.remove_file(&temporary);
        publish
    })
    .await
    .map_err(workspace_io_error)?;

    Ok(Json(FileTransferResponse {
        path: path_string(&relative),
        size: content_size,
    }))
}

async fn write_file(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<WriteFileRequest>,
) -> ApiResult<FileContentResponse> {
    if request.content.len() > 2_000_000 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "text file exceeds 2 MB".into(),
        ));
    }
    let root = ensure_workspace(&state, &id).await?;
    let relative = safe_relative(&request.path)?;
    reject_protected_server_artifact(&relative)?;
    if !is_editable(&relative) {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "this file type is read-only".into(),
        ));
    }
    let operation = server_operation_lock(&state, &format!("files:{id}")).await;
    let _guard = operation.lock().await;
    let content = workspace_fs::within_workspace(root, {
        let relative = relative.clone();
        let content = request.content;
        move |workspace| {
            let parent = relative.parent().unwrap_or(StdPath::new(""));
            workspace.create_dir_all(parent)?;
            match workspace.symlink_metadata(&relative) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "symbolic links are not allowed",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            workspace.write(&relative, content.as_bytes())?;
            Ok(content)
        }
    })
    .await
    .map_err(workspace_io_error)?;
    if path_string(&relative) == "server.properties" {
        let mut data = state.inner.write().await;
        data.configs.insert(id, content.clone());
        persist(&state, &data).await.map_err(internal)?
    }
    Ok(Json(FileContentResponse {
        path: path_string(&relative),
        size: content.len() as u64,
        content,
        readonly: false,
    }))
}

async fn rename_file(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<RenameFileRequest>,
) -> ApiResult<RenamedFileResponse> {
    let root = ensure_workspace(&state, &id).await?;
    let relative = safe_relative(&request.path)?;
    let new_relative = safe_relative(&request.new_path)?;
    if relative.as_os_str().is_empty() || new_relative.as_os_str().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "workspace root cannot be renamed".into(),
        ));
    }
    if relative == new_relative {
        return Err((
            StatusCode::BAD_REQUEST,
            "source and target paths are identical".into(),
        ));
    }
    if new_relative.starts_with(&relative) {
        return Err((
            StatusCode::BAD_REQUEST,
            "a directory cannot be moved inside itself".into(),
        ));
    }
    reject_protected_server_artifact(&relative)?;
    reject_protected_server_artifact(&new_relative)?;

    let operation = server_operation_lock(&state, &format!("files:{id}")).await;
    let _guard = operation.lock().await;
    let (kind, replacement_config) = workspace_fs::within_workspace(root, {
        let relative = relative.clone();
        let new_relative = new_relative.clone();
        move |workspace| {
            reject_workspace_symlink(workspace, &relative)?;
            let metadata = workspace.metadata(&relative)?;
            let kind = if metadata.is_dir() {
                "folder"
            } else if metadata.is_file() {
                "file"
            } else {
                return Err(workspace_invalid_path("unsupported workspace entry"));
            };
            let parent = new_relative.parent().unwrap_or(StdPath::new(""));
            workspace.create_dir_all(parent)?;
            match workspace.symlink_metadata(&new_relative) {
                Ok(_) => return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let replacement_config = if is_root_server_properties(&new_relative) {
                if !metadata.is_file() {
                    return Err(workspace_invalid_path(
                        "server.properties target must be a text file",
                    ));
                }
                let file = workspace.open(&relative)?;
                let mut bytes = Vec::with_capacity(metadata.len().min(2_000_000) as usize);
                file.take(2_000_001).read_to_end(&mut bytes)?;
                if bytes.len() > 2_000_000 {
                    return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
                }
                Some(String::from_utf8(bytes).map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "server.properties must contain UTF-8 text",
                    )
                })?)
            } else {
                None
            };
            workspace.rename(&relative, workspace, &new_relative)?;
            Ok((kind.to_string(), replacement_config))
        }
    })
    .await
    .map_err(workspace_io_error)?;
    sync_config_after_rename(&state, &id, &relative, &new_relative, replacement_config).await?;
    Ok(Json(RenamedFileResponse {
        path: path_string(&relative),
        new_path: path_string(&new_relative),
        kind,
    }))
}

async fn delete_file(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<DeleteFileRequest>,
) -> ApiResult<DeletedFileResponse> {
    let root = ensure_workspace(&state, &id).await?;
    let relative = safe_relative(&request.path)?;
    if relative.as_os_str().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "workspace root cannot be deleted".into(),
        ));
    }
    reject_protected_server_artifact(&relative)?;

    let operation = server_operation_lock(&state, &format!("files:{id}")).await;
    let _guard = operation.lock().await;
    let recursive = request.recursive;
    let kind = workspace_fs::within_workspace(root, {
        let relative = relative.clone();
        move |workspace| {
            reject_workspace_symlink(workspace, &relative)?;
            let metadata = workspace.metadata(&relative)?;
            if metadata.is_dir() {
                if !recursive {
                    return Err(workspace_invalid_path(
                        "deleting a directory requires recursive=true",
                    ));
                }
                workspace.remove_dir_all(&relative)?;
                Ok("folder".to_string())
            } else if metadata.is_file() {
                workspace.remove_file(&relative)?;
                Ok("file".to_string())
            } else {
                Err(workspace_invalid_path("unsupported workspace entry"))
            }
        }
    })
    .await
    .map_err(workspace_io_error)?;
    if is_root_server_properties(&relative) {
        let mut data = state.inner.write().await;
        let is_server = data
            .servers
            .iter()
            .any(|workspace| workspace.id == id && workspace.kind == "server");
        if is_server && data.configs.remove(&id).is_some() {
            persist(&state, &data).await.map_err(internal)?;
        }
    }
    Ok(Json(DeletedFileResponse {
        path: path_string(&relative),
        kind,
    }))
}

fn is_root_server_properties(path: &StdPath) -> bool {
    path == StdPath::new("server.properties")
}

fn is_protected_server_artifact(path: &StdPath) -> bool {
    path.components().count() == 1
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    "server.jar" | "server.jar.part" | "server.jar.backup"
                )
            })
}

fn reject_protected_server_artifact(path: &StdPath) -> Result<(), (StatusCode, String)> {
    if is_protected_server_artifact(path) {
        return Err((
            StatusCode::FORBIDDEN,
            "server core artifacts cannot be overwritten, renamed, or deleted".into(),
        ));
    }
    Ok(())
}

async fn sync_config_after_rename(
    state: &AppState,
    id: &str,
    old_path: &StdPath,
    new_path: &StdPath,
    replacement_config: Option<String>,
) -> Result<(), (StatusCode, String)> {
    if !is_root_server_properties(old_path) && !is_root_server_properties(new_path) {
        return Ok(());
    }
    let mut data = state.inner.write().await;
    let is_server = data
        .servers
        .iter()
        .any(|workspace| workspace.id == id && workspace.kind == "server");
    if !is_server {
        return Ok(());
    }
    if is_root_server_properties(old_path) {
        data.configs.remove(id);
    }
    if let Some(config) = replacement_config {
        data.configs.insert(id.to_string(), config);
    }
    persist(state, &data).await.map_err(internal)
}

async fn create_directory(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<CreateDirectoryRequest>,
) -> ApiResult<FileListResponse> {
    let root = ensure_workspace(&state, &id).await?;
    let relative = safe_relative(&request.path)?;
    if relative.as_os_str().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "directory path is required".into()));
    }
    workspace_fs::within_workspace(root, {
        let relative = relative.clone();
        move |workspace| workspace.create_dir_all(&relative)
    })
    .await
    .map_err(workspace_io_error)?;
    list_files(
        Path(id),
        Query(FileQuery {
            path: relative.parent().map(path_string),
        }),
        State(state),
    )
    .await
}
async fn ensure_workspace(state: &AppState, id: &str) -> Result<PathBuf, (StatusCode, String)> {
    let workspace = {
        let data = state.inner.read().await;
        data.servers
            .iter()
            .find(|server| server.id == id)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, "workspace not found".into()))?
    };
    if workspace.kind == "project" {
        let root = runtime::project_directory(id);
        fs::create_dir_all(&root)
            .await
            .map_err(|error| internal(error.to_string()))?;
        return Ok(root);
    }
    let root = runtime::server_directory(id);
    fs::create_dir_all(root.join("plugins"))
        .await
        .map_err(|error| internal(error.to_string()))?;
    fs::create_dir_all(root.join("logs"))
        .await
        .map_err(|error| internal(error.to_string()))?;
    Ok(root)
}

pub(crate) async fn ensure_provision_workspace(state: &AppState, id: &str) -> Result<(), String> {
    let (config, memory_gb) = {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == id)
            .ok_or_else(|| "server not found".to_string())?;
        if server.kind != "server" {
            return Err("operation 'provision' is only available for server workspaces".into());
        }
        (
            data.configs.get(id).cloned().unwrap_or_default(),
            server.memory_gb,
        )
    };
    let root = runtime::server_directory(id);
    fs::create_dir_all(root.join("plugins"))
        .await
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(root.join("logs"))
        .await
        .map_err(|error| error.to_string())?;
    let required_files = [
        (root.join("server.properties"), config),
        (root.join("eula.txt"), "eula=true\n".into()),
        (
            root.join("start.ps1"),
            render_start_script(memory_gb).map_err(str::to_string)?,
        ),
        (
            root.join("start.sh"),
            render_shell_start_script(memory_gb).map_err(str::to_string)?,
        ),
    ];
    for (path, content) in required_files {
        if !fs::try_exists(&path)
            .await
            .map_err(|error| error.to_string())?
        {
            fs::write(&path, content)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    make_shell_script_executable(&root.join("start.sh"))
        .await
        .map_err(|error| error.to_string())
}
fn safe_relative(value: &str) -> Result<PathBuf, (StatusCode, String)> {
    if value.len() > 1024
        || value.chars().any(char::is_control)
        || value.contains('\\')
        || value.split('/').any(is_unsafe_windows_component)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "path is too long or contains unsafe characters".into(),
        ));
    }
    let mut result = PathBuf::new();
    for component in std::path::Path::new(value).components() {
        match component {
            Component::Normal(part) => result.push(part),
            Component::CurDir => {}
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "absolute paths and parent traversal are not allowed".into(),
                ));
            }
        }
    }
    Ok(result)
}

fn validate_upload_filename(value: &str) -> Result<(), (StatusCode, String)> {
    if value.is_empty()
        || value.len() > 255
        || value.chars().any(char::is_control)
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || is_unsafe_windows_component(value)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "uploaded file name must be a single safe path component".into(),
        ));
    }
    Ok(())
}

fn is_unsafe_windows_component(component: &str) -> bool {
    if component.is_empty() || component == "." || component == ".." {
        return false;
    }
    if component.contains(':')
        || component
            .as_bytes()
            .last()
            .is_some_and(|byte| matches!(byte, b'.' | b' '))
    {
        return true;
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

fn safe_download_filename(path: &StdPath) -> String {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let sanitized: String = filename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "download".into()
    } else {
        sanitized
    }
}

fn workspace_io_error(error: std::io::Error) -> (StatusCode, String) {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, "workspace path was not found".into())
        }
        std::io::ErrorKind::AlreadyExists => {
            (StatusCode::CONFLICT, "target path already exists".into())
        }
        std::io::ErrorKind::PermissionDenied => (
            StatusCode::FORBIDDEN,
            "workspace path is not accessible or contains a symbolic link".into(),
        ),
        std::io::ErrorKind::InvalidInput => (StatusCode::BAD_REQUEST, error.to_string()),
        std::io::ErrorKind::InvalidData => (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "file exceeds the {} MiB transfer limit",
                MAX_FILE_TRANSFER_BYTES / 1024 / 1024
            ),
        ),
        std::io::ErrorKind::Unsupported => (StatusCode::UNSUPPORTED_MEDIA_TYPE, error.to_string()),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "workspace file operation failed".into(),
        ),
    }
}

fn workspace_invalid_path(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

fn reject_workspace_symlink(workspace: &CapDir, path: &StdPath) -> std::io::Result<()> {
    if workspace.symlink_metadata(path)?.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "symbolic links are not allowed",
        ));
    }
    Ok(())
}

fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
fn is_editable(path: &std::path::Path) -> bool {
    // 编辑权限由安全路径解析、符号链接检查和核心产物保护负责；扩展名
    // 只是编辑器提示，不能阻止用户创建 LICENSE、无扩展名配置或自定义脚本。
    // 二进制文件仍不会进入文本编辑器：read_file 在 UTF-8 解码失败时会拒绝打开。
    let _ = path;
    true
}
async fn get_mirrors(State(state): State<AppState>) -> Json<Vec<MirrorInfo>> {
    Json(state.inner.read().await.mirrors.clone())
}
async fn preview_downloads(
    State(state): State<AppState>,
    Json(request): Json<DownloadPreviewRequest>,
) -> ApiResult<DownloadPreviewResponse> {
    if request.core.trim().is_empty() || request.version.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "core and version are required".into(),
        ));
    }
    let data = state.inner.read().await;
    let selected: Vec<&MirrorInfo> = if request.mirror_ids.is_empty() {
        data.mirrors
            .iter()
            .filter(|mirror| mirror.enabled)
            .collect()
    } else {
        data.mirrors
            .iter()
            .filter(|mirror| request.mirror_ids.contains(&mirror.id))
            .collect()
    };
    if selected.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "at least one mirror must be selected".into(),
        ));
    }
    let mut candidates: Vec<DownloadCandidate> = selected
        .into_iter()
        .map(|mirror| {
            let supported = mirror.enabled
                && mirror
                    .cores
                    .iter()
                    .any(|core| core == "*" || core.eq_ignore_ascii_case(&request.core));
            let url = mirror
                .base_url
                .replace("{core}", &request.core.to_lowercase())
                .replace("{version}", &request.version)
                .replace("{filename}", "server.jar");
            DownloadCandidate {
                mirror_id: mirror.id.clone(),
                mirror_name: mirror.name.clone(),
                url,
                priority: mirror.priority,
                region: mirror.region.clone(),
                supported,
            }
        })
        .collect();
    candidates.sort_by_key(|candidate| candidate.priority);
    Ok(Json(DownloadPreviewResponse {
        filename: format!(
            "{}-{}-server.jar",
            request.core.to_lowercase(),
            request.version
        ),
        core: request.core,
        version: request.version,
        candidates,
        strategy: "按优先级顺序尝试，失败后自动切换下一镜像",
    }))
}
async fn get_automation(State(state): State<AppState>) -> Json<AutomationResponse> {
    let data = state.inner.read().await;
    let tasks = data.tasks.clone();
    Json(AutomationResponse {
        approvals_required: tasks
            .iter()
            .filter(|task| task.status == "awaiting_approval")
            .count(),
        running: tasks
            .iter()
            .filter(|task| matches!(task.status.as_str(), "queued" | "running" | "cancelling"))
            .count(),
        tasks,
    })
}
async fn create_automation_task(
    State(state): State<AppState>,
    Json(request): Json<CreateAutomationTask>,
) -> ApiResult<TaskInfo> {
    if request.title.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "task title is required".into()));
    }
    if !["low", "medium", "high"].contains(&request.risk.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "invalid risk level".into()));
    }
    let kind = task_executor::normalize_requested_kind(&request.kind).ok_or((
        StatusCode::BAD_REQUEST,
        "该任务类型尚未接入安全执行器".into(),
    ))?;
    let mut data = state.inner.write().await;
    if !data
        .servers
        .iter()
        .any(|server| server.id == request.server_id)
    {
        return Err((StatusCode::NOT_FOUND, "server not found".into()));
    }
    let previous_tasks = data.tasks.clone();
    let (status, progress, approved_by) = effective_task_start(&request.risk, &data.ai.review_mode);
    let mut task = new_task_record(
        request.server_id,
        request.title.trim().into(),
        kind.into(),
        status.into(),
        progress,
        request.risk,
        approved_by.map(Into::into),
    );
    task.events.push(TaskEvent {
        at: task.created_at.clone(),
        level: "info".into(),
        message: if status == "awaiting_approval" {
            "任务计划已持久化，等待人工批准。".into()
        } else {
            "任务计划已持久化，等待执行器领取。".into()
        },
    });
    data.tasks.insert(0, task.clone());
    trim_task_history(&mut data.tasks, 50);
    if let Err(error) = persist(&state, &data).await {
        data.tasks = previous_tasks;
        return Err(internal(error));
    }
    drop(data);
    if task.status == "queued" {
        task_executor::spawn(state, task.id).await;
    }
    Ok(Json(task))
}
async fn approve_task(Path(id): Path<Uuid>, State(state): State<AppState>) -> ApiResult<TaskInfo> {
    let mut data = state.inner.write().await;
    let task_index = data
        .tasks
        .iter()
        .position(|task| task.id == id)
        .ok_or((StatusCode::NOT_FOUND, "task not found".into()))?;
    let previous = data.tasks[task_index].clone();
    let task = &mut data.tasks[task_index];
    if task.status != "awaiting_approval" {
        return Err((
            StatusCode::CONFLICT,
            format!("任务当前状态 {} 不允许批准", task.status),
        ));
    }
    if !task_executor::is_executable_kind(&task.kind) {
        return Err((StatusCode::CONFLICT, "任务没有可执行的结构化指令".into()));
    }
    task.status = "queued".into();
    task.progress = 0;
    task.approved_by = Some("user".into());
    task.updated_at = Local::now().to_rfc3339();
    task.events.push(TaskEvent {
        at: task.updated_at.clone(),
        level: "info".into(),
        message: "用户已批准任务，等待执行器领取。".into(),
    });
    let result = task.clone();
    if let Err(error) = persist(&state, &data).await {
        data.tasks[task_index] = previous;
        return Err(internal(error));
    }
    drop(data);
    task_executor::spawn(state, id).await;
    Ok(Json(result))
}
async fn cancel_task(Path(id): Path<Uuid>, State(state): State<AppState>) -> ApiResult<TaskInfo> {
    task_executor::request_cancel(&state, id).await.map(Json)
}
async fn rollback_task(Path(id): Path<Uuid>, State(state): State<AppState>) -> ApiResult<TaskInfo> {
    task_executor::schedule_rollback(&state, id).await.map(Json)
}
async fn get_community(State(state): State<AppState>) -> Json<CommunityResponse> {
    let data = state.inner.read().await;
    Json(CommunityResponse {
        total_balance: data.players.iter().map(|player| player.balance).sum(),
        online_players: data
            .players
            .iter()
            .filter(|player| player.status == "online")
            .count(),
        inflation_rate: 18.4,
        players: data.players.clone(),
        feedback: data.feedback.clone(),
        polls: data.polls.clone(),
    })
}
async fn create_poll(
    State(state): State<AppState>,
    Json(request): Json<CreatePollRequest>,
) -> ApiResult<PollInfo> {
    if request.title.trim().is_empty() || request.options.len() < 2 {
        return Err((
            StatusCode::BAD_REQUEST,
            "poll needs a title and at least two options".into(),
        ));
    }
    let poll = PollInfo {
        id: Uuid::new_v4(),
        server_id: request.server_id,
        title: request.title.trim().into(),
        status: "active".into(),
        options: request
            .options
            .into_iter()
            .filter(|option| !option.trim().is_empty())
            .map(|label| PollOption {
                id: Uuid::new_v4(),
                label,
                votes: 0,
            })
            .collect(),
        closes_at: (Local::now() + chrono::Duration::days(3)).to_rfc3339(),
        created_at: Local::now().to_rfc3339(),
    };
    let mut data = state.inner.write().await;
    data.polls.insert(0, poll.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(poll))
}
async fn vote_poll(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<VoteRequest>,
) -> ApiResult<PollInfo> {
    let mut data = state.inner.write().await;
    let poll = data
        .polls
        .iter_mut()
        .find(|poll| poll.id == id)
        .ok_or((StatusCode::NOT_FOUND, "poll not found".into()))?;
    if poll.status != "active" {
        return Err((StatusCode::CONFLICT, "poll is closed".into()));
    }
    let option = poll
        .options
        .iter_mut()
        .find(|option| option.id == request.option_id)
        .ok_or((StatusCode::NOT_FOUND, "option not found".into()))?;
    option.votes += 1;
    let result = poll.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(result))
}
async fn cluster_feedback(State(state): State<AppState>) -> Json<FeedbackCluster> {
    let data = state.inner.read().await;
    let mut categories = HashMap::new();
    for item in &data.feedback {
        *categories.entry(item.category.clone()).or_insert(0) += 1
    }
    Json(FeedbackCluster {
        positive: data
            .feedback
            .iter()
            .filter(|item| item.sentiment == "positive")
            .count(),
        neutral: data
            .feedback
            .iter()
            .filter(|item| item.sentiment == "neutral")
            .count(),
        negative: data
            .feedback
            .iter()
            .filter(|item| item.sentiment == "negative")
            .count(),
        summary: "玩家最关注新玩法节奏、经济平衡与服务器性能，建议先在镜像服进行七日灰度测试。"
            .into(),
        categories,
    })
}
async fn player_action(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<PlayerActionRequest>,
) -> ApiResult<PlayerActionResponse> {
    let mut data = state.inner.write().await;
    let player = data
        .players
        .iter_mut()
        .find(|player| player.id == id)
        .ok_or((StatusCode::NOT_FOUND, "player not found".into()))?;
    let preview = !request.execute;
    if request.execute {
        match request.action.as_str() {
            "kick" => player.status = "offline".into(),
            "ban" => player.status = "banned".into(),
            "warn" => {}
            _ => return Err((StatusCode::BAD_REQUEST, "unsupported player action".into())),
        }
    }
    let result = player.clone();
    if request.execute {
        persist(&state, &data).await.map_err(internal)?
    }
    Ok(Json(PlayerActionResponse {
        player: result,
        action: request.action,
        preview,
        message: if preview {
            format!("预览：将执行操作，原因：{}", request.reason)
        } else {
            "操作已写入玩家管理状态".into()
        },
    }))
}
async fn get_integrations(State(state): State<AppState>) -> Json<IntegrationsResponse> {
    let data = state.inner.read().await;
    Json(IntegrationsResponse {
        integrations: data.integrations.clone(),
        skills: data.skills.clone(),
    })
}
async fn toggle_integration(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<IntegrationInfo> {
    let mut data = state.inner.write().await;
    let item = data
        .integrations
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or((StatusCode::NOT_FOUND, "integration not found".into()))?;
    item.enabled = !item.enabled;
    item.status = if item.enabled {
        "ready".into()
    } else {
        "disabled".into()
    };
    let result = item.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(result))
}
async fn test_integration(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<IntegrationInfo> {
    let mut data = state.inner.write().await;
    let item = data
        .integrations
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or((StatusCode::NOT_FOUND, "integration not found".into()))?;
    if !item.enabled {
        return Err((StatusCode::CONFLICT, "integration is disabled".into()));
    }
    item.status = "connected".into();
    item.latency_ms = Some(24 + (id.len() as u32 * 3));
    let result = item.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(result))
}
async fn toggle_skill(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<SkillInfo> {
    let mut data = state.inner.write().await;
    let skill = data
        .skills
        .iter_mut()
        .find(|skill| skill.id == id)
        .ok_or((StatusCode::NOT_FOUND, "skill not found".into()))?;
    skill.enabled = !skill.enabled;
    let result = skill.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(result))
}
fn internal(error: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error)
}

/// 从用户或规划对话中的结构化事实提取核心与 Minecraft 版本。
///
/// 这只接受明确出现的核心名称和形如 `1.12.2` 的版本号，不根据“最新”“
/// 推荐”等模糊措辞猜测下载目标。这样开服任务可以复用规划结果，同时不
/// 会把普通聊天误变成任意命令执行。
pub(crate) fn extract_server_plan(text: &str) -> Option<(String, String)> {
    const CORES: &[(&str, &str)] = &[
        ("paper", "Paper"),
        ("purpur", "Purpur"),
        ("spigot", "Spigot"),
        ("folia", "Folia"),
        ("leaves", "Leaves"),
        ("fabric", "Fabric"),
        ("velocity", "Velocity"),
        ("vanilla", "Vanilla"),
    ];
    let lower = text.to_ascii_lowercase();
    let core = CORES
        .iter()
        .find(|(needle, _)| lower.contains(needle))
        .map(|(_, name)| (*name).to_string())?;
    let version = text
        .char_indices()
        .filter_map(|(index, _)| text.get(index..).map(|tail| (index, tail)))
        .find_map(|(_, tail)| {
            let tail = tail.strip_prefix("1.")?;
            let end = tail
                .char_indices()
                .find(|(_, character)| !character.is_ascii_digit() && *character != '.')
                .map(|(index, _)| index)
                .unwrap_or(tail.len());
            let candidate = format!("1.{}", &tail[..end]);
            let parts: Vec<_> = candidate.split('.').collect();
            if (parts.len() == 2 || parts.len() == 3)
                && parts[1..].iter().all(|part| !part.is_empty())
                && parts[1..].iter().all(|part| part.parse::<u32>().is_ok())
            {
                Some(candidate)
            } else {
                None
            }
        })?;
    Some((core, version))
}
pub(crate) fn classify_intent(message: &str) -> &'static str {
    let command = message
        .trim()
        .trim_end_matches(['。', '！', '!', '？', '?'])
        .trim();
    let compact = command
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    if compact.contains("开服")
        || compact.contains("开服务器")
        || compact.contains("启动服务器")
        || compact.contains("启动这台服务器")
        || compact.contains("把服务器开起来")
        || compact.contains("准备并启动")
    {
        "server_bootstrap"
    } else if matches!(
        command,
        "停止服务器"
            | "安全停服"
            | "请停止服务器"
            | "请安全停服"
            | "帮我停止服务器"
            | "帮我安全停服"
    ) {
        "server_stop"
    } else if message.contains("报错") || message.contains("诊断") || message.contains("检查日志")
    {
        "diagnostic"
    } else if message.contains("投票") || message.contains("意见") {
        "vote"
    } else if message.contains("宣传") || message.contains("文案") {
        "promotion"
    } else if message.contains("插件") || message.contains("Codex") || message.contains("镜像")
    {
        "plugin"
    } else {
        "general"
    }
}
pub(crate) fn task_title(intent: &str) -> &'static str {
    match intent {
        "diagnostic" => "分析服务器日志",
        "server_start" | "server_bootstrap" => "准备并启动服务器",
        "server_stop" => "安全停止服务器",
        "vote" => "创建玩家玩法投票",
        "promotion" => "生成服务器宣传内容",
        "plugin" => "插件镜像服交付测试",
        _ => "AI 服务器管理任务",
    }
}
pub(crate) fn rule_reply(intent: &str) -> &'static str {
    match intent {
        "diagnostic" => {
            "我已创建只读日志诊断任务。执行器会分析最近的服务器日志并生成可下载报告，不会自动修改文件。"
        }
        "server_start" | "server_bootstrap" => {
            "已创建真实开服任务：先从资源中心解析核心，找不到时按镜像和官方网络源回退；缺少匹配 Java 会自动安装托管运行时，然后启动服务器并等待真实就绪标记。所有阶段、日志、产物和失败补偿都会记录在任务详情中。"
        }
        "server_stop" => "我已创建安全停服任务。执行器会等待 Java 进程真实退出后再报告完成。",
        "vote" => {
            "玩法投票草案已生成：包含玩法摘要、奖励方案、经济影响和三档实施范围，可发布到游戏内公告、Discord 与 Web 面板。"
        }
        "promotion" => {
            "本周宣传任务已创建，主题为“深暗遗迹悬赏季”，将生成长文、短视频口播和三组社群短文案。"
        }
        "plugin" => {
            "已建立插件交付链：需求文档 → MCP 交付 Codex → 自动构建 → 镜像服部署 → 兼容性回归 → 玩家灰度测试。"
        }
        _ => {
            "我已分析你的需求。只有具备结构化参数和安全边界的服务器操作才会进入执行器；其余内容会保留为建议，不会伪装成已执行任务。"
        }
    }
}
pub(crate) fn intent_risk(intent: &str) -> &'static str {
    match intent {
        "server_stop" => "high",
        "server_start" | "server_bootstrap" => "medium",
        _ => "low",
    }
}
/// 依据风险等级与全局审核模式决定任务的持久化初始状态。
/// 自动任务也必须先进入 queued，由执行器原子领取后才能转为 running。
pub(crate) fn effective_task_start(
    risk: &str,
    review_mode: &str,
) -> (&'static str, u8, Option<&'static str>) {
    match (review_mode, risk) {
        ("full", _) => ("queued", 0, Some("auto")),
        ("auto", "low") => ("queued", 0, None),
        ("auto", "medium") => ("queued", 0, Some("ai")),
        (_, "low") => ("queued", 0, None),
        _ => ("awaiting_approval", 0, None),
    }
}
fn default_risk() -> String {
    "low".into()
}

fn default_memory_gb() -> u8 {
    DEFAULT_MEMORY_GB
}

fn default_location() -> String {
    "local".into()
}

fn default_workspace_kind() -> String {
    "server".into()
}

pub(crate) fn require_server_kind(
    workspace: &ServerInfo,
    operation: &str,
) -> Result<(), (StatusCode, String)> {
    if workspace.kind == "server" {
        Ok(())
    } else {
        Err((
            StatusCode::CONFLICT,
            format!("operation '{operation}' is only available for server workspaces"),
        ))
    }
}

fn default_operation_state() -> String {
    "idle".into()
}

const DEFAULT_MEMORY_GB: u8 = 8;
const MIN_MEMORY_GB: u8 = 2;
const MAX_MEMORY_GB: u8 = 64;
const INITIAL_HEAP_GB: u8 = 2;

fn validate_server_name(name: &str) -> Result<(), &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("server name is required");
    }
    if name.chars().count() > 64 {
        return Err("server name cannot exceed 64 characters");
    }
    if name.chars().any(char::is_control) {
        return Err("server name cannot contain control characters");
    }
    Ok(())
}

fn validate_project_name(name: &str) -> Result<(), &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("project name is required");
    }
    if name.chars().count() > 64 {
        return Err("project name cannot exceed 64 characters");
    }
    if name.chars().any(char::is_control) {
        return Err("project name cannot contain control characters");
    }
    Ok(())
}

fn validate_server_port(port: u16) -> Result<(), &'static str> {
    if port >= 1024 {
        Ok(())
    } else {
        Err("server port must be between 1024 and 65535")
    }
}

fn validate_catalog_server_template(
    catalog: &catalog::CatalogState,
    core: &str,
    minecraft_version: &str,
) -> Result<(), &'static str> {
    let core = core.trim();
    let minecraft_version = minecraft_version.trim();
    let project = catalog
        .core_projects
        .iter()
        .find(|project| {
            project.name.eq_ignore_ascii_case(core) || project.slug.eq_ignore_ascii_case(core)
        })
        .ok_or("server core is not available in the catalog")?;
    if catalog.core_versions.iter().any(|version| {
        version.project == project.slug
            && version
                .minecraft_versions
                .iter()
                .any(|item| item == minecraft_version)
    }) {
        Ok(())
    } else {
        Err("minecraft version is not available for the selected core")
    }
}

fn validate_memory_gb(memory_gb: u8) -> Result<(), &'static str> {
    if (MIN_MEMORY_GB..=MAX_MEMORY_GB).contains(&memory_gb) {
        Ok(())
    } else {
        Err("memory must be between 2 and 64 GB")
    }
}

fn server_java_args(memory_gb: u8) -> Result<Vec<String>, &'static str> {
    validate_memory_gb(memory_gb)?;
    Ok(vec![
        format!("-Xms{}G", memory_gb.min(INITIAL_HEAP_GB)),
        format!("-Xmx{memory_gb}G"),
        "-jar".into(),
        "server.jar".into(),
        "nogui".into(),
    ])
}

fn render_start_script(memory_gb: u8) -> Result<String, &'static str> {
    Ok(format!(
        "$ErrorActionPreference = 'Stop'\njava {}\n",
        server_java_args(memory_gb)?.join(" ")
    ))
}

fn render_shell_start_script(memory_gb: u8) -> Result<String, &'static str> {
    Ok(format!(
        "#!/usr/bin/env sh\nset -eu\nexec java {}\n",
        server_java_args(memory_gb)?.join(" ")
    ))
}

#[cfg(unix)]
async fn make_shell_script_executable(path: &StdPath) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path).await?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).await
}

#[cfg(not(unix))]
async fn make_shell_script_executable(_path: &StdPath) -> std::io::Result<()> {
    Ok(())
}

fn legacy_servers_missing_memory(data: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
        return false;
    };
    value
        .get("servers")
        .and_then(|servers| servers.as_array())
        .is_some_and(|servers| {
            servers
                .iter()
                .any(|server| server.get("memory_gb").is_none())
        })
}
fn seed_players() -> Vec<PlayerInfo> {
    vec![
        PlayerInfo {
            id: Uuid::new_v4(),
            server_id: "sculk".into(),
            name: "Aurora_7".into(),
            status: "online".into(),
            role: "管理员".into(),
            balance: 12840,
            playtime_hours: 486,
            ping: 28,
            joined_at: "2025-11-08".into(),
        },
        PlayerInfo {
            id: Uuid::new_v4(),
            server_id: "sculk".into(),
            name: "DeepMiner".into(),
            status: "online".into(),
            role: "玩家".into(),
            balance: 7340,
            playtime_hours: 212,
            ping: 42,
            joined_at: "2026-01-17".into(),
        },
        PlayerInfo {
            id: Uuid::new_v4(),
            server_id: "sculk".into(),
            name: "MossFox".into(),
            status: "online".into(),
            role: "建筑师".into(),
            balance: 9820,
            playtime_hours: 366,
            ping: 35,
            joined_at: "2025-12-03".into(),
        },
        PlayerInfo {
            id: Uuid::new_v4(),
            server_id: "sculk".into(),
            name: "RedstoneCat".into(),
            status: "offline".into(),
            role: "玩家".into(),
            balance: 4210,
            playtime_hours: 147,
            ping: 0,
            joined_at: "2026-02-21".into(),
        },
        PlayerInfo {
            id: Uuid::new_v4(),
            server_id: "mirror".into(),
            name: "PluginTester".into(),
            status: "online".into(),
            role: "测试员".into(),
            balance: 500,
            playtime_hours: 34,
            ping: 19,
            joined_at: "2026-07-12".into(),
        },
    ]
}
fn seed_feedback() -> Vec<FeedbackInfo> {
    vec![
        FeedbackInfo {
            id: Uuid::new_v4(),
            server_id: "sculk".into(),
            player: "DeepMiner".into(),
            content: "希望遗迹悬赏能增加组队难度。".into(),
            category: "新玩法".into(),
            sentiment: "positive".into(),
            status: "new".into(),
            created_at: Local::now().to_rfc3339(),
        },
        FeedbackInfo {
            id: Uuid::new_v4(),
            server_id: "sculk".into(),
            player: "MossFox".into(),
            content: "商店绿宝石价格上涨太快。".into(),
            category: "经济".into(),
            sentiment: "negative".into(),
            status: "reviewed".into(),
            created_at: Local::now().to_rfc3339(),
        },
        FeedbackInfo {
            id: Uuid::new_v4(),
            server_id: "sculk".into(),
            player: "Aurora_7".into(),
            content: "周末高峰偶尔出现区块加载延迟。".into(),
            category: "性能".into(),
            sentiment: "neutral".into(),
            status: "new".into(),
            created_at: Local::now().to_rfc3339(),
        },
    ]
}
fn seed_polls() -> Vec<PollInfo> {
    vec![PollInfo {
        id: Uuid::new_v4(),
        server_id: "sculk".into(),
        title: "下周优先上线哪个玩法？".into(),
        status: "active".into(),
        options: vec![
            PollOption {
                id: Uuid::new_v4(),
                label: "深暗遗迹悬赏".into(),
                votes: 42,
            },
            PollOption {
                id: Uuid::new_v4(),
                label: "村庄贸易赛季".into(),
                votes: 31,
            },
            PollOption {
                id: Uuid::new_v4(),
                label: "空岛远征".into(),
                votes: 18,
            },
        ],
        closes_at: (Local::now() + chrono::Duration::days(2)).to_rfc3339(),
        created_at: Local::now().to_rfc3339(),
    }]
}
fn seed_integrations() -> Vec<IntegrationInfo> {
    vec![
        IntegrationInfo {
            id: "codex-mcp".into(),
            name: "Codex MCP".into(),
            kind: "mcp".into(),
            status: "connected".into(),
            enabled: true,
            endpoint: "stdio://codex".into(),
            latency_ms: Some(18),
            capabilities: vec!["代码生成".into(), "插件构建".into(), "测试交付".into()],
        },
        IntegrationInfo {
            id: "discord-mcp".into(),
            name: "Discord 社区".into(),
            kind: "mcp".into(),
            status: "ready".into(),
            enabled: true,
            endpoint: "https://discord.example.com/mcp".into(),
            latency_ms: None,
            capabilities: vec!["公告发布".into(), "意见收集".into(), "投票同步".into()],
        },
        IntegrationInfo {
            id: "metrics-mcp".into(),
            name: "Metrics Gateway".into(),
            kind: "mcp".into(),
            status: "ready".into(),
            enabled: true,
            endpoint: "http://127.0.0.1:9090/mcp".into(),
            latency_ms: None,
            capabilities: vec!["性能指标".into(), "告警".into()],
        },
    ]
}
fn seed_skills() -> Vec<SkillInfo> {
    vec![
        skills::builtin_skill_info(),
        skills::builtin_server_skill_info(),
        SkillInfo {
            id: "paper-ops".into(),
            name: "Paper 运维专家".into(),
            description: "配置优化、性能诊断与安全基线。".into(),
            source: "builtin".into(),
            enabled: true,
            version: "1.2.0".into(),
        },
        SkillInfo {
            id: "plugin-curator".into(),
            name: "插件策展师".into(),
            description: "插件选择、依赖检查与配置生成。".into(),
            source: "builtin".into(),
            enabled: true,
            version: "1.1.0".into(),
        },
        SkillInfo {
            id: "economy-analyst".into(),
            name: "经济分析师".into(),
            description: "追踪货币流通、物价与玩家贫富差异。".into(),
            source: "workspace".into(),
            enabled: true,
            version: "0.8.4".into(),
        },
        SkillInfo {
            id: "community-writer".into(),
            name: "社区内容助手".into(),
            description: "公告、活动文案、投票与反馈摘要。".into(),
            source: "workspace".into(),
            enabled: false,
            version: "0.6.1".into(),
        },
    ]
}
fn seed_mirrors() -> Vec<MirrorInfo> {
    vec![
        MirrorInfo {
            id: "primary-cn".into(),
            name: "主镜像（预留）".into(),
            base_url: "https://mirror-primary.example.com/minecraft/{core}/{version}/{filename}"
                .into(),
            enabled: true,
            priority: 10,
            cores: vec!["*".into()],
            region: "中国大陆".into(),
        },
        MirrorInfo {
            id: "backup-cn".into(),
            name: "备用镜像（预留）".into(),
            base_url: "https://mirror-backup.example.com/releases/{core}/{version}/{filename}"
                .into(),
            enabled: true,
            priority: 20,
            cores: vec![
                "Paper".into(),
                "Purpur".into(),
                "Fabric".into(),
                "Velocity".into(),
            ],
            region: "中国大陆".into(),
        },
        MirrorInfo {
            id: "official-fallback".into(),
            name: "官方源回退".into(),
            base_url: "https://downloads.example.com/official/{core}/{version}/{filename}".into(),
            enabled: true,
            priority: 90,
            cores: vec!["Paper".into(), "Purpur".into()],
            region: "全球".into(),
        },
    ]
}
fn initial_state() -> PersistedState {
    PersistedState {
        // A fresh installation is an empty workspace. Resource catalogs and
        // built-in settings still receive their defaults below, but runtime
        // entities must only appear after the user creates them.
        servers: Vec::new(),
        tasks: Vec::new(),
        configs: HashMap::new(),
        logs: HashMap::new(),
        mirrors: seed_mirrors(),
        players: Vec::new(),
        feedback: Vec::new(),
        polls: Vec::new(),
        integrations: seed_integrations(),
        skills: seed_skills(),
        catalog: catalog::seed_catalog(),
        ai: ai::AiSettings::default(),
        ui: prefs::UiSettings::default(),
        resource_sync: resource_sync::ResourceSyncState::default(),
        bots: bots::BotState::default(),
        conversations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_server(id: &str, status: &str, task: &str) -> ServerInfo {
        ServerInfo {
            id: id.into(),
            kind: "server".into(),
            name: format!("{id} server"),
            core: "Paper".into(),
            version: "1.21.4".into(),
            status: status.into(),
            players: "0 / 60".into(),
            memory: 0,
            memory_gb: DEFAULT_MEMORY_GB,
            cpu: 0,
            port: 25565,
            task: task.into(),
            location: "本地".into(),
            pid: None,
            runtime_generation: None,
            started_at: None,
            operation_state: "idle".into(),
            core_ready: false,
            last_error: None,
        }
    }

    async fn test_state_with_workspace(id: &str, kind: &str) -> (AppState, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("sculk-files-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&directory).await.unwrap();
        let file = directory.join("state.json");
        let file_lock = acquire_state_lock(&file).unwrap();
        let mut persisted = initial_state();
        let mut workspace = test_server(id, "stopped", "idle");
        workspace.kind = kind.into();
        persisted.servers.push(workspace);
        let state = AppState {
            inner: Arc::new(RwLock::new(persisted)),
            file,
            _file_lock: Arc::new(file_lock),
            processes: Arc::new(RwLock::new(HashMap::new())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            channels: Arc::new(RwLock::new(HashMap::new())),
            downloads: Arc::new(RwLock::new(HashMap::new())),
            runtime_install: Arc::new(Mutex::new(())),
            task_controls: Arc::new(RwLock::new(HashMap::new())),
            cloud: cloud::CloudRuntime::disabled_for_test(),
        };
        (state, directory)
    }

    #[tokio::test]
    async fn workspace_rename_and_recursive_delete_enforce_safety_contract() {
        let id = format!("project-files-{}", Uuid::new_v4().simple());
        let (state, state_directory) = test_state_with_workspace(&id, "project").await;
        let root = runtime::project_directory(&id);
        fs::create_dir_all(root.join("src")).await.unwrap();
        fs::write(root.join("src").join("old.rs"), "fn main() {}")
            .await
            .unwrap();

        let renamed = rename_file(
            Path(id.clone()),
            State(state.clone()),
            Json(RenameFileRequest {
                path: "src/old.rs".into(),
                new_path: "src/main.rs".into(),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(renamed.kind, "file");
        assert!(fs::metadata(root.join("src").join("main.rs")).await.is_ok());

        fs::write(root.join("src").join("occupied.rs"), "occupied")
            .await
            .unwrap();
        let conflict = rename_file(
            Path(id.clone()),
            State(state.clone()),
            Json(RenameFileRequest {
                path: "src/main.rs".into(),
                new_path: "src/occupied.rs".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(conflict.0, StatusCode::CONFLICT);

        let traversal = rename_file(
            Path(id.clone()),
            State(state.clone()),
            Json(RenameFileRequest {
                path: "src/main.rs".into(),
                new_path: "../outside.rs".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(traversal.0, StatusCode::BAD_REQUEST);

        let without_recursive = delete_file(
            Path(id.clone()),
            State(state.clone()),
            Json(DeleteFileRequest {
                path: "src".into(),
                recursive: false,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(without_recursive.0, StatusCode::BAD_REQUEST);
        assert!(fs::metadata(root.join("src")).await.is_ok());

        let deleted = delete_file(
            Path(id.clone()),
            State(state.clone()),
            Json(DeleteFileRequest {
                path: "src".into(),
                recursive: true,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(deleted.kind, "folder");
        assert!(fs::metadata(root.join("src")).await.is_err());

        drop(state);
        fs::remove_dir_all(&root).await.unwrap();
        fs::remove_dir_all(state_directory).await.unwrap();
    }

    #[test]
    fn core_artifacts_are_protected_at_the_workspace_root() {
        for name in ["server.jar", "server.jar.part", "server.jar.backup"] {
            let path = PathBuf::from(name);
            assert!(is_protected_server_artifact(&path));
            assert_eq!(
                reject_protected_server_artifact(&path).unwrap_err().0,
                StatusCode::FORBIDDEN
            );
        }
        assert!(!is_protected_server_artifact(StdPath::new(
            "plugins/server.jar"
        )));
        assert!(reject_protected_server_artifact(StdPath::new("server.properties")).is_ok());
    }

    #[test]
    fn upload_names_and_download_headers_are_sanitized() {
        assert!(validate_upload_filename("backup.zip").is_ok());
        for name in ["", ".", "..", "a/b", "a\\b", "line\nfeed"] {
            assert!(validate_upload_filename(name).is_err(), "accepted {name:?}");
        }
        assert_eq!(
            safe_download_filename(StdPath::new("logs/latest.log")),
            "latest.log"
        );
        assert_eq!(
            safe_download_filename(StdPath::new("备份 文件.zip")),
            "_____.zip"
        );
    }

    #[test]
    fn process_metrics_are_applied_only_to_the_current_generation() {
        let generation = Uuid::new_v4();
        let mut server = test_server("metrics", "online", "running");
        server.runtime_generation = Some(generation);
        assert!(apply_process_metrics(
            &mut server,
            "metrics",
            generation,
            runtime::ProcessMetrics {
                cpu: 42,
                memory: 1536,
            },
        ));
        assert_eq!(server.cpu, 42);
        assert_eq!(server.memory, 1536);
        assert!(!apply_process_metrics(
            &mut server,
            "metrics",
            Uuid::new_v4(),
            runtime::ProcessMetrics { cpu: 1, memory: 1 },
        ));
        assert_eq!(server.cpu, 42);
        assert_eq!(server.memory, 1536);
    }

    #[tokio::test]
    async fn upload_file_is_bounded_and_never_overwrites_existing_entries() {
        use axum::extract::FromRequest;

        let id = format!("server-upload-{}", Uuid::new_v4().simple());
        let (state, state_directory) = test_state_with_workspace(&id, "server").await;
        let root = runtime::server_directory(&id);
        fs::create_dir_all(root.join("logs")).await.unwrap();
        let boundary = "sculk-test-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"path\"\r\n\r\nlogs\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"upload.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n--{boundary}--\r\n"
        );
        let request = axum::http::Request::builder()
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let multipart = Multipart::from_request(request, &()).await.unwrap();
        let uploaded = upload_file(Path(id.clone()), State(state.clone()), multipart)
            .await
            .unwrap()
            .0;
        assert_eq!(uploaded.path, "logs/upload.txt");
        assert_eq!(uploaded.size, 5);
        assert_eq!(
            fs::read_to_string(root.join("logs/upload.txt"))
                .await
                .unwrap(),
            "hello"
        );

        let conflict_body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"path\"\r\n\r\nlogs\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"upload.txt\"\r\n\r\nchanged\r\n--{boundary}--\r\n"
        );
        let conflict_request = axum::http::Request::builder()
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(conflict_body))
            .unwrap();
        let conflict_multipart = Multipart::from_request(conflict_request, &())
            .await
            .unwrap();
        let conflict = upload_file(Path(id.clone()), State(state.clone()), conflict_multipart)
            .await
            .unwrap_err();
        assert_eq!(conflict.0, StatusCode::CONFLICT);
        assert_eq!(
            fs::read_to_string(root.join("logs/upload.txt"))
                .await
                .unwrap(),
            "hello"
        );

        drop(state);
        fs::remove_dir_all(&root).await.unwrap();
        fs::remove_dir_all(state_directory).await.unwrap();
    }

    #[tokio::test]
    async fn server_properties_mutations_keep_persisted_config_in_sync() {
        let id = format!("server-files-{}", Uuid::new_v4().simple());
        let (state, state_directory) = test_state_with_workspace(&id, "server").await;
        let root = runtime::server_directory(&id);
        fs::create_dir_all(&root).await.unwrap();
        fs::write(root.join("server.properties"), "view-distance=10")
            .await
            .unwrap();
        state
            .inner
            .write()
            .await
            .configs
            .insert(id.clone(), "view-distance=10".into());

        let _ = delete_file(
            Path(id.clone()),
            State(state.clone()),
            Json(DeleteFileRequest {
                path: "server.properties".into(),
                recursive: false,
            }),
        )
        .await
        .unwrap();
        assert!(!state.inner.read().await.configs.contains_key(&id));
        assert!(fs::metadata(root.join("server.properties")).await.is_err());

        fs::write(root.join("replacement.properties"), "view-distance=16")
            .await
            .unwrap();
        let _ = rename_file(
            Path(id.clone()),
            State(state.clone()),
            Json(RenameFileRequest {
                path: "replacement.properties".into(),
                new_path: "server.properties".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            state
                .inner
                .read()
                .await
                .configs
                .get(&id)
                .map(String::as_str),
            Some("view-distance=16")
        );

        drop(state);
        fs::remove_dir_all(&root).await.unwrap();
        fs::remove_dir_all(state_directory).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_paths_reject_intermediate_symbolic_links() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("sculk-symlink-{}", Uuid::new_v4()));
        let outside = std::env::temp_dir().join(format!("sculk-outside-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("real")).await.unwrap();
        fs::create_dir_all(&outside).await.unwrap();
        fs::write(outside.join("file.txt"), "private")
            .await
            .unwrap();
        symlink(&outside, root.join("alias")).unwrap();
        let error = workspace_fs::within_workspace(root.clone(), move |workspace| {
            let mut file = workspace.open("alias/file.txt")?;
            let mut content = String::new();
            file.read_to_string(&mut content)?;
            Ok(content)
        })
        .await
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(root).await.unwrap();
        fs::remove_dir_all(outside).await.unwrap();
    }

    #[test]
    fn delete_confirmation_gate_requires_exact_phrase() {
        assert!(validate_delete_confirmation(false, None).is_ok());
        assert!(validate_delete_confirmation(false, Some("whatever")).is_ok());
        assert!(validate_delete_confirmation(true, None).is_err());
        assert!(validate_delete_confirmation(true, Some("DELETE ALL")).is_err());
        assert!(validate_delete_confirmation(true, Some("delete all")).is_ok());
    }

    #[test]
    fn legacy_server_json_defaults_memory_limit_to_eight_gb() {
        let legacy = r#"{
            "id":"legacy",
            "name":"Legacy Server",
            "core":"Paper",
            "version":"1.21.4",
            "status":"stopped",
            "players":"0 / 60",
            "memory":37,
            "cpu":0,
            "port":25565,
            "task":"idle"
        }"#;
        let server: ServerInfo = serde_json::from_str(legacy).unwrap();

        assert_eq!(server.memory, 37);
        assert_eq!(server.kind, "server");
        assert_eq!(server.memory_gb, DEFAULT_MEMORY_GB);
        assert_eq!(server.operation_state, "idle");
        assert!(!server.core_ready);
        assert!(server.last_error.is_none());
        assert_eq!(
            serde_json::to_value(server).unwrap()["memory_gb"],
            DEFAULT_MEMORY_GB
        );
        assert!(legacy_servers_missing_memory(&format!(
            r#"{{"servers":[{legacy}]}}"#
        )));
    }

    #[test]
    fn project_kind_round_trips_through_persisted_state() {
        let mut state = initial_state();
        let mut project = test_server("project-test", "ready", "ready");
        project.kind = "project".into();
        project.core.clear();
        project.version.clear();
        project.port = 0;
        state.servers.push(project);

        let encoded = serde_json::to_string(&state).unwrap();
        let decoded: PersistedState = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.servers[0].kind, "project");
        assert_eq!(decoded.servers[0].id, "project-test");
    }

    #[test]
    fn server_only_operations_reject_projects_with_conflict() {
        let mut project = test_server("project-test", "ready", "ready");
        project.kind = "project".into();

        let error = require_server_kind(&project, "provision").unwrap_err();

        assert_eq!(error.0, StatusCode::CONFLICT);
        assert!(error.1.contains("only available for server workspaces"));
    }

    #[test]
    fn project_editor_accepts_source_files_without_allowing_path_traversal() {
        for path in [
            "src/main.rs",
            "src/App.vue",
            "package.json",
            "Dockerfile",
            ".gitignore",
            "LICENSE",
            "eula",
            "config/custom.server.script",
            "config/没有扩展名",
        ] {
            assert!(is_editable(StdPath::new(path)), "{path} should be editable");
        }
        assert!(safe_relative("src/main.rs").is_ok());
        assert!(safe_relative("../outside.rs").is_err());
        assert!(safe_relative("/outside.rs").is_err());
    }

    #[tokio::test]
    async fn creating_project_workspace_only_creates_the_root_directory() {
        let parent =
            std::env::temp_dir().join(format!("sculk-project-{}", Uuid::new_v4().simple()));
        let directory = parent.join("project-test");

        create_empty_project_directory(&directory).await.unwrap();

        assert!(fs::metadata(&directory).await.unwrap().is_dir());
        assert!(
            fs::read_dir(&directory)
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_none()
        );
        assert!(fs::metadata(directory.join("plugins")).await.is_err());
        assert!(
            fs::metadata(directory.join("server.properties"))
                .await
                .is_err()
        );

        fs::remove_dir_all(parent).await.unwrap();
    }

    #[test]
    fn memory_limits_accept_boundaries_and_reject_out_of_range_values() {
        for memory_gb in [MIN_MEMORY_GB, DEFAULT_MEMORY_GB, MAX_MEMORY_GB] {
            assert!(validate_memory_gb(memory_gb).is_ok());
        }
        for memory_gb in [0, 1, 65, u8::MAX] {
            assert!(validate_memory_gb(memory_gb).is_err());
            assert!(server_java_args(memory_gb).is_err());
        }
    }

    #[test]
    fn portable_server_fields_reject_injection_and_privileged_ports() {
        assert!(validate_server_name("生存服").is_ok());
        assert!(validate_server_name("line one\nserver-port=1").is_err());
        assert!(validate_server_name(&"a".repeat(65)).is_err());
        assert!(validate_server_port(1024).is_ok());
        assert!(validate_server_port(65535).is_ok());
        assert!(validate_server_port(1023).is_err());
    }

    #[test]
    fn portable_server_core_and_minecraft_version_must_match_catalog() {
        let catalog = catalog::seed_catalog();
        assert!(validate_catalog_server_template(&catalog, "Paper", "1.21.4").is_ok());
        assert!(validate_catalog_server_template(&catalog, "missing-core", "1.21.4").is_err());
        assert!(validate_catalog_server_template(&catalog, "Paper", "0.0.0").is_err());
    }

    #[test]
    fn java_args_use_each_servers_configured_heap_limit() {
        assert_eq!(
            server_java_args(4).unwrap(),
            ["-Xms2G", "-Xmx4G", "-jar", "server.jar", "nogui"]
        );
        assert_eq!(
            server_java_args(12).unwrap(),
            ["-Xms2G", "-Xmx12G", "-jar", "server.jar", "nogui"]
        );
        assert_eq!(
            server_java_args(64).unwrap(),
            ["-Xms2G", "-Xmx64G", "-jar", "server.jar", "nogui"]
        );
    }

    #[test]
    fn start_script_is_derived_from_the_same_java_arguments() {
        for memory_gb in [MIN_MEMORY_GB, DEFAULT_MEMORY_GB, 12, MAX_MEMORY_GB] {
            let args = server_java_args(memory_gb).unwrap();
            assert_eq!(
                render_start_script(memory_gb).unwrap(),
                format!("$ErrorActionPreference = 'Stop'\njava {}\n", args.join(" "))
            );
            assert_eq!(
                render_shell_start_script(memory_gb).unwrap(),
                format!("#!/usr/bin/env sh\nset -eu\nexec java {}\n", args.join(" "))
            );
        }
    }

    #[test]
    fn process_readiness_requires_a_known_server_marker() {
        assert!(process_ready_line(
            "Paper",
            "[Server thread/INFO]: Done (6.123s)! For help, type \"help\""
        ));
        assert!(process_ready_line("Velocity", "Done! Proxy is ready"));
        assert!(process_ready_line(
            "Fabric",
            "Server is Listening on /0.0.0.0:25565"
        ));
        assert!(!process_ready_line("Paper", "Preparing spawn area: 100%"));
    }

    #[test]
    fn executable_chat_actions_require_an_unambiguous_command() {
        assert_eq!(classify_intent("请启动服务器。"), "server_bootstrap");
        assert_eq!(classify_intent("帮我开服"), "server_bootstrap");
        assert_eq!(classify_intent("把服务器开起来"), "server_bootstrap");
        assert_eq!(classify_intent("帮我安全停服"), "server_stop");
        assert_eq!(classify_intent("为什么不能停止服务器？"), "general");
        assert_eq!(classify_intent("不要停止服务器"), "general");
    }

    #[test]
    fn plan_extraction_requires_explicit_core_and_version() {
        assert_eq!(
            extract_server_plan("推荐 Paper 1.12.2，使用 Java 8"),
            Some(("Paper".into(), "1.12.2".into()))
        );
        assert_eq!(extract_server_plan("使用最新 Paper"), None);
        assert_eq!(
            extract_server_plan("Purpur 1.21.4 RPG"),
            Some(("Purpur".into(), "1.21.4".into()))
        );
    }

    #[test]
    fn automatic_tasks_are_queued_before_the_executor_claims_them() {
        for (risk, mode) in [
            ("low", "approval"),
            ("low", "auto"),
            ("medium", "auto"),
            ("high", "full"),
        ] {
            assert_eq!(effective_task_start(risk, mode).0, "queued");
        }
        assert_eq!(
            effective_task_start("high", "approval").0,
            "awaiting_approval"
        );
    }

    #[test]
    fn task_history_trimming_never_drops_active_work() {
        let mut tasks = Vec::new();
        for index in 0..4 {
            tasks.push(new_task_record(
                "sculk".into(),
                format!("历史任务 {index}"),
                "diagnostic".into(),
                "completed".into(),
                100,
                "low".into(),
                None,
            ));
        }
        for status in ["awaiting_approval", "queued", "running", "cancelling"] {
            tasks.push(new_task_record(
                "sculk".into(),
                format!("活动任务 {status}"),
                "diagnostic".into(),
                status.into(),
                10,
                "low".into(),
                None,
            ));
        }

        trim_task_history(&mut tasks, 2);

        assert_eq!(
            tasks
                .iter()
                .filter(|task| task.status == "completed")
                .count(),
            2
        );
        for status in ["awaiting_approval", "queued", "running", "cancelling"] {
            assert!(tasks.iter().any(|task| task.status == status));
        }
    }

    #[test]
    fn stale_runtime_reconciliation_clears_only_managed_runtime_state() {
        let generation = Uuid::new_v4();
        let mut state = initial_state();
        let mut stale = test_server("stale", "online", "运行中");
        stale.pid = Some(4242);
        stale.runtime_generation = Some(generation);
        stale.started_at = Some("2026-07-29T10:00:00+08:00".into());
        stale.cpu = 32;
        stale.memory = 64;
        stale.players = "3 / 60".into();
        state.servers.push(stale);
        state
            .servers
            .push(test_server("stopped", "stopped", "已停止"));

        assert!(reconcile_stale_runtime_state(&mut state));

        let stale = state
            .servers
            .iter()
            .find(|server| server.id == "stale")
            .unwrap();
        assert_eq!(stale.status, "warning");
        assert_eq!(stale.task, "上次后端退出后运行状态待确认");
        assert_eq!(stale.players, "0 / 60");
        assert_eq!(stale.cpu, 0);
        assert_eq!(stale.memory, 0);
        assert!(stale.pid.is_none());
        assert!(stale.runtime_generation.is_none());
        assert!(stale.started_at.is_none());
        assert!(state.logs["stale"].last().unwrap().contains("原 PID 4242"));

        let stopped = state
            .servers
            .iter()
            .find(|server| server.id == "stopped")
            .unwrap();
        assert_eq!(stopped.status, "stopped");
        assert_eq!(stopped.task, "已停止");
        assert!(!state.logs.contains_key("stopped"));
    }

    #[test]
    fn runtime_reconciliation_is_driven_by_operation_state_not_display_text() {
        let mut state = initial_state();
        let mut server = test_server("transitioning", "stopped", "任意展示文案");
        server.operation_state = "starting".into();
        state.servers.push(server);

        assert!(reconcile_stale_runtime_state(&mut state));
        let server = &state.servers[0];
        assert_eq!(server.operation_state, "idle");
        assert_eq!(server.status, "warning");
        assert!(server.last_error.is_some());
    }

    #[test]
    fn legacy_core_and_provision_operation_are_repaired_from_runtime_facts() {
        let mut server = test_server("legacy", "stopped", "旧状态");
        server.operation_state = "provisioning".into();
        server.last_error = Some("stale".into());

        assert!(repair_server_operation_metadata(&mut server, true, false));
        assert!(server.core_ready);
        assert_eq!(server.operation_state, "idle");
        assert!(server.last_error.is_none());

        assert!(repair_server_operation_metadata(&mut server, false, true));
        assert!(!server.core_ready);
        assert_eq!(server.operation_state, "provisioning");
        assert!(server.last_error.is_none());
    }

    #[test]
    fn retry_reuses_active_or_completed_ready_provision_tasks() {
        let mut completed = new_task_record(
            "sculk".into(),
            "完成".into(),
            "server_provision".into(),
            "completed".into(),
            100,
            "low".into(),
            None,
        );
        let active = new_task_record(
            "sculk".into(),
            "重试".into(),
            "server_provision".into(),
            "running".into(),
            50,
            "low".into(),
            None,
        );
        completed.finished_at = Some(Local::now().to_rfc3339());
        let tasks = vec![completed.clone(), active.clone()];

        assert_eq!(
            reusable_provision_task(&tasks, "sculk", false).unwrap().id,
            active.id
        );
        assert_eq!(
            reusable_provision_task(&[completed.clone()], "sculk", true)
                .unwrap()
                .id,
            completed.id
        );
        assert!(reusable_provision_task(&[completed], "sculk", false).is_none());
    }

    #[test]
    fn start_gate_rejects_active_failed_and_unready_provisioning() {
        let mut server = test_server("sculk", "stopped", "已停止");
        server.core_ready = true;
        let active = new_task_record(
            "sculk".into(),
            "初始化".into(),
            "server_provision".into(),
            "running".into(),
            50,
            "low".into(),
            None,
        );
        assert!(server_start_blocker(&server, &[active]).is_some());

        let mut failed = new_task_record(
            "sculk".into(),
            "初始化".into(),
            "server_provision".into(),
            "failed".into(),
            90,
            "low".into(),
            None,
        );
        failed.error = Some("Java incompatible".into());
        assert_eq!(
            server_start_blocker(&server, &[failed]).as_deref(),
            Some("Java incompatible")
        );

        server.core_ready = false;
        assert!(server_start_blocker(&server, &[]).is_some());
        server.core_ready = true;
        assert!(server_start_blocker(&server, &[]).is_none());
    }

    #[tokio::test]
    async fn process_exit_waiter_observes_actor_completion() {
        let (sender, mut receiver) = watch::channel(None);
        let expected = ProcessExit {
            success: true,
            code: Some(0),
            forced: false,
            startup_timeout: false,
        };
        let sent = expected.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            sender.send(Some(sent)).unwrap();
        });

        let actual = wait_for_process_exit(&mut receiver, Duration::from_secs(1))
            .await
            .expect("actor completion should reach the waiter");
        assert_eq!(actual.success, expected.success);
        assert_eq!(actual.code, expected.code);
        assert_eq!(actual.forced, expected.forced);
        assert_eq!(actual.startup_timeout, expected.startup_timeout);
    }

    #[cfg(windows)]
    fn cooperative_test_child() -> tokio::process::Command {
        let mut command = tokio::process::Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ErrorActionPreference='Stop'; Write-Output 'Done (0.1s)!'; while (($line = [Console]::In.ReadLine()) -ne $null) { if ($line -eq 'stop') { exit 0 }; Write-Output ('ECHO:' + $line) }",
        ]);
        command
    }

    #[cfg(unix)]
    fn cooperative_test_child() -> tokio::process::Command {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args([
            "-c",
            "printf '%s\\n' 'Done (0.1s)!'; while IFS= read -r line; do [ \"$line\" = stop ] && exit 0; printf 'ECHO:%s\\n' \"$line\"; done",
        ]);
        command
    }

    #[tokio::test]
    async fn process_actor_gracefully_stops_a_real_child_and_clears_runtime_state() {
        let directory =
            std::env::temp_dir().join(format!("sculk-process-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&directory).await.unwrap();
        let file = directory.join("state.json");
        let file_lock = acquire_state_lock(&file).unwrap();

        let mut command = cooperative_test_child();
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        process_platform::configure_managed_command(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let generation = Uuid::new_v4();
        let (control, commands) = mpsc::channel(4);
        let (exit_sender, exit) = watch::channel(None);
        let managed = ManagedProcess {
            generation,
            pid,
            guard: Arc::new(process_platform::create_process_guard().unwrap()),
            control,
            exit,
        };
        let mut persisted = initial_state();
        let mut server = test_server("actor-test", "warning", "启动中");
        server.pid = Some(pid);
        server.runtime_generation = Some(generation);
        server.started_at = Some(Local::now().to_rfc3339());
        persisted.servers.push(server);
        let state = AppState {
            inner: Arc::new(RwLock::new(persisted)),
            file,
            _file_lock: Arc::new(file_lock),
            processes: Arc::new(RwLock::new(HashMap::from([(
                "actor-test".into(),
                managed.clone(),
            )]))),
            shutting_down: Arc::new(AtomicBool::new(false)),
            operation_locks: Arc::new(Mutex::new(HashMap::new())),
            channels: Arc::new(RwLock::new(HashMap::new())),
            downloads: Arc::new(RwLock::new(HashMap::new())),
            runtime_install: Arc::new(Mutex::new(())),
            task_controls: Arc::new(RwLock::new(HashMap::new())),
            cloud: cloud::CloudRuntime::disabled_for_test(),
        };

        tokio::spawn(run_process_actor(
            state.clone(),
            "actor-test".into(),
            "Paper".into(),
            managed.clone(),
            child,
            stdin,
            stdout,
            stderr,
            commands,
            exit_sender,
        ));
        request_graceful_stop(&managed).await.unwrap();
        let mut exit = managed.exit.clone();
        let report = wait_for_process_exit(&mut exit, Duration::from_secs(10))
            .await
            .expect("the real child should stop after receiving the stop command");

        assert!(report.success);
        assert!(!report.forced);
        assert!(!state.processes.read().await.contains_key("actor-test"));
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == "actor-test")
            .unwrap();
        assert_eq!(server.status, "stopped");
        assert_eq!(server.task, "已停止");
        assert!(server.pid.is_none());
        assert!(server.runtime_generation.is_none());
        assert!(
            data.logs["actor-test"]
                .iter()
                .any(|line| line.contains("Done (0.1s)!"))
        );
        drop(data);
        drop(state);
        fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn loading_missing_state_initializes_an_empty_workspace() {
        let directory =
            std::env::temp_dir().join(format!("sculk-state-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&directory).await.unwrap();
        let path = directory.join("state.json");

        let state = load_state(&path).await;

        assert!(state.servers.is_empty());
        assert!(state.tasks.is_empty());
        assert!(state.configs.is_empty());
        assert!(state.logs.is_empty());
        assert!(state.players.is_empty());
        assert!(state.feedback.is_empty());
        assert!(state.polls.is_empty());
        assert!(state.conversations.is_empty());
        assert!(!state.mirrors.is_empty());
        assert!(!state.catalog.core_projects.is_empty());
        assert!(
            state
                .skills
                .iter()
                .any(|skill| skill.id == skills::MINECRAFT_PLUGIN_SKILL_ID)
        );
        assert!(path.is_file());

        fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn loading_legacy_state_preserves_servers_and_persists_memory_defaults() {
        let directory =
            std::env::temp_dir().join(format!("sculk-state-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&directory).await.unwrap();
        let path = directory.join("state.json");
        let mut value = serde_json::to_value(initial_state()).unwrap();
        let servers = value["servers"].as_array_mut().unwrap();
        servers.push(serde_json::json!({
            "id":"custom-server",
            "name":"Custom Server",
            "core":"Paper",
            "version":"1.21.4",
            "status":"stopped",
            "players":"0 / 60",
            "memory":0,
            "cpu":0,
            "port":25568,
            "task":"idle"
        }));
        for server in servers.iter_mut() {
            server.as_object_mut().unwrap().remove("memory_gb");
        }
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap())
            .await
            .unwrap();

        let state = load_state(&path).await;
        assert_eq!(state.servers.len(), 1);
        assert!(
            state
                .servers
                .iter()
                .any(|server| server.id == "custom-server")
        );
        assert!(
            state
                .servers
                .iter()
                .all(|server| server.memory_gb == DEFAULT_MEMORY_GB)
        );
        let persisted: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).await.unwrap()).unwrap();
        assert!(
            persisted["servers"]
                .as_array()
                .unwrap()
                .iter()
                .all(|server| server["memory_gb"] == DEFAULT_MEMORY_GB)
        );

        fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn atomic_state_write_keeps_previous_version_as_backup() {
        let directory =
            std::env::temp_dir().join(format!("sculk-state-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&directory).await.unwrap();
        let path = directory.join("state.json");
        let first = initial_state();
        write_state_file(&path, &first, true).await.unwrap();

        let mut second = first.clone();
        second.servers.clear();
        write_state_file(&path, &second, true).await.unwrap();

        let (current, _) = read_state(&path).await.unwrap();
        let (backup, _) = read_state(&state_sidecar_path(&path, ".bak"))
            .await
            .unwrap();
        assert!(current.servers.is_empty());
        assert_eq!(backup.servers.len(), first.servers.len());

        fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    async fn loading_corrupt_primary_recovers_valid_backup() {
        let directory =
            std::env::temp_dir().join(format!("sculk-state-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&directory).await.unwrap();
        let path = directory.join("state.json");
        let expected = initial_state();
        fs::write(&path, b"{broken").await.unwrap();
        fs::write(
            state_sidecar_path(&path, ".bak"),
            serde_json::to_vec_pretty(&expected).unwrap(),
        )
        .await
        .unwrap();

        let restored = load_state(&path).await;
        let (persisted, _) = read_state(&path).await.unwrap();
        assert_eq!(restored.servers.len(), expected.servers.len());
        assert_eq!(persisted.servers.len(), expected.servers.len());

        fs::remove_dir_all(directory).await.unwrap();
    }
}
