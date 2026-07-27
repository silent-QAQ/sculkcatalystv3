use crate::acp::AcpClient;
use crate::{
    AppState, TaskInfo, classify_intent, effective_task_start, intent_risk, internal, persist,
    rule_reply, task_title,
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
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
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
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AiAgent {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
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
        }
    }
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
}

impl From<&AiSettings> for AiSettingsView {
    fn from(settings: &AiSettings) -> Self {
        Self {
            providers: settings.providers.iter().map(Into::into).collect(),
            scenarios: settings.scenarios.clone(),
            default_binding: settings.default_binding.clone(),
            review_mode: settings.review_mode.clone(),
            agents: settings.agents.clone(),
            active_agent: settings.active_agent.clone(),
        }
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
struct UpsertAgentRequest {
    name: String,
    kind: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
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
) -> Option<(AiProvider, String)> {
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
            return Some((provider.clone(), binding.model_id.clone()));
        }
    }
    None
}

async fn get_settings(State(state): State<AppState>) -> Json<AiSettingsView> {
    let data = state.inner.read().await;
    Json((&data.ai).into())
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
    let body = json!({
        "model": request.model_id,
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 16,
        "stream": false,
    });
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

fn validate_agent_request(request: &UpsertAgentRequest) -> Result<(String, String), ApiError> {
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
    Ok((name, command))
}

async fn create_agent(
    State(state): State<AppState>,
    Json(request): Json<UpsertAgentRequest>,
) -> ApiResult<AiAgent> {
    let (name, command) = validate_agent_request(&request)?;
    let agent = AiAgent {
        id: format!("agent-{}", &Uuid::new_v4().simple().to_string()[..8]),
        name,
        kind: request.kind,
        command,
        args: request.args,
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
    let (name, command) = validate_agent_request(&request)?;
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
    let outcome = acp_handshake(&agent).await;
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
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<Event>(64);
    tokio::spawn(run_chat_stream(state, request, tx));
    Sse::new(ReceiverStream::new(rx).map(Ok::<Event, Infallible>))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn run_chat_stream(state: AppState, request: ChatStreamRequest, tx: mpsc::Sender<Event>) {
    let (settings, server_context, language, persona, is_planning) = {
        let data = state.inner.read().await;
        let server = data
            .servers
            .iter()
            .find(|server| server.id == request.server_id);
        let is_planning = server.map(|s| s.status == "planning").unwrap_or(false);
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
            crate::prefs::language_directive(&data.ui.language).to_string(),
            crate::prefs::persona_directive(&data.ui.personalization),
            is_planning,
        )
    };
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
        match stream_acp(
            &agent,
            &request,
            &server_context,
            &settings.review_mode,
            &language,
            &persona,
            &tx,
            &mut full_reply,
        )
        .await
        {
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

    if !handled && let Some((provider, model)) = resolved {
        match stream_upstream(
            &provider,
            &model,
            &request,
            &server_context,
            &language,
            &persona,
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
        let text = format!(
            "{}\n\n目标服务器：{}",
            rule_reply(classify_intent(&request.message)),
            server_context
        );
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
        if let Some(conversation_id) = request.conversation_id.as_deref()
            && !crate::conversations::append_exchange(
                &mut data,
                &request.server_id,
                conversation_id,
                &request.message,
                &full_reply,
                vec!["审阅执行计划".into(), "在镜像服运行".into()],
            )
        {
            eprintln!("[ai] conversation {conversation_id} 不存在，消息未持久化");
        }
        if let Err(error) = persist(&state, &data).await {
            eprintln!("[ai] 任务持久化失败：{error}");
        }
        task
    };
    let done = json!({
        "id": Uuid::new_v4(),
        "time": Local::now().format("%H:%M").to_string(),
        "actions": ["审阅执行计划", "在镜像服运行"],
        "task": task,
        "conversation_id": request.conversation_id,
    });
    let _ = send_event(&tx, "done", &done).await;
}

/// 通过 ACP 协议驱动外部 Agent 完成一轮对话。
/// 权限请求按审核模式自动应答：full/auto 选择放行选项，approval 拒绝（工具型操作需在自动化面板批准）。
async fn stream_acp(
    agent: &AiAgent,
    request: &ChatStreamRequest,
    server_context: &str,
    review_mode: &str,
    language: &str,
    persona: &str,
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
    request: &ChatStreamRequest,
    server_context: &str,
    language: &str,
    persona: &str,
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
    let body = json!({ "model": model, "messages": messages, "stream": true });

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
    let meta = json!({ "provider": provider.name, "model": model, "fallback": false });
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
