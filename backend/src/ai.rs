use crate::acp::AcpClient;
use crate::cli_tools::{CLAUDE_EFFORTS, CODEX_EFFORTS, MODEL_EFFORTS};
use crate::{
    AppState, effective_task_start, intent_risk, internal, persist, rule_reply, task_title,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post, put},
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    convert::Infallible,
    fs,
    io::Read,
    path::{Path as FsPath, PathBuf},
    process::{ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, watch},
};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use url::Url;
use uuid::Uuid;

type ApiError = (StatusCode, String);
type ApiResult<T> = Result<Json<T>, ApiError>;

pub(crate) const SCENARIOS: [&str; 7] = [
    "chat",
    "automation",
    "setup",
    "config",
    "community",
    "speech",
    "repair",
];
const REVIEW_MODES: [&str; 3] = ["approval", "auto", "full"];
const AGENT_KINDS: [&str; 5] = ["codex", "claude-code", "openclaw", "hermes", "custom"];
const HISTORY_CHAR_LIMIT: usize = 16_000;
const CHAT_MESSAGE_CHAR_LIMIT: usize = 64_000;
const CHAT_HISTORY_TURN_LIMIT: usize = crate::conversations::MAX_MESSAGES;
const ACP_MAX_TEXT_FILE_BYTES: u64 = 2_000_000;
const MAX_ASR_AUDIO_BYTES: usize = 25 * 1024 * 1024;
const SPEECH_RECOGNITION_MODES: [&str; 2] = ["browser", "model"];
const CLI_MAX_TOTAL_OUTPUT_BYTES: usize = 1024 * 1024;
const CLI_MAX_VISIBLE_OUTPUT_BYTES: usize = CLI_MAX_TOTAL_OUTPUT_BYTES;
const CLI_MAX_STDERR_CAPTURE_BYTES: usize = 64 * 1024;
const CLI_OUTPUT_READ_CHUNK_BYTES: usize = 8 * 1024;
const CLI_MAX_REDACTION_SECRET_BYTES: usize = 4 * 1024;
const CODEX_FULL_ACCESS_ENV: &str = "SCULK_ALLOW_CODEX_FULL";
const CODEX_TRUSTED_COMMAND_ENV: &str = "SCULK_CODEX_TRUSTED_COMMAND";
const CLI_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TMPDIR",
    "TMP",
    "TEMP",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
];

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AiProvider {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) models: Vec<AiModel>,
    #[serde(default)]
    pub(crate) models_synced_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AiModel {
    pub(crate) id: String,
    pub(crate) enabled: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct ModelBinding {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AiAgent {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default = "default_agent_transport")]
    pub(crate) transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) enabled: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SpeechRecognitionSettings {
    #[serde(default = "default_speech_recognition_mode")]
    pub(crate) mode: String,
    #[serde(default = "default_speech_recognition_language")]
    pub(crate) language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) provider_id: Option<String>,
    #[serde(default = "default_speech_recognition_model")]
    pub(crate) model_id: String,
}

impl Default for SpeechRecognitionSettings {
    fn default() -> Self {
        Self {
            mode: default_speech_recognition_mode(),
            language: default_speech_recognition_language(),
            provider_id: None,
            model_id: default_speech_recognition_model(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AiSettings {
    #[serde(default)]
    pub(crate) providers: Vec<AiProvider>,
    #[serde(default)]
    pub(crate) scenarios: HashMap<String, ModelBinding>,
    #[serde(default)]
    pub(crate) default_binding: Option<ModelBinding>,
    #[serde(default = "default_review_mode")]
    pub(crate) review_mode: String,
    #[serde(default)]
    pub(crate) agents: Vec<AiAgent>,
    #[serde(default)]
    pub(crate) active_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
    #[serde(default)]
    pub(crate) speech_recognition: SpeechRecognitionSettings,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            scenarios: HashMap::new(),
            default_binding: None,
            review_mode: default_review_mode(),
            agents: Vec::new(),
            active_agent: None,
            reasoning_effort: None,
            speech_recognition: SpeechRecognitionSettings::default(),
        }
    }
}

fn default_agent_transport() -> String {
    "acp".into()
}

fn default_review_mode() -> String {
    "approval".into()
}

fn default_speech_recognition_mode() -> String {
    "browser".into()
}

fn default_speech_recognition_language() -> String {
    "zh-CN".into()
}

fn default_speech_recognition_model() -> String {
    "whisper-1".into()
}

#[derive(Serialize)]
struct AiProviderView {
    id: String,
    name: String,
    base_url: String,
    enabled: bool,
    api_key_masked: String,
    has_key: bool,
    models: Vec<AiModel>,
    models_synced_at: Option<String>,
}

impl From<&AiProvider> for AiProviderView {
    fn from(provider: &AiProvider) -> Self {
        Self {
            id: provider.id.clone(),
            name: provider.name.clone(),
            base_url: provider.base_url.clone(),
            enabled: provider.enabled,
            api_key_masked: mask_key(&provider.api_key),
            has_key: !provider.api_key.is_empty(),
            models: provider.models.clone(),
            models_synced_at: provider.models_synced_at.clone(),
        }
    }
}

#[derive(Serialize)]
struct AiSettingsView {
    providers: Vec<AiProviderView>,
    scenarios: HashMap<String, ModelBinding>,
    default_binding: Option<ModelBinding>,
    review_mode: String,
    codex_full_access_available: bool,
    codex_full_access_ready_agent_ids: Vec<String>,
    agents: Vec<AiAgent>,
    active_agent: Option<String>,
    reasoning_effort: Option<String>,
    reasoning_effort_values: &'static [&'static str],
    detected_agents: Vec<crate::cli_tools::DetectedAgent>,
    speech_recognition: SpeechRecognitionSettings,
}

impl AiSettingsView {
    fn new(settings: &AiSettings, detected_agents: Vec<crate::cli_tools::DetectedAgent>) -> Self {
        let codex_full_access_available = codex_full_access_allowed();
        let codex_full_access_ready_agent_ids = if codex_full_access_available {
            settings
                .agents
                .iter()
                .filter(|agent| {
                    agent.kind == "codex"
                        && agent.transport == "cli"
                        && codex_command_is_trusted(&agent.command)
                })
                .map(|agent| agent.id.clone())
                .collect()
        } else {
            Vec::new()
        };
        Self {
            providers: settings.providers.iter().map(Into::into).collect(),
            scenarios: settings.scenarios.clone(),
            default_binding: settings.default_binding.clone(),
            review_mode: settings.review_mode.clone(),
            codex_full_access_available,
            codex_full_access_ready_agent_ids,
            agents: settings.agents.clone(),
            active_agent: settings.active_agent.clone(),
            reasoning_effort: settings.reasoning_effort.clone(),
            reasoning_effort_values: MODEL_EFFORTS,
            detected_agents,
            speech_recognition: settings.speech_recognition.clone(),
        }
    }
}

impl From<&AiSettings> for AiSettingsView {
    fn from(settings: &AiSettings) -> Self {
        Self::new(settings, crate::cli_tools::cached_detected_agents())
    }
}

#[derive(Deserialize)]
struct UpsertProviderRequest {
    name: String,
    base_url: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct ToggleModelRequest {
    model_id: String,
}

#[derive(Deserialize)]
struct ModelIdRequest {
    model_id: String,
}

#[derive(Deserialize)]
struct TestModelRequest {
    provider_id: String,
    model_id: String,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

#[derive(Serialize)]
struct TestResult {
    ok: bool,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Deserialize)]
struct SetScenarioRequest {
    scenario: String,
    binding: Option<ModelBinding>,
}

#[derive(Deserialize)]
struct SetReviewModeRequest {
    mode: String,
}

#[derive(Deserialize)]
struct SetReasoningEffortRequest {
    reasoning_effort: Option<String>,
}

#[derive(Serialize)]
struct TranscriptionResult {
    text: String,
}

#[derive(Deserialize)]
struct UpsertAgentRequest {
    name: String,
    kind: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_agent_transport")]
    transport: String,
    #[serde(default)]
    reasoning_effort: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct SetActiveAgentRequest {
    agent_id: Option<String>,
}

#[derive(Clone, Deserialize)]
struct ChatTurn {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatStreamRequest {
    server_id: String,
    message: String,
    #[serde(default)]
    history: Vec<ChatTurn>,
    #[serde(default)]
    model_override: Option<ModelBinding>,
    #[serde(default)]
    agent_override: Option<String>, // "default" 强制内置模型直连；agent id 强制走该 Agent；None 按 active_agent
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

fn skill_query_for_request(request: &ChatStreamRequest, is_planning: bool) -> String {
    let mut parts = vec![request.message.as_str()];
    parts.extend(
        request
            .history
            .iter()
            .rev()
            .filter(|turn| turn.role == "user")
            .take(6)
            .map(|turn| turn.content.as_str()),
    );
    let mut query = parts.join("\n");
    if is_planning {
        query.insert_str(0, "当前处于服务器规划模式。\n");
    }
    query.chars().take(8_000).collect()
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ai/settings", get(get_settings))
        .route("/api/ai/providers", post(create_provider))
        .route(
            "/api/ai/providers/{id}",
            put(update_provider).delete(delete_provider),
        )
        .route("/api/ai/providers/{id}/models/sync", post(sync_models))
        .route("/api/ai/providers/{id}/models/toggle", post(toggle_model))
        .route("/api/ai/providers/{id}/models/add", post(add_model))
        .route("/api/ai/providers/{id}/models/remove", post(remove_model))
        .route("/api/ai/test", post(test_model))
        .route("/api/ai/scenarios", put(set_scenario))
        .route("/api/ai/review-mode", put(set_review_mode))
        .route("/api/ai/reasoning-effort", put(set_reasoning_effort))
        .route("/api/ai/speech-recognition", put(set_speech_recognition))
        .route(
            "/api/ai/transcriptions",
            post(transcribe_audio).layer(DefaultBodyLimit::max(MAX_ASR_AUDIO_BYTES + 1024 * 1024)),
        )
        .route("/api/ai/agents", post(create_agent))
        .route(
            "/api/ai/agents/{id}",
            put(update_agent).delete(delete_agent),
        )
        .route("/api/ai/agents/{id}/test", post(test_agent))
        .route("/api/ai/agents/active", put(set_active_agent))
        .route("/api/chat/stream", post(chat_stream))
}

fn mask_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 4 {
        "****".into()
    } else {
        format!(
            "****{}",
            chars[chars.len() - 4..].iter().collect::<String>()
        )
    }
}

fn snippet(text: &str, limit: usize) -> String {
    let mut short: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        short.push('…');
    }
    short
}

/// base_url 末尾带不带 /v1 均可，统一归一到 OpenAI 风格 /v1 前缀。
fn upstream_url(base_url: &str, path: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{trimmed}{path}")
    } else {
        format!("{trimmed}/v1{path}")
    }
}

fn authed(builder: reqwest::RequestBuilder, api_key: &str) -> reqwest::RequestBuilder {
    if api_key.is_empty() {
        builder
    } else {
        builder.bearer_auth(api_key)
    }
}

fn validate_speech_recognition_settings(
    settings: &SpeechRecognitionSettings,
    providers: &[AiProvider],
) -> Result<(), ApiError> {
    if !SPEECH_RECOGNITION_MODES.contains(&settings.mode.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "语音识别模式仅支持 browser 或 model".into(),
        ));
    }
    let language = settings.language.trim();
    if language.is_empty()
        || language.len() > 35
        || (language != "auto"
            && !language
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-'))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "识别语言必须是 auto 或有效的 BCP 47 语言标签".into(),
        ));
    }
    if settings.model_id.trim().is_empty() || settings.model_id.trim().chars().count() > 200 {
        return Err((
            StatusCode::BAD_REQUEST,
            "ASR 模型 ID 不能为空且不能超过 200 个字符".into(),
        ));
    }
    if settings.mode == "model" {
        let provider_id = settings
            .provider_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or((StatusCode::BAD_REQUEST, "ASR 模型模式必须选择提供商".into()))?;
        if !providers.iter().any(|provider| provider.id == provider_id) {
            return Err((StatusCode::BAD_REQUEST, "ASR 提供商不存在".into()));
        }
    }
    Ok(())
}

fn upstream_transcription_language(language: &str) -> Option<String> {
    let language = language.trim();
    if language.is_empty() || language == "auto" {
        None
    } else {
        Some(
            language
                .split('-')
                .next()
                .unwrap_or(language)
                .to_ascii_lowercase(),
        )
    }
}

fn transcription_text(payload: &Value) -> Option<String> {
    payload["text"]
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn validate_base_url(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    let parsed = Url::parse(&trimmed).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "base_url 必须是合法的 HTTP(S) 地址".to_string(),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err((
            StatusCode::BAD_REQUEST,
            "base_url 仅支持 http 或 https".into(),
        ));
    }
    Ok(trimmed)
}

/// 解析生效模型：请求级覆盖 → 情景绑定 → 全局默认；提供商必须存在且启用。
pub(crate) fn resolve_binding(
    settings: &AiSettings,
    scenario: &str,
    override_binding: Option<&ModelBinding>,
) -> Option<(AiProvider, String, Option<String>)> {
    let candidates = [
        override_binding,
        settings.scenarios.get(scenario),
        settings.default_binding.as_ref(),
    ];
    for binding in candidates.into_iter().flatten() {
        if let Some(provider) = settings
            .providers
            .iter()
            .find(|provider| provider.id == binding.provider_id && provider.enabled)
        {
            return Some((
                provider.clone(),
                binding.model_id.clone(),
                binding.reasoning_effort.clone(),
            ));
        }
    }
    None
}

/// 为后台自动化执行一次非流式文本生成。调用方负责控制任务幂等与持久化。
pub(crate) async fn complete_text(
    settings: &AiSettings,
    scenario: &str,
    system: &str,
    user: &str,
) -> Result<String, String> {
    let (provider, model, binding_effort) = resolve_binding(settings, scenario, None)
        .ok_or_else(|| format!("未配置可用的 {scenario} AI 模型"))?;
    let mut body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ],
        "stream": false
    });
    if let Some(effort) = binding_effort
        .as_ref()
        .or(settings.reasoning_effort.as_ref())
    {
        body["reasoning_effort"] = json!(effort);
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| error.to_string())?;
    let url = upstream_url(&provider.base_url, "/chat/completions");
    let response = authed(client.post(url), &provider.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let payload: Value = response.json().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "AI HTTP {}：{}",
            status.as_u16(),
            payload["error"]["message"]
                .as_str()
                .unwrap_or("上游请求失败")
        ));
    }
    payload["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| "AI 返回内容为空".into())
}

async fn get_settings(State(state): State<AppState>) -> Json<AiSettingsView> {
    let settings = state.inner.read().await.ai.clone();
    let detected_agents = crate::cli_tools::detected_agents().await;
    Json(AiSettingsView::new(&settings, detected_agents))
}

async fn create_provider(
    State(state): State<AppState>,
    Json(request): Json<UpsertProviderRequest>,
) -> ApiResult<AiProviderView> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "提供商名称不能为空".into()));
    }
    let base_url = validate_base_url(&request.base_url)?;
    let provider = AiProvider {
        id: format!("prov-{}", &Uuid::new_v4().simple().to_string()[..8]),
        name,
        base_url,
        api_key: request.api_key.unwrap_or_default().trim().to_string(),
        enabled: request.enabled.unwrap_or(true),
        models: Vec::new(),
        models_synced_at: None,
    };
    let mut data = state.inner.write().await;
    data.ai.providers.push(provider.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json((&provider).into()))
}

