use crate::{AppState, PersistedState, ai::ModelBinding, internal, persist};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

type ApiError = (StatusCode, String);
type ApiResult<T> = Result<Json<T>, ApiError>;

pub(crate) const MAX_MESSAGES: usize = 500;
const DEFAULT_TITLE: &str = "新对话";
const AUTO_TITLE_CHARS: usize = 20;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ChatMessage {
    pub(crate) id: String,
    pub(crate) role: String,
    pub(crate) content: String,
    pub(crate) time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) actions: Option<Vec<String>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Conversation {
    pub(crate) id: String,
    pub(crate) server_id: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) group: Option<String>,
    #[serde(default)]
    pub(crate) pinned: bool,
    #[serde(default)]
    pub(crate) archived: bool,
    #[serde(default)]
    pub(crate) unread: bool,
    #[serde(default)]
    pub(crate) model_binding: Option<ModelBinding>,
    #[serde(default)]
    pub(crate) agent_override: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    #[serde(default)]
    pub(crate) messages: Vec<ChatMessage>,
}

#[derive(Serialize)]
pub(crate) struct ConversationSummary {
    id: String,
    server_id: String,
    title: String,
    group: Option<String>,
    pinned: bool,
    archived: bool,
    unread: bool,
    model_binding: Option<ModelBinding>,
    agent_override: Option<String>,
    created_at: String,
    updated_at: String,
    message_count: usize,
}

impl From<&Conversation> for ConversationSummary {
    fn from(conversation: &Conversation) -> Self {
        Self {
            id: conversation.id.clone(),
            server_id: conversation.server_id.clone(),
            title: conversation.title.clone(),
            group: conversation.group.clone(),
            pinned: conversation.pinned,
            archived: conversation.archived,
            unread: conversation.unread,
            model_binding: conversation.model_binding.clone(),
            agent_override: conversation.agent_override.clone(),
            created_at: conversation.created_at.clone(),
            updated_at: conversation.updated_at.clone(),
            message_count: conversation.messages.len(),
        }
    }
}

#[derive(Deserialize)]
struct CreateConversationRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    group: Option<String>,
}

#[derive(Deserialize)]
struct UpdateConversationRequest {
    #[serde(default)]
    title: Option<String>,
    /// Some("") 约定为清除分组
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    pinned: Option<bool>,
    #[serde(default)]
    archived: Option<bool>,
    #[serde(default)]
    unread: Option<bool>,
}

#[derive(Deserialize)]
struct ConversationExecutionRequest {
    #[serde(default)]
    model_binding: Option<ModelBinding>,
    #[serde(default)]
    agent_override: Option<String>,
}

#[derive(Serialize)]
struct DeletedResponse {
    id: String,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/servers/{id}/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/api/conversations/{id}",
            get(get_conversation)
                .put(update_conversation)
                .delete(delete_conversation),
        )
        .route(
            "/api/conversations/{id}/execution",
            put(update_conversation_execution),
        )
        .route("/api/conversations/{id}/fork", post(fork_conversation))
}

fn now_rfc3339() -> String {
    Local::now().to_rfc3339()
}

fn now_hm() -> String {
    Local::now().format("%H:%M").to_string()
}

fn new_id() -> String {
    format!("conv-{}", &Uuid::new_v4().simple().to_string()[..8])
}

pub(crate) fn new_conversation(
    server_id: &str,
    title: Option<String>,
    group: Option<String>,
) -> Conversation {
    let now = now_rfc3339();
    Conversation {
        id: new_id(),
        server_id: server_id.to_string(),
        title: title
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_TITLE.into()),
        group: group.filter(|value| !value.trim().is_empty()),
        pinned: false,
        archived: false,
        unread: false,
        model_binding: None,
        agent_override: None,
        created_at: now.clone(),
        updated_at: now,
        messages: Vec::new(),
    }
}

pub(crate) fn assistant_message(content: &str, actions: Option<Vec<String>>) -> ChatMessage {
    ChatMessage {
        id: Uuid::new_v4().simple().to_string(),
        role: "assistant".into(),
        content: content.to_string(),
        time: now_hm(),
        actions,
    }
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let mut result: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        result.push('…');
    }
    result
}

