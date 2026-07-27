mod acp;
mod ai;
mod catalog;
mod cloud;
mod conversations;
mod download;
mod prefs;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderValue, Method, StatusCode},
    response::Response,
    routing::{delete, get, post},
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{File as StdFile, OpenOptions as StdOpenOptions},
    path::{Component, Path as StdPath, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::ChildStdin,
    sync::{Mutex, RwLock, broadcast},
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
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<String>>>>,
    downloads: Arc<RwLock<HashMap<String, download::DownloadStatus>>>,
    cloud: cloud::CloudRuntime,
}
#[derive(Clone)]
struct ManagedProcess {
    pid: Option<u32>,
    stdin: Arc<Mutex<ChildStdin>>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ServerInfo {
    pub(crate) id: String,
    name: String,
    core: String,
    version: String,
    pub(crate) status: String,
    players: String,
    memory: u8,
    #[serde(default = "default_memory_gb")]
    memory_gb: u8,
    cpu: u8,
    port: u16,
    task: String,
    #[serde(default = "default_location")]
    location: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct TaskInfo {
    id: Uuid,
    server_id: String,
    title: String,
    kind: String,
    status: String,
    progress: u8,
    created_at: String,
    #[serde(default = "default_risk")]
    risk: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    approved_by: Option<String>,
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
#[derive(Serialize)]
struct SystemInfo {
    java_installed: bool,
    java_version: Option<String>,
    java_home: Option<String>,
    os: String,
    arch: String,
    data_dir: String,
    recommended_java: u8,
    cores: Vec<String>,
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
    directory: String,
    files: Vec<String>,
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

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[tokio::main]
async fn main() {
    let file = PathBuf::from(
        std::env::var("SCULK_STATE_FILE").unwrap_or_else(|_| "data/state.json".into()),
    );
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
        channels: Arc::new(RwLock::new(HashMap::new())),
        downloads: Arc::new(RwLock::new(HashMap::new())),
        cloud,
    };
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
    let app = Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/dashboard", get(get_dashboard))
        .route("/api/chat", post(chat))
        .route("/api/system", get(get_system_info))
        .route("/api/servers", post(create_server))
        .route("/api/servers/plan", post(plan_server))
        .route("/api/servers/{id}", delete(delete_server))
        .route("/api/servers/{id}/action", post(server_action))
        .route("/api/servers/{id}/command", post(run_command))
        .route(
            "/api/servers/{id}/config",
            get(get_config).put(update_config),
        )
        .route("/api/servers/{id}/logs", get(get_logs))
        .route("/api/servers/{id}/ws/logs", get(ws_logs))
        .route("/api/servers/{id}/files", get(list_files))
        .route("/api/servers/{id}/file", get(read_file).put(write_file))
        .route("/api/servers/{id}/directory", post(create_directory))
        .route("/api/download/mirrors", get(get_mirrors))
        .route("/api/download/preview", post(preview_downloads))
        .route("/api/automation", get(get_automation))
        .route("/api/automation/tasks", post(create_automation_task))
        .route("/api/tasks/{id}/approve", post(approve_task))
        .route("/api/tasks/{id}/cancel", post(cancel_task))
        .route("/api/community", get(get_community))
        .route("/api/polls", post(create_poll))
        .route("/api/polls/{id}/vote", post(vote_poll))
        .route("/api/feedback/cluster", post(cluster_feedback))
        .route("/api/players/{id}/action", post(player_action))
        .route("/api/integrations", get(get_integrations))
        .route("/api/integrations/{id}/toggle", post(toggle_integration))
        .route("/api/integrations/{id}/test", post(test_integration))
        .route("/api/skills/{id}/toggle", post(toggle_skill))
        .merge(catalog::router())
        .merge(cloud::router())
        .merge(download::router())
        .merge(ai::router())
        .merge(prefs::router())
        .merge(conversations::router())
        .with_state(state)
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
    axum::serve(listener, app).await.expect("API server failed");
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
        .read(true)
        .write(true)
        .open(state_sidecar_path(file, ".lock"))?;
    lock.try_lock()?;
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

async fn load_state(file: &PathBuf) -> PersistedState {
    if let Ok((mut state, data)) = read_state(file).await {
        let needs_persist = state.catalog.migrate() || legacy_servers_missing_memory(&data);
        if needs_persist {
            if let Err(error) = write_state_file(file, &state, true).await {
                eprintln!("failed to persist migrated state: {error}");
            }
        }
        return state;
    }

    let backup = state_sidecar_path(file, ".bak");
    if let Ok((mut state, _)) = read_state(&backup).await {
        state.catalog.migrate();
        if let Err(error) = write_state_file(file, &state, false).await {
            eprintln!("failed to restore state backup: {error}");
        } else {
            eprintln!("restored state from {}", backup.display());
        }
        return state;
    }

    let state = seed_state();
    if let Err(error) = write_state_file(file, &state, false).await {
        eprintln!("failed to initialize state: {error}");
    }
    state
}
async fn persist(state: &AppState, data: &PersistedState) -> Result<(), String> {
    write_state_file(&state.file, data, true).await
}
async fn get_system_info() -> Json<SystemInfo> {
    let java = tokio::process::Command::new("java")
        .arg("-version")
        .output()
        .await
        .ok();
    let java_version = java.as_ref().and_then(|output| {
        let text = String::from_utf8_lossy(&output.stderr);
        text.lines().next().map(|line| line.trim().to_string())
    });
    Json(SystemInfo {
        java_installed: java
            .as_ref()
            .map(|output| output.status.success())
            .unwrap_or(false),
        java_version,
        java_home: std::env::var("JAVA_HOME").ok(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        data_dir: "backend/data/servers".into(),
        recommended_java: 21,
        cores: vec![
            "Paper".into(),
            "Purpur".into(),
            "Fabric".into(),
            "Velocity".into(),
        ],
    })
}
async fn create_server(
    State(state): State<AppState>,
    Json(request): Json<CreateServerRequest>,
) -> ApiResult<CreateServerResponse> {
    if request.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "server name is required".into()));
    }
    validate_location(request.location.as_deref())?;
    if !request.eula_accepted {
        return Err((
            StatusCode::BAD_REQUEST,
            "Minecraft EULA must be accepted".into(),
        ));
    }
    let start_script = render_start_script(request.memory_gb)
        .map_err(|message| (StatusCode::BAD_REQUEST, message.into()))?;
    let mut data = state.inner.write().await;
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
    let directory = PathBuf::from("data/servers").join(&id);
    fs::create_dir_all(directory.join("plugins"))
        .await
        .map_err(|error| internal(error.to_string()))?;
    fs::create_dir_all(directory.join("logs"))
        .await
        .map_err(|error| internal(error.to_string()))?;
    let config = format!(
        "# {}\nserver-port={}\nmax-players=60\nview-distance=10\nsimulation-distance=8\nonline-mode=true\ndifficulty=normal\npvp=true\nmotd=§3{} §8| §fPowered by Sculk Catalyst",
        request.name.trim(),
        request.port,
        request.name.trim()
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
    let server = ServerInfo {
        id: id.clone(),
        name: request.name.trim().into(),
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
        TaskInfo {
            id: Uuid::new_v4(),
            server_id: id.clone(),
            title: "选择核心镜像并预览下载接口".into(),
            kind: "bootstrap".into(),
            status: "queued".into(),
            progress: 10,
            created_at: Local::now().to_rfc3339(),
            risk: "low".into(),
            approved_by: None,
        },
    );
    data.servers.push(server.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(CreateServerResponse {
        server,
        directory: directory.to_string_lossy().to_string(),
        files: vec![
            "server.properties".into(),
            "eula.txt".into(),
            "start.ps1".into(),
            "plugins/".into(),
            "logs/".into(),
        ],
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
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "server name is required".into()));
    }
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
    // 运行中的服务器先安全停止：发送 stop 后轮询等待进程退出（wait 任务会移除 map 条目）。
    let process = state.processes.read().await.get(&id).cloned();
    if let Some(process) = process {
        {
            let mut stdin = process.stdin.lock().await;
            let _ = stdin.write_all(b"stop\n").await;
            let _ = stdin.flush().await;
        }
        let mut stopped = false;
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if !state.processes.read().await.contains_key(&id) {
                stopped = true;
                break;
            }
        }
        if !stopped {
            return Err((
                StatusCode::CONFLICT,
                "服务器进程未能在 10 秒内停止，请稍后重试".into(),
            ));
        }
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
        let base = PathBuf::from("data/servers");
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
    let mut data = state.inner.write().await;
    let risk = intent_risk(intent);
    let (status, progress, approved_by) = effective_task_start(risk, &data.ai.review_mode);
    let task = TaskInfo {
        id: Uuid::new_v4(),
        server_id: request.server_id.clone(),
        title: task_title(intent).into(),
        kind: intent.into(),
        status: status.into(),
        progress,
        created_at: Local::now().to_rfc3339(),
        risk: risk.into(),
        approved_by: approved_by.map(Into::into),
    };
    data.tasks.insert(0, task.clone());
    data.tasks.truncate(30);
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(ChatResponse {
        id: Uuid::new_v4(),
        message: format!("{}\n\n目标服务器：{}", body, request.server_id),
        time: Local::now().format("%H:%M").to_string(),
        actions: vec!["审阅执行计划".into(), "在镜像服运行".into()],
        task: Some(task),
    }))
}
async fn server_action(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<ActionRequest>,
) -> ApiResult<ActionResponse> {
    match request.action.as_str() {
        "start" => start_server(state, id).await,
        "stop" => stop_server(state, id).await,
        _ => Err((StatusCode::BAD_REQUEST, "unsupported action".into())),
    }
}
async fn start_server(state: AppState, id: String) -> ApiResult<ActionResponse> {
    let downloads = state.downloads.read().await;
    if downloads.get(&id).is_some_and(download::is_active) {
        return Err((
            StatusCode::CONFLICT,
            "核心下载进行中，请等待校验和安装完成后再启动服务器".into(),
        ));
    }
    let mut processes = state.processes.write().await;
    if processes.contains_key(&id) {
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
        let java_args = server_java_args(server.memory_gb).map_err(|_| {
            (
                StatusCode::CONFLICT,
                "服务器内存配置无效，必须在 2 到 64 GB 之间".into(),
            )
        })?;
        (server, java_args)
    };
    let directory = PathBuf::from("data/servers").join(&id);
    if fs::metadata(directory.join("server.jar")).await.is_err() {
        return Err((
            StatusCode::CONFLICT,
            "server.jar 尚未就绪，请先执行初始化任务".into(),
        ));
    }
    let java = std::env::var("JAVA_HOME")
        .ok()
        .map(|home| PathBuf::from(home).join("bin/java.exe"))
        .filter(|path| path.exists())
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "java".into());
    let mut child = tokio::process::Command::new(java)
        .current_dir(&directory)
        .args(java_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("无法启动 Java：{error}"),
            )
        })?;
    let pid = child.id();
    let stdin = child.stdin.take().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "无法连接服务器标准输入".into(),
    ))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    processes.insert(
        id.clone(),
        ManagedProcess {
            pid,
            stdin: Arc::new(Mutex::new(stdin)),
        },
    );
    drop(processes);
    drop(downloads);
    let line = format!(
        "[{} AI]: 正在使用 Java 启动 {}，内存上限 {} GB，PID {:?}。",
        Local::now().format("%H:%M:%S"),
        server.name,
        server.memory_gb,
        pid
    );
    broadcast_line(&state, &id, &line).await;
    let updated = {
        let mut data = state.inner.write().await;
        let server = data
            .servers
            .iter_mut()
            .find(|server| server.id == id)
            .unwrap();
        server.status = "warning".into();
        server.task = "首次启动中".into();
        let updated = server.clone();
        data.logs.entry(id.clone()).or_default().push(line.clone());
        persist(&state, &data).await.map_err(internal)?;
        updated
    };
    if let Some(stdout) = stdout {
        let output_state = state.clone();
        let output_id = id.clone();
        tokio::spawn(async move {
            stream_output(output_state, output_id, BufReader::new(stdout)).await
        });
    }
    if let Some(stderr) = stderr {
        let error_state = state.clone();
        let error_id = id.clone();
        tokio::spawn(
            async move { stream_output(error_state, error_id, BufReader::new(stderr)).await },
        );
    }
    let wait_state = state.clone();
    let wait_id = id.clone();
    tokio::spawn(async move {
        let result = child.wait().await;
        wait_state.processes.write().await.remove(&wait_id);
        let exit_line = format!(
            "[{} INFO]: Java 进程已退出：{:?}",
            Local::now().format("%H:%M:%S"),
            result
        );
        broadcast_line(&wait_state, &wait_id, &exit_line).await;
        let mut data = wait_state.inner.write().await;
        if let Some(server) = data.servers.iter_mut().find(|server| server.id == wait_id) {
            server.status = "stopped".into();
            server.cpu = 0;
            server.memory = 0;
            server.players = "0 / 60".into();
            server.task = "已停止".into()
        }
        data.logs.entry(wait_id).or_default().push(exit_line);
        let _ = persist(&wait_state, &data).await;
    });
    Ok(Json(ActionResponse {
        server: updated,
        log: line,
    }))
}
async fn stop_server(state: AppState, id: String) -> ApiResult<ActionResponse> {
    let process = state.processes.read().await.get(&id).cloned();
    let has_process = process.is_some();
    if let Some(ref process) = process {
        let mut stdin = process.stdin.lock().await;
        stdin
            .write_all(b"stop\n")
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        stdin
            .flush()
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    }
    let line = format!(
        "[{} AI]: 已发送安全停服指令{}。",
        Local::now().format("%H:%M:%S"),
        process
            .as_ref()
            .and_then(|process| process.pid)
            .map(|pid| format!("给 PID {pid}"))
            .unwrap_or_default()
    );
    broadcast_line(&state, &id, &line).await;
    let updated = {
        let mut data = state.inner.write().await;
        let server = data
            .servers
            .iter_mut()
            .find(|server| server.id == id)
            .ok_or((StatusCode::NOT_FOUND, "server not found".into()))?;
        server.status = if has_process {
            "warning".into()
        } else {
            "stopped".into()
        };
        server.task = if has_process {
            "正在保存并停止".into()
        } else {
            "已停止".into()
        };
        let updated = server.clone();
        data.logs.entry(id).or_default().push(line.clone());
        persist(&state, &data).await.map_err(internal)?;
        updated
    };
    Ok(Json(ActionResponse {
        server: updated,
        log: line,
    }))
}
async fn stream_output<R: tokio::io::AsyncRead + Unpin>(
    state: AppState,
    id: String,
    reader: BufReader<R>,
) {
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let online = line.contains("Done (");
        broadcast_line(&state, &id, &line).await;
        let mut data = state.inner.write().await;
        data.logs.entry(id.clone()).or_default().push(line);
        if let Some(logs) = data.logs.get_mut(&id)
            && logs.len() > 1000
        {
            logs.drain(0..logs.len() - 1000);
        }
        if online && let Some(server) = data.servers.iter_mut().find(|server| server.id == id) {
            server.status = "online".into();
            server.cpu = 12;
            server.memory = 38;
            server.task = "运行中".into()
        }
        let _ = persist(&state, &data).await;
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
    let process = state.processes.read().await.get(&id).cloned();
    let time = Local::now().format("%H:%M:%S");
    let lines = if let Some(process) = process {
        // 进程运行中：真实写入 Java 标准输入，输出由 stream_output 经日志流回传
        let mut stdin = process.stdin.lock().await;
        stdin
            .write_all(format!("{command}\n").as_bytes())
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("命令写入失败：{error}"),
                )
            })?;
        stdin.flush().await.map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("命令写入失败：{error}"),
            )
        })?;
        vec![
            format!("> {command}"),
            format!("[{time} AI]: 命令已发送到服务器进程，输出将实时显示在日志流中。"),
        ]
    } else {
        let data = state.inner.read().await;
        let output = command_output(&command, &data.servers, &id);
        vec![
            format!("> {command}"),
            format!("[{time} SIM]: {output}"),
            format!("[{time} AI]: 服务器进程未运行，以上为模拟输出。"),
        ]
    };
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
    let directory = resolve_existing(&root, &relative).await?;
    let metadata = fs::metadata(&directory)
        .await
        .map_err(|error| (StatusCode::NOT_FOUND, error.to_string()))?;
    if !metadata.is_dir() {
        return Err((StatusCode::BAD_REQUEST, "path is not a directory".into()));
    }
    let mut reader = fs::read_dir(&directory)
        .await
        .map_err(|error| internal(error.to_string()))?;
    let mut entries = Vec::new();
    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|error| internal(error.to_string()))?
    {
        let metadata = entry
            .metadata()
            .await
            .map_err(|error| internal(error.to_string()))?;
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| internal(error.to_string()))?;
        if file_type.is_symlink() {
            continue;
        }
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
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs()),
        });
    }
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
    let target = resolve_existing(&root, &relative).await?;
    let metadata = fs::metadata(&target)
        .await
        .map_err(|error| (StatusCode::NOT_FOUND, error.to_string()))?;
    if !metadata.is_file() {
        return Err((StatusCode::BAD_REQUEST, "path is not a file".into()));
    }
    if metadata.len() > 2_000_000 {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "text file exceeds 2 MB".into(),
        ));
    }
    let bytes = fs::read(&target)
        .await
        .map_err(|error| internal(error.to_string()))?;
    let content = String::from_utf8(bytes).map_err(|_| {
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "binary files cannot be opened in the text editor".into(),
        )
    })?;
    Ok(Json(FileContentResponse {
        path: path_string(&relative),
        content,
        size: metadata.len(),
        readonly: !is_editable(&relative),
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
    if !is_editable(&relative) {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "this file type is read-only".into(),
        ));
    }
    let target = resolve_for_write(&root, &relative).await?;
    fs::write(&target, request.content.as_bytes())
        .await
        .map_err(|error| internal(error.to_string()))?;
    if path_string(&relative) == "server.properties" {
        let mut data = state.inner.write().await;
        data.configs.insert(id, request.content.clone());
        persist(&state, &data).await.map_err(internal)?
    }
    Ok(Json(FileContentResponse {
        path: path_string(&relative),
        size: request.content.len() as u64,
        content: request.content,
        readonly: false,
    }))
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
    let target = resolve_for_write(&root, &relative).await?;
    fs::create_dir_all(target)
        .await
        .map_err(|error| internal(error.to_string()))?;
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
    let config = {
        let data = state.inner.read().await;
        if !data.servers.iter().any(|server| server.id == id) {
            return Err((StatusCode::NOT_FOUND, "server not found".into()));
        }
        data.configs.get(id).cloned().unwrap_or_default()
    };
    let root = PathBuf::from("data/servers").join(id);
    fs::create_dir_all(root.join("plugins"))
        .await
        .map_err(|error| internal(error.to_string()))?;
    fs::create_dir_all(root.join("logs"))
        .await
        .map_err(|error| internal(error.to_string()))?;
    let properties = root.join("server.properties");
    if fs::metadata(&properties).await.is_err() {
        fs::write(&properties, config)
            .await
            .map_err(|error| internal(error.to_string()))?
    }
    Ok(root)
}
fn safe_relative(value: &str) -> Result<PathBuf, (StatusCode, String)> {
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
async fn resolve_existing(
    root: &PathBuf,
    relative: &PathBuf,
) -> Result<PathBuf, (StatusCode, String)> {
    let root_canonical = fs::canonicalize(root)
        .await
        .map_err(|error| internal(error.to_string()))?;
    let target = root.join(relative);
    let metadata = fs::symlink_metadata(&target)
        .await
        .map_err(|error| (StatusCode::NOT_FOUND, error.to_string()))?;
    if metadata.file_type().is_symlink() {
        return Err((
            StatusCode::FORBIDDEN,
            "symbolic links are not allowed".into(),
        ));
    }
    let canonical = fs::canonicalize(&target)
        .await
        .map_err(|error| (StatusCode::NOT_FOUND, error.to_string()))?;
    if !canonical.starts_with(&root_canonical) {
        return Err((
            StatusCode::FORBIDDEN,
            "path escapes server workspace".into(),
        ));
    }
    Ok(canonical)
}
async fn resolve_for_write(
    root: &PathBuf,
    relative: &PathBuf,
) -> Result<PathBuf, (StatusCode, String)> {
    let root_canonical = fs::canonicalize(root)
        .await
        .map_err(|error| internal(error.to_string()))?;
    let target = root.join(relative);
    let parent = target
        .parent()
        .ok_or((StatusCode::BAD_REQUEST, "invalid path".into()))?;
    fs::create_dir_all(parent)
        .await
        .map_err(|error| internal(error.to_string()))?;
    let parent_canonical = fs::canonicalize(parent)
        .await
        .map_err(|error| internal(error.to_string()))?;
    if !parent_canonical.starts_with(&root_canonical) {
        return Err((
            StatusCode::FORBIDDEN,
            "path escapes server workspace".into(),
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(&target).await
        && metadata.file_type().is_symlink()
    {
        return Err((
            StatusCode::FORBIDDEN,
            "symbolic links are not allowed".into(),
        ));
    }
    Ok(target)
}
fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
fn is_editable(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "properties"
                    | "yml"
                    | "yaml"
                    | "json"
                    | "toml"
                    | "txt"
                    | "conf"
                    | "cfg"
                    | "ini"
                    | "md"
                    | "ps1"
                    | "sh"
                    | "log"
            )
        })
        .unwrap_or(false)
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
            .filter(|task| task.status == "queued" && task.risk != "low")
            .count(),
        running: tasks.iter().filter(|task| task.status == "running").count(),
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
    let mut data = state.inner.write().await;
    let (status, progress, approved_by) = effective_task_start(&request.risk, &data.ai.review_mode);
    let task = TaskInfo {
        id: Uuid::new_v4(),
        server_id: request.server_id,
        title: request.title.trim().into(),
        kind: request.kind,
        status: status.into(),
        progress,
        created_at: Local::now().to_rfc3339(),
        risk: request.risk,
        approved_by: approved_by.map(Into::into),
    };
    data.tasks.insert(0, task.clone());
    data.tasks.truncate(50);
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(task))
}
async fn approve_task(Path(id): Path<Uuid>, State(state): State<AppState>) -> ApiResult<TaskInfo> {
    let mut data = state.inner.write().await;
    let task = data
        .tasks
        .iter_mut()
        .find(|task| task.id == id)
        .ok_or((StatusCode::NOT_FOUND, "task not found".into()))?;
    task.status = "running".into();
    task.progress = 10;
    task.approved_by = Some("user".into());
    let result = task.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(result))
}
async fn cancel_task(Path(id): Path<Uuid>, State(state): State<AppState>) -> ApiResult<TaskInfo> {
    let mut data = state.inner.write().await;
    let task = data
        .tasks
        .iter_mut()
        .find(|task| task.id == id)
        .ok_or((StatusCode::NOT_FOUND, "task not found".into()))?;
    task.status = "cancelled".into();
    let result = task.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(result))
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
pub(crate) fn classify_intent(message: &str) -> &'static str {
    if message.contains("报错") || message.contains("修复") {
        "repair"
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
        "repair" => "自动诊断与修复",
        "vote" => "创建玩家玩法投票",
        "promotion" => "生成服务器宣传内容",
        "plugin" => "插件镜像服交付测试",
        _ => "AI 服务器管理任务",
    }
}
pub(crate) fn rule_reply(intent: &str) -> &'static str {
    match intent {
        "repair" => {
            "我已创建故障诊断任务：解析最新日志与崩溃报告，对照插件依赖和 Java 环境生成修复补丁，并优先在镜像服验证。"
        }
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
            "我已将需求拆分为可审阅任务。无风险检查会自动执行；涉及停服、覆盖配置、玩家资产或经济参数时会请求批准。"
        }
    }
}
pub(crate) fn intent_risk(intent: &str) -> &'static str {
    if intent == "repair" { "medium" } else { "low" }
}
/// 依据风险等级与全局审核模式决定任务的初始状态。
/// approval：仅 low 自动执行；auto：low/medium 自动，medium 记 AI 代批；full：全部自动。
pub(crate) fn effective_task_start(
    risk: &str,
    review_mode: &str,
) -> (&'static str, u8, Option<&'static str>) {
    match (review_mode, risk) {
        ("full", _) => ("running", 12, Some("auto")),
        ("auto", "low") => ("running", 12, None),
        ("auto", "medium") => ("running", 12, Some("ai")),
        (_, "low") => ("running", 12, None),
        _ => ("queued", 0, None),
    }
}
fn command_output(command: &str, servers: &[ServerInfo], id: &str) -> String {
    match command.trim() {
        "list" => servers
            .iter()
            .find(|s| s.id == id)
            .map(|s| {
                format!(
                    "There are {} players online.",
                    s.players.split('/').next().unwrap_or("0").trim()
                )
            })
            .unwrap_or_default(),
        "tps" => "TPS from last 1m, 5m, 15m: 19.98, 19.96, 19.97".into(),
        "save-all" => "Saved the game".into(),
        _ => "Command executed successfully by Sculk Agent.".into(),
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

const DEFAULT_MEMORY_GB: u8 = 8;
const MIN_MEMORY_GB: u8 = 2;
const MAX_MEMORY_GB: u8 = 64;
const INITIAL_HEAP_GB: u8 = 2;

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
fn seed_state() -> PersistedState {
    let servers = vec![
        ServerInfo {
            id: "sculk".into(),
            name: "Sculk 生存服".into(),
            core: "Paper".into(),
            version: "1.21.4".into(),
            status: "online".into(),
            players: "18 / 60".into(),
            memory: 63,
            memory_gb: 8,
            cpu: 28,
            port: 25565,
            task: "玩法迭代".into(),
            location: "local".into(),
        },
        ServerInfo {
            id: "mirror".into(),
            name: "镜像测试服".into(),
            core: "Purpur".into(),
            version: "1.21.4".into(),
            status: "warning".into(),
            players: "3 / 12".into(),
            memory: 41,
            memory_gb: 8,
            cpu: 16,
            port: 25566,
            task: "插件测试".into(),
            location: "local".into(),
        },
        ServerInfo {
            id: "event".into(),
            name: "周末活动服".into(),
            core: "Fabric".into(),
            version: "1.21.1".into(),
            status: "stopped".into(),
            players: "0 / 40".into(),
            memory: 0,
            memory_gb: 8,
            cpu: 0,
            port: 25567,
            task: "待部署".into(),
            location: "local".into(),
        },
    ];
    let mut configs = HashMap::new();
    for server in &servers {
        configs.insert(server.id.clone(),format!("# {}\nserver-port={}\nmax-players=60\nview-distance=10\nsimulation-distance=8\nonline-mode=true\ndifficulty=hard\npvp=true\nmotd=§3Sculk Realm §8| §fAI 驱动的新世代生存服",server.name,server.port));
    }
    let mut logs = HashMap::new();
    logs.insert(
        "sculk".into(),
        vec![
            "[10:23:41 INFO]: Starting minecraft server version 1.21.4".into(),
            "[10:23:50 INFO]: Done (8.731s)! For help, type help".into(),
        ],
    );
    PersistedState {
        servers,
        tasks: vec![],
        configs,
        logs,
        mirrors: seed_mirrors(),
        players: seed_players(),
        feedback: seed_feedback(),
        polls: seed_polls(),
        integrations: seed_integrations(),
        skills: seed_skills(),
        catalog: catalog::seed_catalog(),
        ai: ai::AiSettings::default(),
        ui: prefs::UiSettings::default(),
        conversations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(server.memory_gb, DEFAULT_MEMORY_GB);
        assert_eq!(
            serde_json::to_value(server).unwrap()["memory_gb"],
            DEFAULT_MEMORY_GB
        );
        assert!(legacy_servers_missing_memory(&format!(
            r#"{{"servers":[{legacy}]}}"#
        )));
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
        }
    }

    #[tokio::test]
    async fn loading_legacy_state_preserves_servers_and_persists_memory_defaults() {
        let directory =
            std::env::temp_dir().join(format!("sculk-state-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&directory).await.unwrap();
        let path = directory.join("state.json");
        let mut value = serde_json::to_value(seed_state()).unwrap();
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
        assert_eq!(state.servers.len(), 4);
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
        let first = seed_state();
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
        let expected = seed_state();
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