async fn update_provider(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<UpsertProviderRequest>,
) -> ApiResult<AiProviderView> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "提供商名称不能为空".into()));
    }
    let base_url = validate_base_url(&request.base_url)?;
    let mut data = state.inner.write().await;
    let provider = data
        .ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == id)
        .ok_or((StatusCode::NOT_FOUND, "provider not found".to_string()))?;
    provider.name = name;
    if provider.base_url != base_url {
        provider.base_url = base_url;
        provider.models.clear();
        provider.models_synced_at = None;
    }
    if let Some(key) = request.api_key
        && !key.trim().is_empty()
    {
        provider.api_key = key.trim().to_string();
    }
    if let Some(enabled) = request.enabled {
        provider.enabled = enabled;
    }
    let view: AiProviderView = (&*provider).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn delete_provider(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<AiSettingsView> {
    let mut data = state.inner.write().await;
    let before = data.ai.providers.len();
    data.ai.providers.retain(|provider| provider.id != id);
    if data.ai.providers.len() == before {
        return Err((StatusCode::NOT_FOUND, "provider not found".into()));
    }
    data.ai
        .scenarios
        .retain(|_, binding| binding.provider_id != id);
    if data
        .ai
        .default_binding
        .as_ref()
        .is_some_and(|binding| binding.provider_id == id)
    {
        data.ai.default_binding = None;
    }
    if data.ai.speech_recognition.provider_id.as_deref() == Some(id.as_str()) {
        data.ai.speech_recognition.mode = default_speech_recognition_mode();
        data.ai.speech_recognition.provider_id = None;
    }
    let view: AiSettingsView = (&data.ai).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn sync_models(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<AiProviderView> {
    let (base_url, api_key) = {
        let data = state.inner.read().await;
        let provider = data
            .ai
            .providers
            .iter()
            .find(|provider| provider.id == id)
            .ok_or((StatusCode::NOT_FOUND, "provider not found".to_string()))?;
        (provider.base_url.clone(), provider.api_key.clone())
    };
    let url = upstream_url(&base_url, "/models");
    let client = reqwest::Client::new();
    let response = authed(client.get(&url), &api_key)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("上游请求失败：{error}")))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("上游返回 {}：{}", status.as_u16(), snippet(&body, 200)),
        ));
    }
    let value: Value = response.json().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("模型列表解析失败：{error}"),
        )
    })?;
    let list = value["data"]
        .as_array()
        .or_else(|| value["models"].as_array())
        .or_else(|| value.as_array());
    let mut ids: Vec<String> = list
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item["id"]
                        .as_str()
                        .or_else(|| item.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Err((StatusCode::BAD_GATEWAY, "未能从上游解析出模型列表".into()));
    }
    ids.sort();
    ids.dedup();
    let mut data = state.inner.write().await;
    let provider = data
        .ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == id)
        .ok_or((StatusCode::NOT_FOUND, "provider not found".to_string()))?;
    let previous: HashMap<String, bool> = provider
        .models
        .iter()
        .map(|model| (model.id.clone(), model.enabled))
        .collect();
    provider.models = ids
        .into_iter()
        .map(|model_id| AiModel {
            enabled: previous.get(&model_id).copied().unwrap_or(false),
            id: model_id,
        })
        .collect();
    provider.models_synced_at = Some(Local::now().to_rfc3339());
    let view: AiProviderView = (&*provider).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn toggle_model(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<ToggleModelRequest>,
) -> ApiResult<AiProviderView> {
    let mut data = state.inner.write().await;
    let provider = data
        .ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == id)
        .ok_or((StatusCode::NOT_FOUND, "provider not found".to_string()))?;
    let model = provider
        .models
        .iter_mut()
        .find(|model| model.id == request.model_id)
        .ok_or((StatusCode::NOT_FOUND, "model not found".to_string()))?;
    model.enabled = !model.enabled;
    let view: AiProviderView = (&*provider).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

/// 手动添加模型（上游 /v1/models 不全或想直接指定模型 ID 时使用），默认启用。
async fn add_model(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<ModelIdRequest>,
) -> ApiResult<AiProviderView> {
    let model_id = request.model_id.trim().to_string();
    if model_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "模型 ID 不能为空".into()));
    }
    let mut data = state.inner.write().await;
    let provider = data
        .ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == id)
        .ok_or((StatusCode::NOT_FOUND, "provider not found".to_string()))?;
    if provider.models.iter().any(|model| model.id == model_id) {
        return Err((StatusCode::BAD_REQUEST, "该模型已存在".into()));
    }
    provider.models.push(AiModel {
        id: model_id,
        enabled: true,
    });
    let view: AiProviderView = (&*provider).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

/// 移除模型，并同步清理引用它的情景绑定与默认绑定。
async fn remove_model(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<ModelIdRequest>,
) -> ApiResult<AiSettingsView> {
    let mut data = state.inner.write().await;
    let provider = data
        .ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == id)
        .ok_or((StatusCode::NOT_FOUND, "provider not found".to_string()))?;
    let before = provider.models.len();
    provider.models.retain(|model| model.id != request.model_id);
    if provider.models.len() == before {
        return Err((StatusCode::NOT_FOUND, "model not found".into()));
    }
    data.ai
        .scenarios
        .retain(|_, binding| !(binding.provider_id == id && binding.model_id == request.model_id));
    if data
        .ai
        .default_binding
        .as_ref()
        .is_some_and(|binding| binding.provider_id == id && binding.model_id == request.model_id)
    {
        data.ai.default_binding = None;
    }
    let view: AiSettingsView = (&data.ai).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn test_model(
    State(state): State<AppState>,
    Json(request): Json<TestModelRequest>,
) -> ApiResult<TestResult> {
    validate_effort(request.reasoning_effort.as_deref(), MODEL_EFFORTS)?;
    let (base_url, api_key) = {
        let data = state.inner.read().await;
        let provider = data
            .ai
            .providers
            .iter()
            .find(|provider| provider.id == request.provider_id)
            .ok_or((StatusCode::NOT_FOUND, "provider not found".to_string()))?;
        (provider.base_url.clone(), provider.api_key.clone())
    };
    let url = upstream_url(&base_url, "/chat/completions");
    let mut body = json!({
        "model": request.model_id,
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 16,
        "stream": false,
    });
    if let Some(effort) = request.reasoning_effort.as_deref() {
        body["reasoning_effort"] = json!(effort);
    }
    let client = reqwest::Client::new();
    let started = Instant::now();
    let outcome = authed(client.post(&url), &api_key)
        .timeout(Duration::from_secs(15))
        .json(&body)
        .send()
        .await;
    let latency_ms = started.elapsed().as_millis() as u64;
    let response = match outcome {
        Ok(response) => response,
        Err(error) => {
            return Ok(Json(TestResult {
                ok: false,
                latency_ms,
                reply: None,
                error: Some(format!("请求失败：{error}")),
            }));
        }
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Ok(Json(TestResult {
            ok: false,
            latency_ms,
            reply: None,
            error: Some(format!("HTTP {}：{}", status.as_u16(), snippet(&body, 200))),
        }));
    }
    match response.json::<Value>().await {
        Ok(value) => {
            let reply = value["choices"][0]["message"]["content"]
                .as_str()
                .map(|text| snippet(text, 80));
            Ok(Json(TestResult {
                ok: true,
                latency_ms,
                reply,
                error: None,
            }))
        }
        Err(error) => Ok(Json(TestResult {
            ok: false,
            latency_ms,
            reply: None,
            error: Some(format!("响应解析失败：{error}")),
        })),
    }
}

async fn set_scenario(
    State(state): State<AppState>,
    Json(request): Json<SetScenarioRequest>,
) -> ApiResult<AiSettingsView> {
    let scenario = request.scenario.as_str();
    if scenario != "default" && !SCENARIOS.contains(&scenario) {
        return Err((StatusCode::BAD_REQUEST, "invalid scenario".into()));
    }
    let mut data = state.inner.write().await;
    if let Some(binding) = &request.binding {
        validate_effort(binding.reasoning_effort.as_deref(), MODEL_EFFORTS)?;
        if !data
            .ai
            .providers
            .iter()
            .any(|provider| provider.id == binding.provider_id)
        {
            return Err((StatusCode::BAD_REQUEST, "绑定的提供商不存在".into()));
        }
        if binding.model_id.trim().is_empty() {
            return Err((StatusCode::BAD_REQUEST, "模型 ID 不能为空".into()));
        }
    }
    if scenario == "default" {
        data.ai.default_binding = request.binding;
    } else if let Some(binding) = request.binding {
        data.ai.scenarios.insert(scenario.to_string(), binding);
    } else {
        data.ai.scenarios.remove(scenario);
    }
    let view: AiSettingsView = (&data.ai).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn set_review_mode(
    State(state): State<AppState>,
    Json(request): Json<SetReviewModeRequest>,
) -> ApiResult<AiSettingsView> {
    if !REVIEW_MODES.contains(&request.mode.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "invalid review mode".into()));
    }
    let mut data = state.inner.write().await;
    data.ai.review_mode = request.mode;
    let view: AiSettingsView = (&data.ai).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn set_reasoning_effort(
    State(state): State<AppState>,
    Json(request): Json<SetReasoningEffortRequest>,
) -> ApiResult<AiSettingsView> {
    validate_effort(request.reasoning_effort.as_deref(), MODEL_EFFORTS)?;
    let mut data = state.inner.write().await;
    data.ai.reasoning_effort = request.reasoning_effort;
    let view: AiSettingsView = (&data.ai).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn set_speech_recognition(
    State(state): State<AppState>,
    Json(mut request): Json<SpeechRecognitionSettings>,
) -> ApiResult<AiSettingsView> {
    request.mode = request.mode.trim().to_ascii_lowercase();
    request.language = request.language.trim().to_string();
    request.provider_id = request
        .provider_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
    request.model_id = request.model_id.trim().to_string();
    let mut data = state.inner.write().await;
    validate_speech_recognition_settings(&request, &data.ai.providers)?;
    data.ai.speech_recognition = request;
    let view: AiSettingsView = (&data.ai).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn transcribe_audio(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> ApiResult<TranscriptionResult> {
    let (settings, provider) = {
        let data = state.inner.read().await;
        let settings = data.ai.speech_recognition.clone();
        if settings.mode != "model" {
            return Err((
                StatusCode::CONFLICT,
                "当前使用浏览器语音识别模式，不接受音频上传".into(),
            ));
        }
        let provider_id = settings
            .provider_id
            .as_deref()
            .ok_or((StatusCode::CONFLICT, "尚未配置 ASR 提供商".into()))?;
        let provider = data
            .ai
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .cloned()
            .ok_or((StatusCode::CONFLICT, "配置的 ASR 提供商已不存在".into()))?;
        if !provider.enabled {
            return Err((StatusCode::CONFLICT, "配置的 ASR 提供商已停用".into()));
        }
        if settings.model_id.trim().is_empty() {
            return Err((StatusCode::CONFLICT, "尚未配置 ASR 模型".into()));
        }
        (settings, provider)
    };

    let mut audio: Option<(Vec<u8>, String, Option<String>)> = None;
    while let Some(field) = multipart.next_field().await.map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("无效的录音上传请求：{error}"),
        )
    })? {
        let name = field
            .name()
            .ok_or((StatusCode::BAD_REQUEST, "multipart 字段必须有名称".into()))?
            .to_string();
        if name != "audio" && name != "file" {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("不支持的 multipart 字段：{name}"),
            ));
        }
        if audio.is_some() {
            return Err((StatusCode::BAD_REQUEST, "只能上传一个录音文件".into()));
        }
        let raw_filename = field.file_name().unwrap_or("recording.webm");
        let filename = raw_filename
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("recording.webm")
            .chars()
            .filter(|character| {
                character.is_ascii_alphanumeric() || ['.', '-', '_'].contains(character)
            })
            .take(120)
            .collect::<String>();
        let filename = if filename.is_empty() {
            "recording.webm".to_string()
        } else {
            filename
        };
        let content_type = field.content_type().map(str::to_string);
        let bytes = field
            .bytes()
            .await
            .map_err(|error| (StatusCode::BAD_REQUEST, format!("读取录音失败：{error}")))?;
        if bytes.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "录音文件为空".into()));
        }
        if bytes.len() > MAX_ASR_AUDIO_BYTES {
            return Err((StatusCode::PAYLOAD_TOO_LARGE, "录音不能超过 25 MiB".into()));
        }
        audio = Some((bytes.to_vec(), filename, content_type));
    }
    let (bytes, filename, content_type) =
        audio.ok_or((StatusCode::BAD_REQUEST, "缺少 audio 录音字段".into()))?;

    let mut audio_part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
    if let Some(content_type) = content_type {
        audio_part = audio_part
            .mime_str(&content_type)
            .map_err(|_| (StatusCode::BAD_REQUEST, "录音 Content-Type 无效".into()))?;
    }
    let mut form = reqwest::multipart::Form::new()
        .text("model", settings.model_id.clone())
        .text("response_format", "json")
        .part("file", audio_part);
    if let Some(language) = upstream_transcription_language(&settings.language) {
        form = form.text("language", language);
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| internal(error.to_string()))?;
    let url = upstream_url(&provider.base_url, "/audio/transcriptions");
    let response = authed(client.post(url), &provider.api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|error| {
            let status = if error.is_timeout() {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::BAD_GATEWAY
            };
            (status, format!("ASR 请求失败：{error}"))
        })?;
    let status = response.status();
    let body = response.bytes().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("读取 ASR 响应失败：{error}"),
        )
    })?;
    if body.len() > 1024 * 1024 {
        return Err((StatusCode::BAD_GATEWAY, "ASR 响应过大".into()));
    }
    let body_text = String::from_utf8_lossy(&body).into_owned();
    if !status.is_success() {
        let redacted = redact_secrets(body_text, std::slice::from_ref(&provider.api_key));
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("ASR HTTP {}：{}", status.as_u16(), snippet(&redacted, 300)),
        ));
    }
    let payload: Value = serde_json::from_slice(&body).map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("ASR 响应不是有效 JSON：{error}"),
        )
    })?;
    let text = transcription_text(&payload)
        .ok_or((StatusCode::BAD_GATEWAY, "ASR 响应缺少有效文本".into()))?;
    Ok(Json(TranscriptionResult { text }))
}

fn validate_effort(effort: Option<&str>, supported: &[&str]) -> Result<(), ApiError> {
    let Some(effort) = effort else {
        return Ok(());
    };
    if supported.contains(&effort) {
        Ok(())
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            format!(
                "不支持 reasoning_effort={effort}；可选值：{}",
                supported.join(", ")
            ),
        ))
    }
}

pub(crate) fn validate_model_reasoning_effort(effort: Option<&str>) -> Result<(), String> {
    validate_effort(effort, MODEL_EFFORTS).map_err(|(_, error)| error)
}

fn agent_efforts(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "codex" => Some(CODEX_EFFORTS),
        "claude-code" => Some(CLAUDE_EFFORTS),
        _ => None,
    }
}

fn workspace_directory(workspace_id: &str, kind: &str) -> PathBuf {
    if kind == "project" {
        crate::runtime::project_directory(workspace_id)
    } else {
        crate::runtime::server_directory(workspace_id)
    }
}

fn validate_agent_request(
    request: &UpsertAgentRequest,
) -> Result<(String, String, String, Option<String>), ApiError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Agent 名称不能为空".into()));
    }
    if !AGENT_KINDS.contains(&request.kind.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "invalid agent kind".into()));
    }
    let command = request.command.trim().to_string();
    if command.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Agent 启动命令不能为空".into()));
    }
    if !matches!(request.transport.as_str(), "acp" | "cli") {
        return Err((StatusCode::BAD_REQUEST, "invalid agent transport".into()));
    }
    if request.transport == "cli" {
        let efforts = agent_efforts(&request.kind).ok_or((
            StatusCode::BAD_REQUEST,
            "原生 CLI 传输仅支持 codex 与 claude-code".into(),
        ))?;
        validate_effort(request.reasoning_effort.as_deref(), efforts)?;
        if request.kind == "codex" && !request.args.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "原生 Codex CLI 参数由受控运行时管理，不能自定义".into(),
            ));
        }
    } else if request.reasoning_effort.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "ACP 适配器没有统一的 reasoning_effort 契约；请使用原生 CLI 传输或清空该值".into(),
        ));
    }
    Ok((
        name,
        command,
        request.transport.clone(),
        request.reasoning_effort.clone(),
    ))
}

async fn create_agent(
    State(state): State<AppState>,
    Json(request): Json<UpsertAgentRequest>,
) -> ApiResult<AiAgent> {
    let (name, command, transport, reasoning_effort) = validate_agent_request(&request)?;
    let agent = AiAgent {
        id: format!("agent-{}", &Uuid::new_v4().simple().to_string()[..8]),
        name,
        kind: request.kind,
        command,
        args: request.args,
        transport,
        reasoning_effort,
        enabled: request.enabled.unwrap_or(true),
    };
    let mut data = state.inner.write().await;
    data.ai.agents.push(agent.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(agent))
}

async fn update_agent(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<UpsertAgentRequest>,
) -> ApiResult<AiAgent> {
    let (name, command, transport, reasoning_effort) = validate_agent_request(&request)?;
    let mut data = state.inner.write().await;
    let agent = data
        .ai
        .agents
        .iter_mut()
        .find(|agent| agent.id == id)
        .ok_or((StatusCode::NOT_FOUND, "agent not found".to_string()))?;
    agent.name = name;
    agent.kind = request.kind;
    agent.command = command;
    agent.args = request.args;
    agent.transport = transport;
    agent.reasoning_effort = reasoning_effort;
    if let Some(enabled) = request.enabled {
        agent.enabled = enabled;
    }
    let disabled = !agent.enabled;
    let result = agent.clone();
    if disabled && data.ai.active_agent.as_deref() == Some(id.as_str()) {
        data.ai.active_agent = None;
    }
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(result))
}