/// 在已持有 write-lock 的 PersistedState 上追加一轮问答；返回 false 表示对话不存在。
pub(crate) fn append_exchange(
    data: &mut PersistedState,
    server_id: &str,
    conversation_id: &str,
    user_content: &str,
    assistant_content: &str,
    assistant_actions: Vec<String>,
) -> bool {
    let Some(conversation) = data.conversations.iter_mut().find(|conversation| {
        conversation.id == conversation_id && conversation.server_id == server_id
    }) else {
        return false;
    };
    conversation.messages.push(ChatMessage {
        id: Uuid::new_v4().simple().to_string(),
        role: "user".into(),
        content: user_content.to_string(),
        time: now_hm(),
        actions: None,
    });
    if !assistant_content.trim().is_empty() {
        conversation.messages.push(ChatMessage {
            id: Uuid::new_v4().simple().to_string(),
            role: "assistant".into(),
            content: assistant_content.to_string(),
            time: now_hm(),
            actions: if assistant_actions.is_empty() {
                None
            } else {
                Some(assistant_actions)
            },
        });
    }
    if conversation.messages.len() > MAX_MESSAGES {
        let excess = conversation.messages.len() - MAX_MESSAGES;
        conversation.messages.drain(..excess);
    }
    if conversation.title == DEFAULT_TITLE && !user_content.trim().is_empty() {
        conversation.title = truncate_chars(user_content.trim(), AUTO_TITLE_CHARS);
    }
    conversation.updated_at = now_rfc3339();
    true
}

fn forked(original: &Conversation) -> Conversation {
    let now = now_rfc3339();
    Conversation {
        id: new_id(),
        server_id: original.server_id.clone(),
        title: format!("{}（分叉）", original.title),
        group: original.group.clone(),
        pinned: false,
        archived: false,
        unread: false,
        model_binding: original.model_binding.clone(),
        agent_override: original.agent_override.clone(),
        created_at: now.clone(),
        updated_at: now,
        messages: original.messages.clone(),
    }
}

async fn list_conversations(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<Vec<ConversationSummary>> {
    let data = state.inner.read().await;
    if !data.servers.iter().any(|server| server.id == id) {
        return Err((StatusCode::NOT_FOUND, "server not found".into()));
    }
    Ok(Json(
        data.conversations
            .iter()
            .filter(|conversation| conversation.server_id == id)
            .map(Into::into)
            .collect(),
    ))
}

async fn create_conversation(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<CreateConversationRequest>,
) -> ApiResult<Conversation> {
    let mut data = state.inner.write().await;
    if !data.servers.iter().any(|server| server.id == id) {
        return Err((StatusCode::NOT_FOUND, "server not found".into()));
    }
    let conversation = new_conversation(&id, request.title, request.group);
    data.conversations.push(conversation.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(conversation))
}

async fn get_conversation(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<Conversation> {
    let data = state.inner.read().await;
    data.conversations
        .iter()
        .find(|conversation| conversation.id == id)
        .cloned()
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "conversation not found".into()))
}

async fn update_conversation(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<UpdateConversationRequest>,
) -> ApiResult<ConversationSummary> {
    let mut data = state.inner.write().await;
    let conversation = data
        .conversations
        .iter_mut()
        .find(|conversation| conversation.id == id)
        .ok_or((StatusCode::NOT_FOUND, "conversation not found".to_string()))?;
    let mut touched = false;
    if let Some(title) = request.title {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "标题不能为空".into()));
        }
        conversation.title = title;
        touched = true;
    }
    if let Some(group) = request.group {
        let group = group.trim().to_string();
        conversation.group = if group.is_empty() { None } else { Some(group) };
        touched = true;
    }
    if let Some(pinned) = request.pinned {
        conversation.pinned = pinned;
    }
    if let Some(archived) = request.archived {
        conversation.archived = archived;
    }
    if let Some(unread) = request.unread {
        conversation.unread = unread;
    }
    if touched {
        conversation.updated_at = now_rfc3339();
    }
    let summary: ConversationSummary = (&*conversation).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(summary))
}

