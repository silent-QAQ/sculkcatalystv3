use crate::AppState;
use axum::{
    Json, Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::Response,
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    env,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{RwLock, mpsc, oneshot},
    time::timeout,
};
use uuid::Uuid;

pub(crate) const PROTOCOL_VERSION: u16 = 2;
const MAX_FRAME_BYTES: usize = 512 * 1024;
const MAX_PLAYERS_PER_BATCH: usize = 500;
const MAX_SNAPSHOTS_PER_SERVER: usize = 1_000;
const MAX_INVENTORY_SLOTS: usize = 256;
const MAX_CONTAINER_DEPTH: usize = 3;
const MAX_TOTAL_CONTAINER_ITEMS: usize = 512;
const MAX_PAPI_FIELDS: usize = 10;
const MAX_TEXT_BYTES: usize = 256;
const CLOCK_SKEW: Duration = Duration::from_secs(30);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const CHALLENGE_TTL: Duration = Duration::from_secs(10);
const SNAPSHOT_TTL: Duration = Duration::from_secs(5);
const SNAPSHOT_STALE_AFTER: Duration = Duration::from_secs(15);
const MAX_SEQUENCE_GAP: u64 = 100_000;
const WIRE_ENVELOPE_FIELDS: [&str; 10] = [
    "protocol_version",
    "type",
    "request_id",
    "server_id",
    "instance_id",
    "session_id",
    "seq",
    "sent_at",
    "payload_json",
    "signature",
];

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
pub(crate) struct BridgeEnvelope<T> {
    pub(crate) protocol_version: u16,
    pub(crate) message_type: String,
    pub(crate) request_id: Option<String>,
    pub(crate) server_id: String,
    pub(crate) instance_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) sequence: u64,
    pub(crate) sent_at: i64,
    pub(crate) payload: T,
    pub(crate) signature: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBridgeEnvelope {
    protocol_version: u16,
    #[serde(rename = "type")]
    message_type: String,
    request_id: Option<String>,
    server_id: String,
    instance_id: String,
    session_id: Option<String>,
    #[serde(rename = "seq")]
    sequence: u64,
    #[serde(rename = "sent_at")]
    sent_at: i64,
    payload_json: String,
    signature: Option<String>,
}

#[derive(Clone, Debug)]
struct ReceivedEnvelope {
    envelope: BridgeEnvelope<Value>,
    payload_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize)]