async fn delete_agent(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<AiSettingsView> {
    let mut data = state.inner.write().await;
    let before = data.ai.agents.len();
    data.ai.agents.retain(|agent| agent.id != id);
    if data.ai.agents.len() == before {
        return Err((StatusCode::NOT_FOUND, "agent not found".into()));
    }
    if data.ai.active_agent.as_deref() == Some(id.as_str()) {
        data.ai.active_agent = None;
    }
    let view: AiSettingsView = (&data.ai).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn set_active_agent(
    State(state): State<AppState>,
    Json(request): Json<SetActiveAgentRequest>,
) -> ApiResult<AiSettingsView> {
    let mut data = state.inner.write().await;
    if let Some(agent_id) = &request.agent_id {
        let agent = data
            .ai
            .agents
            .iter()
            .find(|agent| agent.id == *agent_id)
            .ok_or((StatusCode::NOT_FOUND, "agent not found".to_string()))?;
        if !agent.enabled {
            return Err((StatusCode::BAD_REQUEST, "该 Agent 已禁用，请先启用".into()));
        }
    }
    data.ai.active_agent = request.agent_id;
    let view: AiSettingsView = (&data.ai).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

/// ACP 握手测试：initialize → 等待响应，返回协议版本与耗时。
async fn test_agent(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<TestResult> {
    let agent = {
        let data = state.inner.read().await;
        data.ai
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .cloned()
            .ok_or((StatusCode::NOT_FOUND, "agent not found".to_string()))?
    };
    let started = Instant::now();
    let outcome = if agent.transport == "cli" {
        crate::cli_tools::probe_command_version(&agent.command)
            .await
            .map(|version| format!("原生 CLI 可用 · {version}"))
    } else {
        acp_handshake(&agent).await
    };
    let latency_ms = started.elapsed().as_millis() as u64;
    Ok(Json(match outcome {
        Ok(info) => TestResult {
            ok: true,
            latency_ms,
            reply: Some(info),
            error: None,
        },
        Err(error) => TestResult {
            ok: false,
            latency_ms,
            reply: None,
            error: Some(error),
        },
    }))
}

async fn acp_handshake(agent: &AiAgent) -> Result<String, String> {
    let mut client = AcpClient::spawn(&agent.command, &agent.args).await?;
    let result = async {
        let id = client
            .send_request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}},
                }),
            )
            .await?;
        let value = client.wait_response(id, Duration::from_secs(15)).await?;
        let version = value["protocolVersion"]
            .as_i64()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into());
        Ok(format!("ACP 握手成功 · protocolVersion {version}"))
    }
    .await;
    client.shutdown().await;
    result
}

enum StreamOutcome {
    Completed,
    FailedBeforeOutput(String),
    FailedMidway(String),
    PolicyDenied(String),
    ClientGone,
}

async fn send_event(tx: &mpsc::Sender<Event>, name: &str, data: &Value) -> Result<(), ()> {
    tx.send(Event::default().event(name).data(data.to_string()))
        .await
        .map_err(|_| ())
}

async fn chat_stream(
    State(state): State<AppState>,
    Json(request): Json<ChatStreamRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    validate_chat_request(&state, &request).await?;
    let (tx, rx) = mpsc::channel::<Event>(64);
    tokio::spawn(run_chat_stream(state, request, tx));
    Ok(
        Sse::new(ReceiverStream::new(rx).map(Ok::<Event, Infallible>))
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(15))),
    )
}

async fn validate_chat_request(
    state: &AppState,
    request: &ChatStreamRequest,
) -> Result<(), ApiError> {
    validate_chat_payload(request)?;
    let data = state.inner.read().await;
    if !data
        .servers
        .iter()
        .any(|workspace| workspace.id == request.server_id)
    {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }
    if let Some(conversation_id) = request.conversation_id.as_deref()
        && !data.conversations.iter().any(|conversation| {
            conversation.id == conversation_id && conversation.server_id == request.server_id
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "conversation does not belong to this workspace".into(),
        ));
    }
    validate_chat_execution_targets(
        &data.ai,
        request.model_override.as_ref(),
        request.agent_override.as_deref(),
    )
}

fn validate_chat_payload(request: &ChatStreamRequest) -> Result<(), ApiError> {
    if request.message.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "消息内容不能为空".into()));
    }
    if request.message.chars().count() > CHAT_MESSAGE_CHAR_LIMIT {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("消息内容不能超过 {CHAT_MESSAGE_CHAR_LIMIT} 个字符"),
        ));
    }
    if request.history.len() > CHAT_HISTORY_TURN_LIMIT {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("历史消息不能超过 {CHAT_HISTORY_TURN_LIMIT} 条"),
        ));
    }
    Ok(())
}

fn validate_chat_execution_targets(
    settings: &AiSettings,
    model_override: Option<&ModelBinding>,
    agent_override: Option<&str>,
) -> Result<(), ApiError> {
    if let Some(binding) = model_override {
        validate_effort(binding.reasoning_effort.as_deref(), MODEL_EFFORTS)?;
        let enabled = settings.providers.iter().any(|provider| {
            provider.id == binding.provider_id
                && provider.enabled
                && provider
                    .models
                    .iter()
                    .any(|model| model.id == binding.model_id && model.enabled)
        });
        if !enabled {
            return Err((
                StatusCode::BAD_REQUEST,
                "model_override is missing or disabled".into(),
            ));
        }
    }
    if let Some(agent_id) = agent_override
        && agent_id != "default"
        && !settings
            .agents
            .iter()
            .any(|agent| agent.id == agent_id && agent.enabled)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "agent_override is missing or disabled".into(),
        ));
    }
    Ok(())
}

fn latest_task_for_chat<'a>(
    data: &'a crate::PersistedState,
    server_id: &str,
    conversation_id: Option<&str>,
) -> Option<&'a crate::TaskInfo> {
    let linked = conversation_id
        .and_then(|conversation_id| {
            data.conversations.iter().find(|conversation| {
                conversation.id == conversation_id && conversation.server_id == server_id
            })
        })
        .and_then(|conversation| {
            conversation
                .messages
                .iter()
                .rev()
                .filter_map(|message| message.task_id)
                .find_map(|task_id| data.tasks.iter().find(|task| task.id == task_id))
        });
    linked.or_else(|| data.tasks.iter().find(|task| task.server_id == server_id))
}

fn active_bootstrap_task<'a>(
    data: &'a crate::PersistedState,
    server_id: &str,
) -> Option<&'a crate::TaskInfo> {
    data.tasks.iter().find(|task| {
        task.server_id == server_id
            && matches!(
                task.kind.as_str(),
                "server_bootstrap" | "server_provision" | "bootstrap"
            )
            && matches!(
                task.status.as_str(),
                "awaiting_approval" | "queued" | "running" | "cancelling"
            )
    })
}

fn is_task_follow_up(message: &str) -> bool {
    let compact = message
        .trim()
        .trim_end_matches(['。', '！', '!', '？', '?'])
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        compact.as_str(),
        "继续"
            | "继续吧"
            | "下一步"
            | "状态"
            | "进度"
            | "任务状态"
            | "查看状态"
            | "现在怎么样"
            | "现在怎么样了"
            | "怎么样了"
            | "完成了吗"
            | "好了吗"
            | "ok"
    )
}

fn human_server_status(status: &str) -> &str {
    match status {
        "online" => "在线",
        "starting" => "启动中",
        "stopping" => "停止中",
        "planning" => "规划中",
        "error" => "异常",
        _ => "已停止",
    }
}

fn current_server_fact(server: Option<&crate::ServerInfo>) -> String {
    let Some(server) = server else {
        return "当前工作区记录不存在。".into();
    };
    let core = if server.core.trim().is_empty() {
        "核心尚未确定".into()
    } else {
        format!("{} {}", server.core, server.version)
    };
    let port = if server.port == 0 {
        "端口尚未分配".into()
    } else {
        format!("端口 {}", server.port)
    };
    let mut fact = format!(
        "当前服务器“{}”{}，{}，{}，核心文件{}就绪，操作状态为 {}。",
        server.name,
        human_server_status(&server.status),
        core,
        port,
        if server.core_ready { "已" } else { "未" },
        server.operation_state
    );
    if let Some(error) = server.last_error.as_deref()
        && !error.trim().is_empty()
    {
        fact.push_str(&format!(" 最近错误：{error}"));
    }
    fact
}

fn task_follow_up_reply(task: &crate::TaskInfo, server: Option<&crate::ServerInfo>) -> String {
    let fact = current_server_fact(server);
    match task.status.as_str() {
        "awaiting_approval" => format!(
            "任务“{}”正在等待批准（{}%），批准前不会执行。\n\n{}",
            task.title, task.progress, fact
        ),
        "queued" => format!(
            "任务“{}”已进入执行队列（{}%），尚未开始执行。\n\n{}",
            task.title, task.progress, fact
        ),
        "running" | "cancelling" => {
            let event = task
                .events
                .last()
                .map(|event| event.message.as_str())
                .unwrap_or("执行器正在处理。");
            format!(
                "任务“{}”{}（{}%）。最新事件：{}\n\n{}",
                task.title,
                if task.status == "cancelling" {
                    "正在取消并安全收尾"
                } else {
                    "正在执行"
                },
                task.progress,
                event,
                fact
            )
        }
        "completed" => format!(
            "任务“{}”已经完成。{}\n\n{}\n\n基础开服不会重复提交；请直接告诉我接下来要配置的玩法、插件或文件。",
            task.title,
            task.summary.as_deref().unwrap_or("执行器已确认完成。"),
            fact
        ),
        "failed" | "interrupted" | "rollback_failed" => format!(
            "任务“{}”未能完成：{}\n\n{}\n\n我不会自动重复执行；确认原因后可明确要求“重新执行任务”。",
            task.title,
            task.error.as_deref().unwrap_or("执行器未返回具体错误。"),
            fact
        ),
        "cancelled" => format!(
            "任务“{}”已取消。{}\n\n{}\n\n如需重试，请明确说“重新执行任务”。",
            task.title,
            task.summary.as_deref().unwrap_or("执行器已完成安全收尾。"),
            fact
        ),
        _ => format!(
            "任务“{}”当前状态为 {}（{}%）。\n\n{}",
            task.title, task.status, task.progress, fact
        ),
    }
}

fn workspace_runtime_context(
    server: Option<&crate::ServerInfo>,
    latest_task: Option<&crate::TaskInfo>,
    fallback_id: &str,
) -> String {
    let mut context = server
        .map(|server| current_server_fact(Some(server)))
        .unwrap_or_else(|| format!("工作区 {fallback_id} 不存在于后端状态中。"));
    if let Some(task) = latest_task {
        context.push_str(&format!(
            " 最近关联任务：id={}，标题={}，类型={}，状态={}，进度={}%，摘要={}，错误={}，结束时间={}。",
            task.id,
            task.title,
            task.kind,
            task.status,
            task.progress,
            task.summary.as_deref().unwrap_or("无"),
            task.error.as_deref().unwrap_or("无"),
            task.finished_at.as_deref().unwrap_or("未结束")
        ));
    } else {
        context.push_str(" 当前没有关联任务记录。");
    }
    context.push_str(" 以上内容来自后端持久化状态，是当前事实；旧对话中的进行中文案不能覆盖它。除非本轮响应实际返回 task.id，否则不得声称已创建或提交任务。");
    context
}

