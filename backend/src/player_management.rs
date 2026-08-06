use super::{AppState, ServerInfo, persist, player_bridge, workspace_directory_for_server};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use cap_std::{ambient_authority, fs::Dir as CapDir};
use chrono::{DateTime, Local, Utc};
use fastnbt::Value as NbtValue;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path as StdPath, PathBuf},
};
use tokio::fs;
use uuid::Uuid;

const MAX_PLAYER_DATA_FILES: usize = 1_000;
const MAX_PLAYER_DATA_BYTES: u64 = 8 * 1024 * 1024;
const MAX_NESTED_ITEMS: usize = 64;
const MAX_CONTAINER_DEPTH: usize = 4;
const MAX_PAPI_FIELDS: usize = 10;
const MAX_PAPI_PLACEHOLDER_BYTES: usize = 128;
const MAX_AUXILIARY_FILE_BYTES: usize = 2 * 1024 * 1024;

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PlayerManagementState {
    #[serde(default)]
    profiles: HashMap<String, PlayerProfileMetadata>,
    #[serde(default)]
    papi_fields: HashMap<String, Vec<PapiField>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PlayerProfileMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PapiField {
    id: Uuid,
    label: String,
    placeholder: String,
    enabled: bool,
}

#[derive(Deserialize)]
struct PlayerListQuery {
    query: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Deserialize)]
struct UpdatePlayerRequest {
    display_name: Option<String>,
    role: Option<String>,
    note: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct UpdatePapiFieldsRequest {
    fields: Vec<PapiFieldInput>,
}

#[derive(Deserialize)]
struct PapiFieldInput {
    id: Option<Uuid>,
    label: String,
    placeholder: String,
    #[serde(default = "default_papi_field_enabled")]
    enabled: bool,
}

fn default_papi_field_enabled() -> bool {
    true
}

#[derive(Clone, Serialize)]
struct PlayerDataSource {
    kind: String,
    available: bool,
    world: Option<String>,
    detail: String,
    #[serde(default)]
    freshness: String,
    #[serde(default)]
    bridge_connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fallback_reason: Option<String>,
}

#[derive(Serialize)]
struct PlayerListResponse {
    source: PlayerDataSource,
    players: Vec<PlayerListItem>,
    total: usize,
    warnings: Vec<String>,
}

#[derive(Clone, Serialize)]
struct PlayerListItem {
    key: String,
    uuid: Option<String>,
    name: String,
    profile: PlayerProfileMetadata,
    status: String,
    source: String,
    level: Option<i32>,
    dimension: Option<String>,
    position: Option<PlayerPosition>,
    game_mode: Option<String>,
    updated_at: Option<String>,
}

#[derive(Serialize)]
struct PlayerDetailResponse {
    player: PlayerDetail,
    source: PlayerDataSource,
}

#[derive(Clone, Serialize)]
struct PlayerDetail {
    key: String,
    uuid: Option<String>,
    name: String,
    profile: PlayerProfileMetadata,
    status: String,
    source: String,
    level: Option<i32>,
    experience_progress: Option<f32>,
    total_experience: Option<i32>,
    dimension: Option<String>,
    position: Option<PlayerPosition>,
    game_mode: Option<String>,
    health: Option<f32>,
    food_level: Option<i32>,
    updated_at: Option<String>,
    inventory: Option<InventoryView>,
    ender_chest: Option<InventoryView>,
}

#[derive(Clone, Serialize)]
struct PlayerPosition {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Clone, Serialize)]
struct InventoryView {
    slots: Vec<InventorySlot>,
}

#[derive(Clone, Serialize)]
struct InventorySlot {
    slot: i16,
    item: Option<PlayerItem>,
}

#[derive(Clone, Serialize)]
struct PlayerItem {
    id: String,
    count: Option<u32>,
    name: Option<String>,
    lore: Vec<String>,
    container: Option<ContainerPreview>,
}

#[derive(Clone, Serialize)]
struct ContainerPreview {
    kind: String,
    size: usize,
    slots: Vec<InventorySlot>,
}

#[derive(Clone)]
struct PlayerSnapshot {
    level: Option<i32>,
    experience_progress: Option<f32>,
    total_experience: Option<i32>,
    dimension: Option<String>,
    position: Option<PlayerPosition>,
    game_mode: Option<String>,
    health: Option<f32>,
    food_level: Option<i32>,
    inventory: Option<InventoryView>,
    ender_chest: Option<InventoryView>,
}

#[derive(Clone)]
struct PlayerRecord {
    key: String,
    uuid: Option<Uuid>,
    name: String,
    status: String,
    source: String,
    snapshot: Option<PlayerSnapshot>,
    updated_at: Option<String>,
}

#[derive(Serialize)]
struct PapiFieldsResponse {
    detected: bool,
    runtime_available: bool,
    fields: Vec<PapiField>,
    message: String,
}

#[derive(Serialize)]
struct PapiValuesResponse {
    detected: bool,
    runtime_available: bool,
    fields: Vec<PapiValue>,
    message: String,
}

#[derive(Serialize)]
struct PapiValue {
    id: Uuid,
    label: String,
    placeholder: String,
    value: Option<String>,
    status: String,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/servers/{server_id}/players", get(list_players))
        .route(
            "/api/servers/{server_id}/players/{player_key}",
            get(get_player).put(update_player),
        )
        .route(
            "/api/servers/{server_id}/players/{player_key}/papi",
            get(get_player_papi_values),
        )
        .route(
            "/api/servers/{server_id}/papi/fields",
            get(get_papi_fields).put(update_papi_fields),
        )
}

async fn list_players(
    Path(server_id): Path<String>,
    State(state): State<AppState>,
    Query(query): Query<PlayerListQuery>,
) -> ApiResult<PlayerListResponse> {
    let (source, mut records, mut warnings) = collect_player_records(&state, &server_id).await?;
    let metadata = profile_snapshot(&state, &server_id).await;
    let query_text = query.query.unwrap_or_default().trim().to_ascii_lowercase();
    let mut players = records
        .drain(..)
        .map(|record| {
            let profile = metadata.get(&record.key).cloned().unwrap_or_default();
            player_list_item(record, profile)
        })
        .filter(|player| query_text.is_empty() || player_matches(player, &query_text))
        .collect::<Vec<_>>();
    sort_players(&mut players, query.sort.as_deref(), query.order.as_deref());
    if !source.available {
        warnings
            .push("未发现可读取的世界 playerdata；只有受管控制台识别到的在线玩家会显示。".into());
    }
    let total = players.len();
    Ok(Json(PlayerListResponse {
        source,
        players,
        total,
        warnings,
    }))
}