struct HelloPayload {
    client_nonce: String,
    server_nonce: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    runtime_generation: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct HelloInitPayload {
    client_nonce: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BridgePresence {
    pub(crate) uuid: Uuid,
    pub(crate) name: String,
    pub(crate) online: bool,
    #[serde(default)]
    pub(crate) observed_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct PresenceSyncPayload {
    #[serde(default)]
    players: Vec<BridgePresence>,
    #[serde(default = "default_presence_complete")]
    complete: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct PlayerDeltaPayload {
    #[serde(default)]
    action: Option<String>,
    player: BridgePlayerSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BridgePlayerSnapshot {
    pub(crate) uuid: Uuid,
    pub(crate) name: String,
    pub(crate) online: bool,
    #[serde(default)]
    pub(crate) observed_at: i64,
    #[serde(default)]
    pub(crate) level: Option<i32>,
    #[serde(default)]
    pub(crate) experience_progress: Option<f32>,
    #[serde(default)]
    pub(crate) total_experience: Option<i32>,
    #[serde(default)]
    pub(crate) dimension: Option<String>,
    #[serde(default)]
    pub(crate) position: Option<BridgePosition>,
    #[serde(default)]
    pub(crate) game_mode: Option<String>,
    #[serde(default)]
    pub(crate) health: Option<f32>,
    #[serde(default)]
    pub(crate) food_level: Option<i32>,
    #[serde(default)]
    pub(crate) inventory: Option<BridgeInventoryView>,
    #[serde(default)]
    pub(crate) ender_chest: Option<BridgeInventoryView>,
    #[serde(default)]
    pub(crate) papi: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BridgePosition {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BridgeInventoryView {
    pub(crate) slots: Vec<BridgeInventorySlot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BridgeInventorySlot {
    pub(crate) slot: i16,
    pub(crate) item: Option<BridgeItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BridgeItem {
    pub(crate) id: String,
    pub(crate) count: u32,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) lore: Vec<String>,
    #[serde(default)]
    pub(crate) container: Option<BridgeContainerPreview>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BridgeContainerPreview {
    pub(crate) kind: String,
    pub(crate) size: usize,
    pub(crate) slots: Vec<BridgeInventorySlot>,
}

#[derive(Clone, Debug)]
pub(crate) struct BridgeSnapshotView {
    pub(crate) snapshot: BridgePlayerSnapshot,
    pub(crate) received_at: Instant,
}

impl BridgeSnapshotView {
    pub(crate) fn freshness(&self) -> &'static str {
        let age = self.received_at.elapsed();
        if age <= SNAPSHOT_TTL {
            "live"
        } else if age <= SNAPSHOT_STALE_AFTER {
            "stale"
        } else {
            "expired"
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct BridgePapiRequestField {
    pub(crate) id: String,
    pub(crate) placeholder: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BridgePapiFieldValue {
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) value: Option<String>,
    #[serde(default)]
    pub(crate) error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct BridgePapiResponse {
    pub(crate) status: String,
    pub(crate) player_uuid: Uuid,
    #[serde(default)]
    pub(crate) fields: HashMap<String, BridgePapiFieldValue>,
    #[serde(default)]
    pub(crate) error_code: Option<String>,
}

#[derive(Clone)]
pub(crate) struct BridgeRuntime {
    inner: Arc<RwLock<BridgeState>>,
}

struct BridgeState {
    tokens: HashMap<String, String>,
    sessions: HashMap<String, BridgeSession>,
    snapshots: HashMap<String, HashMap<Uuid, CachedSnapshot>>,
    presence: HashMap<String, HashMap<Uuid, BridgePresence>>,
    presence_staging: HashMap<String, HashMap<Uuid, BridgePresence>>,
    pending_papi: HashMap<String, PendingPapiRequest>,
    pending_snapshots: HashMap<String, PendingSnapshotRequest>,
}

#[derive(Clone)]
struct BridgeSession {
    connection_id: Uuid,
    instance_id: String,
    session_id: String,
    client_key: Vec<u8>,
    server_key: Vec<u8>,
    sender: mpsc::Sender<String>,
    last_seen: Instant,
    last_sequence: u64,
    outbound_sequence: u64,
    capabilities: Vec<String>,
}

struct CachedSnapshot {
    snapshot: BridgePlayerSnapshot,
    received_at: Instant,
    sequence: u64,
}

struct PendingPapiRequest {
    server_id: String,
    sender: oneshot::Sender<Result<BridgePapiResponse, String>>,
}

struct PendingSnapshotRequest {
    server_id: String,
    player_uuid: Uuid,
    sender: oneshot::Sender<Result<BridgeSnapshotView, String>>,
}

struct AuthenticatedHello {
    payload: HelloPayload,
    token: String,
}

struct RegisteredSession {
    connection_id: Uuid,
    session_id: String,
    server_key: Vec<u8>,
}

enum ValidatedInbound {
    Presence {
        players: HashMap<Uuid, BridgePresence>,
        complete: bool,
    },
    PlayerDelta {
        action: Option<String>,
        player: BridgePlayerSnapshot,
    },
    Snapshot {
        request_id: Option<String>,
        result: Result<BridgePlayerSnapshot, String>,
    },
    Papi {
        request_id: String,
        response: BridgePapiResponse,
    },
    Heartbeat,
    Bye,
}

impl Default for BridgeRuntime {
    fn default() -> Self {
        Self::from_tokens(HashMap::new())
    }
}

impl BridgeRuntime {
    pub(crate) fn from_env() -> Self {
        let mut tokens = HashMap::new();
        if let Ok(token) = env::var("SCULK_BRIDGE_TOKEN") {
            if valid_token(&token) {
                tokens.insert("*".into(), token);
            }
        }
        if let Ok(server_tokens) = env::var("SCULK_BRIDGE_TOKENS") {
            for entry in server_tokens.split(',') {
                let Some((server_id, token)) = entry.split_once('=') else {
                    continue;
                };
                if valid_server_id(server_id) && valid_token(token) {
                    tokens.insert(server_id.trim().to_string(), token.trim().to_string());
                }
            }
        }
        Self::from_tokens(tokens)
    }

    fn from_tokens(tokens: HashMap<String, String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(BridgeState {
                tokens,
                sessions: HashMap::new(),
                snapshots: HashMap::new(),
                presence: HashMap::new(),
                presence_staging: HashMap::new(),
                pending_papi: HashMap::new(),
                pending_snapshots: HashMap::new(),
            })),
        }
    }

    #[cfg(test)]
    fn with_token(server_id: &str, token: &str) -> Self {
        let mut tokens = HashMap::new();
        tokens.insert(server_id.to_string(), token.to_string());
        Self::from_tokens(tokens)
    }

    pub(crate) async fn status(&self, server_id: &str) -> BridgeStatusResponse {
        let state = self.inner.read().await;
        let session = state.sessions.get(server_id);
        let connected =
            session.is_some_and(|session| session.last_seen.elapsed() <= SNAPSHOT_STALE_AFTER);
        let snapshot_count = state.snapshots.get(server_id).map_or(0, HashMap::len);
        BridgeStatusResponse {
            server_id: server_id.to_string(),
            connected,
            instance_id: session.map(|session| session.instance_id.clone()),
            protocol_version: PROTOCOL_VERSION,
            capabilities: session
                .map(|session| session.capabilities.clone())
                .unwrap_or_default(),
            last_seen_ms_ago: session.map(|session| session.last_seen.elapsed().as_millis() as u64),
            snapshot_count,
            detail: if connected {
                "Paper/Folia 桥接已连接，玩家数据由插件提供".into()
            } else {
                "未连接；玩家管理将使用 playerdata 离线兜底".into()
            },
        }
    }

    pub(crate) async fn shutdown(&self) {
        let mut state = self.inner.write().await;
        state.sessions.clear();
        state.presence.clear();
        state.presence_staging.clear();
        state.snapshots.clear();
        for (_, pending) in state.pending_papi.drain() {
            let _ = pending.sender.send(Err("Paper/Folia 桥接已关闭".into()));
        }
        for (_, pending) in state.pending_snapshots.drain() {
            let _ = pending.sender.send(Err("Paper/Folia 桥接已关闭".into()));
        }
    }

    pub(crate) async fn snapshots(&self, server_id: &str) -> Vec<BridgeSnapshotView> {
        let state = self.inner.read().await;
        state
            .snapshots
            .get(server_id)
            .into_iter()
            .flat_map(|snapshots| snapshots.values())
            .filter(|cached| cached.received_at.elapsed() <= SNAPSHOT_STALE_AFTER)
            .map(|cached| BridgeSnapshotView {
                snapshot: cached.snapshot.clone(),
                received_at: cached.received_at,
            })
            .collect()
    }

    pub(crate) async fn presences(&self, server_id: &str) -> Vec<BridgePresence> {
        let state = self.inner.read().await;
        if !state
            .sessions
            .get(server_id)
            .is_some_and(|session| session.last_seen.elapsed() <= SNAPSHOT_STALE_AFTER)
        {
            return Vec::new();
        }
        state
            .presence
            .get(server_id)
            .map(|players| players.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) async fn request_papi(
        &self,
        server_id: &str,
        player_uuid: Uuid,
        fields: Vec<BridgePapiRequestField>,
    ) -> Result<BridgePapiResponse, String> {
        if fields.is_empty() || fields.len() > MAX_PAPI_FIELDS {
            return Err("PAPI 字段数量无效".into());
        }
        if fields.iter().any(|field| {
            field.id.is_empty()
                || field.id.len() > 64
                || field.placeholder.is_empty()
                || field.placeholder.len() > MAX_TEXT_BYTES
        }) {
            return Err("PAPI 字段格式无效".into());
        }
        let request_id = Uuid::new_v4().to_string();
        let (reply_sender, reply_receiver) = oneshot::channel();
        let sender = {
            let mut state = self.inner.write().await;
            let session = state
                .sessions
                .get_mut(server_id)
                .filter(|session| session.last_seen.elapsed() <= SNAPSHOT_STALE_AFTER)
                .ok_or_else(|| "Paper/Folia 桥接未连接".to_string())?;
            if !session
                .capabilities
                .iter()
                .any(|capability| capability == "papi_read")
            {
                return Err("Paper/Folia 桥接未声明 papi_read 能力".into());
            }
            session.outbound_sequence = session.outbound_sequence.saturating_add(1);
            let instance_id = session.instance_id.clone();
            let sequence = session.outbound_sequence;
            let message_sender = session.sender.clone();
            let envelope = BridgeEnvelope {
                protocol_version: PROTOCOL_VERSION,
                message_type: "papi_request".into(),
                request_id: Some(request_id.clone()),
                server_id: server_id.to_string(),
                instance_id,
                session_id: Some(session.session_id.clone()),
                sequence,
                sent_at: unix_millis(),
                payload: json!({
                    "player_uuid": player_uuid,
                    "fields": fields,
                }),
                signature: None,
            };
            let encoded = encode_signed_envelope(
                &envelope,
                &session.server_key,
                SignatureDirection::ServerToClient,
            )
            .map_err(|error| format!("PAPI 桥接请求编码失败：{error}"))?;
            state.pending_papi.insert(
                request_id.clone(),
                PendingPapiRequest {
                    server_id: server_id.to_string(),
                    sender: reply_sender,
                },
            );
            (message_sender, encoded)
        };
        if sender.0.try_send(sender.1).is_err() {
            self.inner.write().await.pending_papi.remove(&request_id);
            return Err("PAPI 桥接队列已满或连接已关闭".into());
        }
        match timeout(Duration::from_secs(2), reply_receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("PAPI 桥接请求已取消".into()),
            Err(_) => {
                self.inner.write().await.pending_papi.remove(&request_id);
                Err("PAPI 桥接请求超时".into())
            }
        }
    }

    pub(crate) async fn request_snapshot(
        &self,
        server_id: &str,
        player_uuid: Uuid,
    ) -> Result<BridgeSnapshotView, String> {
        let request_id = Uuid::new_v4().to_string();
        let (reply_sender, reply_receiver) = oneshot::channel();
        let (message_sender, encoded) = {
            let mut state = self.inner.write().await;
            let session = state
                .sessions
                .get_mut(server_id)
                .filter(|session| session.last_seen.elapsed() <= SNAPSHOT_STALE_AFTER)
                .ok_or_else(|| "Paper/Folia 桥接未连接".to_string())?;
            if !session
                .capabilities
                .iter()
                .any(|capability| capability == "snapshot")
            {
                return Err("Paper/Folia 桥接未声明 snapshot 能力".into());
            }
            session.outbound_sequence = session.outbound_sequence.saturating_add(1);
            let envelope = BridgeEnvelope {
                protocol_version: PROTOCOL_VERSION,
                message_type: "snapshot_request".into(),
                request_id: Some(request_id.clone()),
                server_id: server_id.to_string(),
                instance_id: session.instance_id.clone(),
                session_id: Some(session.session_id.clone()),
                sequence: session.outbound_sequence,
                sent_at: unix_millis(),
                payload: json!({
                    "player_uuid": player_uuid,
                    "sections": ["basic", "inventory", "ender_chest"],
                }),
                signature: None,
            };
            let encoded = encode_signed_envelope(
                &envelope,
                &session.server_key,
                SignatureDirection::ServerToClient,
            )
            .map_err(|error| format!("快照桥接请求编码失败：{error}"))?;
            let message_sender = session.sender.clone();
            state.pending_snapshots.insert(
                request_id.clone(),
                PendingSnapshotRequest {
                    server_id: server_id.to_string(),
                    player_uuid,
                    sender: reply_sender,
                },
            );
            (message_sender, encoded)
        };
        if message_sender.try_send(encoded).is_err() {
            self.inner
                .write()
                .await
                .pending_snapshots
                .remove(&request_id);
            return Err("快照桥接队列已满或连接已关闭".into());
        }
        match timeout(Duration::from_secs(2), reply_receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.inner
                    .write()
                    .await
                    .pending_snapshots
                    .remove(&request_id);
                Err("快照桥接请求已取消".into())
            }
            Err(_) => {
                self.inner
                    .write()
                    .await
                    .pending_snapshots
                    .remove(&request_id);
                Err("快照桥接请求超时".into())
            }
        }
    }

    async fn authenticate(
        &self,
        received: &ReceivedEnvelope,
        expected_client_nonce: &str,
        expected_server_nonce: &str,
    ) -> Result<AuthenticatedHello, String> {
        let envelope = &received.envelope;
        if envelope.message_type != "hello" {
            return Err("challenge 后首条桥接消息必须是 hello".into());
        }
        validate_envelope_metadata(envelope, true)?;
        if envelope.session_id.is_some() {
            return Err("hello 不得携带 session_id".into());
        }
        if !timestamp_is_current(envelope.sent_at, CLOCK_SKEW) {
            return Err("hello sent_at 已过期或超前".into());
        }
        let payload: HelloPayload = serde_json::from_value(envelope.payload.clone())
            .map_err(|error| format!("hello payload 无效：{error}"))?;
        if payload.client_nonce != expected_client_nonce
            || payload.server_nonce != expected_server_nonce
            || !valid_nonce(&payload.client_nonce)
            || !valid_nonce(&payload.server_nonce)
        {
            return Err("hello challenge nonce 不匹配或无效".into());
        }
        let state = self.inner.read().await;
        let token = state
            .tokens
            .get(&envelope.server_id)
            .or_else(|| state.tokens.get("*"))
            .cloned()
            .ok_or_else(|| "未配置该服务器的桥接凭据".to_string())?;
        verify_envelope_signature(
            token.as_bytes(),
            SignatureDirection::Hello,
            envelope,
            &received.payload_bytes,
            envelope.signature.as_deref(),
        )?;
        Ok(AuthenticatedHello { payload, token })
    }

    async fn register(
        &self,
        envelope: &BridgeEnvelope<Value>,
        hello: &AuthenticatedHello,
        sender: mpsc::Sender<String>,
        state: &AppState,
    ) -> Result<RegisteredSession, String> {
        let server = state
            .inner
            .read()
            .await
            .servers
            .iter()
            .find(|server| server.id == envelope.server_id)
            .cloned()
            .ok_or_else(|| "桥接服务器不存在".to_string())?;
        let runtime_generation = server.runtime_generation;
        let parsed_generation = hello
            .payload
            .runtime_generation
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok());
        if let (Some(expected), Some(actual)) = (runtime_generation, parsed_generation) {
            if expected != actual {
                return Err("桥接 runtime_generation 与服务器当前实例不一致".into());
            }
        }
        let now = Instant::now();
        let connection_id = Uuid::new_v4();
        let session_id = Uuid::new_v4().to_string();
        let client_key = derive_session_key(
            &hello.token,
            SignatureDirection::ClientToServer,
            &envelope.server_id,
            &envelope.instance_id,
            &hello.payload.client_nonce,
            &hello.payload.server_nonce,
            &session_id,
        )?;
        let server_key = derive_session_key(
            &hello.token,
            SignatureDirection::ServerToClient,
            &envelope.server_id,
            &envelope.instance_id,
            &hello.payload.client_nonce,
            &hello.payload.server_nonce,
            &session_id,
        )?;
        let mut registry = self.inner.write().await;
        registry.sessions.insert(
            envelope.server_id.clone(),
            BridgeSession {
                connection_id,
                instance_id: envelope.instance_id.clone(),
                session_id: session_id.clone(),
                client_key,
                server_key: server_key.clone(),
                sender,
                last_seen: now,
                last_sequence: envelope.sequence,
                outbound_sequence: 1,
                capabilities: hello
                    .payload
                    .capabilities
                    .iter()
                    .filter(|capability| valid_capability(capability))
                    .take(64)
                    .cloned()
                    .collect(),
            },
        );
        registry.presence.remove(&envelope.server_id);
        registry.presence_staging.remove(&envelope.server_id);
        Ok(RegisteredSession {
            connection_id,
            session_id,
            server_key,
        })
    }

    async fn accept_envelope(
        &self,
        connection_id: Uuid,
        received: ReceivedEnvelope,
    ) -> Result<(), String> {
        let envelope = received.envelope;
        validate_envelope_metadata(&envelope, false)?;
        if !timestamp_is_current(envelope.sent_at, CLOCK_SKEW) {
            return Err("桥接帧 sent_at 已过期或超前".into());
        }
        let mut state = self.inner.write().await;
        {
            let session = state
                .sessions
                .get_mut(&envelope.server_id)
                .ok_or_else(|| "桥接会话不存在".to_string())?;
            if session.instance_id != envelope.instance_id {
                return Err("桥接 instance_id 不匹配".into());
            }
            if session.connection_id != connection_id {
                return Err("桥接连接已被新的会话替换".into());
            }
            if envelope.session_id.as_deref() != Some(session.session_id.as_str()) {
                return Err("桥接 session_id 不匹配".into());
            }
            verify_envelope_signature(
                &session.client_key,
                SignatureDirection::ClientToServer,
                &envelope,
                &received.payload_bytes,
                envelope.signature.as_deref(),
            )?;
            if envelope.sequence <= session.last_sequence {
                return Err("桥接 seq 必须单调递增".into());
            }
            if envelope.sequence - session.last_sequence > MAX_SEQUENCE_GAP {
                return Err("桥接 seq 跳跃过大，需要重新握手".into());
            }
        }
        let inbound = validate_inbound(&envelope)?;
        let server_id = envelope.server_id.clone();
        apply_inbound(&mut state, &server_id, envelope.sequence, inbound);
        let session = state
            .sessions
            .get_mut(&server_id)
            .filter(|session| session.connection_id == connection_id)
            .ok_or_else(|| "桥接会话已失效".to_string())?;
        session.last_sequence = envelope.sequence;
        session.last_seen = Instant::now();
        Ok(())
    }

    async fn enqueue_error(
        &self,
        server_id: &str,
        instance_id: &str,
        connection_id: Uuid,
        request_id: Option<String>,
        code: &str,
        message: &str,
    ) -> Result<(), ()> {
        let (sender, encoded) = {
            let mut state = self.inner.write().await;
            let session = state
                .sessions
                .get_mut(server_id)
                .filter(|session| {
                    session.instance_id == instance_id && session.connection_id == connection_id
                })
                .ok_or(())?;
            session.outbound_sequence = session.outbound_sequence.saturating_add(1);
            let envelope = BridgeEnvelope {
                protocol_version: PROTOCOL_VERSION,
                message_type: "error".into(),
                request_id,
                server_id: server_id.to_string(),
                instance_id: session.instance_id.clone(),
                session_id: Some(session.session_id.clone()),
                sequence: session.outbound_sequence,
                sent_at: unix_millis(),
                payload: json!({
                    "code": code,
                    "message": message.chars().take(MAX_TEXT_BYTES).collect::<String>(),
                }),
                signature: None,
            };
            let encoded = encode_signed_envelope(
                &envelope,
                &session.server_key,
                SignatureDirection::ServerToClient,
            )
            .map_err(|_| ())?;
            (session.sender.clone(), encoded)
        };
        sender.try_send(encoded).map_err(|_| ())
    }

    async fn disconnect(&self, server_id: &str, instance_id: &str, connection_id: Uuid) {
        let mut state = self.inner.write().await;
        if state.sessions.get(server_id).is_some_and(|session| {
            session.instance_id == instance_id && session.connection_id == connection_id
        }) {
            state.sessions.remove(server_id);
            state.presence.remove(server_id);
            state.presence_staging.remove(server_id);
            let mut retained_papi = HashMap::new();
            for (request_id, pending) in state.pending_papi.drain() {
                if pending.server_id == server_id {
                    let _ = pending.sender.send(Err("Paper/Folia 桥接已断开".into()));
                } else {
                    retained_papi.insert(request_id, pending);
                }
            }
            state.pending_papi = retained_papi;
            let mut retained_snapshots = HashMap::new();
            for (request_id, pending) in state.pending_snapshots.drain() {
                if pending.server_id == server_id {
                    let _ = pending.sender.send(Err("Paper/Folia 桥接已断开".into()));
                } else {
                    retained_snapshots.insert(request_id, pending);
                }
            }
            state.pending_snapshots = retained_snapshots;
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct BridgeStatusResponse {
    pub(crate) server_id: String,
    pub(crate) connected: bool,
    pub(crate) instance_id: Option<String>,
    pub(crate) protocol_version: u16,
    pub(crate) capabilities: Vec<String>,
    pub(crate) last_seen_ms_ago: Option<u64>,
    pub(crate) snapshot_count: usize,
    pub(crate) detail: String,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/bridge/v1/ws", get(bridge_ws))
        .route("/api/servers/{server_id}/bridge/status", get(bridge_status))
}

async fn bridge_status(
    axum::extract::Path(server_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> Result<Json<BridgeStatusResponse>, (StatusCode, String)> {
    if !state
        .inner
        .read()
        .await
        .servers
        .iter()
        .any(|server| server.id == server_id)
    {
        return Err((StatusCode::NOT_FOUND, "未找到服务器".into()));
    }
    Ok(Json(state.bridge.status(&server_id).await))
}

async fn bridge_ws(
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    Ok(upgrade.on_upgrade(move |socket| bridge_socket(socket, state)))
}

async fn bridge_socket(mut socket: WebSocket, state: AppState) {
    let first = timeout(HELLO_TIMEOUT, socket.recv()).await;
    let Some(Ok(Message::Text(first))) = first.ok().flatten() else {
        close_unauthenticated(&mut socket, "hello_init 超时或消息类型无效").await;
        return;
    };
    if first.len() > MAX_FRAME_BYTES {
        close_unauthenticated(&mut socket, "hello_init 帧超过大小限制").await;
        return;
    }
    let hello_init = match parse_wire_envelope(first.as_ref()) {
        Ok(value) => value,
        Err(_) => {
            close_unauthenticated(&mut socket, "hello_init JSON 无效").await;
            return;
        }
    };
    let init_payload = match validate_hello_init(&hello_init.envelope) {
        Ok(payload) => payload,
        Err(_) => {
            close_unauthenticated(&mut socket, "hello_init 无效").await;
            return;
        }
    };
    let server_nonce = random_nonce();
    let challenge = BridgeEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_type: "challenge".into(),
        request_id: hello_init.envelope.request_id.clone(),
        server_id: hello_init.envelope.server_id.clone(),
        instance_id: hello_init.envelope.instance_id.clone(),
        session_id: None,
        sequence: 1,
        sent_at: unix_millis(),
        payload: json!({
            "client_nonce": init_payload.client_nonce.clone(),
            "server_nonce": server_nonce.clone(),
            "expires_at": unix_millis() + CHALLENGE_TTL.as_millis() as i64,
        }),
        signature: None,
    };
    let challenge_text = match encode_unsigned_envelope(&challenge) {
        Ok(value) => value,
        Err(_) => return,
    };
    if socket
        .send(Message::Text(challenge_text.into()))
        .await
        .is_err()
    {
        return;
    }

    let second = timeout(HELLO_TIMEOUT, socket.recv()).await;
    let Some(Ok(Message::Text(second))) = second.ok().flatten() else {
        close_unauthenticated(&mut socket, "hello 超时或消息类型无效").await;
        return;
    };
    if second.len() > MAX_FRAME_BYTES {
        close_unauthenticated(&mut socket, "hello 帧超过大小限制").await;
        return;
    }
    let hello_envelope = match parse_wire_envelope(second.as_ref()) {
        Ok(value) => value,
        Err(_) => {
            close_unauthenticated(&mut socket, "hello JSON 无效").await;
            return;
        }
    };
    if hello_envelope.envelope.server_id != hello_init.envelope.server_id
        || hello_envelope.envelope.instance_id != hello_init.envelope.instance_id
        || hello_envelope.envelope.sequence <= hello_init.envelope.sequence
    {
        close_unauthenticated(&mut socket, "hello 未绑定到当前 challenge").await;
        return;
    }
    let hello = match state
        .bridge
        .authenticate(&hello_envelope, &init_payload.client_nonce, &server_nonce)
        .await
    {
        Ok(payload) => payload,
        Err(_) => {
            close_unauthenticated(&mut socket, "hello 认证失败").await;
            return;
        }
    };
    let (sender, mut receiver) = mpsc::channel::<String>(64);
    let registered = match state
        .bridge
        .register(&hello_envelope.envelope, &hello, sender, &state)
        .await
    {
        Ok(registered) => registered,
        Err(error) => {
            let _ = send_initial_error(
                &mut socket,
                &hello_envelope.envelope,
                "registration_rejected",
                &error,
            )
            .await;
            return;
        }
    };
    let connection_id = registered.connection_id;
    let server_id = hello_envelope.envelope.server_id.clone();
    let instance_id = hello_envelope.envelope.instance_id.clone();
    let ack = BridgeEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_type: "hello_ack".into(),
        request_id: hello_envelope.envelope.request_id.clone(),
        server_id: server_id.clone(),
        instance_id: instance_id.clone(),
        session_id: Some(registered.session_id),
        sequence: 1,
        sent_at: unix_millis(),
        payload: json!({
            "accepted": true,
            "protocol_version": PROTOCOL_VERSION,
            "client_nonce": hello.payload.client_nonce,
            "server_nonce": hello.payload.server_nonce,
            "capabilities": ["presence_sync", "player_delta", "snapshot_request", "snapshot_response", "papi"],
        }),
        signature: None,
    };
    let ack = match encode_signed_envelope(
        &ack,
        &registered.server_key,
        SignatureDirection::ServerToClient,
    ) {
        Ok(value) => value,
        Err(_) => return,
    };
    if socket.send(Message::Text(ack.into())).await.is_err() {
        state
            .bridge
            .disconnect(&server_id, &instance_id, connection_id)
            .await;
        return;
    }

    loop {
        tokio::select! {
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > MAX_FRAME_BYTES {
                            if send_error(
                                &state.bridge,
                                &server_id,
                                &instance_id,
                                connection_id,
                                None,
                                "frame_too_large",
                                "桥接帧超过大小限制",
                            ).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        let parsed = parse_wire_envelope(text.as_ref());
                        match parsed {
                            Ok(received) => {
                                let request_id = received.envelope.request_id.clone();
                                if let Err(error) = state.bridge.accept_envelope(connection_id, received).await {
                                    if send_error(
                                        &state.bridge,
                                        &server_id,
                                        &instance_id,
                                        connection_id,
                                        request_id,
                                        "invalid_message",
                                        &error,
                                    ).await.is_err() {
                                        break;
                                    }
                                    break;
                                }
                            }
                            Err(_) => {
                                if send_error(
                                    &state.bridge,
                                    &server_id,
                                    &instance_id,
                                    connection_id,
                                    None,
                                    "invalid_json",
                                    "桥接 JSON 无效",
                                ).await.is_err() {
                                    break;
                                }
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Binary(_))) => {
                        if send_error(
                            &state.bridge,
                            &server_id,
                            &instance_id,
                            connection_id,
                            None,
                            "binary_not_allowed",
                            "桥接只接受 JSON 文本",
                        ).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                }
            }
            outgoing = receiver.recv() => {
                let Some(outgoing) = outgoing else { break; };
                if socket.send(Message::Text(outgoing.into())).await.is_err() { break; }
            }
        }
    }
    state
        .bridge
        .disconnect(&server_id, &instance_id, connection_id)
        .await;
}

async fn close_unauthenticated(socket: &mut WebSocket, reason: &str) {
    let _ = socket
        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
            code: axum::extract::ws::close_code::POLICY,
            reason: reason.into(),
        })))
        .await;
}

async fn send_initial_error(
    socket: &mut WebSocket,
    hello: &BridgeEnvelope<Value>,
    code: &str,
    message: &str,
) -> Result<(), ()> {
    let envelope = BridgeEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_type: "error".into(),
        request_id: hello.request_id.clone(),
        server_id: hello.server_id.clone(),
        instance_id: hello.instance_id.clone(),
        session_id: None,
        sequence: 1,
        sent_at: unix_millis(),
        payload: json!({
            "code": code,
            "message": message.chars().take(MAX_TEXT_BYTES).collect::<String>(),
        }),
        signature: None,
    };
    socket
        .send(Message::Text(
            encode_unsigned_envelope(&envelope).map_err(|_| ())?.into(),
        ))
        .await
        .map_err(|_| ())
}

async fn send_error(
    bridge: &BridgeRuntime,
    server_id: &str,
    instance_id: &str,
    connection_id: Uuid,
    request_id: Option<String>,
    code: &str,
    message: &str,
) -> Result<(), ()> {
    bridge
        .enqueue_error(
            server_id,
            instance_id,
            connection_id,
            request_id,
            code,
            message,
        )
        .await
}

fn parse_snapshot_payload(payload: Value) -> Result<BridgePlayerSnapshot, String> {
    if payload.get("snapshot").is_some() {
        serde_json::from_value(payload.get("snapshot").cloned().unwrap_or(Value::Null))
            .map_err(|error| format!("snapshot_response 无效：{error}"))
    } else if payload.get("player").is_some() {
        serde_json::from_value(payload.get("player").cloned().unwrap_or(Value::Null))
            .map_err(|error| format!("player_snapshot 无效：{error}"))
    } else {
        serde_json::from_value(payload).map_err(|error| format!("玩家快照无效：{error}"))
    }
}

fn default_presence_complete() -> bool {
    true
}

fn validate_hello_init(envelope: &BridgeEnvelope<Value>) -> Result<HelloInitPayload, String> {
    if envelope.message_type != "hello_init" {
        return Err("首条桥接消息必须是 hello_init".into());
    }
    validate_envelope_metadata(envelope, true)?;
    if envelope.session_id.is_some() || envelope.signature.is_some() {
        return Err("hello_init 不得携带会话认证字段".into());
    }
    if !timestamp_is_current(envelope.sent_at, CLOCK_SKEW) {
        return Err("hello_init sent_at 已过期或超前".into());
    }
    let payload: HelloInitPayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|error| format!("hello_init payload 无效：{error}"))?;
    if !valid_nonce(&payload.client_nonce) {
        return Err("hello_init client_nonce 无效".into());
    }
    Ok(payload)
}

fn validate_envelope_metadata(
    envelope: &BridgeEnvelope<Value>,
    handshake: bool,
) -> Result<(), String> {
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(format!(
            "不支持的桥接协议版本 {}",
            envelope.protocol_version
        ));
    }
    if !valid_server_id(&envelope.server_id) || !valid_instance_id(&envelope.instance_id) {
        return Err("server_id 或 instance_id 无效".into());
    }
    if envelope.sequence == 0 || envelope.sequence > i64::MAX as u64 || envelope.sent_at <= 0 {
        return Err("桥接帧的 seq/sent_at 无效".into());
    }
    if envelope
        .session_id
        .as_deref()
        .is_some_and(|session_id| Uuid::parse_str(session_id).is_err())
    {
        return Err("桥接 session_id 无效".into());
    }
    if envelope
        .signature
        .as_deref()
        .is_some_and(|signature| signature.is_empty() || signature.len() > 128)
    {
        return Err("桥接 signature 格式无效".into());
    }
    if !handshake && envelope.session_id.is_none() {
        return Err("桥接帧缺少 session_id".into());
    }
    Ok(())
}

fn validate_inbound(envelope: &BridgeEnvelope<Value>) -> Result<ValidatedInbound, String> {
    match envelope.message_type.as_str() {
        "presence_sync" => {
            let payload: PresenceSyncPayload = serde_json::from_value(envelope.payload.clone())
                .map_err(|error| format!("presence_sync 无效：{error}"))?;
            if payload.players.len() > MAX_PLAYERS_PER_BATCH {
                return Err("presence_sync 玩家数量超限".into());
            }
            let mut players = HashMap::with_capacity(payload.players.len());
            for player in payload.players {
                validate_presence(&player)?;
                players.insert(player.uuid, player);
            }
            Ok(ValidatedInbound::Presence {
                players,
                complete: payload.complete,
            })
        }
        "player_delta" => {
            let payload: PlayerDeltaPayload = serde_json::from_value(envelope.payload.clone())
                .map_err(|error| format!("player_delta 无效：{error}"))?;
            validate_snapshot(&payload.player)?;
            Ok(ValidatedInbound::PlayerDelta {
                action: payload.action,
                player: payload.player,
            })
        }
        "snapshot_response" | "player_snapshot" => {
            let request_id = envelope.request_id.clone();
            let result = if envelope.message_type == "snapshot_response" {
                match envelope.payload.get("status").and_then(Value::as_str) {
                    Some("ok") | None => parse_snapshot_payload(envelope.payload.clone()),
                    Some(status) => Err(snapshot_response_error(&envelope.payload, status)),
                }
            } else {
                parse_snapshot_payload(envelope.payload.clone())
            };
            let result = result.and_then(|snapshot| {
                validate_snapshot(&snapshot)?;
                Ok(snapshot)
            });
            Ok(ValidatedInbound::Snapshot { request_id, result })
        }
        "papi_response" => {
            let request_id = envelope
                .request_id
                .clone()
                .ok_or_else(|| "papi_response 缺少 request_id".to_string())?;
            let response: BridgePapiResponse = serde_json::from_value(envelope.payload.clone())
                .map_err(|error| format!("papi_response 无效：{error}"))?;
            validate_papi_response(&response)?;
            Ok(ValidatedInbound::Papi {
                request_id,
                response,
            })
        }
        "heartbeat" => Ok(ValidatedInbound::Heartbeat),
        "bye" => Ok(ValidatedInbound::Bye),
        other => Err(format!("不允许的桥接消息类型 {other}")),
    }
}

fn apply_inbound(
    state: &mut BridgeState,
    server_id: &str,
    sequence: u64,
    inbound: ValidatedInbound,
) {
    match inbound {
        ValidatedInbound::Presence { players, complete } => {
            let staging = state
                .presence_staging
                .entry(server_id.to_string())
                .or_default();
            staging.extend(players);
            if complete {
                let presence = state.presence_staging.remove(server_id).unwrap_or_default();
                if let Some(snapshots) = state.snapshots.get_mut(server_id) {
                    for (uuid, snapshot) in snapshots {
                        if !presence.contains_key(uuid) {
                            snapshot.snapshot.online = false;
                        }
                    }
                }
                state.presence.insert(server_id.to_string(), presence);
            }
        }
        ValidatedInbound::PlayerDelta { action, player } => {
            if matches!(action.as_deref(), Some("leave" | "quit")) {
                if let Some(snapshot) = state
                    .snapshots
                    .entry(server_id.to_string())
                    .or_default()
                    .get_mut(&player.uuid)
                {
                    snapshot.snapshot.online = false;
                }
                let presence = BridgePresence {
                    uuid: player.uuid,
                    name: player.name,
                    online: false,
                    observed_at: player.observed_at,
                };
                state
                    .presence
                    .entry(server_id.to_string())
                    .or_default()
                    .insert(presence.uuid, presence.clone());
            } else {
                let presence = BridgePresence {
                    uuid: player.uuid,
                    name: player.name.clone(),
                    online: player.online,
                    observed_at: player.observed_at,
                };
                state
                    .presence
                    .entry(server_id.to_string())
                    .or_default()
                    .insert(presence.uuid, presence.clone());
                store_snapshot(state, server_id, player, sequence);
            }
        }
        ValidatedInbound::Snapshot { request_id, result } => match result {
            Ok(snapshot) => {
                let received_at = store_snapshot(state, server_id, snapshot.clone(), sequence);
                if let Some(request_id) = request_id.as_deref() {
                    complete_snapshot_request(
                        state,
                        request_id,
                        server_id,
                        Ok(BridgeSnapshotView {
                            snapshot,
                            received_at,
                        }),
                    );
                }
            }
            Err(error) => {
                if let Some(request_id) = request_id.as_deref() {
                    complete_snapshot_request(state, request_id, server_id, Err(error));
                }
            }
        },
        ValidatedInbound::Papi {
            request_id,
            response,
        } => {
            if let Some(pending) = state.pending_papi.remove(&request_id) {
                if pending.server_id == server_id {
                    let _ = pending.sender.send(Ok(response));
                } else {
                    let _ = pending.sender.send(Err("PAPI 桥接服务器不匹配".into()));
                }
            }
        }
        ValidatedInbound::Heartbeat | ValidatedInbound::Bye => {}
    }
}

fn store_snapshot(
    state: &mut BridgeState,
    server_id: &str,
    snapshot: BridgePlayerSnapshot,
    sequence: u64,
) -> Instant {
    let received_at = Instant::now();
    let snapshots = state.snapshots.entry(server_id.to_string()).or_default();
    if snapshots
        .get(&snapshot.uuid)
        .is_some_and(|cached| cached.sequence > sequence)
    {
        return received_at;
    }
    snapshots.insert(
        snapshot.uuid,
        CachedSnapshot {
            snapshot,
            received_at,
            sequence,
        },
    );
    while snapshots.len() > MAX_SNAPSHOTS_PER_SERVER {
        let Some(oldest) = snapshots
            .iter()
            .min_by_key(|(_, cached)| cached.received_at)
            .map(|(uuid, _)| *uuid)
        else {
            break;
        };
        snapshots.remove(&oldest);
    }
    received_at
}

fn complete_snapshot_request(
    state: &mut BridgeState,
    request_id: &str,
    server_id: &str,
    result: Result<BridgeSnapshotView, String>,
) {
    let Some(pending) = state.pending_snapshots.remove(request_id) else {
        return;
    };
    let result = if pending.server_id != server_id {
        Err("快照桥接服务器不匹配".into())
    } else if result
        .as_ref()
        .ok()
        .is_some_and(|snapshot| snapshot.snapshot.uuid != pending.player_uuid)
    {
        Err("snapshot_response 玩家 UUID 不匹配".into())
    } else {
        result
    };
    let _ = pending.sender.send(result);
}

fn snapshot_response_error(payload: &Value, status: &str) -> String {
    let detail = payload
        .get("error_code")
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(status);
    format!(
        "快照桥接请求失败：{}",
        detail.chars().take(MAX_TEXT_BYTES).collect::<String>()
    )
}

fn validate_presence(player: &BridgePresence) -> Result<(), String> {
    if !valid_player_name(&player.name)
        || !timestamp_is_current(player.observed_at, SNAPSHOT_STALE_AFTER)
    {
        return Err("玩家名称或观测时间无效".into());
    }
    Ok(())
}

fn validate_snapshot(snapshot: &BridgePlayerSnapshot) -> Result<(), String> {
    if !valid_player_name(&snapshot.name)
        || !timestamp_is_current(snapshot.observed_at, SNAPSHOT_STALE_AFTER)
        || !is_finite_position(snapshot.position.as_ref())
    {
        return Err("玩家快照身份或坐标无效".into());
    }
    if let Some(level) = snapshot.level {
        if !(0..=10_000).contains(&level) {
            return Err("玩家等级超出范围".into());
        }
    }
    let mut total_items = 0;
    if let Some(inventory) = &snapshot.inventory {
        validate_inventory(inventory, 0, &mut total_items)?;
    }
    if let Some(ender_chest) = &snapshot.ender_chest {
        validate_inventory(ender_chest, 0, &mut total_items)?;
    }
    if snapshot.papi.len() > MAX_PAPI_FIELDS {
        return Err("PAPI 字段数量超限".into());
    }
    for (key, value) in &snapshot.papi {
        if key.len() > 64 || value.len() > MAX_TEXT_BYTES {
            return Err("PAPI 字段超出大小限制".into());
        }
    }
    Ok(())
}

fn validate_papi_response(response: &BridgePapiResponse) -> Result<(), String> {
    if response.fields.len() > MAX_PAPI_FIELDS {
        return Err("PAPI 响应字段数量超限".into());
    }
    for (field_id, field) in &response.fields {
        if field_id.is_empty()
            || field_id.len() > 64
            || field.status.len() > 32
            || field
                .value
                .as_ref()
                .is_some_and(|value| value.len() > MAX_TEXT_BYTES)
            || field
                .error_code
                .as_ref()
                .is_some_and(|code| code.len() > 64)
        {
            return Err("PAPI 响应字段超限".into());
        }
    }
    Ok(())
}

fn validate_inventory(
    inventory: &BridgeInventoryView,
    depth: usize,
    total_items: &mut usize,
) -> Result<(), String> {
    if depth > MAX_CONTAINER_DEPTH || inventory.slots.len() > MAX_INVENTORY_SLOTS {
        return Err("容器槽位或递归深度超限".into());
    }
    let mut slots = HashSet::new();
    for slot in &inventory.slots {
        if !slots.insert(slot.slot) {
            return Err("容器存在重复槽位".into());
        }
        if let Some(item) = &slot.item {
            *total_items += 1;
            if *total_items > MAX_TOTAL_CONTAINER_ITEMS {
                return Err("物品预览总数量超限".into());
            }
            if item.id.is_empty() || item.id.len() > 128 || item.count > 99_999 {
                return Err("物品字段无效".into());
            }
            if item
                .name
                .as_ref()
                .is_some_and(|name| name.len() > MAX_TEXT_BYTES)
                || item.lore.len() > 12
                || item.lore.iter().any(|line| line.len() > MAX_TEXT_BYTES)
            {
                return Err("物品文本超出大小限制".into());
            }
            if let Some(container) = &item.container {
                if container.kind.len() > 64 || container.size > MAX_INVENTORY_SLOTS {
                    return Err("容器预览字段无效".into());
                }
                validate_inventory(
                    &BridgeInventoryView {
                        slots: container.slots.clone(),
                    },
                    depth + 1,
                    total_items,
                )?;
            }
        }
    }
    Ok(())
}

fn is_finite_position(position: Option<&BridgePosition>) -> bool {
    position.is_none_or(|position| {
        position.x.is_finite() && position.y.is_finite() && position.z.is_finite()
    })
}

#[derive(Clone, Copy)]
enum SignatureDirection {
    Hello,
    ClientToServer,
    ServerToClient,
}

impl SignatureDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Hello => "hello",
            Self::ClientToServer => "c2s",
            Self::ServerToClient => "s2c",
        }
    }
}

fn parse_wire_envelope(raw: &str) -> Result<ReceivedEnvelope, String> {
    let wire: WireBridgeEnvelope =
        serde_json::from_str(raw).map_err(|error| format!("桥接 JSON 无效：{error}"))?;
    let raw_value: Value =
        serde_json::from_str(raw).map_err(|error| format!("桥接 JSON 无效：{error}"))?;
    let fields = raw_value
        .as_object()
        .ok_or_else(|| "桥接 JSON 必须是对象".to_string())?;
    if fields.len() != WIRE_ENVELOPE_FIELDS.len()
        || !WIRE_ENVELOPE_FIELDS
            .iter()
            .all(|field| fields.contains_key(*field))
    {
        return Err("桥接信封必须恰好包含 v2 的 10 个字段".into());
    }
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(&wire.payload_json)
        .map_err(|_| "payload_json 编码无效".to_string())?;
    if payload_bytes.is_empty() || payload_bytes.len() > MAX_FRAME_BYTES {
        return Err("payload_json 大小无效".into());
    }
    let payload: Value = serde_json::from_slice(&payload_bytes)
        .map_err(|error| format!("payload_json JSON 无效：{error}"))?;
    if !payload.is_object() {
        return Err("payload_json 必须是 JSON 对象".into());
    }
    Ok(ReceivedEnvelope {
        envelope: BridgeEnvelope {
            protocol_version: wire.protocol_version,
            message_type: wire.message_type,
            request_id: wire.request_id,
            server_id: wire.server_id,
            instance_id: wire.instance_id,
            session_id: wire.session_id,
            sequence: wire.sequence,
            sent_at: wire.sent_at,
            payload,
            signature: wire.signature,
        },
        payload_bytes,
    })
}

fn encode_unsigned_envelope(envelope: &BridgeEnvelope<Value>) -> Result<String, String> {
    let payload_bytes = serde_json::to_vec(&envelope.payload)
        .map_err(|error| format!("桥接 payload 编码失败：{error}"))?;
    encode_wire_envelope(envelope, payload_bytes, None)
}

fn encode_signed_envelope(
    envelope: &BridgeEnvelope<Value>,
    key: &[u8],
    direction: SignatureDirection,
) -> Result<String, String> {
    let payload_bytes = serde_json::to_vec(&envelope.payload)
        .map_err(|error| format!("桥接 payload 编码失败：{error}"))?;
    let signature = sign_envelope(key, direction, envelope, &payload_bytes)?;
    encode_wire_envelope(envelope, payload_bytes, Some(signature))
}

fn encode_wire_envelope(
    envelope: &BridgeEnvelope<Value>,
    payload_bytes: Vec<u8>,
    signature: Option<String>,
) -> Result<String, String> {
    serde_json::to_string(&WireBridgeEnvelope {
        protocol_version: envelope.protocol_version,
        message_type: envelope.message_type.clone(),
        request_id: envelope.request_id.clone(),
        server_id: envelope.server_id.clone(),
        instance_id: envelope.instance_id.clone(),
        session_id: envelope.session_id.clone(),
        sequence: envelope.sequence,
        sent_at: envelope.sent_at,
        payload_json: URL_SAFE_NO_PAD.encode(payload_bytes),
        signature,
    })
    .map_err(|error| format!("桥接帧编码失败：{error}"))
}

fn sign_envelope(
    key: &[u8],
    direction: SignatureDirection,
    envelope: &BridgeEnvelope<Value>,
    payload_bytes: &[u8],
) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "桥接签名密钥无效".to_string())?;
    mac.update(canonical_envelope(direction, envelope, payload_bytes).as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn verify_envelope_signature(
    key: &[u8],
    direction: SignatureDirection,
    envelope: &BridgeEnvelope<Value>,
    payload_bytes: &[u8],
    signature: Option<&str>,
) -> Result<(), String> {
    let signature = signature.ok_or_else(|| "桥接帧缺少 signature".to_string())?;
    let provided = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| "桥接 signature 编码无效".to_string())?;
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "桥接签名密钥无效".to_string())?;
    mac.update(canonical_envelope(direction, envelope, payload_bytes).as_bytes());
    mac.verify_slice(&provided)
        .map_err(|_| "桥接 signature 校验失败".to_string())
}