async fn run_chat_stream(state: AppState, request: ChatStreamRequest, tx: mpsc::Sender<Event>) {
    if let Err((_, message)) = validate_effort(request.reasoning_effort.as_deref(), MODEL_EFFORTS) {
        let _ = send_event(&tx, "error", &json!({ "message": message })).await;
        return;
    }
    let (
        settings,
        server_context,
        workspace_directory,
        workspace_kind,
        language,
        persona,
        is_planning,
        server_snapshot,
        latest_task,
        bootstrap_inflight,
    ) = {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == request.server_id);
        let is_planning = server.map(|s| s.status == "planning").unwrap_or(false);
        let workspace_directory = server.map(crate::workspace_directory_for_server);
        let workspace_kind = server.map(|server| server.kind.clone());
        let latest_task = latest_task_for_chat(
            &data,
            &request.server_id,
            request.conversation_id.as_deref(),
        )
        .cloned();
        let context = workspace_runtime_context(server, latest_task.as_ref(), &request.server_id);
        (
            data.ai.clone(),
            context,
            workspace_directory,
            workspace_kind,
            crate::prefs::language_directive(&data.ui.language).to_string(),
            crate::prefs::persona_directive(&data.ui.personalization),
            is_planning,
            server.cloned(),
            latest_task,
            active_bootstrap_task(&data, &request.server_id).cloned(),
        )
    };
    let intent = crate::classify_workspace_intent(&request.message, is_planning);
    let skill_query = skill_query_for_request(&request, is_planning);
    let is_plugin_request = crate::skills::is_minecraft_plugin_request(&skill_query);
    let is_server_request = is_planning || crate::skills::is_minecraft_server_request(&skill_query);
    let mut skill_context = String::new();
    if is_server_request && crate::skills::server_is_enabled(&state).await {
        skill_context.push_str(&crate::skills::server_context_for_request(&skill_query));
    }
    if is_plugin_request && crate::skills::is_enabled(&state).await {
        if !skill_context.is_empty() {
            skill_context.push_str("\n\n");
        }
        skill_context.push_str(&crate::skills::context_for_request(&skill_query));
    }
    let plugin_context = if is_plugin_request {
        crate::resource_sync::plugin_context_for_ai(&state, &skill_query).await
    } else {
        String::new()
    };
    let intelligence_context = if is_server_request {
        crate::server_intelligence::context_for_request(&state, &skill_query).await
    } else {
        String::new()
    };
    if !intelligence_context.is_empty() {
        if !skill_context.is_empty() {
            skill_context.push_str("\n\n");
        }
        skill_context.push_str(&intelligence_context);
    }
    let qq_escalation = if is_server_request {
        crate::bots::maybe_ask_knowledge_group(&state, &request.message, &server_context).await
    } else {
        String::new()
    };
    if !qq_escalation.is_empty() {
        skill_context.push_str("\n\n[QQ 群协查状态]\n");
        skill_context.push_str(&qq_escalation);
    }
    let scenario = if is_planning { "setup" } else { "chat" };
    let resolved = resolve_binding(&settings, scenario, request.model_override.as_ref());
    let mut full_reply = String::new();
    let mut fallback = resolved.is_none();
    let mut handled = false;
    let mut existing_task = None;

    let deterministic_reply = if is_task_follow_up(&request.message) {
        latest_task.as_ref().map(|task| {
            existing_task = Some(task.clone());
            task_follow_up_reply(task, server_snapshot.as_ref())
        })
    } else if intent == "server_bootstrap" {
        if let Some(task) = bootstrap_inflight.as_ref() {
            existing_task = Some(task.clone());
            Some(format!(
                "同一服务器已有开服任务在处理，我不会重复提交。\n\n{}",
                task_follow_up_reply(task, server_snapshot.as_ref())
            ))
        } else if server_snapshot
            .as_ref()
            .is_some_and(|server| server.core_ready && server.status != "planning")
        {
            existing_task = latest_task.clone().filter(|task| {
                matches!(
                    task.kind.as_str(),
                    "server_bootstrap" | "server_provision" | "bootstrap"
                )
            });
            Some(format!(
                "这台服务器已经完成基础创建，我不会重复初始化或覆盖现有文件。\n\n{}\n\n如需启动、重启或重装，请明确说明对应操作。",
                current_server_fact(server_snapshot.as_ref())
            ))
        } else {
            Some(rule_reply(intent).to_string())
        }
    } else {
        None
    };

    // 短状态追问和明确开服请求都由后端事实直接回答，避免模型根据旧历史猜测。
    if let Some(text) = deterministic_reply {
        let meta = json!({
            "provider": Value::Null,
            "model": Value::Null,
            "fallback": false,
            "executor": intent == "server_bootstrap",
            "reused_task": existing_task.is_some(),
        });
        if send_event(&tx, "meta", &meta).await.is_err() {
            return;
        }
        for piece in text.chars().collect::<Vec<_>>().chunks(4) {
            let piece = piece.iter().collect::<String>();
            full_reply.push_str(&piece);
            if send_event(&tx, "delta", &json!({ "content": piece }))
                .await
                .is_err()
            {
                return;
            }
        }
        handled = true;
        fallback = false;
    }

    // Agent 选择：请求覆盖 "default" 强制内置直连；指定 id 或全局 active_agent 走 ACP。
    let agent_choice = match request.agent_override.as_deref() {
        Some("default") => None,
        Some(agent_id) => settings
            .agents
            .iter()
            .find(|agent| agent.id == agent_id && agent.enabled)
            .cloned(),
        None => settings.active_agent.as_ref().and_then(|active| {
            settings
                .agents
                .iter()
                .find(|agent| agent.id == *active && agent.enabled)
                .cloned()
        }),
    };

    if !handled && let Some(agent) = agent_choice {
        if let Err(reason) = validate_full_access_agent(&agent, &settings.review_mode) {
            let _ = send_event(&tx, "error", &json!({ "message": reason })).await;
            return;
        }
        let agent_effort = request
            .reasoning_effort
            .as_deref()
            .or(agent.reasoning_effort.as_deref())
            .or(settings.reasoning_effort.as_deref());
        let outcome = if agent.transport == "cli" {
            let Some(supported) = agent_efforts(&agent.kind) else {
                let _ = send_event(
                    &tx,
                    "error",
                    &json!({ "message": "该 Agent 类型不支持原生 CLI 传输" }),
                )
                .await;
                return;
            };
            if let Err((_, message)) = validate_effort(agent_effort, supported) {
                let _ = send_event(&tx, "error", &json!({ "message": message })).await;
                return;
            }
            stream_cli_agent(
                &agent,
                agent_effort,
                &settings.review_mode,
                codex_full_access_allowed(),
                &request,
                workspace_directory.as_ref(),
                workspace_kind.as_deref(),
                &server_context,
                &language,
                &persona,
                &skill_context,
                &plugin_context,
                &tx,
                &mut full_reply,
            )
            .await
        } else {
            if request.reasoning_effort.is_some() {
                let _ = send_event(
                    &tx,
                    "error",
                    &json!({ "message": "ACP 适配器不支持统一的 reasoning_effort 契约" }),
                )
                .await;
                return;
            }
            stream_acp(
                &state,
                &agent,
                &request,
                &server_context,
                workspace_directory.as_ref(),
                &settings.review_mode,
                &language,
                &persona,
                &skill_context,
                &plugin_context,
                &tx,
                &mut full_reply,
            )
            .await
        };
        match outcome {
            StreamOutcome::Completed => {
                if full_reply.trim().is_empty() {
                    fallback = resolved.is_none();
                } else {
                    handled = true;
                    fallback = false;
                }
            }
            StreamOutcome::FailedBeforeOutput(reason) => {
                // 外部 Agent 已被选中时，不能悄悄切换到内置模型，更不能继续
                // 进入后续任务推断路径；回复显示的执行者必须与实际执行者一致。
                eprintln!("[ai] Agent {} 调用失败：{reason}", agent.name);
                let _ = send_event(
                    &tx,
                    "error",
                    &json!({ "message": format!("{} 调用失败：{reason}", agent.name) }),
                )
                .await;
                return;
            }
            StreamOutcome::FailedMidway(reason) => {
                let _ = send_event(&tx, "error", &json!({ "message": reason })).await;
                handled = true;
                fallback = false;
            }
            StreamOutcome::PolicyDenied(reason) => {
                let _ = send_event(&tx, "error", &json!({ "message": reason })).await;
                return;
            }
            StreamOutcome::ClientGone => return,
        }
    }

    if !handled && let Some((provider, model, binding_effort)) = resolved {
        let model_effort = request
            .reasoning_effort
            .as_deref()
            .or(binding_effort.as_deref())
            .or(settings.reasoning_effort.as_deref());
        match stream_upstream(
            &provider,
            &model,
            model_effort,
            &request,
            &server_context,
            &language,
            &persona,
            &skill_context,
            &plugin_context,
            &tx,
            &mut full_reply,
        )
        .await
        {
            StreamOutcome::Completed => {
                if full_reply.trim().is_empty() {
                    fallback = true;
                }
            }
            StreamOutcome::FailedBeforeOutput(reason) => {
                eprintln!(
                    "[ai] 上游 {} 调用失败，回退本地规则：{reason}",
                    provider.name
                );
                fallback = true;
            }
            StreamOutcome::FailedMidway(reason) => {
                let _ = send_event(&tx, "error", &json!({ "message": reason })).await;
            }
            StreamOutcome::PolicyDenied(reason) => {
                let _ = send_event(&tx, "error", &json!({ "message": reason })).await;
                return;
            }
            StreamOutcome::ClientGone => return,
        }
    }

    if fallback {
        let meta = json!({ "provider": Value::Null, "model": Value::Null, "fallback": true });
        if send_event(&tx, "meta", &meta).await.is_err() {
            return;
        }
        let text = if plugin_context.is_empty() {
            format!("{}\n\n目标服务器：{}", rule_reply(intent), server_context)
        } else {
            format!(
                "已按主流插件库 → 开源插件库 → 普通插件库 → 付费插件库排序检索。\n\n{plugin_context}\n\n目标服务器：{server_context}"
            )
        };
        let text = if intelligence_context.is_empty() {
            text
        } else {
            format!("{text}\n\n{intelligence_context}")
        };
        let chars: Vec<char> = text.chars().collect();
        for piece in chars.chunks(4) {
            let piece: String = piece.iter().collect();
            full_reply.push_str(&piece);
            if send_event(&tx, "delta", &json!({ "content": piece }))
                .await
                .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    let risk = intent_risk(intent);
    let host_total_memory = if intent == "server_bootstrap" {
        crate::runtime::total_memory_bytes().await
    } else {
        None
    };
    let (task, task_created) = {
        let mut data = state.inner.write().await;
        // 规划工作区最初没有 core/version。开服请求必须先把对话中明确出现的
        // 结构化方案绑定到服务器记录，再交给执行器；不能只把推荐文本当作已配置。
        if intent == "server_bootstrap"
            && existing_task.is_none()
            && let Some(server_index) = data
                .servers
                .iter()
                .position(|server| server.id == request.server_id && server.kind == "server")
        {
            let needs_plan = data.servers[server_index].core.trim().is_empty()
                || data.servers[server_index].version.trim().is_empty();
            if needs_plan {
                let mut user_candidates = vec![request.message.clone()];
                let mut assistant_candidates = Vec::new();
                let conversations = request
                    .conversation_id
                    .as_deref()
                    .and_then(|conversation_id| {
                        data.conversations.iter().find(|conversation| {
                            conversation.id == conversation_id
                                && conversation.server_id == request.server_id
                        })
                    })
                    .into_iter()
                    .chain(
                        request
                            .conversation_id
                            .is_none()
                            .then(|| {
                                data.conversations
                                    .iter()
                                    .filter(|conversation| {
                                        conversation.server_id == request.server_id
                                    })
                                    .max_by_key(|conversation| conversation.updated_at.clone())
                            })
                            .flatten(),
                    );
                for conversation in conversations {
                    for message in conversation.messages.iter().rev() {
                        if message.role.eq_ignore_ascii_case("user") {
                            user_candidates.push(message.content.clone());
                        } else if message.role.eq_ignore_ascii_case("assistant") {
                            assistant_candidates.push(message.content.clone());
                        }
                    }
                }
                let inferred_plan = user_candidates
                    .iter()
                    .find_map(|text| crate::extract_server_plan_details(text))
                    .or_else(|| {
                        assistant_candidates
                            .iter()
                            .find_map(|text| crate::extract_server_plan_details(text))
                    })
                    .or_else(|| {
                        crate::catalog::recommended_minecraft_version(&data.catalog, "Paper")
                            .map(|version| ("Paper".into(), version, None))
                    });
                let candidates = user_candidates
                    .iter()
                    .chain(assistant_candidates.iter())
                    .chain(std::iter::once(&full_reply))
                    .collect::<Vec<_>>();
                if let Some((core, version, resource_id)) = inferred_plan {
                    let server_id = data.servers[server_index].id.clone();
                    let server_name = data.servers[server_index].name.clone();
                    let expected_players = candidates
                        .iter()
                        .find_map(|text| crate::server_intelligence::expected_player_count(text));
                    let modded = candidates.iter().any(|text| {
                        let lower = text.to_ascii_lowercase();
                        lower.contains("模组")
                            || lower.contains("modpack")
                            || lower.contains("forge")
                            || lower.contains("fabric")
                    });
                    let memory_gb = crate::runtime::recommended_server_memory_gb(
                        host_total_memory,
                        expected_players,
                        modded,
                    );
                    let max_players = expected_players
                        .unwrap_or(12)
                        .saturating_mul(2)
                        .clamp(10, 500);
                    let port = if data.servers[server_index].port == 0 {
                        let used_ports: std::collections::HashSet<u16> = data
                            .servers
                            .iter()
                            .filter(|item| item.port != 0 && item.id != server_id)
                            .map(|item| item.port)
                            .collect();
                        (25565..=65535)
                            .find(|port| !used_ports.contains(port))
                            .unwrap_or(25565)
                    } else {
                        data.servers[server_index].port
                    };
                    let server = &mut data.servers[server_index];
                    server.core = core.clone();
                    server.core_resource_id = resource_id;
                    server.version = version.clone();
                    server.status = "stopped".into();
                    server.port = port;
                    server.memory_gb = memory_gb;
                    server.operation_state = "provisioning".into();
                    server.task =
                        format!("已绑定 {core} {version}，自动分配 {memory_gb} GB，准备初始化");
                    server.last_error = None;
                    let config = format!(
                        "# {}\nserver-port={}\nmax-players={}\nview-distance=10\nsimulation-distance=8\nonline-mode=true\ndifficulty=normal\npvp=true\nmotd=§3{} §8| §fPowered by Sculk Catalyst",
                        server_name, port, max_players, server_name
                    );
                    data.configs.insert(server_id.clone(), config);
                    data.logs.entry(server_id).or_default();
                }
            } else if data.servers[server_index].status == "planning" {
                data.servers[server_index].status = "stopped".into();
                data.servers[server_index].operation_state = "provisioning".into();
            }
        }
        let workspace_exists = data
            .servers
            .iter()
            .any(|server| server.id == request.server_id);
        let mut task_created = false;
        let task = if let Some(task) = existing_task.clone() {
            Some(task)
        } else if workspace_exists && crate::task_executor::is_executable_kind(intent) {
            let reusable = (intent == "server_bootstrap")
                .then(|| active_bootstrap_task(&data, &request.server_id).cloned())
                .flatten();
            let already_prepared = intent == "server_bootstrap"
                && data.servers.iter().any(|server| {
                    server.id == request.server_id
                        && server.core_ready
                        && server.status != "planning"
                });
            if let Some(task) = reusable {
                Some(task)
            } else if already_prepared {
                data.tasks
                    .iter()
                    .find(|task| {
                        task.server_id == request.server_id
                            && matches!(
                                task.kind.as_str(),
                                "server_bootstrap" | "server_provision" | "bootstrap"
                            )
                    })
                    .cloned()
            } else {
                let (status, progress, approved_by) =
                    effective_task_start(risk, &data.ai.review_mode);
                let mut task = crate::new_task_record(
                    request.server_id.clone(),
                    task_title(intent).into(),
                    intent.into(),
                    status.into(),
                    progress,
                    risk.into(),
                    approved_by.map(Into::into),
                );
                task.events.push(crate::TaskEvent {
                    at: task.created_at.clone(),
                    level: "info".into(),
                    message: if intent == "server_bootstrap" {
                        "开服计划已绑定到当前工作区，执行器将按资源、运行时、核心、启动顺序执行。"
                            .into()
                    } else {
                        "结构化服务器操作已从对话创建。".into()
                    },
                });
                data.tasks.insert(0, task.clone());
                crate::trim_task_history(&mut data.tasks, 30);
                task_created = true;
                Some(task)
            }
        } else {
            None
        };
        let actions = task
            .as_ref()
            .map(|_| vec!["查看任务详情".into()])
            .unwrap_or_default();
        if let Some(conversation_id) = request.conversation_id.as_deref()
            && !crate::conversations::append_exchange(
                &mut data,
                &request.server_id,
                conversation_id,
                &request.message,
                &full_reply,
                actions,
                task.as_ref().map(|task| task.id),
            )
        {
            eprintln!("[ai] conversation {conversation_id} 不存在，消息未持久化");
        }
        if let Err(error) = persist(&state, &data).await {
            let _ = send_event(
                &tx,
                "error",
                &json!({ "message": format!("对话结果持久化失败：{error}") }),
            )
            .await;
            return;
        }
        (task, task_created)
    };
    if task_created
        && let Some(task) = task.as_ref()
        && task.status == "queued"
    {
        crate::task_executor::spawn(state.clone(), task.id).await;
    }
    let done = json!({
        "id": Uuid::new_v4(),
        "time": Local::now().format("%H:%M").to_string(),
        "actions": if task.is_some() { vec!["查看任务详情"] } else { Vec::<&str>::new() },
        "task": task,
        "conversation_id": request.conversation_id,
    });
    let _ = send_event(&tx, "done", &done).await;
}

/// 原生 CLI 复用现有对话链路。Codex 的权限由已持久化的审核档位决定，
/// 而不是由前端或 Agent 配置中的任意参数决定。
struct CliInvocation {
    args: Vec<String>,
    codex_full_access: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexPermissionProfile {
    ReadOnly,
    FullAccess,
}

fn codex_full_access_allowed() -> bool {
    environment_opt_in(CODEX_FULL_ACCESS_ENV)
        && backend_listens_on_loopback()
        && trusted_codex_command().is_some()
}

fn environment_opt_in(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| parse_opt_in_value(&value))
}

fn parse_opt_in_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn backend_listens_on_loopback() -> bool {
    let bind_address =
        std::env::var("SCULK_BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let bind_address = bind_address.trim();
    if bind_address.starts_with("localhost:") {
        return true;
    }
    bind_address
        .parse::<std::net::SocketAddr>()
        .map(|address| address.ip().is_loopback())
        .unwrap_or(false)
}

fn trusted_codex_command() -> Option<PathBuf> {
    let configured = std::env::var_os(CODEX_TRUSTED_COMMAND_ENV)?;
    let path = PathBuf::from(configured);
    if !path.is_absolute() {
        return None;
    }
    let canonical = fs::canonicalize(path).ok()?;
    canonical.is_file().then_some(canonical)
}

fn codex_command_is_trusted(command: &str) -> bool {
    let Some(trusted) = trusted_codex_command() else {
        return false;
    };
    let configured = PathBuf::from(command);
    configured.is_absolute() && fs::canonicalize(configured).ok().as_ref() == Some(&trusted)
}

fn codex_full_access_requirement_message() -> String {
    "Codex 完全访问已拒绝。仅可在回环监听下设置 SCULK_ALLOW_CODEX_FULL=true，且将 SCULK_CODEX_TRUSTED_COMMAND 设置为已存在的绝对 Codex 可执行路径后重启后端。".into()
}

fn codex_permission_profile(
    review_mode: &str,
    full_access_allowed: bool,
) -> Result<CodexPermissionProfile, String> {
    match review_mode {
        // The HTTP SSE contract has no command-approval round trip. Until that exists,
        // neither approval nor auto can safely turn into a writable Codex session.
        "approval" | "auto" => Ok(CodexPermissionProfile::ReadOnly),
        "full" if full_access_allowed => Ok(CodexPermissionProfile::FullAccess),
        "full" => Err(codex_full_access_requirement_message()),
        _ => Err("无效的 Codex 审核模式".into()),
    }
}

fn validate_full_access_agent(agent: &AiAgent, review_mode: &str) -> Result<(), String> {
    if review_mode != "full" || (agent.kind == "codex" && agent.transport == "cli") {
        return Ok(());
    }
    Err(
        "完全访问仅支持原生 Codex CLI。请在当前对话中选择 Codex CLI，或切回请求批准/替我审核模式。"
            .into(),
    )
}

fn cli_invocation(
    agent: &AiAgent,
    reasoning_effort: Option<&str>,
    review_mode: &str,
    full_access_allowed: bool,
) -> Result<CliInvocation, String> {
    let mut args = if agent.kind == "codex" {
        if !agent.args.is_empty() {
            return Err("原生 Codex CLI 参数由受控运行时管理，不能自定义".into());
        }
        Vec::new()
    } else {
        agent.args.clone()
    };
    match agent.kind.as_str() {
        "codex" => {
            let profile = codex_permission_profile(review_mode, full_access_allowed)?;
            args.extend([
                "--ask-for-approval".into(),
                match profile {
                    CodexPermissionProfile::ReadOnly => "never",
                    CodexPermissionProfile::FullAccess => "never",
                }
                .into(),
            ]);
            if profile == CodexPermissionProfile::FullAccess {
                args.push("--search".into());
            }
            args.push("exec".into());
            if let Some(effort) = reasoning_effort {
                validate_effort(Some(effort), CODEX_EFFORTS).map_err(|(_, error)| error)?;
                args.extend(["-c".into(), format!("model_reasoning_effort='{effort}'")]);
            }
            args.extend([
                "--sandbox".into(),
                match profile {
                    CodexPermissionProfile::ReadOnly => "read-only",
                    CodexPermissionProfile::FullAccess => "danger-full-access",
                }
                .into(),
            ]);
            args.push("--skip-git-repo-check".into());
            args.push("--ephemeral".into());
            if profile == CodexPermissionProfile::ReadOnly {
                args.extend(["--ignore-user-config".into(), "--ignore-rules".into()]);
            }
            args.extend(["--color".into(), "never".into(), "-".into()]);
            return Ok(CliInvocation {
                args,
                codex_full_access: profile == CodexPermissionProfile::FullAccess,
            });
        }
        "claude-code" => {
            args.extend(["-p".into(), "--output-format".into(), "text".into()]);
            if let Some(effort) = reasoning_effort {
                validate_effort(Some(effort), CLAUDE_EFFORTS).map_err(|(_, error)| error)?;
                args.extend(["--effort".into(), effort.into()]);
            }
        }
        _ => return Err("该 Agent 类型没有原生 CLI 调用契约".into()),
    }
    Ok(CliInvocation {
        args,
        codex_full_access: false,
    })
}

fn build_cli_prompt(
    request: &ChatStreamRequest,
    server_context: &str,
    language: &str,
    persona: &str,
    skill_context: &str,
    plugin_context: &str,
) -> String {
    let mut prompt = format!("[Sculk Catalyst 工作台] 当前工作区：{server_context}。{language}");
    if !persona.is_empty() {
        prompt.push('\n');
        prompt.push_str(persona);
    }
    if !skill_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(skill_context);
    }
    if !plugin_context.is_empty() {
        prompt.push_str("\n\n可用插件资源上下文：\n");
        prompt.push_str(plugin_context);
    }
    if !request.history.is_empty() {
        prompt.push_str("\n\n最近对话：\n");
        let mut total = 0usize;
        let mut kept = Vec::new();
        for turn in request.history.iter().rev() {
            if !matches!(turn.role.as_str(), "user" | "assistant") {
                continue;
            }
            total += turn.content.chars().count();
            if total > HISTORY_CHAR_LIMIT {
                break;
            }
            kept.push(turn);
        }
        for turn in kept.into_iter().rev() {
            prompt.push_str(&turn.role);
            prompt.push_str(": ");
            prompt.push_str(&turn.content);
            prompt.push('\n');
        }
    }
    prompt.push_str("\n用户：");
    prompt.push_str(&request.message);
    prompt
}

enum CliOutputRead {
    Eof,
    ClientGone,
    OutputLimit,
    TimedOut(String),
    Failed(String),
}

struct CliOutputBudget {
    limit: usize,
    consumed: AtomicUsize,
}

impl CliOutputBudget {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            consumed: AtomicUsize::new(0),
        }
    }

    fn try_consume(&self, bytes: usize) -> bool {
        let mut current = self.consumed.load(Ordering::Relaxed);
        loop {
            let Some(next) = current.checked_add(bytes) else {
                return false;
            };
            if next > self.limit {
                return false;
            }
            match self.consumed.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(observed) => current = observed,
            }
        }
    }
}