async fn get_player(
    Path((server_id, player_key)): Path<(String, String)>,
    State(state): State<AppState>,
) -> ApiResult<PlayerDetailResponse> {
    let (mut source, records, _) = collect_player_records(&state, &server_id).await?;
    let mut record = records
        .into_iter()
        .find(|record| record.key == player_key)
        .ok_or((StatusCode::NOT_FOUND, "未找到玩家快照或在线记录".into()))?;
    let bridge_status = state.bridge.status(&server_id).await;
    let can_refresh_snapshot = record.status == "online"
        && record.uuid.is_some()
        && bridge_status.connected
        && bridge_status
            .capabilities
            .iter()
            .any(|capability| capability == "snapshot");
    if can_refresh_snapshot && let Some(uuid) = record.uuid {
        match state.bridge.request_snapshot(&server_id, uuid).await {
            Ok(live) => {
                let freshness = live.freshness();
                let snapshot = live.snapshot;
                record.name = snapshot.name.clone();
                record.status = if snapshot.online { "online" } else { "offline" }.into();
                record.source = format!("paper_bridge_{freshness}");
                record.snapshot = Some(player_snapshot_from_bridge(&snapshot));
                record.updated_at = bridge_timestamp(snapshot.observed_at);
                source.available = true;
                source.kind = "paper_bridge".into();
                source.freshness = freshness.into();
                source.bridge_connected = true;
                source.fallback_reason = None;
                source.detail = "Paper/Folia 插件已按需返回该玩家的实时快照。".into();
            }
            Err(_) => {
                source.freshness = "stale".into();
                source.fallback_reason = Some("snapshot_request_failed".into());
                source.detail =
                    "Paper/Folia 实时快照请求未完成；详情使用最近桥接缓存或 playerdata 兜底。"
                        .into();
            }
        }
    }
    let profile = profile_snapshot(&state, &server_id)
        .await
        .get(&record.key)
        .cloned()
        .unwrap_or_default();
    Ok(Json(PlayerDetailResponse {
        player: player_detail(record, profile),
        source,
    }))
}

