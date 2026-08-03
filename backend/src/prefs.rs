use crate::{AppState, internal, persist};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

type ApiError = (StatusCode, String);
type ApiResult<T> = Result<Json<T>, ApiError>;

const LANGUAGES: [&str; 3] = ["auto", "zh-CN", "en-US"];
const BACKGROUND_MODES: [&str; 3] = ["solid", "gradient", "image"];
const FONT_FAMILIES: [&str; 4] = ["default", "system", "serif", "mono"];
const MENU_FONT_FAMILIES: [&str; 5] = ["inherit", "default", "system", "serif", "mono"];
const PROTOCOLS: [&str; 2] = ["ssh", "sftp"];

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct BackgroundSettings {
    pub(crate) mode: String,
    pub(crate) solid: String,
    pub(crate) gradient: String,
    #[serde(default = "default_gradient_colors")]
    pub(crate) gradient_colors: Vec<String>,
    pub(crate) image_url: String,
    /// 背景图之上遮罩的不透明度（0-95），值越大界面越沉、图片越淡。
    pub(crate) image_opacity: u8,
    /// 鑳屾櫙鍥剧墖鐨勪腑蹇冧綅缃笌缂╂斁锛屽潎涓?0-100 鐧惧垎姣斻€?    #[serde(default = "default_image_position")]
    #[serde(default = "default_image_position")]
    pub(crate) image_position_x: u8,
    #[serde(default = "default_image_position")]
    pub(crate) image_position_y: u8,
    #[serde(default = "default_image_scale")]
    pub(crate) image_scale: u8,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            mode: "gradient".into(),
            solid: "#e4c8e1".into(),
            gradient: "mesh".into(),
            gradient_colors: default_gradient_colors(),
            image_url: String::new(),
            image_opacity: 72,
            image_position_x: default_image_position(),
            image_position_y: default_image_position(),
            image_scale: default_image_scale(),
        }
    }
}

fn default_gradient_colors() -> Vec<String> {
    vec!["#f9a9d0".into(), "#a8e5ff".into(), "#5cb3ff".into()]
}

fn default_image_position() -> u8 {
    50
}