struct CliOutputRedactor {
    secrets: Vec<String>,
    utf8_tail: Vec<u8>,
    text_tail: String,
    tail_chars: usize,
}

impl CliOutputRedactor {
    fn new(secrets: &[String]) -> Self {
        let secrets = normalized_cli_redaction_secrets(secrets.iter().cloned());
        let tail_chars = secrets
            .iter()
            .map(|secret| secret.chars().count().saturating_sub(1))
            .max()
            .unwrap_or_default();
        Self {
            secrets,
            utf8_tail: Vec::new(),
            text_tail: String::new(),
            tail_chars,
        }
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> String {
        let text = self.decode_utf8(bytes, false);
        self.push_text(&text)
    }

    fn finish(&mut self) -> String {
        let text = self.decode_utf8(&[], true);
        let mut output = self.push_text(&text);
        output.push_str(&self.flush_text_tail());
        output
    }

    fn decode_utf8(&mut self, bytes: &[u8], finish: bool) -> String {
        let mut input = std::mem::take(&mut self.utf8_tail);
        input.extend_from_slice(bytes);

        let mut output = String::new();
        let mut offset = 0;
        loop {
            match std::str::from_utf8(&input[offset..]) {
                Ok(valid) => {
                    output.push_str(valid);
                    break;
                }
                Err(error) => {
                    let valid_bytes = error.valid_up_to();
                    if valid_bytes > 0 {
                        let valid = std::str::from_utf8(&input[offset..offset + valid_bytes])
                            .expect("UTF-8 valid prefix");
                        output.push_str(valid);
                        offset += valid_bytes;
                    }
                    match error.error_len() {
                        Some(invalid_bytes) => {
                            output.push('\u{fffd}');
                            offset += invalid_bytes;
                        }
                        None if finish => {
                            output.push_str(&String::from_utf8_lossy(&input[offset..]));
                            break;
                        }
                        None => {
                            self.utf8_tail.extend_from_slice(&input[offset..]);
                            break;
                        }
                    }
                }
            }
        }
        output
    }

    fn push_text(&mut self, text: &str) -> String {
        self.text_tail.push_str(text);
        let safe_end = prefix_before_last_chars(&self.text_tail, self.tail_chars);
        self.drain_text_tail(safe_end)
    }

    fn flush_text_tail(&mut self) -> String {
        let safe_end = self.text_tail.len();
        self.drain_text_tail(safe_end)
    }

    fn drain_text_tail(&mut self, safe_end: usize) -> String {
        let text = std::mem::take(&mut self.text_tail);
        let mut output = String::new();
        let mut cursor = 0;
        while cursor < safe_end {
            if let Some(secret) = self
                .secrets
                .iter()
                .find(|secret| text[cursor..].starts_with(secret.as_str()))
            {
                output.push_str("[REDACTED]");
                cursor += secret.len();
            } else {
                let character = text[cursor..]
                    .chars()
                    .next()
                    .expect("cursor remains at a UTF-8 character boundary");
                output.push(character);
                cursor += character.len_utf8();
            }
        }
        self.text_tail = text[cursor..].to_string();
        output
    }
}

#[derive(Default)]
struct CliStderrRead {
    output: Vec<u8>,
    output_limit_exceeded: bool,
}

enum CliExitWait {
    Exited(ExitStatus),
    OutputLimit,
    Failed(String),
    TimedOut,
}

fn prefix_before_last_chars(text: &str, chars_to_keep: usize) -> usize {
    if chars_to_keep == 0 {
        return text.len();
    }
    text.char_indices()
        .rev()
        .nth(chars_to_keep.saturating_sub(1))
        .map(|(index, _)| index)
        .unwrap_or_default()
}

fn append_cli_output(full_reply: &mut String, piece: &str) -> bool {
    if full_reply.len().saturating_add(piece.len()) > CLI_MAX_VISIBLE_OUTPUT_BYTES {
        return false;
    }
    full_reply.push_str(piece);
    true
}

fn cli_output_limit_message() -> String {
    format!(
        "CLI 输出超过 {} MiB 上限，已终止",
        CLI_MAX_TOTAL_OUTPUT_BYTES / 1024 / 1024
    )
}

async fn read_cli_output<R: AsyncRead + Unpin>(
    reader: R,
    tx: &mpsc::Sender<Event>,
    full_reply: &mut String,
    secrets: &[String],
    output_budget: Arc<CliOutputBudget>,
    mut output_limit: watch::Receiver<bool>,
    idle_timeout: Duration,
) -> CliOutputRead {
    let mut reader = BufReader::with_capacity(CLI_OUTPUT_READ_CHUNK_BYTES, reader);
    let mut redactor = CliOutputRedactor::new(secrets);
    let mut output_limit_open = true;
    loop {
        if *output_limit.borrow() {
            return CliOutputRead::OutputLimit;
        }
        let next = tokio::select! {
            _ = tx.closed() => return CliOutputRead::ClientGone,
            changed = output_limit.changed(), if output_limit_open => {
                match changed {
                    Ok(()) if *output_limit.borrow() => return CliOutputRead::OutputLimit,
                    Ok(()) => continue,
                    Err(_) => {
                        output_limit_open = false;
                        continue;
                    }
                }
            }
            result = tokio::time::timeout(idle_timeout, reader.fill_buf()) => result,
        };
        let (consumed, piece, eof) = match next {
            Err(_) => {
                return CliOutputRead::TimedOut(format!(
                    "CLI 连续 {} 秒没有输出，已终止",
                    idle_timeout.as_secs()
                ));
            }
            Ok(Err(error)) => return CliOutputRead::Failed(error.to_string()),
            Ok(Ok(bytes)) if bytes.is_empty() => (0, redactor.finish(), true),
            Ok(Ok(bytes)) => {
                if !output_budget.try_consume(bytes.len()) {
                    return CliOutputRead::OutputLimit;
                }
                (bytes.len(), redactor.push_bytes(bytes), false)
            }
        };
        if consumed > 0 {
            reader.consume(consumed);
        }
        if !piece.is_empty() {
            if !append_cli_output(full_reply, &piece) {
                return CliOutputRead::OutputLimit;
            }
            if send_event(tx, "delta", &json!({ "content": piece }))
                .await
                .is_err()
            {
                return CliOutputRead::ClientGone;
            }
        }
        if eof {
            return CliOutputRead::Eof;
        }
    }
}

async fn read_cli_last_message_file(
    child: &mut Child,
    path: &FsPath,
    tx: &mpsc::Sender<Event>,
    full_reply: &mut String,
    secrets: &[String],
    output_budget: Arc<CliOutputBudget>,
    output_limit: &mut watch::Receiver<bool>,
) -> CliOutputRead {
    let mut previous_length = None;
    let mut exited_at = None;
    loop {
        if *output_limit.borrow() {
            return CliOutputRead::OutputLimit;
        }

        match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return CliOutputRead::Failed("Codex CLI 输出文件不是普通文件".into());
            }
            Ok(_) => match tokio::fs::read(path).await {
                Ok(bytes) if bytes.is_empty() => {
                    previous_length = Some(0);
                }
                Ok(bytes) if previous_length == Some(bytes.len()) => {
                    if !output_budget.try_consume(bytes.len()) {
                        return CliOutputRead::OutputLimit;
                    }
                    let mut redactor = CliOutputRedactor::new(secrets);
                    let mut text = redactor.push_bytes(&bytes);
                    text.push_str(&redactor.finish());
                    if text.is_empty() {
                        return CliOutputRead::Failed("Codex CLI 未返回文本".into());
                    }
                    for chunk in text.chars().collect::<Vec<_>>().chunks(2_048) {
                        let chunk = chunk.iter().collect::<String>();
                        if !append_cli_output(full_reply, &chunk) {
                            return CliOutputRead::OutputLimit;
                        }
                        if send_event(tx, "delta", &json!({ "content": chunk }))
                            .await
                            .is_err()
                        {
                            return CliOutputRead::ClientGone;
                        }
                    }
                    return CliOutputRead::Eof;
                }
                Ok(bytes) => {
                    previous_length = Some(bytes.len());
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    previous_length = None;
                }
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
                Err(error) => return CliOutputRead::Failed(error.to_string()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                previous_length = None;
            }
            Err(error) => return CliOutputRead::Failed(error.to_string()),
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let observed = exited_at.get_or_insert_with(Instant::now);
                if observed.elapsed() >= Duration::from_secs(1) {
                    return CliOutputRead::Failed(format!(
                        "Codex CLI 已退出但未写入最终回复：{status}"
                    ));
                }
            }
            Ok(None) => {}
            Err(error) => return CliOutputRead::Failed(error.to_string()),
        }

        tokio::select! {
            _ = tx.closed() => return CliOutputRead::ClientGone,
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    }
}

async fn read_cli_stderr<R: AsyncRead + Unpin>(
    mut reader: R,
    output_budget: Arc<CliOutputBudget>,
    output_limit: watch::Sender<bool>,
) -> CliStderrRead {
    let mut output = Vec::new();
    let mut buffer = [0u8; CLI_OUTPUT_READ_CHUNK_BYTES];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => {
                return CliStderrRead {
                    output,
                    output_limit_exceeded: false,
                };
            }
            Ok(read) => {
                if !output_budget.try_consume(read) {
                    let _ = output_limit.send(true);
                    return CliStderrRead {
                        output,
                        output_limit_exceeded: true,
                    };
                }
                let retained = (CLI_MAX_STDERR_CAPTURE_BYTES - output.len()).min(read);
                output.extend_from_slice(&buffer[..retained]);
            }
        }
    }
}

async fn wait_for_cli_exit(
    child: &mut Child,
    output_limit: &mut watch::Receiver<bool>,
) -> CliExitWait {
    if *output_limit.borrow() {
        return CliExitWait::OutputLimit;
    }

    let timeout = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(timeout);
    let wait = child.wait();
    tokio::pin!(wait);
    let mut output_limit_open = true;
    loop {
        tokio::select! {
            result = &mut wait => match result {
                Ok(status) => return CliExitWait::Exited(status),
                Err(error) => return CliExitWait::Failed(error.to_string()),
            },
            _ = &mut timeout => return CliExitWait::TimedOut,
            changed = output_limit.changed(), if output_limit_open => {
                match changed {
                    Ok(()) if *output_limit.borrow() => return CliExitWait::OutputLimit,
                    Ok(()) => continue,
                    Err(_) => output_limit_open = false,
                }
            }
        }
    }
}

fn redact_secrets(mut text: String, secrets: &[String]) -> String {
    for secret in secrets.iter().filter(|secret| secret.len() >= 4) {
        text = text.replace(secret, "[REDACTED]");
    }
    snippet(text.trim(), 1_000)
}

fn normalized_cli_redaction_secrets(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut secrets = values
        .into_iter()
        .filter(|secret| secret.len() >= 4 && secret.len() <= CLI_MAX_REDACTION_SECRET_BYTES)
        .collect::<Vec<_>>();
    secrets.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    secrets.dedup();
    secrets
}

fn redact_cli_text(mut text: String, secrets: &[String]) -> String {
    for secret in secrets.iter().filter(|secret| secret.len() >= 4) {
        text = text.replace(secret, "[REDACTED]");
    }
    text
}

fn cli_redaction_secrets() -> Vec<String> {
    normalized_cli_redaction_secrets(
        std::env::vars()
            .filter(|(name, value)| {
                !value.is_empty() && is_sensitive_cli_environment_variable(name)
            })
            .map(|(_, value)| value),
    )
}

fn sanitized_cli_stderr(stderr: &[u8], secrets: &[String]) -> String {
    snippet(
        redact_cli_text(String::from_utf8_lossy(stderr).into_owned(), secrets).trim(),
        1_000,
    )
}

fn is_sensitive_cli_environment_variable(name: &str) -> bool {
    let normalized = name.to_ascii_uppercase();
    [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "PRIVATE",
        "DATABASE_URL",
        "DB_URL",
        "REDIS_URL",
        "CONNECTION_STRING",
        "JWT",
        "OAUTH",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

#[cfg(test)]
fn is_cli_environment_variable_allowed(name: &str, codex_full_access: bool) -> bool {
    CLI_ENV_ALLOWLIST
        .iter()
        .any(|allowed| name.eq_ignore_ascii_case(allowed))
        || (codex_full_access && name.eq_ignore_ascii_case("CODEX_HOME"))
}

/// The CLI gets only process bootstrap variables. Authentication stays in the
/// user's persisted CLI login instead of inheriting application credentials.
fn configure_cli_environment(command: &mut Command, codex_full_access: bool) {
    command.env_clear();
    for name in CLI_ENV_ALLOWLIST {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if codex_full_access && let Some(value) = std::env::var_os("CODEX_HOME") {
        command.env("CODEX_HOME", value);
    }
}

#[cfg(windows)]
fn hide_cli_window(command: &mut Command) {
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_cli_window(_command: &mut Command) {}

fn checked_cli_workspace_directory(
    workspace_directory: Option<&PathBuf>,
    workspace_kind: Option<&str>,
) -> Result<PathBuf, String> {
    let directory =
        workspace_directory.ok_or_else(|| "CLI 工作区不存在，已拒绝启动。".to_string())?;
    let category = match workspace_kind {
        Some("project") => "projects",
        Some("server") => "servers",
        _ => return Err("CLI 工作区类型无效，已拒绝启动。".into()),
    };
    let metadata = fs::symlink_metadata(directory)
        .map_err(|_| "CLI 工作区不存在或不可访问，已拒绝启动。".to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("CLI 工作区必须是受管的真实目录，已拒绝启动。".into());
    }
    let directory = fs::canonicalize(directory)
        .map_err(|_| "无法解析 CLI 工作区路径，已拒绝启动。".to_string())?;
    let root = crate::runtime::data_root().join(category);
    if let Ok(root) = fs::canonicalize(&root) {
        if directory == root {
            return Err("CLI 工作区路径必须指向具体工作区目录，已拒绝启动。".into());
        }
        if directory.starts_with(&root) {
            return Ok(directory);
        }
    }
    // Imported workspaces are explicitly registered by the user and retain
    // their canonical path in ServerInfo.  Keep the same no-symlink boundary,
    // while allowing those paths outside SCULK_DATA_DIR.
    if crate::server_import::canonicalize_existing_directory(&directory).is_err() {
        return Err("CLI 工作区路径不可访问或包含符号链接，已拒绝启动。".into());
    }
    Ok(directory)
}

#[cfg(windows)]
fn is_windows_batch_script(path: &FsPath) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
}

#[cfg(windows)]
fn is_safe_cmd_batch_path(path: &FsPath) -> bool {
    path.to_str().is_some_and(|value| {
        !value.chars().any(|character| {
            matches!(
                character,
                '"' | '&' | '|' | '<' | '>' | '(' | ')' | '^' | '%' | '!'
            )
        })
    })
}

#[cfg(windows)]
fn is_safe_cmd_runtime_argument(argument: &str) -> bool {
    !argument.is_empty()
        && argument.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '=' | '.' | '\'')
        })
}

#[cfg(windows)]
fn windows_cmd_compatible_batch_path(path: &FsPath) -> Result<PathBuf, String> {
    let raw = path
        .to_str()
        .ok_or_else(|| "Codex 批处理启动器路径不是有效的 Windows 路径".to_string())?;
    // Windows canonical paths have a \\?\ prefix. Keep it for trust checks,
    // but translate it before cmd.exe resolves the batch-file command.
    let value = if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(drive) = raw.strip_prefix(r"\\?\") {
        let bytes = drive.as_bytes();
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || bytes[2] != b'\\'
        {
            return Err("Codex 批处理启动器使用了 cmd.exe 不支持的设备路径".into());
        }
        drive.to_owned()
    } else {
        raw.to_owned()
    };
    let path = PathBuf::from(value);
    if !path.is_absolute() || !is_safe_cmd_batch_path(&path) {
        return Err("Codex 批处理启动器路径包含不受支持的命令解释符".into());
    }
    Ok(path)
}

#[cfg(windows)]
fn windows_cli_working_directory(path: &FsPath) -> Result<PathBuf, String> {
    let raw = path
        .to_str()
        .ok_or_else(|| "CLI 工作目录不是有效的 Windows 路径".to_string())?;
    // Rust canonicalize may return an extended-length path. CreateProcess can
    // accept it, but cmd.exe and the Node wrapper used by codex.cmd cannot
    // reliably use it as their current directory.
    let value = if raw.starts_with(r"\\?\UNC\") {
        // cmd.exe cannot use either extended or normal UNC paths as its
        // current directory. It silently falls back to the Windows directory,
        // which would make the relative Codex output file unreadable here.
        return Err("CLI 工作目录不能位于 UNC 路径，cmd.exe 不支持该当前目录".into());
    } else if let Some(drive) = raw.strip_prefix(r"\\?\") {
        let bytes = drive.as_bytes();
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || bytes[2] != b'\\'
        {
            return Err("CLI 工作目录使用了 cmd.exe 不支持的设备路径".into());
        }
        drive.to_owned()
    } else {
        raw.to_owned()
    };
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("CLI 工作目录必须是绝对路径".into());
    }
    if path.to_str().is_some_and(|value| value.starts_with(r"\\")) {
        return Err("CLI 工作目录不能位于 UNC 路径，cmd.exe 不支持该当前目录".into());
    }
    Ok(path)
}