async fn update_player(
    Path((server_id, player_key)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<UpdatePlayerRequest>,
) -> ApiResult<PlayerDetailResponse> {
    let (source, records, _) = collect_player_records(&state, &server_id).await?;
    let record = records
        .into_iter()
        .find(|record| record.key == player_key)
        .ok_or((StatusCode::NOT_FOUND, "未找到玩家快照或在线记录".into()))?;
    let mut data = state.inner.write().await;
    let profile = data
        .player_management
        .profiles
        .entry(profile_storage_key(&server_id, &record.key))
        .or_default();
    if let Some(display_name) = request.display_name {
        profile.display_name = optional_text(display_name, 48, "显示名称")?;
    }
    if let Some(role) = request.role {
        profile.role = optional_text(role, 48, "身份")?;
    }
    if let Some(note) = request.note {
        profile.note = optional_text(note, 500, "备注")?;
    }
    if let Some(tags) = request.tags {
        profile.tags = normalize_tags(tags)?;
    }
    profile.updated_at = Some(Local::now().to_rfc3339());
    let response_profile = profile.clone();
    persist(&state, &data).await.map_err(internal_error)?;
    Ok(Json(PlayerDetailResponse {
        player: player_detail(record, response_profile),
        source,
    }))
}

async fn get_papi_fields(
    Path(server_id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<PapiFieldsResponse> {
    let _ = player_server_context(&state, &server_id).await?;
    let (detected, runtime_available, message) =
        papi_fields_runtime_status(&state, &server_id).await;
    let fields = state
        .inner
        .read()
        .await
        .player_management
        .papi_fields
        .get(&server_id)
        .cloned()
        .unwrap_or_default();
    Ok(Json(PapiFieldsResponse {
        detected,
        runtime_available,
        fields,
        message,
    }))
}

async fn update_papi_fields(
    Path(server_id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<UpdatePapiFieldsRequest>,
) -> ApiResult<PapiFieldsResponse> {
    let _ = player_server_context(&state, &server_id).await?;
    let fields = normalize_papi_fields(request.fields)?;
    let (detected, runtime_available, message) =
        papi_fields_runtime_status(&state, &server_id).await;
    let mut data = state.inner.write().await;
    data.player_management
        .papi_fields
        .insert(server_id.clone(), fields.clone());
    persist(&state, &data).await.map_err(internal_error)?;
    Ok(Json(PapiFieldsResponse {
        detected,
        runtime_available,
        fields,
        message,
    }))
}

async fn get_player_papi_values(
    Path((server_id, player_key)): Path<(String, String)>,
    State(state): State<AppState>,
) -> ApiResult<PapiValuesResponse> {
    let _ = player_server_context(&state, &server_id).await?;
    let (_, records, _) = collect_player_records(&state, &server_id).await?;
    let record = records
        .into_iter()
        .find(|record| record.key == player_key)
        .ok_or((StatusCode::NOT_FOUND, "未找到玩家快照或在线记录".into()))?;
    let fields = state
        .inner
        .read()
        .await
        .player_management
        .papi_fields
        .get(&server_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|field| field.enabled)
        .collect::<Vec<_>>();
    let bridge_status = state.bridge.status(&server_id).await;
    if bridge_status.connected {
        let papi_available = bridge_supports_papi(&bridge_status.capabilities);
        if !papi_available {
            return Ok(Json(PapiValuesResponse {
                detected: false,
                runtime_available: true,
                fields: unavailable_papi_values(&fields, "bridge_capability_unavailable"),
                message: "Paper/Folia 桥接已连接，但插件未声明 papi_read 能力，无法解析 PlaceholderAPI 变量。"
                    .into(),
            }));
        }
        if fields.is_empty() {
            return Ok(Json(PapiValuesResponse {
                detected: papi_available,
                runtime_available: true,
                fields: Vec::new(),
                message: "尚未配置要显示的 PlaceholderAPI 变量".into(),
            }));
        }
        let Some(uuid) = record.uuid else {
            return Ok(Json(PapiValuesResponse {
                detected: papi_available,
                runtime_available: true,
                fields: unavailable_papi_values(&fields, "player_unresolved"),
                message: "当前在线记录尚未解析 UUID，不能通过 Paper/Folia 桥接查询变量".into(),
            }));
        };
        let request_fields = fields
            .iter()
            .map(|field| player_bridge::BridgePapiRequestField {
                id: field.id.to_string(),
                placeholder: field.placeholder.clone(),
            })
            .collect();
        return match state
            .bridge
            .request_papi(&server_id, uuid, request_fields)
            .await
        {
            Ok(response) if response.player_uuid == uuid => Ok(Json(PapiValuesResponse {
                detected: papi_available,
                runtime_available: true,
                fields: bridge_papi_values(&fields, &response),
                message: if response.status == "ok" {
                    "通过 Paper/Folia 插件的 PlaceholderAPI API 即时解析".into()
                } else {
                    format!(
                        "Paper/Folia 插件未能解析变量：{}",
                        response.error_code.as_deref().unwrap_or("unavailable")
                    )
                },
            })),
            Ok(_) => Ok(Json(PapiValuesResponse {
                detected: false,
                runtime_available: true,
                fields: unavailable_papi_values(&fields, "response_mismatch"),
                message: "Paper/Folia 桥接返回了不匹配的玩家变量响应".into(),
            })),
            Err(error) => Ok(Json(PapiValuesResponse {
                detected: papi_available,
                runtime_available: true,
                fields: unavailable_papi_values(&fields, "bridge_query_failed"),
                message: format!("Paper/Folia 桥接变量查询失败：{error}"),
            })),
        };
    }
    Ok(Json(bridge_unavailable_papi_values_response(&fields)))
}

fn bridge_papi_values(
    fields: &[PapiField],
    response: &player_bridge::BridgePapiResponse,
) -> Vec<PapiValue> {
    fields
        .iter()
        .map(|field| {
            let key = field.id.to_string();
            let value = response.fields.get(&key);
            PapiValue {
                id: field.id,
                label: field.label.clone(),
                placeholder: field.placeholder.clone(),
                value: value.and_then(|value| value.value.clone()),
                status: value.map(|value| value.status.clone()).unwrap_or_else(|| {
                    response
                        .error_code
                        .clone()
                        .unwrap_or_else(|| "unavailable".into())
                }),
            }
        })
        .collect()
}

fn internal_error(error: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error)
}

async fn player_server_context(
    state: &AppState,
    server_id: &str,
) -> Result<(ServerInfo, PathBuf), (StatusCode, String)> {
    let server = state
        .inner
        .read()
        .await
        .servers
        .iter()
        .find(|server| server.id == server_id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "未找到服务器工作区".into()))?;
    if server.kind != "server" {
        return Err((
            StatusCode::BAD_REQUEST,
            "玩家管理仅适用于 Minecraft 服务器工作区".into(),
        ));
    }
    Ok((server.clone(), workspace_directory_for_server(&server)))
}

async fn collect_player_records(
    state: &AppState,
    server_id: &str,
) -> Result<(PlayerDataSource, Vec<PlayerRecord>, Vec<String>), (StatusCode, String)> {
    let (_, root) = player_server_context(state, server_id).await?;
    let (world, mut source) = player_data_source(&root).await;
    let online_names = online_player_names(state, server_id).await;
    let names = load_player_names(&root).await;
    let bridge_status = state.bridge.status(server_id).await;
    let bridge_snapshots = state.bridge.snapshots(server_id).await;
    let bridge_presence = state.bridge.presences(server_id).await;
    let bridge_connected = bridge_status.connected;
    let mut bridge_by_uuid = bridge_snapshots
        .iter()
        .cloned()
        .map(|snapshot| (snapshot.snapshot.uuid, snapshot))
        .collect::<HashMap<_, _>>();
    let presence_by_uuid = bridge_presence
        .iter()
        .cloned()
        .map(|presence| (presence.uuid, presence))
        .collect::<HashMap<_, _>>();
    let mut warnings = Vec::new();
    let mut records = Vec::new();
    let mut known_online_names = HashSet::new();

    if bridge_connected || !bridge_snapshots.is_empty() || !bridge_presence.is_empty() {
        source.available = true;
        source.bridge_connected = bridge_connected;
        source.kind = "paper_bridge".into();
        source.freshness = bridge_snapshots
            .iter()
            .map(player_bridge::BridgeSnapshotView::freshness)
            .min_by_key(|freshness| match *freshness {
                "live" => 0,
                "stale" => 1,
                _ => 2,
            })
            .unwrap_or(if bridge_connected {
                "connected"
            } else {
                "stale"
            })
            .into();
        source.detail = if bridge_connected {
            "Paper/Folia 插件桥接在线；玩家实时数据优先于 playerdata。".into()
        } else {
            "Paper/Folia 插件桥接暂时断开；当前显示的是最近桥接快照或 playerdata 兜底。".into()
        };
        if !bridge_connected {
            source.fallback_reason = Some("paper_bridge_unavailable".into());
        }
    }

    if let Some(world) = world {
        match collect_player_data_files(&root, &world).await {
            Ok(players) => {
                for player in players {
                    let uuid = player.uuid;
                    let fallback_name = names
                        .get(&uuid)
                        .cloned()
                        .unwrap_or_else(|| short_uuid(&uuid));
                    let telemetry_online = online_names
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(&fallback_name));
                    if let Some(error) = player.error {
                        warnings.push(format!("无法读取玩家数据 {uuid}：{error}"));
                    }
                    if let Some(live) = bridge_by_uuid.remove(&uuid) {
                        let live_snapshot = live.snapshot.clone();
                        if live_snapshot.online {
                            known_online_names.insert(live_snapshot.name.to_ascii_lowercase());
                        }
                        records.push(PlayerRecord {
                            key: uuid.to_string(),
                            uuid: Some(uuid),
                            name: live_snapshot.name.clone(),
                            status: if live_snapshot.online {
                                "online"
                            } else {
                                "offline"
                            }
                            .into(),
                            source: format!("paper_bridge_{}", live.freshness()),
                            snapshot: Some(player_snapshot_from_bridge(&live_snapshot)),
                            updated_at: bridge_timestamp(live_snapshot.observed_at)
                                .or(player.updated_at),
                        });
                        continue;
                    }
                    let presence = presence_by_uuid.get(&uuid);
                    let online = presence.map_or(telemetry_online, |presence| presence.online);
                    let name = presence
                        .map(|presence| presence.name.clone())
                        .unwrap_or(fallback_name);
                    if online {
                        known_online_names.insert(name.to_ascii_lowercase());
                    }
                    let source_name = if presence.is_some() {
                        "paper_bridge_presence+world_playerdata"
                    } else {
                        "world_playerdata"
                    };
                    records.push(PlayerRecord {
                        key: uuid.to_string(),
                        uuid: Some(uuid),
                        name,
                        status: if bridge_connected && presence.is_none() {
                            "unknown"
                        } else if online {
                            "online"
                        } else {
                            "offline"
                        }
                        .into(),
                        source: source_name.into(),
                        snapshot: player.snapshot,
                        updated_at: player.updated_at,
                    });
                }
            }
            Err(error) => warnings.push(error),
        }
    }

    for live in bridge_by_uuid.into_values() {
        let freshness = live.freshness();
        let snapshot = live.snapshot;
        if snapshot.online {
            known_online_names.insert(snapshot.name.to_ascii_lowercase());
        }
        records.push(PlayerRecord {
            key: snapshot.uuid.to_string(),
            uuid: Some(snapshot.uuid),
            name: snapshot.name.clone(),
            status: if snapshot.online { "online" } else { "offline" }.into(),
            source: format!("paper_bridge_{freshness}"),
            snapshot: Some(player_snapshot_from_bridge(&snapshot)),
            updated_at: bridge_timestamp(snapshot.observed_at),
        });
    }

    for presence in bridge_presence {
        if records
            .iter()
            .any(|record| record.uuid == Some(presence.uuid))
        {
            continue;
        }
        if presence.online {
            known_online_names.insert(presence.name.to_ascii_lowercase());
        }
        records.push(PlayerRecord {
            key: presence.uuid.to_string(),
            uuid: Some(presence.uuid),
            name: presence.name,
            status: if presence.online { "online" } else { "offline" }.into(),
            source: "paper_bridge_presence".into(),
            snapshot: None,
            updated_at: bridge_timestamp(presence.observed_at),
        });
    }

    if !bridge_connected {
        for name in online_names {
            if known_online_names.contains(&name.to_ascii_lowercase()) {
                continue;
            }
            records.push(PlayerRecord {
                key: runtime_player_key(&name),
                uuid: None,
                name,
                status: "online".into(),
                source: "managed_console".into(),
                snapshot: None,
                updated_at: None,
            });
        }
    }
    Ok((source, records, warnings))
}

fn player_snapshot_from_bridge(snapshot: &player_bridge::BridgePlayerSnapshot) -> PlayerSnapshot {
    PlayerSnapshot {
        level: snapshot.level,
        experience_progress: snapshot.experience_progress,
        total_experience: snapshot.total_experience,
        dimension: snapshot.dimension.clone(),
        position: snapshot.position.as_ref().map(|position| PlayerPosition {
            x: position.x,
            y: position.y,
            z: position.z,
        }),
        game_mode: snapshot.game_mode.clone(),
        health: snapshot.health,
        food_level: snapshot.food_level,
        inventory: snapshot.inventory.as_ref().map(bridge_inventory),
        ender_chest: snapshot.ender_chest.as_ref().map(bridge_inventory),
    }
}

fn bridge_inventory(inventory: &player_bridge::BridgeInventoryView) -> InventoryView {
    InventoryView {
        slots: inventory.slots.iter().map(bridge_slot).collect(),
    }
}

fn bridge_slot(slot: &player_bridge::BridgeInventorySlot) -> InventorySlot {
    InventorySlot {
        slot: slot.slot,
        item: slot.item.as_ref().map(bridge_item),
    }
}

fn bridge_item(item: &player_bridge::BridgeItem) -> PlayerItem {
    PlayerItem {
        id: item.id.clone(),
        count: Some(item.count),
        name: item.name.clone(),
        lore: item.lore.clone(),
        container: item.container.as_ref().map(bridge_container),
    }
}

fn bridge_container(container: &player_bridge::BridgeContainerPreview) -> ContainerPreview {
    ContainerPreview {
        kind: container.kind.clone(),
        size: container.size,
        slots: container.slots.iter().map(bridge_slot).collect(),
    }
}

fn bridge_timestamp(timestamp: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(timestamp).map(|value| value.to_rfc3339())
}

async fn player_data_source(root: &StdPath) -> (Option<PathBuf>, PlayerDataSource) {
    let world_name = configured_world_name(root).await;
    let world = world_name
        .as_deref()
        .and_then(valid_world_relative_path)
        .map(|relative| root.join(relative));
    let Some(world) = world else {
        return (
            None,
            PlayerDataSource {
                kind: "world_playerdata".into(),
                available: false,
                world: world_name,
                detail: "server.properties 的 level-name 无效，未读取玩家数据".into(),
                freshness: "unavailable".into(),
                bridge_connected: false,
                fallback_reason: None,
            },
        );
    };
    let available = fs::symlink_metadata(&world)
        .await
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        && fs::symlink_metadata(world.join("playerdata"))
            .await
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
    let detail = if available {
        "来自世界 playerdata 的离线快照；在线时可能存在保存延迟。".into()
    } else {
        "未发现世界 playerdata 目录。启动并保存过玩家数据后会自动显示。".into()
    };
    (
        available.then_some(world),
        PlayerDataSource {
            kind: "world_playerdata".into(),
            available,
            world: world_name,
            detail,
            freshness: if available { "stale" } else { "unavailable" }.into(),
            bridge_connected: false,
            fallback_reason: None,
        },
    )
}

async fn read_workspace_file(
    root: &StdPath,
    relative: &StdPath,
    maximum_bytes: usize,
) -> Option<Vec<u8>> {
    let root = root.to_owned();
    let relative = relative.to_owned();
    tokio::task::spawn_blocking(move || {
        let workspace = CapDir::open_ambient_dir(root, ambient_authority()).ok()?;
        super::reject_workspace_symlink(&workspace, &relative).ok()?;
        let metadata = workspace.symlink_metadata(&relative).ok()?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return None;
        }
        let file = workspace.open(&relative).ok()?;
        let mut bytes = Vec::new();
        file.take((maximum_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() <= maximum_bytes).then_some(bytes)
    })
    .await
    .ok()
    .flatten()
}

async fn configured_world_name(root: &StdPath) -> Option<String> {
    let properties = read_workspace_file(
        root,
        StdPath::new("server.properties"),
        MAX_AUXILIARY_FILE_BYTES,
    )
    .await
    .and_then(|bytes| String::from_utf8(bytes).ok());
    properties
        .as_deref()
        .and_then(|content| {
            content.lines().find_map(|line| {
                let line = line.trim();
                (!line.starts_with('#'))
                    .then(|| line.split_once('='))
                    .flatten()
                    .and_then(|(key, value)| {
                        (key.trim() == "level-name").then(|| value.trim().to_string())
                    })
            })
        })
        .filter(|value| !value.is_empty())
        .or_else(|| Some("world".into()))
}

fn valid_world_relative_path(value: &str) -> Option<PathBuf> {
    if value.len() > 255 || value.chars().any(char::is_control) || value.contains('\\') {
        return None;
    }
    let path = StdPath::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_owned())
}