async fn update_conversation_execution(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<ConversationExecutionRequest>,
) -> ApiResult<ConversationSummary> {
    let mut data = state.inner.write().await;
    if let Some(binding) = &request.model_binding {
        let valid = data.ai.providers.iter().any(|provider| {
            provider.id == binding.provider_id
                && provider.enabled
                && provider
                    .models
                    .iter()
                    .any(|model| model.id == binding.model_id && model.enabled)
        });
        if !valid {
            return Err((StatusCode::BAD_REQUEST, "模型未启用或不存在".into()));
        }
    }
    if let Some(agent_id) = request.agent_override.as_deref()
        && agent_id != "default"
        && !data
            .ai
            .agents
            .iter()
            .any(|agent| agent.id == agent_id && agent.enabled)
    {
        return Err((StatusCode::BAD_REQUEST, "Agent 未启用或不存在".into()));
    }
    let conversation = data
        .conversations
        .iter_mut()
        .find(|conversation| conversation.id == id)
        .ok_or((StatusCode::NOT_FOUND, "conversation not found".to_string()))?;
    conversation.model_binding = request.model_binding;
    conversation.agent_override = request.agent_override;
    let summary: ConversationSummary = (&*conversation).into();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(summary))
}

async fn delete_conversation(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<DeletedResponse> {
    let mut data = state.inner.write().await;
    let before = data.conversations.len();
    data.conversations
        .retain(|conversation| conversation.id != id);
    if data.conversations.len() == before {
        return Err((StatusCode::NOT_FOUND, "conversation not found".into()));
    }
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(DeletedResponse { id }))
}

async fn fork_conversation(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<Conversation> {
    let mut data = state.inner.write().await;
    let original = data
        .conversations
        .iter()
        .find(|conversation| conversation.id == id)
        .ok_or((StatusCode::NOT_FOUND, "conversation not found".to_string()))?;
    let copy = forked(original);
    data.conversations.push(copy.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(copy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(server_id: &str) -> Conversation {
        new_conversation(server_id, None, None)
    }

    #[test]
    fn conversation_without_optional_fields_deserializes() {
        let json = r#"{
            "id":"conv-1","server_id":"sculk","title":"旧对话",
            "created_at":"2026-01-01T00:00:00+08:00","updated_at":"2026-01-01T00:00:00+08:00"
        }"#;
        let conversation: Conversation = serde_json::from_str(json).unwrap();
        assert!(conversation.group.is_none());
        assert!(!conversation.pinned);
        assert!(conversation.messages.is_empty());
    }

    #[test]
    fn forked_copies_history_and_resets_flags() {
        let mut original = sample("sculk");
        original.title = "调优方案".into();
        original.pinned = true;
        original.unread = true;
        original.group = Some("运维".into());
        original.messages.push(assistant_message("hello", None));
        let copy = forked(&original);
        assert_eq!(copy.title, "调优方案（分叉）");
        assert_eq!(copy.messages.len(), 1);
        assert_eq!(copy.group.as_deref(), Some("运维"));
        assert!(!copy.pinned && !copy.unread && !copy.archived);
        assert_ne!(copy.id, original.id);
    }

    #[test]
    fn auto_title_uses_first_user_message_prefix() {
        let conversation = sample("sculk");
        assert_eq!(conversation.title, DEFAULT_TITLE);
        let long = "帮我把这台生存服的视距调到八并且优化实体刷新频率减少卡顿";
        let titled = truncate_chars(long, AUTO_TITLE_CHARS);
        assert_eq!(titled.chars().count(), AUTO_TITLE_CHARS + 1);
        assert!(titled.ends_with('…'));
    }

    #[test]
    fn append_truncates_to_max_messages() {
        let mut conversation = sample("sculk");
        for index in 0..(MAX_MESSAGES + 10) {
            conversation.messages.push(ChatMessage {
                id: index.to_string(),
                role: "user".into(),
                content: "x".into(),
                time: "00:00".into(),
                actions: None,
            });
        }
        if conversation.messages.len() > MAX_MESSAGES {
            let excess = conversation.messages.len() - MAX_MESSAGES;
            conversation.messages.drain(..excess);
        }
        assert_eq!(conversation.messages.len(), MAX_MESSAGES);
        assert_eq!(conversation.messages[0].id, "10");
    }
}