#[cfg(not(windows))]
fn windows_cli_working_directory(path: &FsPath) -> Result<PathBuf, String> {
    Ok(path.to_owned())
}

#[cfg(windows)]
fn windows_batch_invocation(shim: &FsPath, args: &[String]) -> Result<String, String> {
    let shim = windows_cmd_compatible_batch_path(shim)?;
    if args
        .iter()
        .any(|argument| !is_safe_cmd_runtime_argument(argument))
    {
        return Err("Codex 批处理启动器参数包含不受支持的命令解释符".into());
    }
    Ok(format!(
        r#" /d /s /c ""{}" {}""#,
        shim.display(),
        args.join(" ")
    ))
}

/// Windows does not reliably preserve stdio semantics when CreateProcess is
/// pointed at an npm-generated `.cmd` shim. The command string here contains
/// only the canonical shim path and runtime-owned Codex flags; user prompts
/// stay on stdin and are never interpreted by cmd.exe.
fn cli_command(agent: &AiAgent, invocation: &CliInvocation) -> Result<Command, String> {
    #[cfg(windows)]
    {
        let configured = PathBuf::from(&agent.command);
        if agent.kind == "codex" && configured.is_absolute() && is_windows_batch_script(&configured)
        {
            let shim = fs::canonicalize(&configured)
                .map_err(|_| "无法解析原生 Codex 批处理启动器".to_string())?;
            if !shim.is_file() {
                return Err("Codex 批处理启动器不是普通文件".into());
            }
            let command_interpreter = std::env::var_os("COMSPEC")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "cmd.exe".into());
            let mut command = Command::new(command_interpreter);
            command.raw_arg(windows_batch_invocation(&shim, &invocation.args)?);
            return Ok(command);
        }
    }
    let mut command = Command::new(&agent.command);
    command.args(&invocation.args);
    Ok(command)
}

enum CliResponseCapture {
    Stdout,
    #[cfg(windows)]
    LastMessageFile(PathBuf),
}

impl CliResponseCapture {
    fn uses_last_message_file(&self) -> bool {
        #[cfg(windows)]
        if matches!(self, Self::LastMessageFile(_)) {
            return true;
        }
        false
    }
}

struct CliOutputFileCleanup(Option<PathBuf>);

impl CliOutputFileCleanup {
    fn for_capture(capture: &CliResponseCapture) -> Self {
        #[cfg(windows)]
        if let CliResponseCapture::LastMessageFile(path) = capture {
            return Self(Some(path.clone()));
        }
        Self(None)
    }
}

impl Drop for CliOutputFileCleanup {
    fn drop(&mut self) {
        let Some(path) = self.0.take() else {
            return;
        };
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return;
        };
        if metadata.is_file() && !metadata.file_type().is_symlink() {
            let _ = fs::remove_file(path);
        }
    }
}

fn configure_cli_response_capture(
    agent: &AiAgent,
    invocation: &mut CliInvocation,
    working_directory: &FsPath,
) -> Result<CliResponseCapture, String> {
    #[cfg(windows)]
    {
        let configured = PathBuf::from(&agent.command);
        if agent.kind == "codex" && configured.is_absolute() && is_windows_batch_script(&configured)
        {
            let output_name = format!(".sculk-codex-{}.txt", Uuid::new_v4().simple());
            let output_path = working_directory.join(&output_name);
            if output_path.exists() {
                return Err("无法分配 Codex CLI 输出文件".into());
            }
            let stdin_index = invocation
                .args
                .iter()
                .rposition(|argument| argument == "-")
                .ok_or_else(|| "Codex CLI 缺少受控标准输入参数".to_string())?;
            invocation.args.splice(
                stdin_index..stdin_index,
                ["--output-last-message".into(), output_name],
            );
            return Ok(CliResponseCapture::LastMessageFile(output_path));
        }
    }
    Ok(CliResponseCapture::Stdout)
}

async fn terminate_cli_process_tree(
    child: &mut Child,
    pid: u32,
    guard: &crate::process_platform::ProcessGuard,
) {
    if crate::process_platform::start_kill_tree(child, pid, Some(guard)).is_err() {
        let _ = child.start_kill();
    }
    let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    let _ = crate::process_platform::cleanup_remaining_tree(pid, Some(guard));
}

async fn stream_cli_agent(
    agent: &AiAgent,
    reasoning_effort: Option<&str>,
    review_mode: &str,
    full_access_allowed: bool,
    request: &ChatStreamRequest,
    workspace_directory: Option<&PathBuf>,
    workspace_kind: Option<&str>,
    server_context: &str,
    language: &str,
    persona: &str,
    skill_context: &str,
    plugin_context: &str,
    tx: &mpsc::Sender<Event>,
    full_reply: &mut String,
) -> StreamOutcome {
    let mut invocation =
        match cli_invocation(agent, reasoning_effort, review_mode, full_access_allowed) {
            Ok(invocation) => invocation,
            Err(error) if agent.kind == "codex" => return StreamOutcome::PolicyDenied(error),
            Err(error) => return StreamOutcome::FailedBeforeOutput(error),
        };
    if invocation.codex_full_access && !codex_command_is_trusted(&agent.command) {
        return StreamOutcome::PolicyDenied(
            "Codex 完全访问要求当前 Agent 的启动命令与 SCULK_CODEX_TRUSTED_COMMAND 指向同一个绝对可执行文件。".into(),
        );
    }
    let working_directory =
        match checked_cli_workspace_directory(workspace_directory, workspace_kind) {
            Ok(directory) => directory,
            Err(error) => return StreamOutcome::PolicyDenied(error),
        };
    let working_directory = match windows_cli_working_directory(&working_directory) {
        Ok(directory) => directory,
        Err(error) => return StreamOutcome::PolicyDenied(error),
    };
    let prompt = build_cli_prompt(
        request,
        server_context,
        language,
        persona,
        skill_context,
        plugin_context,
    );
    let response_capture =
        match configure_cli_response_capture(agent, &mut invocation, &working_directory) {
            Ok(capture) => capture,
            Err(error) => return StreamOutcome::FailedBeforeOutput(error),
        };
    let _output_file_cleanup = CliOutputFileCleanup::for_capture(&response_capture);
    let mut command = match cli_command(agent, &invocation) {
        Ok(command) => command,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error),
    };
    command
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    if response_capture.uses_last_message_file() {
        command.stdout(Stdio::null());
    } else {
        command.stdout(Stdio::piped());
    }
    configure_cli_environment(&mut command, invocation.codex_full_access);
    command.current_dir(&working_directory);
    if crate::process_platform::configure_managed_command(&mut command).is_err() {
        return StreamOutcome::FailedBeforeOutput("无法配置受管 CLI 进程".into());
    }
    hide_cli_window(&mut command);
    let guard = match crate::process_platform::create_process_guard() {
        Ok(guard) => guard,
        Err(_) => return StreamOutcome::FailedBeforeOutput("无法初始化 CLI 进程保护".into()),
    };
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error.to_string()),
    };
    let pid = match child.id() {
        Some(pid) => pid,
        None => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return StreamOutcome::FailedBeforeOutput("无法确认 CLI 进程".into());
        }
    };
    if crate::process_platform::bind_process_to_guard(&guard, pid).is_err() {
        terminate_cli_process_tree(&mut child, pid, &guard).await;
        return StreamOutcome::FailedBeforeOutput("无法保护 CLI 进程".into());
    }
    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            return StreamOutcome::FailedBeforeOutput("CLI 标准输入不可用".into());
        }
    };
    // Codex CLI 0.144 on Windows waits for a line terminator even after EOF.
    let mut prompt = prompt;
    prompt.push('\n');
    match tokio::time::timeout(Duration::from_secs(10), stdin.write_all(prompt.as_bytes())).await {
        Err(_) => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            return StreamOutcome::FailedBeforeOutput("写入 CLI 提示词超时".into());
        }
        Ok(Err(error)) => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            return StreamOutcome::FailedBeforeOutput(format!("写入 CLI 提示词失败：{error}"));
        }
        Ok(Ok(())) => {}
    }
    if let Err(error) = stdin.shutdown().await {
        terminate_cli_process_tree(&mut child, pid, &guard).await;
        return StreamOutcome::FailedBeforeOutput(format!("关闭 CLI 标准输入失败：{error}"));
    }

    let stdout = if response_capture.uses_last_message_file() {
        None
    } else {
        match child.stdout.take() {
            Some(stdout) => Some(stdout),
            None => {
                terminate_cli_process_tree(&mut child, pid, &guard).await;
                return StreamOutcome::FailedBeforeOutput("CLI 标准输出不可用".into());
            }
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            return StreamOutcome::FailedBeforeOutput("CLI 标准错误不可用".into());
        }
    };
    let redaction_secrets = cli_redaction_secrets();
    let output_budget = Arc::new(CliOutputBudget::new(CLI_MAX_TOTAL_OUTPUT_BYTES));
    let (output_limit_tx, output_limit_rx) = watch::channel(false);
    let mut exit_output_limit_rx = output_limit_rx.clone();
    let mut file_output_limit_rx = output_limit_rx.clone();
    let stderr_task = tokio::spawn(read_cli_stderr(
        stderr,
        Arc::clone(&output_budget),
        output_limit_tx,
    ));
    let meta = json!({
        "provider": agent.name,
        "model": format!("CLI · {}", agent.kind),
        "fallback": false,
        "transport": "cli",
        "reasoning_effort": reasoning_effort,
        "codex_full_access": invocation.codex_full_access,
    });
    if send_event(tx, "meta", &meta).await.is_err() {
        terminate_cli_process_tree(&mut child, pid, &guard).await;
        stderr_task.abort();
        return StreamOutcome::ClientGone;
    }

    let read = match &response_capture {
        CliResponseCapture::Stdout => {
            let stdout = stdout.expect("stdout capture is configured");
            match tokio::time::timeout(
                Duration::from_secs(30 * 60),
                read_cli_output(
                    stdout,
                    tx,
                    full_reply,
                    &redaction_secrets,
                    output_budget,
                    output_limit_rx,
                    Duration::from_secs(120),
                ),
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(_) => CliOutputRead::TimedOut("CLI 总执行时间超过 30 分钟，已终止".into()),
            }
        }
        #[cfg(windows)]
        CliResponseCapture::LastMessageFile(path) => {
            match tokio::time::timeout(
                Duration::from_secs(30 * 60),
                read_cli_last_message_file(
                    &mut child,
                    path,
                    tx,
                    full_reply,
                    &redaction_secrets,
                    output_budget,
                    &mut file_output_limit_rx,
                ),
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(_) => {
                    CliOutputRead::TimedOut("Codex CLI 总执行时间超过 30 分钟，已终止".into())
                }
            }
        }
    };
    match read {
        CliOutputRead::ClientGone => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            stderr_task.abort();
            return StreamOutcome::ClientGone;
        }
        CliOutputRead::OutputLimit => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            stderr_task.abort();
            let message = cli_output_limit_message();
            return if full_reply.is_empty() {
                StreamOutcome::FailedBeforeOutput(message)
            } else {
                StreamOutcome::FailedMidway(message)
            };
        }
        CliOutputRead::TimedOut(message) => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            stderr_task.abort();
            return if full_reply.is_empty() {
                StreamOutcome::FailedBeforeOutput(message)
            } else {
                StreamOutcome::FailedMidway(message)
            };
        }
        CliOutputRead::Failed(error) => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            stderr_task.abort();
            return if full_reply.is_empty() {
                StreamOutcome::FailedBeforeOutput(error)
            } else {
                StreamOutcome::FailedMidway(error)
            };
        }
        CliOutputRead::Eof => {}
    }
    if response_capture.uses_last_message_file() {
        // The npm Windows command wrapper can retain handles after the last
        // message is written. Once the official CLI output file is read, the
        // managed process tree can be closed.
        terminate_cli_process_tree(&mut child, pid, &guard).await;
        stderr_task.abort();
        if *exit_output_limit_rx.borrow() {
            let message = cli_output_limit_message();
            return if full_reply.is_empty() {
                StreamOutcome::FailedBeforeOutput(message)
            } else {
                StreamOutcome::FailedMidway(message)
            };
        }
        return if full_reply.trim().is_empty() {
            StreamOutcome::FailedBeforeOutput("Codex CLI 未返回文本".into())
        } else {
            StreamOutcome::Completed
        };
    }
    let status = match wait_for_cli_exit(&mut child, &mut exit_output_limit_rx).await {
        CliExitWait::Exited(status) => status,
        CliExitWait::OutputLimit => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            stderr_task.abort();
            let message = cli_output_limit_message();
            return if full_reply.is_empty() {
                StreamOutcome::FailedBeforeOutput(message)
            } else {
                StreamOutcome::FailedMidway(message)
            };
        }
        CliExitWait::Failed(error) => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            stderr_task.abort();
            return StreamOutcome::FailedMidway(error);
        }
        CliExitWait::TimedOut => {
            terminate_cli_process_tree(&mut child, pid, &guard).await;
            stderr_task.abort();
            return StreamOutcome::FailedMidway("CLI 输出已关闭但进程未退出，已终止".into());
        }
    };
    // The CLI can exit while a descendant still holds stderr. Reap that tree
    // before awaiting stderr so a detached helper cannot block the chat stream.
    let _ = crate::process_platform::cleanup_remaining_tree(pid, Some(&guard));
    let stderr = stderr_task.await.unwrap_or_default();
    if stderr.output_limit_exceeded || *exit_output_limit_rx.borrow() {
        let message = cli_output_limit_message();
        return if full_reply.is_empty() {
            StreamOutcome::FailedBeforeOutput(message)
        } else {
            StreamOutcome::FailedMidway(message)
        };
    }
    if !status.success() {
        let error = sanitized_cli_stderr(&stderr.output, &redaction_secrets);
        let error = if error.is_empty() {
            format!("CLI 退出状态：{status}")
        } else {
            error
        };
        return if full_reply.is_empty() {
            StreamOutcome::FailedBeforeOutput(error)
        } else {
            StreamOutcome::FailedMidway(error)
        };
    }
    if full_reply.trim().is_empty() {
        return StreamOutcome::FailedBeforeOutput("CLI 未返回文本".into());
    }
    StreamOutcome::Completed
}

fn acp_relative_path(root: &FsPath, requested: &str) -> Result<PathBuf, String> {
    if requested.trim().is_empty() {
        return Err("ACP 文件路径不能为空".into());
    }
    let requested_path = FsPath::new(requested);
    let relative = if requested_path.is_absolute() {
        requested_path
            .strip_prefix(root)
            .map_err(|_| "ACP 文件路径必须位于当前工作区内".to_string())?
    } else {
        requested_path
    };
    let normalized = relative.to_string_lossy().replace('\\', "/");
    crate::safe_relative(&normalized)
        .map_err(|(_, message)| message)
        .and_then(|path| {
            if path.as_os_str().is_empty() {
                Err("ACP 文件路径不能为空".into())
            } else {
                Ok(path)
            }
        })
}

async fn acp_read_text_file(
    root: &FsPath,
    requested: &str,
    start_line: Option<u64>,
    num_lines: Option<u64>,
) -> Result<Value, String> {
    let relative = acp_relative_path(root, requested)?;
    let bytes = crate::workspace_fs::within_workspace(root.to_path_buf(), {
        let relative = relative.clone();
        move |workspace| {
            let metadata = workspace.symlink_metadata(&relative)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "ACP 路径不是普通文件",
                ));
            }
            let file = workspace.open(&relative)?;
            let mut bytes =
                Vec::with_capacity(metadata.len().min(ACP_MAX_TEXT_FILE_BYTES + 1) as usize);
            file.take(ACP_MAX_TEXT_FILE_BYTES + 1)
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        }
    })
    .await
    .map_err(|error| format!("ACP 读取文件失败：{error}"))?;
    if bytes.len() > ACP_MAX_TEXT_FILE_BYTES as usize {
        return Err("ACP 文本文件超过 2 MB 限制".into());
    }
    let content =
        String::from_utf8(bytes).map_err(|_| "ACP 只能读取 UTF-8 文本文件".to_string())?;
    let start = start_line.unwrap_or(1).max(1) as usize - 1;
    let selected = if start == 0 && num_lines.is_none() {
        content
    } else {
        let limit = num_lines.map(|value| value.min(ACP_MAX_TEXT_FILE_BYTES) as usize);
        content
            .lines()
            .skip(start)
            .take(limit.unwrap_or(usize::MAX))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok(json!({"content": selected}))
}

