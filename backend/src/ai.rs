use crate::acp::AcpClient;
use crate::cli_tools::{CLAUDE_EFFORTS, CODEX_EFFORTS, MODEL_EFFORTS};
use crate::{
    AppState, classify_intent, effective_task_start, intent_risk, internal, persist, rule_reply,
    task_title,
};
use axum::{
    Json, Router,
    extract::{Path, State},
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
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
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
        }
    }
}

fn default_agent_transport() -> String {
    "acp".into()
}

fn default_review_mode() -> String {
    "approval".into()
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
    agents: Vec<AiAgent>,
    active_agent: Option<String>,
    reasoning_effort: Option<String>,
    reasoning_effort_values: &'static [&'static str],
    detected_agents: Vec<crate::cli_tools::DetectedAgent>,
}

impl AiSettingsView {
    fn new(settings: &AiSettings, detected_agents: Vec<crate::cli_tools::DetectedAgent>) -> Self {
        Self {
            providers: settings.providers.iter().map(Into::into).collect(),
            scenarios: settings.scenarios.clone(),
            default_binding: settings.default_binding.clone(),
            review_mode: settings.review_mode.clone(),
            agents: settings.agents.clone(),
            active_agent: settings.active_agent.clone(),
            reasoning_effort: settings.reasoning_effort.clone(),
            reasoning_effort_values: MODEL_EFFORTS,
            detected_agents,
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

async fn run_chat_stream(state: AppState, request: ChatStreamRequest, tx: mpsc::Sender<Event>) {
    if let Err((_, message)) = validate_effort(request.reasoning_effort.as_deref(), MODEL_EFFORTS) {
        let _ = send_event(&tx, "error", &json!({ "message": message })).await;
        return;
    }
    let (settings, server_context, workspace_directory, language, persona, is_planning) = {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == request.server_id);
        let is_planning = server.map(|s| s.status == "planning").unwrap_or(false);
        let workspace_directory =
            server.map(|server| workspace_directory(&server.id, &server.kind));
        let mut context = server
            .map(|server| {
                format!(
                    "{}（{} {}，端口 {}，状态 {}）",
                    server.name, server.core, server.version, server.port, server.status
                )
            })
            .unwrap_or_else(|| request.server_id.clone());
        if is_planning {
            context
                .push_str("（规划中：尚未创建任何文件，需与用户确定服务端核心与版本后再执行创建）");
        }
        (
            data.ai.clone(),
            context,
            workspace_directory,
            crate::prefs::language_directive(&data.ui.language).to_string(),
            crate::prefs::persona_directive(&data.ui.personalization),
            is_planning,
        )
    };
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

    if let Some(agent) = agent_choice {
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
                &request,
                workspace_directory.as_ref(),
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
                &agent,
                &request,
                &server_context,
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
                eprintln!("[ai] Agent {} 调用失败，回退内置模型：{reason}", agent.name);
            }
            StreamOutcome::FailedMidway(reason) => {
                let _ = send_event(&tx, "error", &json!({ "message": reason })).await;
                handled = true;
                fallback = false;
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
            StreamOutcome::ClientGone => return,
        }
    }

    if fallback {
        let meta = json!({ "provider": Value::Null, "model": Value::Null, "fallback": true });
        if send_event(&tx, "meta", &meta).await.is_err() {
            return;
        }
        let text = if plugin_context.is_empty() {
            format!(
                "{}\n\n目标服务器：{}",
                rule_reply(classify_intent(&request.message)),
                server_context
            )
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

    let intent = classify_intent(&request.message);
    let risk = intent_risk(intent);
    let task = {
        let mut data = state.inner.write().await;
        let task = if crate::task_executor::is_executable_kind(intent) {
            let (status, progress, approved_by) = effective_task_start(risk, &data.ai.review_mode);
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
                message: "结构化服务器操作已从对话创建。".into(),
            });
            data.tasks.insert(0, task.clone());
            crate::trim_task_history(&mut data.tasks, 30);
            Some(task)
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
        task
    };
    if let Some(task) = task.as_ref()
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

/// 通过 ACP 协议驱动外部 Agent 完成一轮对话。
/// 权限请求按审核模式自动应答：full/auto 选择放行选项，approval 拒绝（工具型操作需在自动化面板批准）。
struct CliInvocation {
    args: Vec<String>,
}

fn cli_invocation(
    agent: &AiAgent,
    reasoning_effort: Option<&str>,
) -> Result<CliInvocation, String> {
    let mut args = agent.args.clone();
    match agent.kind.as_str() {
        "codex" => {
            args.push("exec".into());
            if let Some(effort) = reasoning_effort {
                validate_effort(Some(effort), CODEX_EFFORTS).map_err(|(_, error)| error)?;
                args.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]);
            }
            args.extend(["--color".into(), "never".into(), "-".into()]);
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
    Ok(CliInvocation { args })
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
    TimedOut(String),
    Failed(String),
}

async fn read_cli_output<R: AsyncRead + Unpin>(
    reader: R,
    tx: &mpsc::Sender<Event>,
    full_reply: &mut String,
    idle_timeout: Duration,
) -> CliOutputRead {
    let mut lines = BufReader::new(reader).lines();
    loop {
        let next = tokio::select! {
            _ = tx.closed() => return CliOutputRead::ClientGone,
            result = tokio::time::timeout(idle_timeout, lines.next_line()) => result,
        };
        let line = match next {
            Err(_) => {
                return CliOutputRead::TimedOut(format!(
                    "CLI 连续 {} 秒没有输出，已终止",
                    idle_timeout.as_secs()
                ));
            }
            Ok(Err(error)) => return CliOutputRead::Failed(error.to_string()),
            Ok(Ok(None)) => return CliOutputRead::Eof,
            Ok(Ok(Some(line))) => line,
        };
        let piece = if full_reply.is_empty() {
            line
        } else {
            format!("\n{line}")
        };
        full_reply.push_str(&piece);
        if send_event(tx, "delta", &json!({ "content": piece }))
            .await
            .is_err()
        {
            return CliOutputRead::ClientGone;
        }
    }
}

async fn read_limited<R: AsyncRead + Unpin>(mut reader: R, limit: usize) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 4096];
    while output.len() < limit {
        let remaining = (limit - output.len()).min(buffer.len());
        match reader.read(&mut buffer[..remaining]).await {
            Ok(0) | Err(_) => break,
            Ok(read) => output.extend_from_slice(&buffer[..read]),
        }
    }
    output
}

fn redact_secrets(mut text: String, secrets: &[String]) -> String {
    for secret in secrets.iter().filter(|secret| secret.len() >= 4) {
        text = text.replace(secret, "[REDACTED]");
    }
    snippet(text.trim(), 1_000)
}

fn sanitized_cli_stderr(stderr: &[u8]) -> String {
    let secrets = std::env::vars()
        .filter(|(name, value)| {
            !value.is_empty()
                && ["KEY", "TOKEN", "SECRET", "PASSWORD"]
                    .iter()
                    .any(|marker| name.to_ascii_uppercase().contains(marker))
        })
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    redact_secrets(String::from_utf8_lossy(stderr).into_owned(), &secrets)
}

fn hide_cli_window(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
}

async fn stream_cli_agent(
    agent: &AiAgent,
    reasoning_effort: Option<&str>,
    request: &ChatStreamRequest,
    workspace_directory: Option<&PathBuf>,
    server_context: &str,
    language: &str,
    persona: &str,
    skill_context: &str,
    plugin_context: &str,
    tx: &mpsc::Sender<Event>,
    full_reply: &mut String,
) -> StreamOutcome {
    let invocation = match cli_invocation(agent, reasoning_effort) {
        Ok(invocation) => invocation,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error),
    };
    let prompt = build_cli_prompt(
        request,
        server_context,
        language,
        persona,
        skill_context,
        plugin_context,
    );
    let mut command = Command::new(&agent.command);
    command
        .args(&invocation.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(directory) = workspace_directory.filter(|directory| directory.is_dir()) {
        command.current_dir(directory);
    }
    hide_cli_window(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return StreamOutcome::FailedBeforeOutput(error.to_string()),
    };
    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => return StreamOutcome::FailedBeforeOutput("CLI 标准输入不可用".into()),
    };
    match tokio::time::timeout(Duration::from_secs(10), stdin.write_all(prompt.as_bytes())).await {
        Err(_) => {
            let _ = child.kill().await;
            return StreamOutcome::FailedBeforeOutput("写入 CLI 提示词超时".into());
        }
        Ok(Err(error)) => {
            let _ = child.kill().await;
            return StreamOutcome::FailedBeforeOutput(format!("写入 CLI 提示词失败：{error}"));
        }
        Ok(Ok(())) => {}
    }
    if let Err(error) = stdin.shutdown().await {
        let _ = child.kill().await;
        return StreamOutcome::FailedBeforeOutput(format!("关闭 CLI 标准输入失败：{error}"));
    }
    drop(stdin);

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => return StreamOutcome::FailedBeforeOutput("CLI 标准输出不可用".into()),
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => return StreamOutcome::FailedBeforeOutput("CLI 标准错误不可用".into()),
    };
    let stderr_task = tokio::spawn(read_limited(stderr, 64 * 1024));
    let meta = json!({
        "provider": agent.name,
        "model": format!("CLI · {}", agent.kind),
        "fallback": false,
        "transport": "cli",
        "reasoning_effort": reasoning_effort,
    });
    if send_event(tx, "meta", &meta).await.is_err() {
        let _ = child.kill().await;
        stderr_task.abort();
        return StreamOutcome::ClientGone;
    }

    let read = match tokio::time::timeout(
        Duration::from_secs(30 * 60),
        read_cli_output(stdout, tx, full_reply, Duration::from_secs(120)),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(_) => CliOutputRead::TimedOut("CLI 总执行时间超过 30 分钟，已终止".into()),
    };
    match read {
        CliOutputRead::ClientGone => {
            let _ = child.kill().await;
            stderr_task.abort();
            return StreamOutcome::ClientGone;
        }
        CliOutputRead::TimedOut(message) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            stderr_task.abort();
            return if full_reply.is_empty() {
                StreamOutcome::FailedBeforeOutput(message)
            } else {
                StreamOutcome::FailedMidway(message)
            };
        }
        CliOutputRead::Failed(error) => {
            let _ = child.kill().await;
            stderr_task.abort();
            return if full_reply.is_empty() {
                StreamOutcome::FailedBeforeOutput(error)
            } else {
                StreamOutcome::FailedMidway(error)
            };
        }
        CliOutputRead::Eof => {}
    }
    let status = match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            stderr_task.abort();
            return StreamOutcome::FailedMidway(error.to_string());
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            stderr_task.abort();
            return StreamOutcome::FailedMidway("CLI 输出已关闭但进程未退出，已终止".into());
        }
    };
    let stderr = stderr_task.await.unwrap_or_default();
    if !status.success() {
        let error = sanitized_cli_stderr(&stderr);
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

async fn stream_acp(
    agent: &AiAgent,
    request: &ChatStreamRequest,
    server_context: &str,
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
        agent,
        request,
        server_context,
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
    agent: &AiAgent,
    request: &ChatStreamRequest,
    server_context: &str,
    review_mode: &str,
    language: &str,
    persona: &str,
    skill_context: &str,
    plugin_context: &str,
    tx: &mpsc::Sender<Event>,
    full_reply: &mut String,
) -> StreamOutcome {
    let handshake_timeout = Duration::from_secs(30);
    let init_id = match client
        .send_request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}},
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
    let cwd = std::env::current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".into());
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
                "session/request_permission" => {
                    let request_id = message["id"].clone();
                    if request_id.is_null() {
                        continue;
                    }
                    let options = message["params"]["options"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let pick = |want_allow: bool| {
                        options
                            .iter()
                            .find(|option| {
                                let kind = option["kind"].as_str().unwrap_or_default();
                                if want_allow {
                                    kind.starts_with("allow")
                                } else {
                                    kind.starts_with("reject")
                                }
                            })
                            .or(options.first())
                            .and_then(|option| option["optionId"].as_str())
                            .map(String::from)
                    };
                    let allow = review_mode != "approval";
                    let outcome = match pick(allow) {
                        Some(option_id) => {
                            json!({"outcome": {"outcome": "selected", "optionId": option_id}})
                        }
                        None => json!({"outcome": {"outcome": "cancelled"}}),
                    };
                    if client.respond(&request_id, outcome).await.is_err() {
                        return mid_fail("权限应答失败".into(), !full_reply.is_empty());
                    }
                }
                _ => {
                    // 其他反向请求（如 fs 读写）一律拒绝，通知直接忽略。
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
        let invocation = cli_invocation(&native_agent("codex"), Some("high")).unwrap();
        assert_eq!(
            invocation.args,
            [
                "exec",
                "-c",
                "model_reasoning_effort=\"high\"",
                "--color",
                "never",
                "-"
            ]
        );
    }

    #[test]
    fn claude_invocation_uses_print_mode_and_effort_flag() {
        let invocation = cli_invocation(&native_agent("claude-code"), Some("max")).unwrap();
        assert_eq!(
            invocation.args,
            ["-p", "--output-format", "text", "--effort", "max"]
        );
    }

    #[test]
    fn target_specific_effort_is_rejected() {
        let error = cli_invocation(&native_agent("codex"), Some("max"))
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
        let outcome = read_cli_output(reader, &tx, &mut reply, Duration::from_secs(1)).await;
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
        let outcome = read_cli_output(reader, &tx, &mut reply, Duration::from_secs(1)).await;
        assert!(matches!(outcome, CliOutputRead::ClientGone));
    }

    #[tokio::test]
    async fn cli_output_has_an_idle_timeout() {
        let (_writer, reader) = duplex(16);
        let (tx, _rx) = mpsc::channel(1);
        let mut reply = String::new();
        let outcome = read_cli_output(reader, &tx, &mut reply, Duration::from_millis(10)).await;
        assert!(matches!(outcome, CliOutputRead::TimedOut(_)));
    }
}