fn default_image_scale() -> u8 {
    100
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AppearanceSettings {
    pub(crate) preset: String,
    /// 强调色；预设会写入各自的强调色，自定义预设可任意调整。
    #[serde(default = "default_accent")]
    pub(crate) accent: String,
    #[serde(default)]
    pub(crate) background: BackgroundSettings,
    pub(crate) font_family: String,
    /// 字体大小（UI 缩放百分比，70-150，100 为标准）。
    #[serde(default = "default_font_size")]
    pub(crate) font_size: u16,
    pub(crate) font_color: String,
    #[serde(default = "default_menu_font_family")]
    pub(crate) menu_font_family: String,
    #[serde(default)]
    pub(crate) menu_font_color: String,
    #[serde(default = "default_card_blur")]
    pub(crate) card_blur: u8,
}

fn default_accent() -> String {
    "#5cb3ff".into()
}

fn default_font_size() -> u16 {
    130
}

fn default_menu_font_family() -> String {
    "inherit".into()
}

fn default_card_blur() -> u8 {
    6
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            preset: "sculk".into(),
            accent: default_accent(),
            background: BackgroundSettings::default(),
            font_family: "default".into(),
            font_size: default_font_size(),
            font_color: "#85baff".into(),
            menu_font_family: default_menu_font_family(),
            menu_font_color: String::new(),
            card_blur: default_card_blur(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct PersonalizationSettings {
    /// 预设键（default/concise/detailed/humorous/formal）或自定义风格描述。
    #[serde(default)]
    pub(crate) chat_style: String,
    #[serde(default)]
    pub(crate) extra_context: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct GitSettings {
    #[serde(default)]
    pub(crate) username: String,
    #[serde(default)]
    pub(crate) email: String,
    pub(crate) default_branch: String,
    #[serde(default)]
    pub(crate) remote_url: String,
    #[serde(default)]
    pub(crate) auto_commit: bool,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            username: String::new(),
            email: String::new(),
            default_branch: "main".into(),
            remote_url: String::new(),
            auto_commit: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AccountSettings {
    pub(crate) nickname: String,
    #[serde(default)]
    pub(crate) email: String,
}

impl Default for AccountSettings {
    fn default() -> Self {
        Self {
            nickname: "服主".into(),
            email: String::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct RemoteConnection {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) protocol: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    #[serde(default)]
    pub(crate) username: String,
    #[serde(default)]
    pub(crate) root_path: String,
    pub(crate) enabled: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct UiSettings {
    pub(crate) language: String,
    #[serde(default)]
    pub(crate) appearance: AppearanceSettings,
    #[serde(default)]
    pub(crate) personalization: PersonalizationSettings,
    #[serde(default)]
    pub(crate) git: GitSettings,
    #[serde(default)]
    pub(crate) account: AccountSettings,
    #[serde(default)]
    pub(crate) connections: Vec<RemoteConnection>,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            language: "auto".into(),
            appearance: AppearanceSettings::default(),
            personalization: PersonalizationSettings::default(),
            git: GitSettings::default(),
            account: AccountSettings::default(),
            connections: Vec::new(),
        }
    }
}

/// 按对话语言设置生成注入系统提示的语言指令。
pub(crate) fn language_directive(language: &str) -> &'static str {
    match language {
        "en-US" => "Reply in English.",
        _ => "请使用简体中文回答。",
    }
}

/// 将个性化设置拼成系统提示附加段：风格 + 额外上下文。
pub(crate) fn persona_directive(personalization: &PersonalizationSettings) -> String {
    let mut parts = Vec::new();
    let style = personalization.chat_style.trim();
    let mapped = match style {
        "" | "default" => "",
        "concise" => "回答尽量简短直接，只给结论与关键步骤。",
        "detailed" => "回答尽量详尽，补充原理说明与可选方案。",
        "humorous" => "语气轻松幽默，可以适度玩梗，但保证信息准确。",
        "formal" => "使用正式、专业的书面语气。",
        custom => custom,
    };
    if !mapped.is_empty() {
        parts.push(format!("对话风格要求：{mapped}"));
    }
    let context = personalization.extra_context.trim();
    if !context.is_empty() {
        parts.push(format!("用户提供的额外背景信息：{context}"));
    }
    parts.join("\n")
}

#[derive(Deserialize)]
struct UpdateUiRequest {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    appearance: Option<AppearanceSettings>,
    #[serde(default)]
    personalization: Option<PersonalizationSettings>,
    #[serde(default)]
    git: Option<GitSettings>,
    #[serde(default)]
    account: Option<AccountSettings>,
}

#[derive(Deserialize)]
struct UpsertConnectionRequest {
    name: String,
    protocol: String,
    host: String,
    port: u16,
    #[serde(default)]
    username: String,
    #[serde(default)]
    root_path: String,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Serialize)]
struct ConnectionTestResult {
    ok: bool,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/ui/settings", get(get_settings).put(update_settings))
        .route("/api/ui/connections", post(create_connection))
        .route(
            "/api/ui/connections/{id}",
            put(update_connection).delete(delete_connection),
        )
        .route("/api/ui/connections/{id}/test", post(test_connection))
}

fn validate_appearance(appearance: &AppearanceSettings) -> Result<(), ApiError> {
    if !BACKGROUND_MODES.contains(&appearance.background.mode.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "invalid background mode".into()));
    }
    if appearance.background.image_opacity > 95 {
        return Err((
            StatusCode::BAD_REQUEST,
            "image_opacity 取值范围 0-95".into(),
        ));
    }
    if appearance.background.image_position_x > 100 || appearance.background.image_position_y > 100
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "image_position_x/image_position_y 取值范围 0-100".into(),
        ));
    }
    if !(75..=200).contains(&appearance.background.image_scale) {
        return Err((
            StatusCode::BAD_REQUEST,
            "image_scale 取值范围 75-200".into(),
        ));
    }
    if !(2..=5).contains(&appearance.background.gradient_colors.len()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "gradient_colors 需要包含 2-5 个颜色".into(),
        ));
    }
    if !(70..=150).contains(&appearance.font_size) {
        return Err((StatusCode::BAD_REQUEST, "font_size 取值范围 70-150".into()));
    }
    if !FONT_FAMILIES.contains(&appearance.font_family.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "invalid font_family".into()));
    }
    if !MENU_FONT_FAMILIES.contains(&appearance.menu_font_family.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "invalid menu_font_family".into()));
    }
    if !appearance.menu_font_color.is_empty() && !is_hex_color(&appearance.menu_font_color) {
        return Err((
            StatusCode::BAD_REQUEST,
            "menu_font_color 需要使用 #RRGGBB 格式".into(),
        ));
    }
    if appearance.card_blur > 40 {
        return Err((StatusCode::BAD_REQUEST, "card_blur 取值范围 0-40".into()));
    }
    Ok(())
}

fn is_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit)
}

async fn get_settings(State(state): State<AppState>) -> Json<UiSettings> {
    let data = state.inner.read().await;
    Json(data.ui.clone())
}

async fn update_settings(
    State(state): State<AppState>,
    Json(request): Json<UpdateUiRequest>,
) -> ApiResult<UiSettings> {
    if let Some(language) = &request.language
        && !LANGUAGES.contains(&language.as_str())
    {
        return Err((StatusCode::BAD_REQUEST, "invalid language".into()));
    }
    if let Some(appearance) = &request.appearance {
        validate_appearance(appearance)?;
    }
    let mut data = state.inner.write().await;
    if let Some(language) = request.language {
        data.ui.language = language;
    }
    if let Some(appearance) = request.appearance {
        data.ui.appearance = appearance;
    }
    if let Some(personalization) = request.personalization {
        data.ui.personalization = personalization;
    }
    if let Some(git) = request.git {
        data.ui.git = git;
    }
    if let Some(account) = request.account {
        data.ui.account = account;
    }
    let view = data.ui.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

fn validate_connection(request: &UpsertConnectionRequest) -> Result<(String, String), ApiError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "连接名称不能为空".into()));
    }
    if !PROTOCOLS.contains(&request.protocol.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "协议仅支持 ssh / sftp".into()));
    }
    let host = request.host.trim().to_string();
    if host.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "主机地址不能为空".into()));
    }
    if request.port == 0 {
        return Err((StatusCode::BAD_REQUEST, "端口无效".into()));
    }
    Ok((name, host))
}