async fn acp_write_text_file(
    state: &AppState,
    server_id: &str,
    root: &FsPath,
    requested: &str,
    content: &str,
) -> Result<Value, String> {
    if content.len() > ACP_MAX_TEXT_FILE_BYTES as usize {
        return Err("ACP 文本文件超过 2 MB 限制".into());
    }
    let relative = acp_relative_path(root, requested)?;
    crate::reject_protected_server_artifact(&relative).map_err(|(_, message)| message)?;
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|error| format!("ACP 创建工作区目录失败：{error}"))?;
    let operation = crate::server_operation_lock(state, &format!("files:{server_id}")).await;
    let _guard = operation.lock().await;
    let content_owned = content.to_string();
    crate::workspace_fs::within_workspace(root.to_path_buf(), {
        let relative = relative.clone();
        let content = content_owned.clone();
        move |workspace| {
            let parent = relative.parent().unwrap_or(FsPath::new(""));
            workspace.create_dir_all(parent)?;
            match workspace.symlink_metadata(&relative) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "ACP 不允许通过符号链接写入",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            workspace.write(&relative, content.as_bytes())?;
            Ok(())
        }
    })
    .await
    .map_err(|error| format!("ACP 写入文件失败：{error}"))?;
    if crate::path_string(&relative) == "server.properties" {
        let mut data = state.inner.write().await;
        data.configs.insert(server_id.to_string(), content_owned);
        crate::persist(state, &data).await?;
    }
    Ok(Value::Null)
}

/// ACP permission prompts come from an external process. Local review preferences
/// must not turn those prompts into an implicit capability grant.
fn acp_permission_outcome(_review_mode: &str, _options: &[Value]) -> Value {
    json!({"outcome": {"outcome": "cancelled"}})
}

async fn stream_acp(
    state: &AppState,
    agent: &AiAgent,
    request: &ChatStreamRequest,
    server_context: &str,
    workspace_directory: Option<&PathBuf>,
    review_mode: &str,
    language: &str,
    persona: &str,
    skill_context: &str,
    plugin_context: &str,
    tx: &mpsc::Sender<Event>,
    full_reply: &mut String,
) -> StreamOutcome {
    let mut client = match AcpClient::spawn(&agent.command, &agent.args).await {
        Ok(client) => client,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error),
    };
    let outcome = stream_acp_inner(
        &mut client,
        state,
        agent,
        request,
        server_context,
        workspace_directory,
        review_mode,
        language,
        persona,
        skill_context,
        plugin_context,
        tx,
        full_reply,
    )
    .await;
    client.shutdown().await;
    outcome
}

async fn stream_acp_inner(
    client: &mut AcpClient,
    state: &AppState,
    agent: &AiAgent,
    request: &ChatStreamRequest,
    server_context: &str,
    workspace_directory: Option<&PathBuf>,
    review_mode: &str,
    language: &str,
    persona: &str,
    skill_context: &str,
    plugin_context: &str,
    tx: &mpsc::Sender<Event>,
    full_reply: &mut String,
) -> StreamOutcome {
    let handshake_timeout = Duration::from_secs(30);
    let fs_available = workspace_directory.is_some_and(|path| path.is_dir());
    let init_id = match client
        .send_request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": {
                        "readTextFile": fs_available,
                        "writeTextFile": fs_available
                    }
                },
            }),
        )
        .await
    {
        Ok(id) => id,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error),
    };
    if let Err(error) = client.wait_response(init_id, handshake_timeout).await {
        return StreamOutcome::FailedBeforeOutput(format!("initialize 失败：{error}"));
    }
    let cwd = workspace_directory
        .filter(|_| fs_available)
        .map(|path| path.to_string_lossy().to_string())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| ".".into());
    let new_id = match client
        .send_request("session/new", json!({"cwd": cwd, "mcpServers": []}))
        .await
    {
        Ok(id) => id,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error),
    };
    let session = match client.wait_response(new_id, handshake_timeout).await {
        Ok(value) => value["sessionId"].as_str().unwrap_or_default().to_string(),
        Err(error) => {
            return StreamOutcome::FailedBeforeOutput(format!("session/new 失败：{error}"));
        }
    };
    if session.is_empty() {
        return StreamOutcome::FailedBeforeOutput("Agent 未返回 sessionId".into());
    }
    let prompt = {
        let mut prompt =
            format!("[Sculk Catalyst 工作台] 当前服务器：{server_context}。{language}");
        if !persona.is_empty() {
            prompt.push('\n');
            prompt.push_str(persona);
        }
        if !skill_context.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(skill_context);
        }
        if !plugin_context.is_empty() {
            prompt.push_str("\n插件资源库候选（已按主流 > 开源 > 普通 > 付费排序）：\n");
            prompt.push_str(plugin_context);
            prompt.push_str("\n推荐插件时保持该优先级，并说明兼容性、开源或付费属性。\n");
        }
        prompt.push_str("\n\n");
        prompt.push_str(&request.message);
        prompt
    };
    let prompt_id = match client
        .send_request(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": prompt}]}),
        )
        .await
    {
        Ok(id) => id,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error),
    };

    let meta = json!({ "provider": agent.name, "model": format!("ACP · {}", agent.kind), "fallback": false });
    if send_event(tx, "meta", &meta).await.is_err() {
        return StreamOutcome::ClientGone;
    }
    let mid_fail = |reason: String, produced: bool| {
        if produced {
            StreamOutcome::FailedMidway(reason)
        } else {
            StreamOutcome::FailedBeforeOutput(reason)
        }
    };
    loop {
        let message = match client.next_message(Duration::from_secs(120)).await {
            Ok(message) => message,
            Err(error) => return mid_fail(error, !full_reply.is_empty()),
        };
        if let Some(method) = message["method"].as_str() {
            match method {
                "session/update" => {
                    let update = &message["params"]["update"];
                    if update["sessionUpdate"].as_str() == Some("agent_message_chunk")
                        && let Some(text) = update["content"]["text"].as_str()
                        && !text.is_empty()
                    {
                        full_reply.push_str(text);
                        if send_event(tx, "delta", &json!({ "content": text }))
                            .await
                            .is_err()
                        {
                            return StreamOutcome::ClientGone;
                        }
                    }
                }
                "fs/read_text_file" | "fs/write_text_file" => {
                    let request_id = message["id"].clone();
                    if request_id.is_null() {
                        continue;
                    }
                    let Some(root) = workspace_directory else {
                        let _ = client
                            .respond_error(&request_id, -32001, "当前工作区没有可用文件根目录")
                            .await;
                        continue;
                    };
                    let session_id = message["params"]["sessionId"].as_str();
                    if session_id != Some(session.as_str()) {
                        let _ = client
                            .respond_error(&request_id, -32602, "sessionId 与当前会话不匹配")
                            .await;
                        continue;
                    }
                    let result = if method == "fs/read_text_file" {
                        acp_read_text_file(
                            root,
                            message["params"]["path"].as_str().unwrap_or_default(),
                            message["params"]["startLine"].as_u64(),
                            message["params"]["numLines"].as_u64(),
                        )
                        .await
                    } else {
                        acp_write_text_file(
                            state,
                            &request.server_id,
                            root,
                            message["params"]["path"].as_str().unwrap_or_default(),
                            message["params"]["content"].as_str().unwrap_or_default(),
                        )
                        .await
                    };
                    match result {
                        Ok(result) => {
                            if client.respond(&request_id, result).await.is_err() {
                                return mid_fail("文件操作响应失败".into(), !full_reply.is_empty());
                            }
                        }
                        Err(error) => {
                            if client
                                .respond_error(&request_id, -32001, &error)
                                .await
                                .is_err()
                            {
                                return mid_fail(
                                    "文件操作错误响应失败".into(),
                                    !full_reply.is_empty(),
                                );
                            }
                        }
                    }
                }
                "session/request_permission" => {
                    let request_id = message["id"].clone();
                    if request_id.is_null() {
                        continue;
                    }
                    let options = message["params"]["options"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let outcome = acp_permission_outcome(review_mode, &options);
                    if client.respond(&request_id, outcome).await.is_err() {
                        return mid_fail("权限应答失败".into(), !full_reply.is_empty());
                    }
                }
                _ => {
                    // 未实现的反向请求拒绝，但不会中断 ACP 会话。
                    let request_id = message["id"].clone();
                    if !request_id.is_null() {
                        let _ = client
                            .respond_error(&request_id, -32601, "method not supported")
                            .await;
                    }
                }
            }
            continue;
        }
        if message["id"].as_i64() == Some(prompt_id) {
            if let Some(error) = message.get("error") {
                let reason = error["message"]
                    .as_str()
                    .unwrap_or("Agent 返回错误")
                    .to_string();
                return mid_fail(reason, !full_reply.is_empty());
            }
            return StreamOutcome::Completed;
        }
    }
}