fn canonical_envelope(
    direction: SignatureDirection,
    envelope: &BridgeEnvelope<Value>,
    payload_bytes: &[u8],
) -> String {
    let digest = URL_SAFE_NO_PAD.encode(Sha256::digest(payload_bytes));
    format!(
        "protocol_version={}\ndirection={}\ntype={}\nrequest_id={}\nserver_id={}\ninstance_id={}\nsession_id={}\nseq={}\nsent_at={}\npayload_sha256={}",
        envelope.protocol_version,
        direction.as_str(),
        envelope.message_type,
        canonical_optional(envelope.request_id.as_deref()),
        canonical_required(&envelope.server_id),
        canonical_required(&envelope.instance_id),
        canonical_optional(envelope.session_id.as_deref()),
        envelope.sequence,
        envelope.sent_at,
        digest,
    )
}

fn derive_session_key(
    token: &str,
    direction: SignatureDirection,
    server_id: &str,
    instance_id: &str,
    client_nonce: &str,
    server_nonce: &str,
    session_id: &str,
) -> Result<Vec<u8>, String> {
    let mut mac =
        HmacSha256::new_from_slice(token.as_bytes()).map_err(|_| "桥接凭据无效".to_string())?;
    let canonical = format!(
        "sculk-catalyst-bridge-v2\ndirection={}\nserver_id={}\ninstance_id={}\nclient_nonce={}\nserver_nonce={}\nsession_id={}",
        direction.as_str(),
        canonical_required(server_id),
        canonical_required(instance_id),
        client_nonce,
        server_nonce,
        canonical_required(session_id),
    );
    mac.update(canonical.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn canonical_required(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn canonical_optional(value: Option<&str>) -> String {
    value
        .filter(|value| !value.is_empty())
        .map(canonical_required)
        .unwrap_or_else(|| "-".into())
}

fn timestamp_is_current(timestamp: i64, max_age: Duration) -> bool {
    if timestamp <= 0 {
        return false;
    }
    let now = unix_millis();
    let oldest = now.saturating_sub(max_age.as_millis().min(i64::MAX as u128) as i64);
    let newest = now.saturating_add(CLOCK_SKEW.as_millis().min(i64::MAX as u128) as i64);
    (oldest..=newest).contains(&timestamp)
}

fn random_nonce() -> String {
    URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes())
}

fn valid_nonce(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_capability(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn valid_token(token: &str) -> bool {
    let trimmed = token.trim();
    trimmed.len() >= 24
        && trimmed.len() <= 256
        && !matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "change-me" | "changeme" | "example"
        )
}

fn valid_server_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_instance_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn valid_player_name(value: &str) -> bool {
    let length = value.chars().count();
    (3..=16).contains(&length)
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TOKEN: &str = "a-bridge-token-with-more-than-24-chars";

    struct TestSession {
        connection_id: Uuid,
        session_id: String,
        client_key: Vec<u8>,
        server_key: Vec<u8>,
    }

    fn test_snapshot(uuid: Uuid) -> BridgePlayerSnapshot {
        BridgePlayerSnapshot {
            uuid,
            name: "player".into(),
            online: true,
            observed_at: unix_millis(),
            level: Some(10),
            experience_progress: Some(0.5),
            total_experience: Some(100),
            dimension: Some("minecraft:overworld".into()),
            position: Some(BridgePosition {
                x: 12.0,
                y: 64.0,
                z: -8.0,
            }),
            game_mode: Some("SURVIVAL".into()),
            health: Some(20.0),
            food_level: Some(20),
            inventory: Some(BridgeInventoryView { slots: Vec::new() }),
            ender_chest: Some(BridgeInventoryView { slots: Vec::new() }),
            papi: HashMap::new(),
        }
    }

    async fn install_test_session(
        runtime: &BridgeRuntime,
        sender: mpsc::Sender<String>,
    ) -> TestSession {
        let connection_id = Uuid::new_v4();
        let session_id = Uuid::new_v4().to_string();
        let client_nonce = "client_nonce_0123456789";
        let server_nonce = "server_nonce_0123456789";
        let client_key = derive_session_key(
            TEST_TOKEN,
            SignatureDirection::ClientToServer,
            "survival",
            "instance-a",
            client_nonce,
            server_nonce,
            &session_id,
        )
        .unwrap();
        let server_key = derive_session_key(
            TEST_TOKEN,
            SignatureDirection::ServerToClient,
            "survival",
            "instance-a",
            client_nonce,
            server_nonce,
            &session_id,
        )
        .unwrap();
        runtime.inner.write().await.sessions.insert(
            "survival".into(),
            BridgeSession {
                connection_id,
                instance_id: "instance-a".into(),
                session_id: session_id.clone(),
                client_key: client_key.clone(),
                server_key: server_key.clone(),
                sender,
                last_seen: Instant::now(),
                last_sequence: 1,
                outbound_sequence: 1,
                capabilities: vec!["snapshot".into(), "papi_read".into()],
            },
        );
        TestSession {
            connection_id,
            session_id,
            client_key,
            server_key,
        }
    }

    fn client_frame(
        session: &TestSession,
        message_type: &str,
        request_id: Option<String>,
        sequence: u64,
        sent_at: i64,
        payload: Value,
    ) -> ReceivedEnvelope {
        let mut envelope = BridgeEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_type: message_type.into(),
            request_id,
            server_id: "survival".into(),
            instance_id: "instance-a".into(),
            session_id: Some(session.session_id.clone()),
            sequence,
            sent_at,
            payload,
            signature: None,
        };
        let payload_bytes = serde_json::to_vec(&envelope.payload).unwrap();
        envelope.signature = Some(
            sign_envelope(
                &session.client_key,
                SignatureDirection::ClientToServer,
                &envelope,
                &payload_bytes,
            )
            .unwrap(),
        );
        ReceivedEnvelope {
            envelope,
            payload_bytes,
        }
    }

    #[test]
    fn signed_wire_round_trip_binds_payload_and_challenge() {
        let payload = json!({
            "client_nonce": "client_nonce_0123456789",
            "server_nonce": "server_nonce_0123456789",
            "capabilities": ["presence"],
        });
        let envelope = BridgeEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_type: "hello".into(),
            request_id: None,
            server_id: "survival".into(),
            instance_id: "instance-a".into(),
            session_id: None,
            sequence: 2,
            sent_at: unix_millis(),
            payload,
            signature: None,
        };
        let wire =
            encode_signed_envelope(&envelope, TEST_TOKEN.as_bytes(), SignatureDirection::Hello)
                .unwrap();
        let received = parse_wire_envelope(&wire).unwrap();
        verify_envelope_signature(
            TEST_TOKEN.as_bytes(),
            SignatureDirection::Hello,
            &received.envelope,
            &received.payload_bytes,
            received.envelope.signature.as_deref(),
        )
        .unwrap();

        let mut tampered = received;
        tampered.envelope.payload["server_nonce"] = Value::String("other_nonce_0123456789".into());
        tampered.payload_bytes = serde_json::to_vec(&tampered.envelope.payload).unwrap();
        assert!(
            verify_envelope_signature(
                TEST_TOKEN.as_bytes(),
                SignatureDirection::Hello,
                &tampered.envelope,
                &tampered.payload_bytes,
                tampered.envelope.signature.as_deref(),
            )
            .is_err()
        );
    }

    #[test]
    fn v2_hmac_test_vector_matches_the_paper_bridge() {
        let payload_bytes = br#"{"player_uuid":"b54cf3f8-9f51-4c4c-a3d1-9584fce2822c"}"#.to_vec();
        let envelope = BridgeEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_type: "snapshot_request".into(),
            request_id: Some("req-42".into()),
            server_id: "server-a".into(),
            instance_id: "instance-a".into(),
            session_id: Some("session-a".into()),
            sequence: 7,
            sent_at: 1_725_000_000_000,
            payload: json!({"player_uuid": "b54cf3f8-9f51-4c4c-a3d1-9584fce2822c"}),
            signature: None,
        };

        assert_eq!(
            canonical_envelope(
                SignatureDirection::ClientToServer,
                &envelope,
                &payload_bytes,
            ),
            concat!(
                "protocol_version=2\n",
                "direction=c2s\n",
                "type=snapshot_request\n",
                "request_id=cmVxLTQy\n",
                "server_id=c2VydmVyLWE\n",
                "instance_id=aW5zdGFuY2UtYQ\n",
                "session_id=c2Vzc2lvbi1h\n",
                "seq=7\n",
                "sent_at=1725000000000\n",
                "payload_sha256=_EaLh-ZpGS1szc1BW_QlBkBYlRTvN2uiNb2dYfwB3Qk",
            )
        );
        assert_eq!(
            sign_envelope(
                b"an-example-secret-token",
                SignatureDirection::ClientToServer,
                &envelope,
                &payload_bytes,
            )
            .unwrap(),
            "VY3g3hic8F-GrBVsE6VUvu2CTEh2LEzPdDj9Bho7z34"
        );
    }

    #[test]
    fn rejects_wire_envelopes_without_the_exact_v2_field_set() {
        let envelope = BridgeEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_type: "hello_init".into(),
            request_id: None,
            server_id: "survival".into(),
            instance_id: "instance-a".into(),
            session_id: None,
            sequence: 1,
            sent_at: unix_millis(),
            payload: json!({"client_nonce": "client_nonce_0123456789"}),
            signature: None,
        };
        let wire = encode_unsigned_envelope(&envelope).unwrap();

        let mut missing_field: Value = serde_json::from_str(&wire).unwrap();
        missing_field.as_object_mut().unwrap().remove("signature");
        assert!(parse_wire_envelope(&serde_json::to_string(&missing_field).unwrap()).is_err());

        let mut extra_field: Value = serde_json::from_str(&wire).unwrap();
        extra_field
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), Value::Bool(true));
        assert!(parse_wire_envelope(&serde_json::to_string(&extra_field).unwrap()).is_err());
    }

    #[test]
    fn rejects_unbounded_snapshot_shape() {
        let mut snapshot = test_snapshot(Uuid::new_v4());
        snapshot.inventory = Some(BridgeInventoryView {
            slots: (0..257)
                .map(|slot| BridgeInventorySlot { slot, item: None })
                .collect(),
        });
        assert!(validate_snapshot(&snapshot).is_err());
    }

    #[test]
    fn rejects_non_minecraft_player_names() {
        let mut snapshot = test_snapshot(Uuid::new_v4());
        snapshot.name = "player-name".into();

        assert!(validate_snapshot(&snapshot).is_err());
        assert!(
            validate_presence(&BridgePresence {
                uuid: snapshot.uuid,
                name: "player.name".into(),
                online: true,
                observed_at: unix_millis(),
            })
            .is_err()
        );
    }

    #[test]
    fn rejects_expired_or_future_observations() {
        let mut snapshot = test_snapshot(Uuid::new_v4());
        snapshot.observed_at = unix_millis() - SNAPSHOT_STALE_AFTER.as_millis() as i64 - 1;
        assert!(validate_snapshot(&snapshot).is_err());

        snapshot.observed_at = unix_millis() + CLOCK_SKEW.as_millis() as i64 + 1;
        assert!(validate_snapshot(&snapshot).is_err());
    }

    #[tokio::test]
    async fn snapshot_request_encodes_and_resolves_matching_response() {
        let runtime = BridgeRuntime::with_token("survival", TEST_TOKEN);
        let (sender, mut receiver) = mpsc::channel(2);
        let session = install_test_session(&runtime, sender).await;
        let player_uuid = Uuid::new_v4();
        let request_runtime = runtime.clone();
        let request = tokio::spawn(async move {
            request_runtime
                .request_snapshot("survival", player_uuid)
                .await
        });

        let outgoing = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let received = parse_wire_envelope(&outgoing).unwrap();
        let envelope = received.envelope;
        assert_eq!(envelope.message_type, "snapshot_request");
        assert_eq!(envelope.server_id, "survival");
        assert_eq!(envelope.instance_id, "instance-a");
        assert_eq!(envelope.sequence, 2);
        assert_eq!(
            envelope.payload.get("player_uuid").and_then(Value::as_str),
            Some(player_uuid.to_string().as_str())
        );
        assert_eq!(
            envelope.payload.get("sections"),
            Some(&json!(["basic", "inventory", "ender_chest"]))
        );
        verify_envelope_signature(
            &session.server_key,
            SignatureDirection::ServerToClient,
            &envelope,
            &received.payload_bytes,
            envelope.signature.as_deref(),
        )
        .unwrap();

        let request_id = envelope.request_id.unwrap();
        let snapshot = test_snapshot(player_uuid);
        runtime
            .accept_envelope(
                session.connection_id,
                client_frame(
                    &session,
                    "snapshot_response",
                    Some(request_id),
                    2,
                    unix_millis(),
                    json!({"status": "ok", "snapshot": snapshot}),
                ),
            )
            .await
            .unwrap();

        let snapshot = request.await.unwrap().unwrap();
        assert_eq!(snapshot.snapshot.uuid, player_uuid);
        assert!(runtime.inner.read().await.pending_snapshots.is_empty());
    }

    #[tokio::test]
    async fn snapshot_request_cleans_waiter_when_queue_is_full() {
        let runtime = BridgeRuntime::with_token("survival", TEST_TOKEN);
        let (sender, _receiver) = mpsc::channel(1);
        sender.try_send("occupied".into()).unwrap();
        install_test_session(&runtime, sender).await;

        let error = runtime
            .request_snapshot("survival", Uuid::new_v4())
            .await
            .unwrap_err();
        assert_eq!(error, "快照桥接队列已满或连接已关闭");
        assert!(runtime.inner.read().await.pending_snapshots.is_empty());
    }

    #[tokio::test]
    async fn authenticated_errors_use_complete_envelopes() {
        let runtime = BridgeRuntime::with_token("survival", TEST_TOKEN);
        let (sender, mut receiver) = mpsc::channel(2);
        let session = install_test_session(&runtime, sender).await;

        send_error(
            &runtime,
            "survival",
            "instance-a",
            session.connection_id,
            Some("request-1".into()),
            "invalid_message",
            "测试错误",
        )
        .await
        .unwrap();

        let outgoing = receiver.recv().await.unwrap();
        let received = parse_wire_envelope(&outgoing).unwrap();
        let envelope = received.envelope;
        assert_eq!(envelope.message_type, "error");
        assert_eq!(envelope.request_id.as_deref(), Some("request-1"));
        assert_eq!(envelope.server_id, "survival");
        assert_eq!(envelope.instance_id, "instance-a");
        assert_eq!(envelope.sequence, 2);
        assert_eq!(
            envelope.payload.get("code").and_then(Value::as_str),
            Some("invalid_message")
        );
        verify_envelope_signature(
            &session.server_key,
            SignatureDirection::ServerToClient,
            &envelope,
            &received.payload_bytes,
            envelope.signature.as_deref(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn rejects_invalid_followup_envelope_metadata() {
        let runtime = BridgeRuntime::with_token("survival", TEST_TOKEN);
        let (sender, _receiver) = mpsc::channel(1);
        let session = install_test_session(&runtime, sender).await;

        let error = runtime
            .accept_envelope(
                session.connection_id,
                ReceivedEnvelope {
                    payload_bytes: b"{}".to_vec(),
                    envelope: BridgeEnvelope {
                        protocol_version: PROTOCOL_VERSION + 1,
                        message_type: "heartbeat".into(),
                        request_id: None,
                        server_id: "survival".into(),
                        instance_id: "instance-a".into(),
                        session_id: Some(session.session_id.clone()),
                        sequence: 2,
                        sent_at: unix_millis(),
                        payload: json!({}),
                        signature: None,
                    },
                },
            )
            .await
            .unwrap_err();
        assert!(error.contains("不支持的桥接协议版本"));
    }

    #[tokio::test]
    async fn rejects_tampered_or_excessive_sequence_without_advancing_session() {
        let runtime = BridgeRuntime::with_token("survival", TEST_TOKEN);
        let (sender, _receiver) = mpsc::channel(1);
        let session = install_test_session(&runtime, sender).await;

        let mut tampered = client_frame(
            &session,
            "heartbeat",
            None,
            2,
            unix_millis(),
            json!({"online_count": 1}),
        );
        tampered.envelope.payload = json!({"online_count": 99});
        tampered.payload_bytes = serde_json::to_vec(&tampered.envelope.payload).unwrap();
        assert!(
            runtime
                .accept_envelope(session.connection_id, tampered)
                .await
                .is_err()
        );
        assert_eq!(
            runtime.inner.read().await.sessions["survival"].last_sequence,
            1
        );

        let excessive = client_frame(
            &session,
            "heartbeat",
            None,
            1 + MAX_SEQUENCE_GAP + 1,
            unix_millis(),
            json!({}),
        );
        assert!(
            runtime
                .accept_envelope(session.connection_id, excessive)
                .await
                .is_err()
        );
        assert_eq!(
            runtime.inner.read().await.sessions["survival"].last_sequence,
            1
        );
    }

    #[tokio::test]
    async fn disconnect_clears_online_presence_but_keeps_snapshots_as_cache() {
        let runtime = BridgeRuntime::with_token("survival", TEST_TOKEN);
        let (sender, _receiver) = mpsc::channel(1);
        let session = install_test_session(&runtime, sender).await;
        let player_id = Uuid::new_v4();
        let mut state = runtime.inner.write().await;
        state.presence.insert(
            "survival".into(),
            HashMap::from([(
                player_id,
                BridgePresence {
                    uuid: player_id,
                    name: "player".into(),
                    online: true,
                    observed_at: unix_millis(),
                },
            )]),
        );
        state.snapshots.insert(
            "survival".into(),
            HashMap::from([(
                player_id,
                CachedSnapshot {
                    snapshot: test_snapshot(player_id),
                    received_at: Instant::now(),
                    sequence: 1,
                },
            )]),
        );
        drop(state);

        runtime
            .disconnect("survival", "instance-a", session.connection_id)
            .await;
        assert!(runtime.presences("survival").await.is_empty());
        assert_eq!(runtime.snapshots("survival").await.len(), 1);
    }
}