async fn create_connection(
    State(state): State<AppState>,
    Json(request): Json<UpsertConnectionRequest>,
) -> ApiResult<UiSettings> {
    let (name, host) = validate_connection(&request)?;
    let connection = RemoteConnection {
        id: format!("conn-{}", &Uuid::new_v4().simple().to_string()[..8]),
        name,
        protocol: request.protocol,
        host,
        port: request.port,
        username: request.username.trim().to_string(),
        root_path: request.root_path.trim().to_string(),
        enabled: request.enabled.unwrap_or(true),
    };
    let mut data = state.inner.write().await;
    data.ui.connections.push(connection);
    let view = data.ui.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn update_connection(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<UpsertConnectionRequest>,
) -> ApiResult<UiSettings> {
    let (name, host) = validate_connection(&request)?;
    let mut data = state.inner.write().await;
    let connection = data
        .ui
        .connections
        .iter_mut()
        .find(|connection| connection.id == id)
        .ok_or((StatusCode::NOT_FOUND, "connection not found".to_string()))?;
    connection.name = name;
    connection.protocol = request.protocol;
    connection.host = host;
    connection.port = request.port;
    connection.username = request.username.trim().to_string();
    connection.root_path = request.root_path.trim().to_string();
    if let Some(enabled) = request.enabled {
        connection.enabled = enabled;
    }
    let view = data.ui.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn delete_connection(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<UiSettings> {
    let mut data = state.inner.write().await;
    let before = data.ui.connections.len();
    data.ui.connections.retain(|connection| connection.id != id);
    if data.ui.connections.len() == before {
        return Err((StatusCode::NOT_FOUND, "connection not found".into()));
    }
    let view = data.ui.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

/// 连通性测试：对 host:port 做 TCP 连接（5 秒超时），不做协议级认证。
async fn test_connection(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<ConnectionTestResult> {
    let (host, port) = {
        let data = state.inner.read().await;
        let connection = data
            .ui
            .connections
            .iter()
            .find(|connection| connection.id == id)
            .ok_or((StatusCode::NOT_FOUND, "connection not found".to_string()))?;
        (connection.host.clone(), connection.port)
    };
    let started = Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await;
    let latency_ms = started.elapsed().as_millis() as u64;
    let result = match outcome {
        Ok(Ok(_)) => ConnectionTestResult {
            ok: true,
            latency_ms,
            error: None,
        },
        Ok(Err(error)) => ConnectionTestResult {
            ok: false,
            latency_ms,
            error: Some(format!("连接失败：{error}")),
        },
        Err(_) => ConnectionTestResult {
            ok: false,
            latency_ms,
            error: Some("连接超时（5 秒）".into()),
        },
    };
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_default_appearance_matches_the_star_blue_theme() {
        let appearance = AppearanceSettings::default();
        assert_eq!(appearance.preset, "sculk");
        assert_eq!(appearance.accent, "#5cb3ff");
        assert_eq!(appearance.background.mode, "gradient");
        assert_eq!(appearance.background.gradient, "mesh");
        assert_eq!(
            appearance.background.gradient_colors,
            ["#f9a9d0", "#a8e5ff", "#5cb3ff"]
        );
        assert!(appearance.background.image_url.is_empty());
        assert_eq!(appearance.font_family, "default");
        assert_eq!(appearance.font_size, 130);
        assert_eq!(appearance.font_color, "#85baff");
        assert_eq!(appearance.menu_font_family, "inherit");
        assert!(appearance.menu_font_color.is_empty());
        assert_eq!(appearance.card_blur, 6);
    }

    #[test]
    fn legacy_appearance_defaults_menu_typography_without_losing_other_values() {
        let value = serde_json::json!({
            "preset": "custom",
            "accent": "#5cb3ff",
            "background": BackgroundSettings::default(),
            "font_family": "serif",
            "font_size": 120,
            "font_color": "#abcdef",
            "card_blur": 8
        });
        let appearance: AppearanceSettings = serde_json::from_value(value).unwrap();

        assert_eq!(appearance.font_family, "serif");
        assert_eq!(appearance.font_size, 120);
        assert_eq!(appearance.menu_font_family, "inherit");
        assert!(appearance.menu_font_color.is_empty());
    }

    #[test]
    fn menu_typography_rejects_unknown_font_and_invalid_color() {
        let mut appearance = AppearanceSettings::default();
        appearance.menu_font_family = "unknown".into();
        assert!(validate_appearance(&appearance).is_err());

        appearance.menu_font_family = "mono".into();
        appearance.menu_font_color = "red".into();
        assert!(validate_appearance(&appearance).is_err());

        appearance.menu_font_color = "#a1B2c3".into();
        assert!(validate_appearance(&appearance).is_ok());
    }
}