async fn stream_upstream(
    provider: &AiProvider,
    model: &str,
    reasoning_effort: Option<&str>,
    request: &ChatStreamRequest,
    server_context: &str,
    language: &str,
    persona: &str,
    skill_context: &str,
    plugin_context: &str,
    tx: &mpsc::Sender<Event>,
    full_reply: &mut String,
) -> StreamOutcome {
    let mut system = format!(
        "你是 Sculk Agent，一款 AI 驱动的 Minecraft 服务器管理工作台助手。当前工作区服务器：{server_context}。{language}聚焦服务器运维、插件、配置与社区运营，回答保持简洁、可执行。"
    );
    system.push_str("\n服务器级变更由工作台的受审计任务执行器处理。不要声称当前会话没有执行器或承诺“恢复后自动执行”；未收到明确创建指令时只给出方案，收到任务状态前也不要宣称已下载、写入或启动成功。基础开服任务只负责核心、Java、工作区、EULA 与启动验证，不能根据模糊的 RPG 需求猜测并安装插件。");
    if !persona.is_empty() {
        system.push('\n');
        system.push_str(persona);
    }
    if !skill_context.is_empty() {
        system.push_str("\n\n");
        system.push_str(skill_context);
    }
    if !plugin_context.is_empty() {
        system.push_str("\n插件资源库候选（已按主流 > 开源 > 普通 > 付费排序）：\n");
        system.push_str(plugin_context);
        system.push_str("\n推荐插件时保持该优先级，并说明兼容性、开源或付费属性。\n");
    }
    // 从最新往前保留 history，总字符不超过上限，且只接受 user/assistant 角色。
    let mut kept: Vec<&ChatTurn> = Vec::new();
    let mut total = 0usize;
    for turn in request.history.iter().rev() {
        if turn.role != "user" && turn.role != "assistant" {
            continue;
        }
        total += turn.content.chars().count();
        if total > HISTORY_CHAR_LIMIT {
            break;
        }
        kept.push(turn);
    }
    kept.reverse();
    let mut messages = vec![json!({ "role": "system", "content": system })];
    for turn in kept {
        messages.push(json!({ "role": turn.role, "content": turn.content }));
    }
    messages.push(json!({ "role": "user", "content": request.message }));
    let mut body = json!({ "model": model, "messages": messages, "stream": true });
    if let Some(effort) = reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }

    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
    {
        Ok(client) => client,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error.to_string()),
    };
    let url = upstream_url(&provider.base_url, "/chat/completions");
    let mut response = match authed(client.post(&url), &provider.api_key)
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error.to_string()),
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return StreamOutcome::FailedBeforeOutput(format!(
            "HTTP {}：{}",
            status.as_u16(),
            snippet(&body, 200)
        ));
    }
    let meta = json!({
        "provider": provider.name,
        "model": model,
        "fallback": false,
        "reasoning_effort": reasoning_effort,
    });
    if send_event(tx, "meta", &meta).await.is_err() {
        return StreamOutcome::ClientGone;
    }

    let mid_fail = |reason: String, produced: bool| {
        if produced {
            StreamOutcome::FailedMidway(reason)
        } else {
            StreamOutcome::FailedBeforeOutput(reason)
        }
    };
    // 字节缓冲避免 chunk 截断 UTF-8 序列或 SSE 行。
    let mut buffer: Vec<u8> = Vec::new();
    let mut finished = false;
    while !finished {
        let chunk = match tokio::time::timeout(Duration::from_secs(60), response.chunk()).await {
            Err(_) => return mid_fail("上游响应超时".into(), !full_reply.is_empty()),
            Ok(Err(error)) => {
                return mid_fail(format!("上游连接中断：{error}"), !full_reply.is_empty());
            }
            Ok(Ok(None)) => break,
            Ok(Ok(Some(bytes))) => bytes,
        };
        buffer.extend_from_slice(&chunk);
        while let Some(position) = buffer.iter().position(|&byte| byte == b'\n') {
            let line_bytes: Vec<u8> = buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            let line = line.trim();
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                finished = true;
                break;
            }
            let Ok(value) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            let Some(content) = value["choices"][0]["delta"]["content"].as_str() else {
                continue;
            };
            if content.is_empty() {
                continue;
            }
            full_reply.push_str(content);
            if send_event(tx, "delta", &json!({ "content": content }))
                .await
                .is_err()
            {
                return StreamOutcome::ClientGone;
            }
        }
    }
    StreamOutcome::Completed
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, duplex};

    fn cli_output_context(
        limit: usize,
    ) -> (
        Arc<CliOutputBudget>,
        watch::Sender<bool>,
        watch::Receiver<bool>,
    ) {
        let (tx, rx) = watch::channel(false);
        (Arc::new(CliOutputBudget::new(limit)), tx, rx)
    }

    fn native_agent(kind: &str) -> AiAgent {
        AiAgent {
            id: "agent-test".into(),
            name: "Test CLI".into(),
            kind: kind.into(),
            command: kind.into(),
            args: Vec::new(),
            transport: "cli".into(),
            reasoning_effort: None,
            enabled: true,
        }
    }

    fn chat_request(message: String) -> ChatStreamRequest {
        ChatStreamRequest {
            server_id: "workspace-1".into(),
            message,
            history: Vec::new(),
            model_override: None,
            agent_override: None,
            conversation_id: None,
            reasoning_effort: None,
        }
    }

    fn settings_with_enabled_targets() -> AiSettings {
        let mut settings = AiSettings::default();
        settings.providers.push(AiProvider {
            id: "provider-1".into(),
            name: "Provider".into(),
            base_url: "https://example.com".into(),
            api_key: String::new(),
            enabled: true,
            models: vec![AiModel {
                id: "model-1".into(),
                enabled: true,
            }],
            models_synced_at: None,
        });
        let mut agent = native_agent("codex");
        agent.id = "agent-1".into();
        settings.agents.push(agent);
        settings
    }

    #[test]
    fn legacy_agent_state_defaults_to_acp_without_reasoning() {
        let agent: AiAgent = serde_json::from_value(json!({
            "id": "legacy",
            "name": "Legacy ACP",
            "kind": "codex",
            "command": "npx",
            "args": ["codex-acp"],
            "enabled": true
        }))
        .unwrap();
        assert_eq!(agent.transport, "acp");
        assert_eq!(agent.reasoning_effort, None);
    }

    #[test]
    fn legacy_model_binding_has_no_reasoning_override() {
        let binding: ModelBinding = serde_json::from_value(json!({
            "provider_id": "openai",
            "model_id": "gpt-5"
        }))
        .unwrap();
        assert_eq!(binding.reasoning_effort, None);
    }

    #[test]
    fn legacy_ai_settings_default_to_browser_speech_recognition() {
        let settings: AiSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(settings.speech_recognition.mode, "browser");
        assert_eq!(settings.speech_recognition.language, "zh-CN");
        assert_eq!(settings.speech_recognition.model_id, "whisper-1");
        assert!(settings.speech_recognition.provider_id.is_none());
    }

    #[test]
    fn speech_recognition_settings_validate_model_provider_without_affecting_browser_mode() {
        let settings = settings_with_enabled_targets();
        let browser = SpeechRecognitionSettings {
            provider_id: Some("missing".into()),
            ..SpeechRecognitionSettings::default()
        };
        assert!(validate_speech_recognition_settings(&browser, &settings.providers).is_ok());

        let model = SpeechRecognitionSettings {
            mode: "model".into(),
            provider_id: Some("provider-1".into()),
            model_id: "whisper-1".into(),
            language: "zh-CN".into(),
        };
        assert!(validate_speech_recognition_settings(&model, &settings.providers).is_ok());
        let missing = SpeechRecognitionSettings {
            provider_id: Some("missing".into()),
            ..model.clone()
        };
        assert!(validate_speech_recognition_settings(&missing, &settings.providers).is_err());
    }

    #[test]
    fn transcription_language_and_payload_are_normalized() {
        assert_eq!(
            upstream_transcription_language("zh-CN").as_deref(),
            Some("zh")
        );
        assert_eq!(
            upstream_transcription_language("EN-us").as_deref(),
            Some("en")
        );
        assert_eq!(upstream_transcription_language("auto"), None);
        assert_eq!(
            transcription_text(&json!({"text": "  你好世界  "})).as_deref(),
            Some("你好世界")
        );
        assert!(transcription_text(&json!({"text": "   "})).is_none());
    }

    #[test]
    fn acp_paths_are_confined_to_the_workspace_root() {
        let root = std::env::temp_dir().join("sculk-acp-workspace");
        assert_eq!(
            acp_relative_path(&root, &root.join("src/main.rs").to_string_lossy()).unwrap(),
            PathBuf::from("src/main.rs")
        );
        assert_eq!(
            acp_relative_path(&root, "src/main.rs").unwrap(),
            PathBuf::from("src/main.rs")
        );
        assert!(acp_relative_path(&root, &root.join("../outside.rs").to_string_lossy()).is_err());
        assert!(acp_relative_path(&root, "../outside.rs").is_err());
    }

    #[test]
    fn acp_permissions_are_fail_closed_for_every_review_mode() {
        let options = vec![
            json!({"optionId": "allow-once", "kind": "allow_once"}),
            json!({"optionId": "allow-always", "kind": "allow_always"}),
            json!({"optionId": "reject", "kind": "reject_once"}),
        ];

        for review_mode in ["approval", "auto", "full"] {
            let outcome = acp_permission_outcome(review_mode, &options);
            assert_eq!(outcome["outcome"]["outcome"], "cancelled");
            assert!(outcome["outcome"].get("optionId").is_none());
        }
    }

    #[tokio::test]
    async fn acp_reads_utf8_text_with_line_ranges() {
        let root = std::env::temp_dir().join(format!("sculk-acp-read-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::write(root.join("notes.txt"), "one\ntwo\nthree\n")
            .await
            .unwrap();
        let result = acp_read_text_file(&root, "notes.txt", Some(2), Some(1))
            .await
            .unwrap();
        assert_eq!(result["content"], "two");
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[test]
    fn skill_query_carries_recent_server_context_into_follow_up_messages() {
        let request = ChatStreamRequest {
            server_id: "server-1".into(),
            message: "按上面的方案继续配置".into(),
            history: vec![
                ChatTurn {
                    role: "assistant".into(),
                    content: "请确认版本".into(),
                },
                ChatTurn {
                    role: "user".into(),
                    content: "我想要 6 人的插件生电服，优先原版机制".into(),
                },
            ],
            model_override: None,
            agent_override: None,
            conversation_id: None,
            reasoning_effort: None,
        };
        let query = skill_query_for_request(&request, true);
        assert!(query.starts_with("当前处于服务器规划模式。"));
        assert!(query.contains("按上面的方案继续配置"));
        assert!(query.contains("插件生电服"));
        assert!(!query.contains("请确认版本"));
    }

    #[test]
    fn persisted_task_facts_override_stale_chat_progress_text() {
        let mut state = crate::initial_state();
        let server: crate::ServerInfo = serde_json::from_value(json!({
            "id": "server-1",
            "kind": "server",
            "name": "生存服",
            "core": "Paper",
            "version": "1.21.4",
            "status": "online",
            "players": "0 / 20",
            "memory": 0,
            "memory_gb": 8,
            "cpu": 0,
            "port": 25565,
            "task": "运行中",
            "operation_state": "idle",
            "core_ready": true
        }))
        .unwrap();
        state.servers.push(server.clone());
        let mut task = crate::new_task_record(
            "server-1".into(),
            "准备并启动服务器".into(),
            "server_bootstrap".into(),
            "completed".into(),
            100,
            "medium".into(),
            Some("auto".into()),
        );
        task.summary = Some("服务器已启动并通过就绪标记确认。".into());
        state.tasks.push(task.clone());
        let conversation = crate::conversations::new_conversation("server-1", None, None);
        let conversation_id = conversation.id.clone();
        state.conversations.push(conversation);
        assert!(crate::conversations::append_exchange(
            &mut state,
            "server-1",
            &conversation_id,
            "开始创建服务器",
            "正在创建任务，尚未确认结果。",
            vec![],
            Some(task.id),
        ));

        let latest = latest_task_for_chat(&state, "server-1", Some(&conversation_id)).unwrap();
        let context = workspace_runtime_context(Some(&server), Some(latest), "server-1");
        let reply = task_follow_up_reply(latest, Some(&server));
        assert_eq!(latest.id, task.id);
        assert!(context.contains("状态=completed"));
        assert!(context.contains("服务器已启动并通过就绪标记确认"));
        assert!(context.contains("旧对话中的进行中文案不能覆盖它"));
        assert!(reply.contains("已经完成"));
        assert!(reply.contains("当前服务器“生存服”在线"));
        assert!(is_task_follow_up("继续"));
        assert!(is_task_follow_up("完成了吗？"));
    }

    #[test]
    fn active_bootstrap_is_reused_instead_of_duplicated() {
        let mut state = crate::initial_state();
        let task = crate::new_task_record(
            "server-1".into(),
            "准备并启动服务器".into(),
            "server_bootstrap".into(),
            "running".into(),
            42,
            "medium".into(),
            Some("auto".into()),
        );
        let id = task.id;
        state.tasks.push(task);
        assert_eq!(active_bootstrap_task(&state, "server-1").unwrap().id, id);
    }

    #[test]
    fn chat_payload_rejects_empty_oversized_and_excessive_history() {
        assert!(validate_chat_payload(&chat_request("   ".into())).is_err());
        assert!(
            validate_chat_payload(&chat_request("x".repeat(CHAT_MESSAGE_CHAR_LIMIT + 1))).is_err()
        );
        let mut request = chat_request("ok".into());
        request.history = (0..=CHAT_HISTORY_TURN_LIMIT)
            .map(|_| ChatTurn {
                role: "user".into(),
                content: "x".into(),
            })
            .collect();
        assert!(validate_chat_payload(&request).is_err());
    }

    #[test]
    fn chat_execution_targets_must_exist_and_be_enabled() {
        let settings = settings_with_enabled_targets();
        let valid = ModelBinding {
            provider_id: "provider-1".into(),
            model_id: "model-1".into(),
            reasoning_effort: Some("high".into()),
        };
        assert!(validate_chat_execution_targets(&settings, Some(&valid), Some("agent-1")).is_ok());
        assert!(validate_chat_execution_targets(&settings, Some(&valid), Some("default")).is_ok());

        let missing_model = ModelBinding {
            model_id: "missing".into(),
            ..valid.clone()
        };
        assert!(validate_chat_execution_targets(&settings, Some(&missing_model), None).is_err());
        assert!(validate_chat_execution_targets(&settings, None, Some("missing-agent")).is_err());
    }

    #[test]
    fn codex_invocation_uses_official_exec_config_order_and_stdin() {
        let invocation =
            cli_invocation(&native_agent("codex"), Some("high"), "approval", false).unwrap();
        assert_eq!(
            invocation.args,
            [
                "--ask-for-approval",
                "never",
                "exec",
                "-c",
                "model_reasoning_effort='high'",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--color",
                "never",
                "-"
            ]
        );
        assert!(!invocation.codex_full_access);
    }

    #[test]
    fn codex_full_access_uses_the_explicit_full_profile_without_the_bypass_flag() {
        let invocation =
            cli_invocation(&native_agent("codex"), Some("high"), "full", true).unwrap();
        assert_eq!(
            invocation.args,
            [
                "--ask-for-approval",
                "never",
                "--search",
                "exec",
                "-c",
                "model_reasoning_effort='high'",
                "--sandbox",
                "danger-full-access",
                "--skip-git-repo-check",
                "--ephemeral",
                "--color",
                "never",
                "-"
            ]
        );
        assert!(invocation.codex_full_access);
        assert!(
            !invocation
                .args
                .iter()
                .any(|arg| arg == "--dangerously-bypass-approvals-and-sandbox")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_batch_invocation_keeps_the_shell_surface_runtime_owned() {
        let args = vec![
            "--ask-for-approval".into(),
            "never".into(),
            "exec".into(),
            "--sandbox".into(),
            "danger-full-access".into(),
            "-".into(),
        ];
        assert_eq!(
            windows_batch_invocation(FsPath::new(r"C:\Codex Tools\codex.cmd"), &args).unwrap(),
            r#" /d /s /c ""C:\Codex Tools\codex.cmd" --ask-for-approval never exec --sandbox danger-full-access -""#
        );
        assert!(
            windows_batch_invocation(FsPath::new(r"C:\Codex Tools\codex.cmd"), &["&".into()])
                .is_err()
        );
        assert_eq!(
            windows_batch_invocation(
                FsPath::new(r"\\?\C:\Codex Tools\codex.cmd"),
                &[
                    "-c".into(),
                    "model_reasoning_effort='high'".into(),
                    "--sandbox".into(),
                    "danger-full-access".into(),
                ],
            )
            .unwrap(),
            r#" /d /s /c ""C:\Codex Tools\codex.cmd" -c model_reasoning_effort='high' --sandbox danger-full-access""#
        );
        assert_eq!(
            windows_cmd_compatible_batch_path(FsPath::new(r"\\?\UNC\server\share\codex.cmd"))
                .unwrap(),
            PathBuf::from(r"\\server\share\codex.cmd")
        );
        assert!(
            windows_cmd_compatible_batch_path(FsPath::new(r"\\?\Volume{abc}\codex.cmd")).is_err()
        );
        assert_eq!(
            windows_cli_working_directory(FsPath::new(r"\\?\C:\workspace\project")).unwrap(),
            PathBuf::from(r"C:\workspace\project")
        );
        assert!(
            windows_cli_working_directory(FsPath::new(r"\\?\UNC\server\share\project")).is_err()
        );
        assert!(windows_cli_working_directory(FsPath::new(r"\\server\share\project")).is_err());
        assert!(windows_cli_working_directory(FsPath::new(r"\\?\Volume{abc}\project")).is_err());
    }

    #[test]
    fn codex_full_access_requires_the_server_environment_gate() {
        let error = cli_invocation(&native_agent("codex"), None, "full", false)
            .err()
            .unwrap();
        assert!(error.contains("SCULK_ALLOW_CODEX_FULL=true"));
        assert_eq!(
            codex_permission_profile("auto", false).unwrap(),
            CodexPermissionProfile::ReadOnly
        );
        assert!(parse_opt_in_value(" true "));
        assert!(!parse_opt_in_value("enabled"));
    }

    #[test]
    fn full_access_only_allows_native_codex_cli() {
        let codex = native_agent("codex");
        assert!(validate_full_access_agent(&codex, "full").is_ok());

        let mut codex_acp = codex.clone();
        codex_acp.transport = "acp".into();
        assert!(validate_full_access_agent(&codex_acp, "full").is_err());
        assert!(validate_full_access_agent(&native_agent("claude-code"), "full").is_err());
        assert!(validate_full_access_agent(&codex_acp, "approval").is_ok());
    }

    #[test]
    fn codex_cli_rejects_uncontrolled_custom_arguments() {
        let mut agent = native_agent("codex");
        agent.args = vec!["--dangerously-bypass-approvals-and-sandbox".into()];
        let error = cli_invocation(&agent, None, "approval", false)
            .err()
            .unwrap();
        assert!(error.contains("不能自定义"));
    }

    #[test]
    fn claude_invocation_uses_print_mode_and_effort_flag() {
        let invocation =
            cli_invocation(&native_agent("claude-code"), Some("max"), "approval", false).unwrap();
        assert_eq!(
            invocation.args,
            ["-p", "--output-format", "text", "--effort", "max"]
        );
    }

    #[test]
    fn target_specific_effort_is_rejected() {
        let error = cli_invocation(&native_agent("codex"), Some("max"), "approval", false)
            .err()
            .unwrap();
        assert!(error.contains("不支持 reasoning_effort=max"));
    }

    #[test]
    fn native_cli_cwd_follows_workspace_kind() {
        assert!(
            workspace_directory("alpha", "project")
                .ends_with(std::path::Path::new("projects").join("alpha"))
        );
        assert!(
            workspace_directory("beta", "server")
                .ends_with(std::path::Path::new("servers").join("beta"))
        );
    }

    #[test]
    fn error_redaction_removes_credentials_and_limits_output() {
        let secret = "sk-secret-value".to_string();
        let text = format!("request failed with {secret} {}", "x".repeat(2_000));
        let redacted = redact_secrets(text, std::slice::from_ref(&secret));
        assert!(!redacted.contains(&secret));
        assert!(redacted.contains("[REDACTED]"));
        assert!(redacted.chars().count() <= 1_001);
    }

    #[test]
    fn cli_process_environment_is_an_explicit_allowlist() {
        let mut command = Command::new("codex");
        configure_cli_environment(&mut command, false);
        let configured = command
            .as_std()
            .get_envs()
            .map(|(name, value)| (name.to_string_lossy().into_owned(), value))
            .collect::<Vec<_>>();

        assert!(configured.iter().all(
            |(name, value)| value.is_some() && is_cli_environment_variable_allowed(name, false)
        ));
        assert!(is_cli_environment_variable_allowed("Path", false));
        assert!(is_cli_environment_variable_allowed("SystemRoot", false));
        assert!(is_cli_environment_variable_allowed("USERPROFILE", false));
        assert!(!is_cli_environment_variable_allowed(
            "OPENAI_API_KEY",
            false
        ));
        assert!(!is_cli_environment_variable_allowed("CODEX_HOME", false));
        assert!(is_cli_environment_variable_allowed("CODEX_HOME", true));
        assert!(is_sensitive_cli_environment_variable("OPENAI_API_KEY"));
        assert!(is_sensitive_cli_environment_variable("SCULK_CLOUD_DB_URL"));
        assert!(is_sensitive_cli_environment_variable("REDIS_URL"));
        assert!(is_sensitive_cli_environment_variable("JWT_SIGNING_SECRET"));
        assert!(!is_sensitive_cli_environment_variable("PATH"));
    }

    #[test]
    fn cli_stdout_and_stderr_use_the_same_secret_redaction() {
        let secret = "sk-secret-value".to_string();
        let mut redactor = CliOutputRedactor::new(std::slice::from_ref(&secret));
        let mut stdout = redactor.push_bytes(b"0123456789abcdefsk-sec");
        assert_eq!(stdout, "01234567");
        stdout.push_str(&redactor.push_bytes(b"ret-value suffix"));
        stdout.push_str(&redactor.finish());
        let stderr = sanitized_cli_stderr(
            b"CLI failed with sk-secret-value in stderr",
            std::slice::from_ref(&secret),
        );

        assert!(!stdout.contains(&secret));
        assert!(stdout.contains("[REDACTED]"));
        assert!(!stderr.contains(&secret));
        assert!(stderr.contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn cli_output_preserves_utf8_lines() {
        let (mut writer, reader) = duplex(128);
        let writer_task = tokio::spawn(async move {
            writer
                .write_all("你好，世界\nsecond line".as_bytes())
                .await
                .unwrap();
            writer.shutdown().await.unwrap();
        });
        let (tx, mut rx) = mpsc::channel(8);
        let mut reply = String::new();
        let (budget, _limit_tx, limit_rx) = cli_output_context(CLI_MAX_TOTAL_OUTPUT_BYTES);
        let outcome = read_cli_output(
            reader,
            &tx,
            &mut reply,
            &[],
            budget,
            limit_rx,
            Duration::from_secs(1),
        )
        .await;
        writer_task.await.unwrap();
        assert!(matches!(outcome, CliOutputRead::Eof));
        assert_eq!(reply, "你好，世界\nsecond line");
        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn cli_output_stops_when_sse_client_is_gone() {
        let (_writer, reader) = duplex(16);
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let mut reply = String::new();
        let (budget, _limit_tx, limit_rx) = cli_output_context(CLI_MAX_TOTAL_OUTPUT_BYTES);
        let outcome = read_cli_output(
            reader,
            &tx,
            &mut reply,
            &[],
            budget,
            limit_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(matches!(outcome, CliOutputRead::ClientGone));
    }

    #[tokio::test]
    async fn cli_output_has_an_idle_timeout() {
        let (_writer, reader) = duplex(16);
        let (tx, _rx) = mpsc::channel(1);
        let mut reply = String::new();
        let (budget, _limit_tx, limit_rx) = cli_output_context(CLI_MAX_TOTAL_OUTPUT_BYTES);
        let outcome = read_cli_output(
            reader,
            &tx,
            &mut reply,
            &[],
            budget,
            limit_rx,
            Duration::from_millis(10),
        )
        .await;
        assert!(matches!(outcome, CliOutputRead::TimedOut(_)));
    }

    #[tokio::test]
    async fn cli_output_limit_stops_streaming_and_bounds_reply() {
        let (mut writer, reader) = duplex(64);
        let writer_task = tokio::spawn(async move {
            writer.write_all(b"0123456789abcdef").await.unwrap();
            writer.shutdown().await.unwrap();
        });
        let (tx, _rx) = mpsc::channel(8);
        let mut reply = String::new();
        let (budget, _limit_tx, limit_rx) = cli_output_context(8);
        let outcome = read_cli_output(
            reader,
            &tx,
            &mut reply,
            &[],
            budget,
            limit_rx,
            Duration::from_secs(1),
        )
        .await;
        writer_task.await.unwrap();

        assert!(matches!(outcome, CliOutputRead::OutputLimit));
        assert!(reply.len() <= 8);
    }

    #[tokio::test]
    async fn cli_stderr_uses_the_shared_output_budget() {
        let (mut writer, reader) = duplex(64);
        let writer_task = tokio::spawn(async move {
            writer.write_all(b"too-much-stderr").await.unwrap();
            writer.shutdown().await.unwrap();
        });
        let (budget, limit_tx, mut limit_rx) = cli_output_context(8);
        let stderr = read_cli_stderr(reader, budget, limit_tx).await;
        writer_task.await.unwrap();

        assert!(stderr.output_limit_exceeded);
        assert!(stderr.output.len() <= 8);
        limit_rx.changed().await.unwrap();
        assert!(*limit_rx.borrow());
    }
}