struct ParsedPlayerData {
    uuid: Uuid,
    snapshot: Option<PlayerSnapshot>,
    updated_at: Option<String>,
    error: Option<String>,
}

async fn collect_player_data_files(
    root: &StdPath,
    world: &StdPath,
) -> Result<Vec<ParsedPlayerData>, String> {
    let root = root.to_owned();
    let world_name = world
        .strip_prefix(root.as_path())
        .map_err(|_| "世界目录不在服务器工作区内".to_string())?
        .to_owned();
    tokio::task::spawn_blocking(move || {
        let workspace = CapDir::open_ambient_dir(root, ambient_authority())
            .map_err(|error| format!("无法打开服务器工作区：{error}"))?;
        (move |workspace: &CapDir| {
            let relative = world_name.join("playerdata");
            super::reject_workspace_symlink(workspace, &relative)
                .map_err(|error| format!("playerdata 路径包含符号链接：{error}"))?;
            let directory_metadata = workspace
                .metadata(&relative)
                .map_err(|error| format!("无法读取 playerdata：{error}"))?;
            if !directory_metadata.is_dir() {
                return Err("playerdata 路径不是目录".into());
            }
            let entries = workspace
                .read_dir(&relative)
                .map_err(|error| format!("读取 playerdata 目录失败：{error}"))?;
            let mut players = Vec::new();
            for entry in entries {
                if players.len() >= MAX_PLAYER_DATA_FILES {
                    return Err(format!(
                        "玩家数据超过 {MAX_PLAYER_DATA_FILES} 份，已拒绝加载"
                    ));
                }
                let entry = entry.map_err(|error| format!("读取玩家数据文件失败：{error}"))?;
                let file_type = entry
                    .file_type()
                    .map_err(|error| format!("读取玩家数据文件类型失败：{error}"))?;
                if file_type.is_symlink() || !file_type.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".dat") {
                    continue;
                }
                let Some(uuid) = name
                    .strip_suffix(".dat")
                    .and_then(|value| Uuid::parse_str(value).ok())
                else {
                    continue;
                };
                let metadata = entry
                    .metadata()
                    .map_err(|error| format!("读取玩家数据元信息失败：{error}"))?;
                if !metadata.is_file() || metadata.len() > MAX_PLAYER_DATA_BYTES {
                    continue;
                }
                let updated_at = metadata.modified().ok().map(|value| {
                    let value: DateTime<Local> = value.into_std().into();
                    value.to_rfc3339()
                });
                let snapshot = match entry.open() {
                    Ok(file) => {
                        let mut compressed = Vec::new();
                        let read_result = file
                            .take(MAX_PLAYER_DATA_BYTES + 1)
                            .read_to_end(&mut compressed);
                        match read_result {
                            Ok(_) if compressed.len() as u64 <= MAX_PLAYER_DATA_BYTES => {
                                match read_gzip_nbt(&compressed) {
                                    Ok(root) => parse_player_snapshot(&root).ok(),
                                    Err(_) => None,
                                }
                            }
                            _ => None,
                        }
                    }
                    Err(_) => None,
                };
                let error = snapshot
                    .is_none()
                    .then(|| "NBT 无法解码或文件读取失败".into());
                players.push(ParsedPlayerData {
                    uuid,
                    snapshot,
                    updated_at,
                    error,
                });
            }
            players.sort_by_key(|player| player.uuid.to_string());
            Ok(players)
        })(&workspace)
    })
    .await
    .map_err(|error| format!("玩家目录读取任务失败：{error}"))?
}

fn read_gzip_nbt(compressed: &[u8]) -> Result<NbtValue, String> {
    let mut decoder = GzDecoder::new(compressed);
    let mut bytes = Vec::new();
    decoder
        .by_ref()
        .take(MAX_PLAYER_DATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("无法解压 NBT：{error}"))?;
    if bytes.len() as u64 > MAX_PLAYER_DATA_BYTES {
        return Err("解压后的玩家数据超出限制".into());
    }
    fastnbt::from_bytes::<NbtValue>(&bytes).map_err(|error| format!("NBT 格式无效：{error}"))
}

fn short_uuid(uuid: &Uuid) -> String {
    format!("{}…", &uuid.simple().to_string()[..8])
}

async fn load_player_names(root: &StdPath) -> HashMap<Uuid, String> {
    let mut result = HashMap::new();
    for filename in [
        "usercache.json",
        "whitelist.json",
        "ops.json",
        "banned-players.json",
    ] {
        let Some(content) =
            read_workspace_file(root, StdPath::new(filename), MAX_AUXILIARY_FILE_BYTES)
                .await
                .and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            continue;
        };
        let Ok(entries) = serde_json::from_str::<Vec<JsonValue>>(&content) else {
            continue;
        };
        for entry in entries {
            let Some(object) = entry.as_object() else {
                continue;
            };
            let Some(name) = object
                .get("name")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|name| is_valid_player_name(name))
            else {
                continue;
            };
            let Some(uuid) = object
                .get("uuid")
                .and_then(JsonValue::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
            else {
                continue;
            };
            result.entry(uuid).or_insert_with(|| name.to_string());
        }
    }
    result
}

async fn online_player_names(state: &AppState, server_id: &str) -> Vec<String> {
    let telemetry = state.telemetry.read().await;
    let Some(record) = telemetry.get(server_id) else {
        return Vec::new();
    };
    if record.value.availability != "available"
        || record
            .player_list_observed_at
            .is_none_or(|observed_at| observed_at.elapsed() >= super::TELEMETRY_STALE_AFTER)
    {
        return Vec::new();
    }
    record
        .value
        .player_names
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|name| is_valid_player_name(name))
        .collect()
}

fn parse_player_snapshot(root: &NbtValue) -> Result<PlayerSnapshot, String> {
    let data = nbt_compound(root).ok_or_else(|| "玩家 NBT 根节点不是 Compound".to_string())?;
    let inventory = nbt_list(data, "Inventory").map(|items| InventoryView {
        slots: fill_slots(&parse_inventory_items(items, 0), &player_inventory_slots()),
    });
    let ender_chest = nbt_list(data, "EnderItems").map(|items| InventoryView {
        slots: fill_slots(
            &parse_inventory_items(items, 0),
            &(0..27).collect::<Vec<_>>(),
        ),
    });
    let level = nbt_i32(data, "XpLevel");
    let experience_progress = nbt_f32(data, "XpP");
    let total_experience = nbt_i32(data, "XpTotal");
    let dimension = nbt_string(data, "Dimension").map(ToString::to_string);
    let position = nbt_list(data, "Pos").and_then(parse_position);
    let game_mode = nbt_i32(data, "playerGameType").map(game_mode_name);
    let health = nbt_f32(data, "Health");
    let food_level = nbt_i32(data, "foodLevel");
    Ok(PlayerSnapshot {
        level,
        experience_progress,
        total_experience,
        dimension,
        position,
        game_mode,
        health,
        food_level,
        inventory,
        ender_chest,
    })
}

fn player_inventory_slots() -> Vec<i16> {
    let mut slots = (0..36).collect::<Vec<i16>>();
    slots.extend([100, 101, 102, 103, -106]);
    slots
}

fn parse_inventory_items(values: &[NbtValue], depth: usize) -> Vec<InventorySlot> {
    values
        .iter()
        .filter_map(|value| {
            let item = nbt_compound(value)?;
            let slot = nbt_i16(item, "Slot")?;
            parse_item(item, Some(slot), depth).map(|item| InventorySlot {
                slot,
                item: Some(item),
            })
        })
        .take(MAX_NESTED_ITEMS)
        .collect()
}

fn fill_slots(items: &[InventorySlot], slots: &[i16]) -> Vec<InventorySlot> {
    let mut by_slot = HashMap::new();
    for item in items {
        by_slot
            .entry(item.slot)
            .or_insert_with(|| item.item.clone());
    }
    slots
        .iter()
        .map(|slot| InventorySlot {
            slot: *slot,
            item: by_slot.remove(slot).flatten(),
        })
        .collect()
}

fn parse_item(
    item: &HashMap<String, NbtValue>,
    slot: Option<i16>,
    depth: usize,
) -> Option<PlayerItem> {
    let id = nbt_string(item, "id")?.to_string();
    let count = nbt_i32(item, "count")
        .or_else(|| nbt_i32(item, "Count"))
        .map(|count| count.max(0) as u32);
    let (name, lore) = item_display_text(item);
    let container = (depth < MAX_CONTAINER_DEPTH)
        .then(|| item_container(item, &id, depth + 1))
        .flatten();
    let _ = slot;
    Some(PlayerItem {
        id,
        count,
        name,
        lore,
        container,
    })
}

fn item_display_text(item: &HashMap<String, NbtValue>) -> (Option<String>, Vec<String>) {
    let components = nbt_compound_by_key(item, "components");
    let modern_name = components
        .and_then(|components| {
            nbt_string(components, "minecraft:custom_name")
                .or_else(|| nbt_string(components, "minecraft:item_name"))
        })
        .map(component_text);
    let modern_lore = components
        .and_then(|components| nbt_list(components, "minecraft:lore"))
        .map(parse_lore)
        .unwrap_or_default();
    if modern_name.is_some() || !modern_lore.is_empty() {
        return (modern_name, modern_lore);
    }
    let display =
        nbt_compound_by_key(item, "tag").and_then(|tag| nbt_compound_by_key(tag, "display"));
    let name = display
        .and_then(|display| nbt_string(display, "Name"))
        .map(component_text);
    let lore = display
        .and_then(|display| nbt_list(display, "Lore"))
        .map(parse_lore)
        .unwrap_or_default();
    (name, lore)
}

fn parse_lore(values: &[NbtValue]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| value.as_str())
        .map(component_text)
        .filter(|value| !value.is_empty())
        .take(12)
        .collect()
}

fn component_text(value: &str) -> String {
    let parsed = serde_json::from_str::<JsonValue>(value).ok();
    let text = parsed
        .as_ref()
        .map(json_component_text)
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| value.to_string());
    let compact = text
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect::<String>();
    compact.trim().to_string()
}

fn json_component_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(value) => value.clone(),
        JsonValue::Array(values) => values.iter().map(json_component_text).collect(),
        JsonValue::Object(values) => {
            let mut result = values
                .get("text")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string();
            if result.is_empty() {
                result = values
                    .get("translate")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_string();
            }
            if let Some(extra) = values.get("extra") {
                result.push_str(&json_component_text(extra));
            }
            result
        }
        _ => String::new(),
    }
}

fn item_container(
    item: &HashMap<String, NbtValue>,
    item_id: &str,
    depth: usize,
) -> Option<ContainerPreview> {
    let kind = container_kind(item_id)?;
    let components = nbt_compound_by_key(item, "components");
    let modern_key = if kind == "bundle" {
        "minecraft:bundle_contents"
    } else {
        "minecraft:container"
    };
    let modern = components
        .and_then(|components| nbt_list(components, modern_key))
        .map(|values| parse_container_entries(values, depth, kind == "bundle"));
    let legacy = nbt_compound_by_key(item, "tag").and_then(|tag| {
        let direct = if kind == "bundle" {
            nbt_list(tag, "BundleContents").or_else(|| nbt_list(tag, "bundle_contents"))
        } else {
            nbt_compound_by_key(tag, "BlockEntityTag")
                .and_then(|block_entity| nbt_list(block_entity, "Items"))
                .or_else(|| nbt_list(tag, "Items"))
        };
        direct.map(|values| parse_container_entries(values, depth, kind == "bundle"))
    });
    let slots = modern.or(legacy)?;
    let size = if kind == "bundle" {
        slots.len().max(1)
    } else {
        27
    };
    Some(ContainerPreview { kind, size, slots })
}

fn container_kind(item_id: &str) -> Option<String> {
    let item_id = item_id.to_ascii_lowercase();
    if item_id.contains("shulker_box") {
        Some("shulker_box".into())
    } else if item_id.ends_with(":bundle") || item_id.ends_with("_bundle") {
        Some("bundle".into())
    } else {
        None
    }
}

fn parse_container_entries(
    values: &[NbtValue],
    depth: usize,
    sequential_slots: bool,
) -> Vec<InventorySlot> {
    let mut entries = Vec::new();
    for (index, value) in values.iter().enumerate().take(MAX_NESTED_ITEMS) {
        let Some(compound) = nbt_compound(value) else {
            continue;
        };
        let nested_item = nbt_compound_by_key(compound, "item").unwrap_or(compound);
        let slot = if sequential_slots {
            index as i16
        } else {
            nbt_i16(compound, "slot")
                .or_else(|| nbt_i16(compound, "Slot"))
                .unwrap_or(index as i16)
        };
        if let Some(item) = parse_item(nested_item, Some(slot), depth) {
            entries.push(InventorySlot {
                slot,
                item: Some(item),
            });
        }
    }
    if sequential_slots {
        return entries;
    }
    fill_slots(&entries, &(0..27).collect::<Vec<_>>())
}

fn nbt_compound(value: &NbtValue) -> Option<&HashMap<String, NbtValue>> {
    match value {
        NbtValue::Compound(value) => Some(value),
        _ => None,
    }
}

fn nbt_compound_by_key<'a>(
    compound: &'a HashMap<String, NbtValue>,
    key: &str,
) -> Option<&'a HashMap<String, NbtValue>> {
    compound.get(key).and_then(nbt_compound)
}

fn nbt_list<'a>(compound: &'a HashMap<String, NbtValue>, key: &str) -> Option<&'a [NbtValue]> {
    match compound.get(key) {
        Some(NbtValue::List(values)) => Some(values),
        _ => None,
    }
}

fn nbt_i32(compound: &HashMap<String, NbtValue>, key: &str) -> Option<i32> {
    compound
        .get(key)
        .and_then(NbtValue::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn nbt_i16(compound: &HashMap<String, NbtValue>, key: &str) -> Option<i16> {
    compound
        .get(key)
        .and_then(NbtValue::as_i64)
        .and_then(|value| i16::try_from(value).ok())
}

fn nbt_f32(compound: &HashMap<String, NbtValue>, key: &str) -> Option<f32> {
    compound
        .get(key)
        .and_then(NbtValue::as_f64)
        .map(|value| value as f32)
}

fn nbt_string<'a>(compound: &'a HashMap<String, NbtValue>, key: &str) -> Option<&'a str> {
    compound.get(key).and_then(NbtValue::as_str)
}

fn parse_position(values: &[NbtValue]) -> Option<PlayerPosition> {
    if values.len() < 3 {
        return None;
    }
    Some(PlayerPosition {
        x: values[0].as_f64()?,
        y: values[1].as_f64()?,
        z: values[2].as_f64()?,
    })
}

fn game_mode_name(value: i32) -> String {
    match value {
        0 => "survival",
        1 => "creative",
        2 => "adventure",
        3 => "spectator",
        _ => "unknown",
    }
    .into()
}

async fn profile_snapshot(
    state: &AppState,
    server_id: &str,
) -> HashMap<String, PlayerProfileMetadata> {
    let prefix = format!("{server_id}:");
    state
        .inner
        .read()
        .await
        .player_management
        .profiles
        .iter()
        .filter_map(|(key, profile)| {
            key.strip_prefix(&prefix)
                .map(|player_key| (player_key.to_string(), profile.clone()))
        })
        .collect()
}

fn profile_storage_key(server_id: &str, player_key: &str) -> String {
    format!("{server_id}:{player_key}")
}

fn runtime_player_key(name: &str) -> String {
    format!("name:{}", name.to_ascii_lowercase())
}

fn player_list_item(record: PlayerRecord, profile: PlayerProfileMetadata) -> PlayerListItem {
    let snapshot = record.snapshot.as_ref();
    PlayerListItem {
        key: record.key,
        uuid: record.uuid.map(|uuid| uuid.to_string()),
        name: record.name,
        profile,
        status: record.status,
        source: record.source,
        level: snapshot.and_then(|snapshot| snapshot.level),
        dimension: snapshot.and_then(|snapshot| snapshot.dimension.clone()),
        position: snapshot.and_then(|snapshot| snapshot.position.clone()),
        game_mode: snapshot.and_then(|snapshot| snapshot.game_mode.clone()),
        updated_at: record.updated_at,
    }
}

fn player_detail(record: PlayerRecord, profile: PlayerProfileMetadata) -> PlayerDetail {
    let snapshot = record.snapshot;
    PlayerDetail {
        key: record.key,
        uuid: record.uuid.map(|uuid| uuid.to_string()),
        name: record.name,
        profile,
        status: record.status,
        source: record.source,
        level: snapshot.as_ref().and_then(|snapshot| snapshot.level),
        experience_progress: snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.experience_progress),
        total_experience: snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.total_experience),
        dimension: snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.dimension.clone()),
        position: snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.position.clone()),
        game_mode: snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.game_mode.clone()),
        health: snapshot.as_ref().and_then(|snapshot| snapshot.health),
        food_level: snapshot.as_ref().and_then(|snapshot| snapshot.food_level),
        updated_at: record.updated_at,
        inventory: snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.inventory.clone()),
        ender_chest: snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.ender_chest.clone()),
    }
}

fn player_matches(player: &PlayerListItem, query: &str) -> bool {
    player.name.to_ascii_lowercase().contains(query)
        || player
            .uuid
            .as_deref()
            .is_some_and(|uuid| uuid.to_ascii_lowercase().contains(query))
        || player
            .profile
            .display_name
            .as_deref()
            .is_some_and(|name| name.to_ascii_lowercase().contains(query))
        || player
            .profile
            .role
            .as_deref()
            .is_some_and(|role| role.to_ascii_lowercase().contains(query))
        || player
            .profile
            .note
            .as_deref()
            .is_some_and(|note| note.to_ascii_lowercase().contains(query))
        || player
            .profile
            .tags
            .iter()
            .any(|tag| tag.to_ascii_lowercase().contains(query))
}

fn sort_players(players: &mut [PlayerListItem], requested_sort: Option<&str>, order: Option<&str>) {
    let sort = requested_sort.unwrap_or("name");
    let descending = matches!(order, Some("desc"));
    players.sort_by(|left, right| {
        let ordering = match sort {
            "status" => player_status_rank(&left.status)
                .cmp(&player_status_rank(&right.status))
                .then_with(|| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                }),
            "level" => left
                .level
                .unwrap_or(-1)
                .cmp(&right.level.unwrap_or(-1))
                .then_with(|| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                }),
            "updated_at" => left.updated_at.cmp(&right.updated_at).then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }),
            _ => left
                .name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase()),
        };
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
}

fn player_status_rank(status: &str) -> u8 {
    match status {
        "online" => 0,
        "offline" => 1,
        _ => 2,
    }
}

fn optional_text(
    value: String,
    max_chars: usize,
    label: &str,
) -> Result<Option<String>, (StatusCode, String)> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > max_chars || value.chars().any(char::is_control) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{label} 长度无效或包含控制字符"),
        ));
    }
    Ok(Some(value.to_string()))
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, (StatusCode, String)> {
    if tags.len() > 8 {
        return Err((StatusCode::BAD_REQUEST, "标签最多 8 个".into()));
    }
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for tag in tags {
        let Some(tag) = optional_text(tag, 32, "标签")? else {
            continue;
        };
        let key = tag.to_ascii_lowercase();
        if seen.insert(key) {
            normalized.push(tag);
        }
    }
    Ok(normalized)
}

fn normalize_papi_fields(
    inputs: Vec<PapiFieldInput>,
) -> Result<Vec<PapiField>, (StatusCode, String)> {
    if inputs.len() > MAX_PAPI_FIELDS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("最多配置 {MAX_PAPI_FIELDS} 个 PlaceholderAPI 变量"),
        ));
    }
    let mut fields = Vec::new();
    let mut labels = HashSet::new();
    let mut ids = HashSet::new();
    for input in inputs {
        let label = optional_text(input.label, 48, "变量名称")?
            .ok_or((StatusCode::BAD_REQUEST, "变量名称不能为空".into()))?;
        if !labels.insert(label.to_ascii_lowercase()) {
            return Err((StatusCode::BAD_REQUEST, "变量名称不能重复".into()));
        }
        let placeholder = input.placeholder.trim().to_string();
        if !is_valid_placeholder(&placeholder) {
            return Err((
                StatusCode::BAD_REQUEST,
                "变量必须为单个安全的 %placeholder% 表达式".into(),
            ));
        }
        let id = input.id.unwrap_or_else(Uuid::new_v4);
        if !ids.insert(id) {
            return Err((StatusCode::BAD_REQUEST, "变量编号不能重复".into()));
        }
        fields.push(PapiField {
            id,
            label,
            placeholder,
            enabled: input.enabled,
        });
    }
    Ok(fields)
}

fn is_valid_placeholder(value: &str) -> bool {
    value.len() >= 3
        && value.len() <= MAX_PAPI_PLACEHOLDER_BYTES
        && value.starts_with('%')
        && value.ends_with('%')
        && value[1..value.len() - 1].chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '_' | '-' | '.' | ':' | '/' | '[' | ']' | '{' | '}'
                )
        })
}

fn is_valid_player_name(value: &str) -> bool {
    let length = value.chars().count();
    (3..=16).contains(&length)
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

async fn papi_fields_runtime_status(state: &AppState, server_id: &str) -> (bool, bool, String) {
    let bridge_status = state.bridge.status(server_id).await;
    if bridge_status.connected {
        return bridge_papi_fields_status(&bridge_status.capabilities);
    }

    bridge_unavailable_papi_fields_status()
}

fn bridge_papi_fields_status(capabilities: &[String]) -> (bool, bool, String) {
    if bridge_supports_papi(capabilities) {
        (
            true,
            true,
            "Paper/Folia 桥接已连接，PlaceholderAPI 变量将通过插件 API 解析。".into(),
        )
    } else {
        (
            false,
            true,
            "Paper/Folia 桥接已连接，但插件未声明 papi_read 能力，无法通过桥接解析 PlaceholderAPI 变量。".into(),
        )
    }
}

fn bridge_supports_papi(capabilities: &[String]) -> bool {
    capabilities
        .iter()
        .any(|capability| capability == "papi_read")
}

fn bridge_unavailable_papi_fields_status() -> (bool, bool, String) {
    (
        false,
        false,
        "Paper/Folia 桥接未连接。为遵守桥接插件的 PlaceholderAPI 白名单，系统不会通过受管控制台解析变量。".into(),
    )
}

fn bridge_unavailable_papi_values_response(fields: &[PapiField]) -> PapiValuesResponse {
    let (detected, runtime_available, message) = bridge_unavailable_papi_fields_status();
    PapiValuesResponse {
        detected,
        runtime_available,
        fields: unavailable_papi_values(fields, "bridge_unavailable"),
        message,
    }
}

fn unavailable_papi_values(fields: &[PapiField], status: &str) -> Vec<PapiValue> {
    fields
        .iter()
        .map(|field| PapiValue {
            id: field.id,
            label: field.label.clone(),
            placeholder: field.placeholder.clone(),
            value: None,
            status: status.into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compound(entries: impl IntoIterator<Item = (&'static str, NbtValue)>) -> NbtValue {
        NbtValue::Compound(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    #[test]
    fn parses_player_inventory_and_modern_shulker_contents() {
        let shulker_item = compound([
            (
                "id",
                NbtValue::String("minecraft:purple_shulker_box".into()),
            ),
            ("count", NbtValue::Int(1)),
            (
                "components",
                compound([(
                    "minecraft:container",
                    NbtValue::List(vec![compound([
                        ("slot", NbtValue::Byte(0)),
                        (
                            "item",
                            compound([
                                ("id", NbtValue::String("minecraft:diamond".into())),
                                ("count", NbtValue::Int(3)),
                            ]),
                        ),
                    ])]),
                )]),
            ),
            ("Slot", NbtValue::Byte(0)),
        ]);
        let root = compound([
            ("XpLevel", NbtValue::Int(24)),
            ("XpP", NbtValue::Float(0.5)),
            ("XpTotal", NbtValue::Int(701)),
            ("Dimension", NbtValue::String("minecraft:overworld".into())),
            (
                "Pos",
                NbtValue::List(vec![
                    NbtValue::Double(12.5),
                    NbtValue::Double(64.0),
                    NbtValue::Double(-3.25),
                ]),
            ),
            ("Inventory", NbtValue::List(vec![shulker_item])),
            ("EnderItems", NbtValue::List(Vec::new())),
        ]);
        let snapshot = parse_player_snapshot(&root).unwrap();
        assert_eq!(snapshot.level, Some(24));
        let inventory = snapshot.inventory.as_ref().unwrap();
        assert_eq!(
            inventory.slots[0].item.as_ref().unwrap().id,
            "minecraft:purple_shulker_box"
        );
        let container = inventory.slots[0]
            .item
            .as_ref()
            .unwrap()
            .container
            .as_ref()
            .unwrap();
        assert_eq!(container.kind, "shulker_box");
        assert_eq!(
            container.slots[0].item.as_ref().unwrap().id,
            "minecraft:diamond"
        );
    }

    #[test]
    fn parses_bundle_contents() {
        let bundle = compound([
            ("id", NbtValue::String("minecraft:bundle".into())),
            ("count", NbtValue::Int(1)),
            (
                "components",
                compound([(
                    "minecraft:bundle_contents",
                    NbtValue::List(vec![compound([
                        ("id", NbtValue::String("minecraft:apple".into())),
                        ("count", NbtValue::Int(8)),
                    ])]),
                )]),
            ),
        ]);
        let item = parse_item(nbt_compound(&bundle).unwrap(), None, 0).unwrap();
        assert_eq!(
            item.container.unwrap().slots[0].item.as_ref().unwrap().id,
            "minecraft:apple"
        );
    }

    #[test]
    fn reports_bridge_papi_capability_for_field_configuration() {
        let available_capabilities = vec!["snapshot".into(), "papi_read".into()];
        let (detected, runtime_available, message) =
            bridge_papi_fields_status(&available_capabilities);
        assert!(detected);
        assert!(runtime_available);
        assert!(message.contains("插件 API"));

        let unavailable_capabilities = vec!["snapshot".into()];
        let (detected, runtime_available, message) =
            bridge_papi_fields_status(&unavailable_capabilities);
        assert!(!detected);
        assert!(runtime_available);
        assert!(message.contains("未声明 papi_read 能力"));
    }

    #[test]
    fn reports_papi_as_unavailable_without_the_bridge() {
        let (detected, runtime_available, message) = bridge_unavailable_papi_fields_status();
        assert!(!detected);
        assert!(!runtime_available);
        assert!(message.contains("不会通过受管控制台"));

        let field = PapiField {
            id: Uuid::nil(),
            label: "等级".into(),
            placeholder: "%player_level%".into(),
            enabled: true,
        };
        let response = bridge_unavailable_papi_values_response(&[field]);
        assert!(!response.detected);
        assert!(!response.runtime_available);
        assert_eq!(response.fields[0].status, "bridge_unavailable");
    }

    #[test]
    fn preserves_missing_nbt_item_count_and_bridge_item_count() {
        let offline_item = compound([("id", NbtValue::String("minecraft:diamond".into()))]);
        let parsed = parse_item(nbt_compound(&offline_item).unwrap(), None, 0).unwrap();
        assert_eq!(parsed.count, None);

        let bridge = player_bridge::BridgeItem {
            id: "minecraft:diamond".into(),
            count: 32,
            name: None,
            lore: Vec::new(),
            container: None,
        };
        assert_eq!(bridge_item(&bridge).count, Some(32));
    }

    #[test]
    fn keeps_missing_snapshot_sections_unavailable() {
        let snapshot = parse_player_snapshot(&compound([])).unwrap();

        assert_eq!(snapshot.level, None);
        assert!(snapshot.inventory.is_none());
        assert!(snapshot.ender_chest.is_none());
    }

    #[test]
    fn rejects_unsafe_placeholder_expressions() {
        assert!(is_valid_placeholder("%player_level%"));
        assert!(!is_valid_placeholder("%player_level% stop"));
        assert!(!is_valid_placeholder("player_level"));
    }
}
