// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

use crate::AppState;
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng as PasswordOsRng},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use std::{env, sync::Arc, time::Instant};
use url::Url;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct CloudRuntime {
    inner: Option<CloudState>,
    message: String,
}

#[derive(Clone)]
struct CloudState {
    db: PgPool,
    redis: redis::aio::ConnectionManager,
    http: reqwest::Client,
    master_key: Arc<[u8; 32]>,
    session_days: i64,
    rate_limit: i64,
}

impl CloudRuntime {
    #[cfg(test)]
    pub(crate) fn disabled_for_test() -> Self {
        Self {
            inner: None,
            message: "测试环境未启用 Sculk Cloud".into(),
        }
    }

    pub(crate) async fn from_env() -> Self {
        if cloud_is_explicitly_disabled() {
            return Self {
                inner: None,
                message: "本地部署已显式禁用 Sculk Cloud".into(),
            };
        }
        let _ = dotenvy::dotenv();
        let _ = dotenvy::from_filename("../.env");
        let database_url = match env::var("DATABASE_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                return Self {
                    inner: None,
                    message: "未配置 DATABASE_URL，Sculk Cloud 当前未启用".into(),
                };
            }
        };
        let redis_url = match env::var("REDIS_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                return Self {
                    inner: None,
                    message: "未配置 REDIS_URL，Sculk Cloud 当前未启用".into(),
                };
            }
        };
        let master_secret = match env::var("SCULK_MASTER_KEY")
            .ok()
            .and_then(|value| validate_master_secret(&value).ok())
        {
            Some(value) => value,
            None => {
                return Self {
                    inner: None,
                    message: "SCULK_MASTER_KEY 必须显式设置为至少 24 个字符的非占位高熵值".into(),
                };
            }
        };

        let setup = async {
            let db = PgPoolOptions::new()
                .max_connections(10)
                .acquire_timeout(std::time::Duration::from_secs(5))
                .connect(&database_url)
                .await
                .map_err(|error| format!("PostgreSQL 连接失败：{error}"))?;
            sqlx::migrate!("./migrations")
                .run(&db)
                .await
                .map_err(|error| format!("数据库迁移失败：{error}"))?;

            let redis_client = redis::Client::open(redis_url)
                .map_err(|error| format!("Redis 地址无效：{error}"))?;
            let redis = redis_client
                .get_connection_manager()
                .await
                .map_err(|error| format!("Redis 连接失败：{error}"))?;

            let digest = Sha256::digest(master_secret.as_bytes());
            let mut master_key = [0_u8; 32];
            master_key.copy_from_slice(&digest);
            let session_days = env::var("SCULK_CLOUD_SESSION_DAYS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(30)
                .clamp(1, 365);
            let rate_limit = env::var("SCULK_CLOUD_RATE_LIMIT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(60)
                .clamp(1, 10_000);

            Ok::<CloudState, String>(CloudState {
                db,
                redis,
                http: reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(90))
                    .build()
                    .map_err(|error| format!("HTTP 客户端创建失败：{error}"))?,
                master_key: Arc::new(master_key),
                session_days,
                rate_limit,
            })
        }
        .await;

        match setup {
            Ok(inner) => {
                println!("Sculk Cloud connected to PostgreSQL and Redis");
                Self {
                    inner: Some(inner),
                    message: "Sculk Cloud 已连接".into(),
                }
            }
            Err(message) => {
                eprintln!("Sculk Cloud disabled: {message}");
                Self {
                    inner: None,
                    message,
                }
            }
        }
    }
}

fn cloud_is_explicitly_disabled() -> bool {
    std::env::var("SCULK_DISABLE_CLOUD")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn validate_master_secret(value: &str) -> Result<String, &'static str> {
    let value = value.trim();
    if value.len() < 24 {
        return Err("SCULK_MASTER_KEY 至少需要 24 个字符");
    }
    if is_known_placeholder_secret(value) {
        return Err("SCULK_MASTER_KEY 不能使用公开占位值");
    }
    Ok(value.to_string())
}

fn is_known_placeholder_secret(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value == "change-me"
        || value == "changeme"
        || value == "example"
        || value.starts_with("replace-with")
        || value.starts_with("replace_with")
        || value.starts_with("change-me")
        || value.starts_with("change_me")
        || value.starts_with("example-")
        || value.starts_with("example_")
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

#[derive(Debug)]
struct CloudError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl CloudError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    fn database(error: sqlx::Error) -> Self {
        eprintln!("Sculk Cloud database error: {error}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "database_error",
            "云服务暂时无法完成该操作",
        )
    }
}

impl IntoResponse for CloudError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorDetail {
                    code: self.code,
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}

type CloudResult<T> = Result<T, CloudError>;

const AGENT_PAIRING_MINUTES: i64 = 10;
const AGENT_ONLINE_SECONDS: i64 = 90;
const AGENT_MAX_CAPABILITIES: usize = 32;
const AGENT_MAX_PERMISSIONS: usize = 4;
const AGENT_PERMISSIONS: [&str; 4] = ["read", "write", "process", "full"];
const AGENT_BOOTSTRAP_CAPABILITIES: [&str; 6] = [
    "heartbeat",
    "tasks-v1",
    "shell-v1",
    "terminal-v1",
    "task-checkpoints-v1",
    "mcp-v1",
];
const AGENT_BOOTSTRAP_PERMISSIONS: [&str; 4] = ["read", "write", "process", "full"];
const AGENT_TASK_INPUT_BYTES: usize = 256 * 1024;
const AGENT_TASK_EVENT_DATA_BYTES: usize = 32 * 1024;
const AGENT_TASK_OUTPUT_BYTES: usize = 1024 * 1024;
const AGENT_TASK_LEASE_SECONDS: i64 = 60;
const TERMINAL_COMMAND_LEASE_SECONDS: i64 = 30;
const TERMINAL_SESSION_LEASE_SECONDS: i64 = 75;
const TERMINAL_MAX_EVENTS: i32 = 20_000;
const TERMINAL_MAX_OUTPUT_BYTES: i64 = 8 * 1024 * 1024;
const TERMINAL_MAX_BATCH: usize = 64;

fn cloud(state: &AppState) -> CloudResult<CloudState> {
    state.cloud.inner.clone().ok_or_else(|| {
        CloudError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "cloud_unavailable",
            state.cloud.message.clone(),
        )
    })
}

#[derive(Serialize)]
struct CloudStatus {
    available: bool,
    message: String,
    features: [&'static str; 12],
}

async fn status(State(state): State<AppState>) -> Json<CloudStatus> {
    Json(CloudStatus {
        available: state.cloud.inner.is_some(),
        message: state.cloud.message.clone(),
        features: [
            "sync",
            "encrypted-credentials",
            "teams",
            "approvals",
            "api-relay",
            "agents",
            "agent-pairing",
            "agent-tasks",
            "terminal-sessions",
            "cloud-conversations",
            "task-checkpoints",
            "deployments-planned",
        ],
    })
}

#[derive(Clone, Serialize, Deserialize)]
struct AuthUser {
    user_id: Uuid,
    session_id: Uuid,
    device_id: Uuid,
    email: String,
    nickname: String,
    role: String,
}

fn bearer(headers: &HeaderMap) -> CloudResult<&str> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "请先登录 Sculk Cloud",
            )
        })
}

fn sha256_hex(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn random_token(prefix: &str) -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    use base64::Engine;
    format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

fn random_pairing_code() -> String {
    let mut bytes = [0_u8; 12];
    OsRng.fill_bytes(&mut bytes);
    use base64::Engine;
    format!(
        "scp_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

fn pairing_is_expired(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    expires_at <= now
}

fn bootstrap_cloud_url() -> CloudResult<String> {
    let value = env::var("SCULK_CLOUD_PUBLIC_URL").map_err(|_| {
        CloudError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "bootstrap_unavailable",
            "未配置 SCULK_CLOUD_PUBLIC_URL，无法生成 Agent 启动配置",
        )
    })?;
    normalize_bootstrap_cloud_url(&value)
}

fn normalize_bootstrap_cloud_url(value: &str) -> CloudResult<String> {
    let mut url = Url::parse(value.trim()).map_err(|_| {
        CloudError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "bootstrap_unavailable",
            "SCULK_CLOUD_PUBLIC_URL 必须是合法的 HTTP(S) 地址",
        )
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CloudError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "bootstrap_unavailable",
            "SCULK_CLOUD_PUBLIC_URL 不能包含凭据、查询参数或片段",
        ));
    }
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(CloudError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "bootstrap_unavailable",
            "SCULK_CLOUD_PUBLIC_URL 必须使用 HTTPS（仅本机开发允许 HTTP）",
        ));
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn valid_agent_token(token: &str) -> bool {
    token.starts_with("sca_") && token.len() > 20
}

fn agent_is_online(status: &str, last_seen_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    status == "active"
        && last_seen_at.is_some_and(|seen| {
            seen <= now + Duration::seconds(5)
                && now.signed_duration_since(seen) <= Duration::seconds(AGENT_ONLINE_SECONDS)
        })
}

fn validate_email(value: &str) -> CloudResult<String> {
    let email = value.trim().to_lowercase();
    let valid = email.len() <= 254
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
    if !valid {
        return Err(CloudError::bad_request("请输入有效的邮箱地址"));
    }
    Ok(email)
}

fn validate_password(value: &str) -> CloudResult<()> {
    if value.len() < 8 || value.len() > 128 {
        return Err(CloudError::bad_request("密码长度需要为 8-128 个字符"));
    }
    Ok(())
}

fn hash_password(password: &str) -> CloudResult<String> {
    let salt = SaltString::generate(&mut PasswordOsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| {
            CloudError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "password_error",
                "密码处理失败",
            )
        })
}

async fn cache_session(cloud: &CloudState, token_hash: &str, user: &AuthUser) {
    let Ok(payload) = serde_json::to_string(user) else {
        return;
    };
    let mut redis = cloud.redis.clone();
    let _: Result<(), _> = redis
        .set_ex(format!("sculk:session:{token_hash}"), payload, 900)
        .await;
}

async fn authenticate(headers: &HeaderMap, cloud: &CloudState) -> CloudResult<AuthUser> {
    let token = bearer(headers)?;
    if !token.starts_with("scs_") {
        return Err(CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_session",
            "登录凭证无效",
        ));
    }
    let token_hash = sha256_hex(token);
    let cache_key = format!("sculk:session:{token_hash}");
    let mut redis = cloud.redis.clone();
    if let Ok(Some(payload)) = redis.get::<_, Option<String>>(&cache_key).await
        && let Ok(user) = serde_json::from_str::<AuthUser>(&payload)
    {
        return Ok(user);
    }

    let row = sqlx::query(
        "SELECT s.id AS session_id, s.device_id, u.id AS user_id, u.email, u.nickname, u.role
         FROM cloud_sessions s
         JOIN cloud_users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > NOW()",
    )
    .bind(&token_hash)
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::UNAUTHORIZED,
            "session_expired",
            "登录已失效，请重新登录",
        )
    })?;
    let user = AuthUser {
        user_id: row.get("user_id"),
        session_id: row.get("session_id"),
        device_id: row.get("device_id"),
        email: row.get("email"),
        nickname: row.get("nickname"),
        role: row.get("role"),
    };
    cache_session(cloud, &token_hash, &user).await;
    let _ = sqlx::query("UPDATE cloud_devices SET last_seen_at = NOW() WHERE id = $1")
        .bind(user.device_id)
        .execute(&cloud.db)
        .await;
    Ok(user)
}

#[derive(Serialize)]
struct ProfileView {
    id: Uuid,
    email: String,
    nickname: String,
    avatar_url: String,
    role: String,
    plan: String,
    locale: String,
    created_at: DateTime<Utc>,
}

fn profile_from_row(row: &sqlx::postgres::PgRow) -> ProfileView {
    ProfileView {
        id: row.get("id"),
        email: row.get("email"),
        nickname: row.get("nickname"),
        avatar_url: row.get("avatar_url"),
        role: row.get("role"),
        plan: row.get("plan"),
        locale: row.get("locale"),
        created_at: row.get("created_at"),
    }
}

async fn fetch_profile(cloud: &CloudState, user_id: Uuid) -> CloudResult<ProfileView> {
    let row = sqlx::query(
        "SELECT id, email, nickname, avatar_url, role, plan, locale, created_at
         FROM cloud_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(profile_from_row(&row))
}

#[derive(Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
    nickname: String,
    #[serde(default)]
    device_name: String,
    #[serde(default)]
    platform: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
    #[serde(default)]
    device_name: String,
    #[serde(default)]
    platform: String,
}

#[derive(Serialize)]
struct AuthResponse {
    access_token: String,
    expires_at: DateTime<Utc>,
    profile: ProfileView,
}

async fn create_session(
    cloud: &CloudState,
    user_id: Uuid,
    email: String,
    nickname: String,
    role: String,
    device_name: &str,
    platform: &str,
) -> CloudResult<(String, DateTime<Utc>)> {
    let device_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let token = random_token("scs_");
    let token_hash = sha256_hex(&token);
    let expires_at = Utc::now() + Duration::days(cloud.session_days);
    let device_name = if device_name.trim().is_empty() {
        "Sculk 工作台"
    } else {
        device_name.trim()
    };
    let platform = if platform.trim().is_empty() {
        "unknown"
    } else {
        platform.trim()
    };

    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    sqlx::query("INSERT INTO cloud_devices (id, user_id, name, platform) VALUES ($1, $2, $3, $4)")
        .bind(device_id)
        .bind(user_id)
        .bind(device_name)
        .bind(platform)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    sqlx::query(
        "INSERT INTO cloud_sessions (id, user_id, device_id, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(device_id)
    .bind(&token_hash)
    .bind(expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;

    cache_session(
        cloud,
        &token_hash,
        &AuthUser {
            user_id,
            session_id,
            device_id,
            email,
            nickname,
            role,
        },
    )
    .await;
    Ok((token, expires_at))
}

async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegisterRequest>,
) -> CloudResult<(StatusCode, Json<AuthResponse>)> {
    let cloud = cloud(&state)?;
    let email = validate_email(&request.email)?;
    validate_password(&request.password)?;
    let nickname = request.nickname.trim();
    if nickname.is_empty() || nickname.chars().count() > 32 {
        return Err(CloudError::bad_request("昵称需要为 1-32 个字符"));
    }
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM cloud_users WHERE LOWER(email) = LOWER($1))",
    )
    .bind(&email)
    .fetch_one(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    if exists {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "email_exists",
            "该邮箱已经注册",
        ));
    }
    let user_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cloud_users")
        .fetch_one(&cloud.db)
        .await
        .map_err(CloudError::database)?;
    let role = if user_count == 0 { "admin" } else { "user" };
    let user_id = Uuid::new_v4();
    let password_hash = hash_password(&request.password)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    sqlx::query(
        "INSERT INTO cloud_users (id, email, password_hash, nickname, role)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(&email)
    .bind(password_hash)
    .bind(nickname)
    .bind(role)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sqlx::query("INSERT INTO cloud_settings (user_id) VALUES ($1)")
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;

    let (access_token, expires_at) = create_session(
        &cloud,
        user_id,
        email.clone(),
        nickname.to_string(),
        role.to_string(),
        &request.device_name,
        &request.platform,
    )
    .await?;
    let profile = fetch_profile(&cloud, user_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(AuthResponse {
            access_token,
            expires_at,
            profile,
        }),
    ))
}

async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> CloudResult<Json<AuthResponse>> {
    let cloud = cloud(&state)?;
    let email = validate_email(&request.email)?;
    let row = sqlx::query(
        "SELECT id, email, nickname, role, password_hash FROM cloud_users WHERE LOWER(email) = LOWER($1)",
    )
    .bind(&email)
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "邮箱或密码不正确",
        )
    })?;
    let password_hash: String = row.get("password_hash");
    let parsed_hash = PasswordHash::new(&password_hash).map_err(|_| {
        CloudError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "password_error",
            "账号密码记录无效",
        )
    })?;
    if Argon2::default()
        .verify_password(request.password.as_bytes(), &parsed_hash)
        .is_err()
    {
        return Err(CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "邮箱或密码不正确",
        ));
    }
    let user_id: Uuid = row.get("id");
    let (access_token, expires_at) = create_session(
        &cloud,
        user_id,
        row.get("email"),
        row.get("nickname"),
        row.get("role"),
        &request.device_name,
        &request.platform,
    )
    .await?;
    Ok(Json(AuthResponse {
        access_token,
        expires_at,
        profile: fetch_profile(&cloud, user_id).await?,
    }))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> CloudResult<StatusCode> {
    let cloud = cloud(&state)?;
    let token = bearer(&headers)?;
    let token_hash = sha256_hex(token);
    let result = sqlx::query(
        "UPDATE cloud_sessions SET revoked_at = NOW() WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(&token_hash)
    .execute(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    if result.rows_affected() == 0 {
        return Err(CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_session",
            "登录凭证无效",
        ));
    }
    let mut redis = cloud.redis.clone();
    let _: Result<(), _> = redis.del(format!("sculk:session:{token_hash}")).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> CloudResult<Json<ProfileView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    Ok(Json(fetch_profile(&cloud, user.user_id).await?))
}

#[derive(Deserialize)]
struct UpdateProfileRequest {
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
    #[serde(default)]
    locale: Option<String>,
}

async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdateProfileRequest>,
) -> CloudResult<Json<ProfileView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let nickname = request.nickname.map(|value| value.trim().to_string());
    if nickname
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.chars().count() > 32)
    {
        return Err(CloudError::bad_request("昵称需要为 1-32 个字符"));
    }
    if request
        .avatar_url
        .as_ref()
        .is_some_and(|value| value.len() > 500)
    {
        return Err(CloudError::bad_request("头像地址过长"));
    }
    if request
        .locale
        .as_ref()
        .is_some_and(|value| !["zh-CN", "en-US"].contains(&value.as_str()))
    {
        return Err(CloudError::bad_request("不支持该语言"));
    }
    sqlx::query(
        "UPDATE cloud_users SET
           nickname = COALESCE($2, nickname),
           avatar_url = COALESCE($3, avatar_url),
           locale = COALESCE($4, locale),
           updated_at = NOW()
         WHERE id = $1",
    )
    .bind(user.user_id)
    .bind(nickname)
    .bind(request.avatar_url.map(|value| value.trim().to_string()))
    .bind(request.locale)
    .execute(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    let token_hash = sha256_hex(bearer(&headers)?);
    let mut redis = cloud.redis.clone();
    let _: Result<(), _> = redis.del(format!("sculk:session:{token_hash}")).await;
    Ok(Json(fetch_profile(&cloud, user.user_id).await?))
}

#[derive(Serialize)]
struct DeviceView {
    id: Uuid,
    name: String,
    platform: String,
    last_seen_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    current: bool,
}

async fn list_devices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Vec<DeviceView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let rows = sqlx::query(
        "SELECT id, name, platform, last_seen_at, created_at
         FROM cloud_devices WHERE user_id = $1 AND revoked_at IS NULL ORDER BY last_seen_at DESC",
    )
    .bind(user.user_id)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(
        rows.iter()
            .map(|row| DeviceView {
                id: row.get("id"),
                name: row.get("name"),
                platform: row.get("platform"),
                last_seen_at: row.get("last_seen_at"),
                created_at: row.get("created_at"),
                current: row.get::<Uuid, _>("id") == user.device_id,
            })
            .collect(),
    ))
}

async fn revoke_device(
    Path(device_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<StatusCode> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    if device_id == user.device_id {
        return Err(CloudError::bad_request("请使用退出登录移除当前设备"));
    }
    let token_hashes = sqlx::query(
        "SELECT token_hash FROM cloud_sessions WHERE user_id = $1 AND device_id = $2 AND revoked_at IS NULL",
    )
    .bind(user.user_id)
    .bind(device_id)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    sqlx::query(
        "UPDATE cloud_sessions SET revoked_at = NOW()
         WHERE user_id = $1 AND device_id = $2 AND revoked_at IS NULL",
    )
    .bind(user.user_id)
    .bind(device_id)
    .execute(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    let result = sqlx::query(
        "UPDATE cloud_devices SET revoked_at = NOW()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(device_id)
    .bind(user.user_id)
    .execute(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    if result.rows_affected() == 0 {
        return Err(CloudError::new(
            StatusCode::NOT_FOUND,
            "device_not_found",
            "设备不存在或已经移除",
        ));
    }
    let mut redis = cloud.redis.clone();
    for row in token_hashes {
        let hash: String = row.get("token_hash");
        let _: Result<(), _> = redis.del(format!("sculk:session:{hash}")).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct AgentPairingCreated {
    id: Uuid,
    pairing_code: String,
    expires_at: DateTime<Utc>,
    status: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAgentBootstrapRequest {
    platform: String,
    name: String,
    workspace_label: String,
    workspace_root: String,
}

struct ValidatedAgentBootstrap {
    platform: String,
    name: String,
    workspace_label: String,
    workspace_root: String,
}

#[derive(Serialize)]
struct AgentBootstrap {
    schema_version: u8,
    permissions_granted_by_current_user: bool,
    pairing_id: Uuid,
    pairing_code: String,
    expires_at: DateTime<Utc>,
    cloud_url: String,
    platform: String,
    name: String,
    workspace_label: String,
    workspace_root: String,
    capabilities: Vec<String>,
    permissions: Vec<String>,
}

#[derive(Deserialize)]
struct ClaimAgentRequest {
    pairing_code: String,
    name: String,
    platform: String,
    version: String,
    workspace_label: String,
    capabilities: Vec<String>,
    permissions: Vec<String>,
    fingerprint: String,
}

struct ValidatedAgentClaim {
    name: String,
    platform: String,
    version: String,
    workspace_label: String,
    capabilities: Vec<String>,
    permissions: Vec<String>,
    fingerprint: String,
}

#[derive(Serialize)]
struct AgentClaimed {
    agent_id: Uuid,
    token: String,
    status: &'static str,
}

#[derive(Serialize)]
struct AgentView {
    id: Uuid,
    name: String,
    platform: String,
    version: String,
    workspace_label: String,
    capabilities: Vec<String>,
    permissions: Vec<String>,
    fingerprint: String,
    status: String,
    last_seen_at: Option<DateTime<Utc>>,
    online: bool,
    claimed_at: DateTime<Utc>,
    confirmed_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct AgentHeartbeat {
    agent_id: Uuid,
    status: String,
    active: bool,
    commands_available: bool,
    server_time: DateTime<Utc>,
    next_heartbeat_seconds: u32,
}

fn bounded_agent_text(
    value: &str,
    field: &'static str,
    minimum: usize,
    maximum: usize,
) -> CloudResult<String> {
    let value = value.trim();
    let length = value.chars().count();
    if length < minimum || length > maximum || value.chars().any(char::is_control) {
        return Err(CloudError::bad_request(format!(
            "{field} length must be between {minimum} and {maximum} characters"
        )));
    }
    Ok(value.to_string())
}

fn validate_capabilities(values: &[String]) -> CloudResult<Vec<String>> {
    if values.len() > AGENT_MAX_CAPABILITIES {
        return Err(CloudError::bad_request("too many agent capabilities"));
    }
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim().to_ascii_lowercase();
        if value.is_empty()
            || value.len() > 64
            || !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
            || normalized.contains(&value)
        {
            return Err(CloudError::bad_request(
                "invalid or duplicate agent capability",
            ));
        }
        normalized.push(value);
    }
    Ok(normalized)
}

fn validate_permissions(values: &[String]) -> CloudResult<Vec<String>> {
    if values.len() > AGENT_MAX_PERMISSIONS {
        return Err(CloudError::bad_request("too many agent permissions"));
    }
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim().to_ascii_lowercase();
        if !AGENT_PERMISSIONS.contains(&value.as_str()) || normalized.contains(&value) {
            return Err(CloudError::bad_request(
                "agent permissions are limited to read, write, process, and full",
            ));
        }
        normalized.push(value);
    }
    Ok(normalized)
}

fn validate_agent_bootstrap(
    request: &CreateAgentBootstrapRequest,
) -> CloudResult<ValidatedAgentBootstrap> {
    let platform = bounded_agent_text(&request.platform, "platform", 1, 64)?;
    if !platform
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(CloudError::bad_request("platform is invalid"));
    }
    Ok(ValidatedAgentBootstrap {
        platform,
        name: bounded_agent_text(&request.name, "name", 1, 64)?,
        workspace_label: bounded_agent_text(&request.workspace_label, "workspace_label", 1, 128)?,
        workspace_root: bounded_agent_text(&request.workspace_root, "workspace_root", 1, 1024)?,
    })
}

fn validate_agent_claim(request: &ClaimAgentRequest) -> CloudResult<ValidatedAgentClaim> {
    let fingerprint = bounded_agent_text(&request.fingerprint, "fingerprint", 8, 128)?;
    if !fingerprint
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, ':' | '-' | '_'))
    {
        return Err(CloudError::bad_request("agent fingerprint is invalid"));
    }
    Ok(ValidatedAgentClaim {
        name: bounded_agent_text(&request.name, "name", 1, 64)?,
        platform: bounded_agent_text(&request.platform, "platform", 1, 64)?,
        version: bounded_agent_text(&request.version, "version", 1, 64)?,
        workspace_label: bounded_agent_text(&request.workspace_label, "workspace_label", 1, 128)?,
        capabilities: validate_capabilities(&request.capabilities)?,
        permissions: validate_permissions(&request.permissions)?,
        fingerprint,
    })
}

fn json_string_list(row: &sqlx::postgres::PgRow, field: &str) -> Vec<String> {
    serde_json::from_value(row.get::<Value, _>(field)).unwrap_or_default()
}

fn agent_view(row: &sqlx::postgres::PgRow, now: DateTime<Utc>) -> AgentView {
    let status: String = row.get("status");
    let last_seen_at = row.get("last_seen_at");
    AgentView {
        id: row.get("id"),
        name: row.get("name"),
        platform: row.get("platform"),
        version: row.get("agent_version"),
        workspace_label: row.get("workspace_label"),
        capabilities: json_string_list(row, "capabilities"),
        permissions: json_string_list(row, "permissions"),
        fingerprint: row.get("fingerprint"),
        online: agent_is_online(&status, last_seen_at, now),
        status,
        last_seen_at,
        claimed_at: row.get("claimed_at"),
        confirmed_at: row.get("confirmed_at"),
        revoked_at: row.get("revoked_at"),
    }
}

const AGENT_SELECT: &str = "SELECT id, name, platform, agent_version, workspace_label,
    capabilities, permissions, fingerprint, status, last_seen_at, claimed_at,
    confirmed_at, revoked_at FROM cloud_agents";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AgentTaskOperation {
    permission: &'static str,
    risk: &'static str,
    approval_required: bool,
    additional_capability: Option<&'static str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAgentTaskRequest {
    agent_id: Uuid,
    operation: String,
    input: Value,
    idempotency_key: Option<String>,
    #[serde(default)]
    team_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentTaskLeaseRequest {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentTaskLeaseTokenRequest {
    lease_token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentTaskEventRequest {
    lease_token: String,
    level: String,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteAgentTaskRequest {
    lease_token: String,
    status: String,
    #[serde(default)]
    output: Option<Value>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    rollback_available: bool,
    #[serde(default)]
    artifacts: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetryAgentTaskRequest {
    mode: String,
    idempotency_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAgentTaskCheckpointRequest {
    lease_token: String,
    checkpoint_key: String,
    kind: String,
    resumable: bool,
    payload: Value,
}

#[derive(Serialize)]
struct AgentTaskEventView {
    seq: i32,
    level: String,
    message: String,
    data: Option<Value>,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct AgentTaskCheckpointView {
    id: Uuid,
    task_id: Uuid,
    seq: i32,
    checkpoint_key: String,
    kind: String,
    resumable: bool,
    payload: Value,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct AgentTaskCheckpointMetadata {
    id: Uuid,
    seq: i32,
    checkpoint_key: String,
    kind: String,
    resumable: bool,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct AgentTaskView {
    id: Uuid,
    agent_id: Uuid,
    team_id: Option<Uuid>,
    operation: String,
    required_permission: String,
    risk: String,
    input: Value,
    status: String,
    idempotency_key: Option<String>,
    source_task_id: Option<Uuid>,
    approval_id: Option<Uuid>,
    lineage_id: Uuid,
    attempt_no: i32,
    retry_of_task_id: Option<Uuid>,
    execution_mode: String,
    resume_checkpoint_id: Option<Uuid>,
    rollback_source_task_id: Option<Uuid>,
    approved_by: Option<Uuid>,
    approved_at: Option<DateTime<Utc>>,
    lease_expires_at: Option<DateTime<Utc>>,
    leased_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    cancelled_at: Option<DateTime<Utc>>,
    cancel_requested_at: Option<DateTime<Utc>>,
    cancel_requested_by: Option<Uuid>,
    cancel_acknowledged_at: Option<DateTime<Utc>>,
    cancel_requested: bool,
    output: Option<Value>,
    error: Option<String>,
    artifacts: Option<Value>,
    rollback_available: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    latest_checkpoint: Option<AgentTaskCheckpointMetadata>,
    can_resume: bool,
    events: Vec<AgentTaskEventView>,
}

#[derive(Serialize)]
struct AgentTaskLeaseResponse {
    lease_token: String,
    lease_expires_at: DateTime<Utc>,
    task: AgentTaskLeaseView,
}

#[derive(Serialize)]
struct AgentTaskControlResponse {
    task_id: Uuid,
    lease_expires_at: DateTime<Utc>,
    cancel_requested: bool,
}

#[derive(Serialize)]
struct AgentTaskLeaseView {
    id: Uuid,
    operation: String,
    input: Value,
    required_permission: String,
    risk: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume: Option<AgentTaskResumeLeaseView>,
}

#[derive(Serialize)]
struct AgentTaskResumeLeaseView {
    source_task_id: Uuid,
    checkpoint_id: Uuid,
    kind: String,
    payload: Value,
}

const AGENT_TASK_SELECT: &str = "SELECT id, agent_id, team_id, operation, required_permission, risk, input,
    status, idempotency_key, source_task_id, approval_id, approved_by, approved_at, lease_expires_at,
    leased_at, started_at, completed_at, cancelled_at, output, error, artifacts,
    rollback_available, created_at, updated_at, lineage_id, attempt_no, retry_of_task_id,
    execution_mode, resume_checkpoint_id, rollback_source_task_id, cancel_requested_at,
    cancel_requested_by, cancel_acknowledged_at FROM cloud_agent_tasks";

fn agent_task_operation(value: &str, allow_rollback: bool) -> CloudResult<AgentTaskOperation> {
    let operation = match value {
        "host.inspect" | "workspace.list" | "log.tail" => AgentTaskOperation {
            permission: "read",
            risk: "low",
            approval_required: false,
            additional_capability: None,
        },
        "workspace.create_directory" | "server.properties.update" => AgentTaskOperation {
            permission: "write",
            risk: "high",
            approval_required: true,
            additional_capability: None,
        },
        "shell.exec" => AgentTaskOperation {
            permission: "full",
            risk: "critical",
            approval_required: true,
            additional_capability: Some("shell-v1"),
        },
        "platform.mcp.read" => AgentTaskOperation {
            permission: "read",
            risk: "low",
            approval_required: false,
            additional_capability: Some("mcp-v1"),
        },
        "platform.mcp.reply" => AgentTaskOperation {
            permission: "write",
            risk: "high",
            approval_required: true,
            additional_capability: Some("mcp-v1"),
        },
        "task.rollback" if allow_rollback => AgentTaskOperation {
            permission: "write",
            risk: "high",
            approval_required: true,
            additional_capability: None,
        },
        "task.rollback" => {
            return Err(CloudError::new(
                StatusCode::FORBIDDEN,
                "rollback_endpoint_required",
                "rollback tasks can only be created through the rollback endpoint",
            ));
        }
        _ => {
            return Err(CloudError::bad_request(
                "operation is not in the agent task allowlist",
            ));
        }
    };
    Ok(operation)
}

fn validate_json_bounds(
    value: &Value,
    maximum_bytes: usize,
    maximum_depth: usize,
    maximum_fields: usize,
) -> CloudResult<()> {
    let encoded = serde_json::to_vec(value)
        .map_err(|_| CloudError::bad_request("task JSON cannot be serialized"))?;
    if encoded.len() > maximum_bytes {
        return Err(CloudError::bad_request("task JSON exceeds the size limit"));
    }
    fn visit(value: &Value, depth: usize, fields: &mut usize, max_depth: usize) -> bool {
        if depth > max_depth {
            return false;
        }
        match value {
            Value::Object(object) => {
                *fields = fields.saturating_add(object.len());
                object
                    .values()
                    .all(|value| visit(value, depth + 1, fields, max_depth))
            }
            Value::Array(items) => {
                *fields = fields.saturating_add(items.len());
                items
                    .iter()
                    .all(|value| visit(value, depth + 1, fields, max_depth))
            }
            _ => true,
        }
    }
    let mut fields = 0;
    if !visit(value, 0, &mut fields, maximum_depth) || fields > maximum_fields {
        return Err(CloudError::bad_request(
            "task JSON is too deeply nested or contains too many fields",
        ));
    }
    Ok(())
}

fn task_input_object(value: &Value) -> CloudResult<&serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| CloudError::bad_request("task input must be a JSON object"))
}

fn require_exact_task_keys(
    object: &serde_json::Map<String, Value>,
    required: &[&str],
    optional: &[&str],
) -> CloudResult<()> {
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(CloudError::bad_request(
            "task input contains missing or unsupported fields",
        ));
    }
    Ok(())
}

fn validate_relative_task_path(value: &Value) -> CloudResult<()> {
    let path = value
        .as_str()
        .ok_or_else(|| CloudError::bad_request("task path must be a string"))?;
    if path.is_empty()
        || path.len() > 512
        || path != path.trim()
        || path.chars().any(char::is_control)
        || path.starts_with(['/', '\\'])
        || path.contains(':')
        || path
            .split(['/', '\\'])
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(CloudError::bad_request(
            "task path must be a safe relative path",
        ));
    }
    Ok(())
}

fn validate_log_tail_path(value: &Value) -> CloudResult<()> {
    validate_relative_task_path(value)?;
    let path = value
        .as_str()
        .expect("validate_relative_task_path validated a string");
    let components: Vec<&str> = path.split(['/', '\\']).collect();
    let directory = components.first().copied().unwrap_or_default();
    if !["logs", "crash-reports"]
        .iter()
        .any(|allowed| directory.eq_ignore_ascii_case(allowed))
        || components.len() < 2
        || components.iter().any(|component| {
            let component = component.to_ascii_lowercase();
            component == ".env"
                || component.starts_with(".env.")
                || matches!(
                    component.as_str(),
                    "database" | "db" | "database.sql" | "db.sql"
                )
                || [".db", ".sqlite", ".sqlite3", ".mdb", ".sql"]
                    .iter()
                    .any(|suffix| component.ends_with(suffix))
        })
    {
        return Err(CloudError::bad_request(
            "log.tail must target a file under logs/ or crash-reports/",
        ));
    }
    Ok(())
}

fn bounded_task_integer(
    object: &serde_json::Map<String, Value>,
    key: &str,
    minimum: u64,
    maximum: u64,
) -> CloudResult<()> {
    let value = object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| CloudError::bad_request(format!("{key} must be an integer")))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(CloudError::bad_request(format!(
            "{key} is outside the allowed range"
        )));
    }
    Ok(())
}

fn validate_server_properties_changes(value: &Value) -> CloudResult<()> {
    let changes = value
        .as_object()
        .filter(|changes| !changes.is_empty() && changes.len() <= 8)
        .ok_or_else(|| CloudError::bad_request("changes must be a non-empty JSON object"))?;
    for (key, value) in changes {
        let valid = match key.as_str() {
            "motd" => value.as_str().is_some_and(|text| {
                !text.is_empty()
                    && text.chars().count() <= 200
                    && !text.chars().any(char::is_control)
            }),
            "max-players" => value
                .as_u64()
                .is_some_and(|value| (1..=1000).contains(&value)),
            "difficulty" => value
                .as_str()
                .is_some_and(|value| ["peaceful", "easy", "normal", "hard"].contains(&value)),
            "gamemode" => value.as_str().is_some_and(|value| {
                ["survival", "creative", "adventure", "spectator"].contains(&value)
            }),
            "pvp" | "white-list" => value.is_boolean(),
            "view-distance" | "simulation-distance" => value
                .as_u64()
                .is_some_and(|value| (2..=32).contains(&value)),
            _ => false,
        };
        if !valid {
            return Err(CloudError::bad_request(format!(
                "invalid server.properties change for {key}"
            )));
        }
    }
    Ok(())
}

fn validate_platform_mcp_input(
    object: &serde_json::Map<String, Value>,
    reply_operation: bool,
) -> CloudResult<()> {
    require_exact_task_keys(object, &["server", "tool", "arguments"], &[])?;
    let server = object["server"]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 64)
        .ok_or_else(|| CloudError::bad_request("MCP server must be a non-empty short string"))?;
    if !server
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(CloudError::bad_request(
            "MCP server contains invalid characters",
        ));
    }
    let tool = object["tool"]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 64)
        .ok_or_else(|| CloudError::bad_request("MCP tool must be a non-empty short string"))?;
    let allowed = if reply_operation {
        ["reply_comment"].as_slice()
    } else {
        ["platform_status", "list_comments"].as_slice()
    };
    if !allowed.contains(&tool) {
        return Err(CloudError::bad_request(
            "MCP tool is not allowed for this task operation",
        ));
    }
    let arguments = object["arguments"]
        .as_object()
        .ok_or_else(|| CloudError::bad_request("MCP arguments must be a JSON object"))?;
    match tool {
        "platform_status" => {
            if !arguments.is_empty() {
                return Err(CloudError::bad_request(
                    "platform_status does not accept arguments",
                ));
            }
        }
        "list_comments" => {
            require_exact_task_keys(arguments, &["video_id"], &["cursor", "limit"])?;
            let video_id = arguments["video_id"]
                .as_str()
                .filter(|value| !value.is_empty() && value.len() <= 256)
                .ok_or_else(|| CloudError::bad_request("video_id must be a non-empty string"))?;
            let _ = video_id;
            if let Some(cursor) = arguments.get("cursor")
                && (cursor
                    .as_str()
                    .is_none_or(|value| value.len() > 256 || value.chars().any(char::is_control)))
            {
                return Err(CloudError::bad_request("cursor is invalid"));
            }
            if arguments.contains_key("limit") {
                bounded_task_integer(arguments, "limit", 1, 100)?;
            }
        }
        "reply_comment" => {
            require_exact_task_keys(
                arguments,
                &["video_id", "comment_id", "content"],
                &["dry_run"],
            )?;
            for key in ["video_id", "comment_id", "content"] {
                let value = arguments[key]
                    .as_str()
                    .filter(|value| {
                        !value.is_empty()
                            && value.len() <= if key == "content" { 500 } else { 256 }
                            && !value.chars().any(char::is_control)
                    })
                    .ok_or_else(|| CloudError::bad_request(format!("{key} is invalid")))?;
                let _ = value;
            }
            if let Some(dry_run) = arguments.get("dry_run")
                && !dry_run.is_boolean()
            {
                return Err(CloudError::bad_request("dry_run must be a boolean"));
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn validate_agent_task_input(
    operation: &str,
    input: &Value,
    allow_rollback: bool,
) -> CloudResult<AgentTaskOperation> {
    let spec = agent_task_operation(operation, allow_rollback)?;
    validate_json_bounds(input, AGENT_TASK_INPUT_BYTES, 6, 64)?;
    let object = task_input_object(input)?;
    match operation {
        "host.inspect" => require_exact_task_keys(object, &[], &[])?,
        "workspace.list" => {
            require_exact_task_keys(object, &["path", "max_entries"], &[])?;
            if object["path"].as_str() != Some(".") {
                validate_relative_task_path(&object["path"])?;
            }
            bounded_task_integer(object, "max_entries", 1, 500)?;
        }
        "log.tail" => {
            require_exact_task_keys(object, &["path", "lines", "max_bytes"], &[])?;
            validate_log_tail_path(&object["path"])?;
            bounded_task_integer(object, "lines", 1, 1000)?;
            bounded_task_integer(object, "max_bytes", 1, 262_144)?;
        }
        "workspace.create_directory" => {
            require_exact_task_keys(object, &["path"], &[])?;
            validate_relative_task_path(&object["path"])?;
        }
        "server.properties.update" => {
            require_exact_task_keys(object, &["path", "changes"], &[])?;
            validate_relative_task_path(&object["path"])?;
            if object["path"]
                .as_str()
                .is_none_or(|path| path.rsplit(['/', '\\']).next() != Some("server.properties"))
            {
                return Err(CloudError::bad_request(
                    "server.properties.update must target server.properties",
                ));
            }
            validate_server_properties_changes(&object["changes"])?;
        }
        "shell.exec" => {
            require_exact_task_keys(object, &["command"], &["cwd", "timeout_seconds"])?;
            let command = object["command"]
                .as_str()
                .filter(|command| !command.is_empty() && command.chars().count() <= 32_768)
                .ok_or_else(|| {
                    CloudError::bad_request("shell command must contain 1-32768 characters")
                })?;
            let _ = command;
            if let Some(cwd) = object.get("cwd")
                && cwd
                    .as_str()
                    .filter(|cwd| cwd.chars().count() <= 1024)
                    .is_none()
            {
                return Err(CloudError::bad_request(
                    "shell cwd must be at most 1024 characters",
                ));
            }
            if object.contains_key("timeout_seconds") {
                bounded_task_integer(object, "timeout_seconds", 1, 1800)?;
            }
        }
        "platform.mcp.read" | "platform.mcp.reply" => {
            validate_platform_mcp_input(object, operation == "platform.mcp.reply")?;
        }
        "task.rollback" => {
            require_exact_task_keys(object, &["source_task_id"], &[])?;
            let source = object["source_task_id"]
                .as_str()
                .and_then(|value| Uuid::parse_str(value).ok());
            if source.is_none() {
                return Err(CloudError::bad_request("source_task_id must be a UUID"));
            }
        }
        _ => unreachable!(),
    }
    Ok(spec)
}

fn validate_idempotency_key(value: Option<String>) -> CloudResult<Option<String>> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty()
                || value.len() > 128
                || !value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "._:-".contains(character)
                })
            {
                return Err(CloudError::bad_request("idempotency_key is invalid"));
            }
            Ok(value.to_string())
        })
        .transpose()
}

fn task_text_contains_token(value: &str) -> bool {
    ["sca_", "scs_", "sk-sc_"].iter().any(|prefix| {
        value.match_indices(prefix).any(|(start, _)| {
            value[start + prefix.len()..]
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || ['-', '_'].contains(character)
                })
                .take(44)
                .count()
                >= 32
        })
    }) || value.match_indices("Bearer ").any(|(start, _)| {
        value[start + "Bearer ".len()..]
            .chars()
            .take_while(|character| {
                !character.is_whitespace()
                    && !['\'', '"', '`', ',', ';', ')', ']', '}'].contains(character)
            })
            .take(21)
            .count()
            >= 20
    })
}

fn task_json_contains_secret(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let key = key
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>();
            matches!(
                key.as_str(),
                "token"
                    | "authorization"
                    | "cookie"
                    | "session"
                    | "password"
                    | "secret"
                    | "credential"
                    | "apikey"
                    | "clientsecret"
                    | "rconpassword"
                    | "command"
                    | "args"
            ) || task_json_contains_secret(value)
        }),
        Value::Array(items) => items.iter().any(task_json_contains_secret),
        Value::String(value) => task_text_contains_token(value),
        _ => false,
    }
}

fn validate_task_event(level: &str, message: &str, data: &Option<Value>) -> CloudResult<()> {
    if !["info", "warn", "error"].contains(&level)
        || message.is_empty()
        || message.chars().count() > 2000
        || message.chars().any(|character| character == '\0')
        || task_text_contains_token(message)
    {
        return Err(CloudError::bad_request("agent task event is invalid"));
    }
    if let Some(data) = data {
        validate_json_bounds(data, AGENT_TASK_EVENT_DATA_BYTES, 6, 512)?;
        if task_json_contains_secret(data) {
            return Err(CloudError::bad_request(
                "agent task event contains forbidden credential fields",
            ));
        }
    }
    Ok(())
}

fn validate_task_artifacts(artifacts: &Option<Value>) -> CloudResult<()> {
    let Some(artifacts) = artifacts else {
        return Ok(());
    };
    validate_json_bounds(artifacts, AGENT_TASK_EVENT_DATA_BYTES, 4, 192)?;
    let items = artifacts
        .as_array()
        .filter(|items| items.len() <= 32)
        .ok_or_else(|| CloudError::bad_request("artifacts must be an array of at most 32 items"))?;
    for item in items {
        let object = task_input_object(item)?;
        require_exact_task_keys(object, &["name", "path", "kind"], &["size_bytes", "sha256"])?;
        let name = object["name"].as_str().unwrap_or_default();
        if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
            return Err(CloudError::bad_request("artifact name is invalid"));
        }
        validate_relative_task_path(&object["path"])?;
        if !object["kind"]
            .as_str()
            .is_some_and(|kind| ["file", "directory", "backup", "log"].contains(&kind))
        {
            return Err(CloudError::bad_request("artifact kind is invalid"));
        }
        if let Some(size) = object.get("size_bytes")
            && size.as_u64().is_none_or(|size| size > 1_099_511_627_776)
        {
            return Err(CloudError::bad_request("artifact size_bytes is invalid"));
        }
        if let Some(hash) = object.get("sha256")
            && hash.as_str().is_none_or(|hash| {
                hash.len() != 64 || !hash.chars().all(|character| character.is_ascii_hexdigit())
            })
        {
            return Err(CloudError::bad_request("artifact sha256 is invalid"));
        }
    }
    Ok(())
}

fn validate_task_completion_values(
    status: &str,
    output: &Option<Value>,
    error: &Option<String>,
    artifacts: &Option<Value>,
    allow_cancelled: bool,
) -> CloudResult<()> {
    if !(["succeeded", "failed"].contains(&status) || allow_cancelled && status == "cancelled") {
        return Err(CloudError::bad_request(
            "completion status must be succeeded, failed, or an acknowledged cancellation",
        ));
    }
    if let Some(output) = output {
        validate_json_bounds(output, AGENT_TASK_OUTPUT_BYTES, 8, 4096)?;
        if task_json_contains_secret(output) {
            return Err(CloudError::bad_request(
                "task output contains forbidden credential fields",
            ));
        }
    }
    if error.as_deref().is_some_and(|error| {
        error.is_empty()
            || error.chars().count() > 4000
            || error.contains('\0')
            || task_text_contains_token(error)
    }) {
        return Err(CloudError::bad_request(
            "task error must be safe and at most 4000 characters",
        ));
    }
    validate_task_artifacts(artifacts)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultCheckpointPayload {
    status: String,
    #[serde(default)]
    output: Option<Value>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    rollback_available: bool,
    #[serde(default)]
    artifacts: Option<Value>,
}

fn validate_task_checkpoint(
    checkpoint_key: &str,
    kind: &str,
    resumable: bool,
    payload: &Value,
) -> CloudResult<()> {
    validate_idempotency_key(Some(checkpoint_key.to_string()))?;
    if !["progress", "result"].contains(&kind) {
        return Err(CloudError::bad_request(
            "checkpoint kind must be progress or result",
        ));
    }
    if kind == "progress" {
        if resumable {
            return Err(CloudError::bad_request(
                "only result checkpoints can be resumable",
            ));
        }
        validate_json_bounds(payload, AGENT_TASK_EVENT_DATA_BYTES, 6, 512)?;
        if task_json_contains_secret(payload) {
            return Err(CloudError::bad_request(
                "task checkpoint contains forbidden credential fields",
            ));
        }
        return Ok(());
    }
    validate_json_bounds(
        payload,
        AGENT_TASK_OUTPUT_BYTES + AGENT_TASK_EVENT_DATA_BYTES,
        9,
        4300,
    )?;
    let result: ResultCheckpointPayload = serde_json::from_value(payload.clone())
        .map_err(|_| CloudError::bad_request("result checkpoint payload is invalid"))?;
    validate_task_completion_values(
        &result.status,
        &result.output,
        &result.error,
        &result.artifacts,
        false,
    )?;
    if resumable && result.status != "succeeded" {
        return Err(CloudError::bad_request(
            "only successful result checkpoints can be resumable",
        ));
    }
    if result.rollback_available && result.status != "succeeded" {
        return Err(CloudError::bad_request(
            "failed result checkpoints cannot be rollback-capable",
        ));
    }
    Ok(())
}

fn agent_task_is_cancellable(status: &str) -> bool {
    ["awaiting_approval", "queued", "leased"].contains(&status)
}

fn agent_task_supports_running_cancellation(operation: &str) -> bool {
    operation == "shell.exec"
}

fn agent_task_event_renews_lease(cancellation_requested: bool) -> bool {
    !cancellation_requested
}

fn ensure_task_accepts_checkpoint(cancellation_requested: bool) -> CloudResult<()> {
    if cancellation_requested {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_task_cancellation_pending",
            "task checkpoints are not accepted after cancellation is requested",
        ));
    }
    Ok(())
}

fn validate_task_cancellation_completion(
    status: &str,
    rollback_available: bool,
    cancellation_requested: bool,
) -> CloudResult<()> {
    if cancellation_requested && status != "cancelled" {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_task_cancellation_pending",
            "task cancellation is pending and must be acknowledged before completion",
        ));
    }
    if status != "cancelled" {
        return Ok(());
    }
    if !cancellation_requested {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_task_cancellation_not_requested",
            "task cancellation was not requested by the user",
        ));
    }
    if rollback_available {
        return Err(CloudError::bad_request(
            "cancelled tasks cannot be rollback-capable",
        ));
    }
    Ok(())
}

fn agent_permissions_allow(permissions: &[String], required: &str) -> bool {
    permissions
        .iter()
        .any(|value| value == required || value == "full")
}

fn validate_task_rollback_result(
    completion_status: &str,
    permission: &str,
    operation: &str,
    rollback_available: bool,
) -> CloudResult<()> {
    if rollback_available
        && (completion_status != "succeeded" || permission != "write" || operation == "shell.exec")
    {
        return Err(CloudError::bad_request(
            "rollback is only available for successful structured write operations",
        ));
    }
    Ok(())
}

async fn create_pending_agent_pairing(
    cloud: &CloudState,
    user_id: Uuid,
) -> CloudResult<(Uuid, String, DateTime<Utc>)> {
    let id = Uuid::new_v4();
    let pairing_code = random_pairing_code();
    let expires_at = Utc::now() + Duration::minutes(AGENT_PAIRING_MINUTES);
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    sqlx::query(
        "UPDATE cloud_agent_pairings SET status = 'expired'
         WHERE user_id = $1 AND status = 'pending'",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sqlx::query(
        "INSERT INTO cloud_agent_pairings (id, user_id, code_hash, expires_at)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(user_id)
    .bind(sha256_hex(&pairing_code))
    .bind(expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((id, pairing_code, expires_at))
}

async fn create_agent_pairing(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<(StatusCode, Json<AgentPairingCreated>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let (id, pairing_code, expires_at) = create_pending_agent_pairing(&cloud, user.user_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(AgentPairingCreated {
            id,
            pairing_code,
            expires_at,
            status: "pending",
        }),
    ))
}

async fn create_agent_bootstrap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateAgentBootstrapRequest>,
) -> CloudResult<(StatusCode, Json<AgentBootstrap>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let cloud_url = bootstrap_cloud_url()?;
    let request = validate_agent_bootstrap(&request)?;
    let (pairing_id, pairing_code, expires_at) =
        create_pending_agent_pairing(&cloud, user.user_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(AgentBootstrap {
            schema_version: 1,
            permissions_granted_by_current_user: true,
            pairing_id,
            pairing_code,
            expires_at,
            cloud_url,
            platform: request.platform,
            name: request.name,
            workspace_label: request.workspace_label,
            workspace_root: request.workspace_root,
            capabilities: AGENT_BOOTSTRAP_CAPABILITIES
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            permissions: AGENT_BOOTSTRAP_PERMISSIONS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        }),
    ))
}

async fn claim_agent_pairing(
    State(state): State<AppState>,
    Json(request): Json<ClaimAgentRequest>,
) -> CloudResult<(StatusCode, Json<AgentClaimed>)> {
    let cloud = cloud(&state)?;
    let pairing_code = bounded_agent_text(&request.pairing_code, "pairing_code", 8, 128)?;
    let validated = validate_agent_claim(&request)?;
    let now = Utc::now();
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let pairing = sqlx::query(
        "SELECT id, user_id, status, expires_at FROM cloud_agent_pairings
         WHERE code_hash = $1 FOR UPDATE",
    )
    .bind(sha256_hex(&pairing_code))
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "pairing_not_found",
            "pairing code is invalid",
        )
    })?;
    let pairing_id: Uuid = pairing.get("id");
    let pairing_status: String = pairing.get("status");
    if pairing_status != "pending" {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "pairing_already_used",
            "pairing code has already been used",
        ));
    }
    let expires_at: DateTime<Utc> = pairing.get("expires_at");
    if pairing_is_expired(expires_at, now) {
        sqlx::query("UPDATE cloud_agent_pairings SET status = 'expired' WHERE id = $1")
            .bind(pairing_id)
            .execute(&mut *transaction)
            .await
            .map_err(CloudError::database)?;
        transaction.commit().await.map_err(CloudError::database)?;
        return Err(CloudError::new(
            StatusCode::GONE,
            "pairing_expired",
            "pairing code has expired",
        ));
    }

    let agent_id = Uuid::new_v4();
    let agent_token = random_token("sca_");
    sqlx::query(
        "INSERT INTO cloud_agents
         (id, user_id, name, platform, agent_version, workspace_label,
          capabilities, permissions, fingerprint, token_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(agent_id)
    .bind(pairing.get::<Uuid, _>("user_id"))
    .bind(validated.name)
    .bind(validated.platform)
    .bind(validated.version)
    .bind(validated.workspace_label)
    .bind(json!(validated.capabilities))
    .bind(json!(validated.permissions))
    .bind(validated.fingerprint)
    .bind(sha256_hex(&agent_token))
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    let result = sqlx::query(
        "UPDATE cloud_agent_pairings
         SET status = 'claimed', claimed_agent_id = $2, claimed_at = NOW()
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(pairing_id)
    .bind(agent_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    if result.rows_affected() != 1 {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "pairing_already_used",
            "pairing code was claimed concurrently",
        ));
    }
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(AgentClaimed {
            agent_id,
            token: agent_token,
            status: "claimed",
        }),
    ))
}

async fn list_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Vec<AgentView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let sql = format!("{AGENT_SELECT} WHERE user_id = $1 ORDER BY created_at DESC");
    let rows = sqlx::query(&sql)
        .bind(user.user_id)
        .fetch_all(&cloud.db)
        .await
        .map_err(CloudError::database)?;
    let now = Utc::now();
    Ok(Json(rows.iter().map(|row| agent_view(row, now)).collect()))
}

async fn confirm_agent_pairing(
    Path(pairing_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<AgentView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let pairing = sqlx::query(
        "SELECT status, claimed_agent_id FROM cloud_agent_pairings
         WHERE id = $1 AND user_id = $2",
    )
    .bind(pairing_id)
    .bind(user.user_id)
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "pairing_not_found",
            "pairing request was not found",
        )
    })?;
    if pairing.get::<String, _>("status") != "claimed" {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "pairing_not_claimed",
            "pairing request is not waiting for confirmation",
        ));
    }
    let agent_id = pairing
        .get::<Option<Uuid>, _>("claimed_agent_id")
        .ok_or_else(|| CloudError::bad_request("pairing request has no claimed agent"))?;
    Ok(Json(
        activate_claimed_agent(&cloud, user.user_id, agent_id).await?,
    ))
}

async fn confirm_agent(
    Path(agent_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<AgentView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    Ok(Json(
        activate_claimed_agent(&cloud, user.user_id, agent_id).await?,
    ))
}

async fn activate_claimed_agent(
    cloud: &CloudState,
    user_id: Uuid,
    agent_id: Uuid,
) -> CloudResult<AgentView> {
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM cloud_agents WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(agent_id)
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "agent was not found",
        )
    })?;
    if status != "claimed" {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_not_claimed",
            "agent is not waiting for confirmation",
        ));
    }
    let pairing = sqlx::query(
        "SELECT id, expires_at FROM cloud_agent_pairings
         WHERE claimed_agent_id = $1 AND user_id = $2 AND status = 'claimed'
         FOR UPDATE",
    )
    .bind(agent_id)
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "pairing_not_claimed",
            "agent pairing is not waiting for confirmation",
        )
    })?;
    let pairing_id: Uuid = pairing.get("id");
    let expires_at: DateTime<Utc> = pairing.get("expires_at");
    if pairing_is_expired(expires_at, Utc::now()) {
        sqlx::query("UPDATE cloud_agent_pairings SET status = 'expired' WHERE id = $1")
            .bind(pairing_id)
            .execute(&mut *transaction)
            .await
            .map_err(CloudError::database)?;
        sqlx::query(
            "UPDATE cloud_agents
             SET status = 'revoked', revoked_at = NOW(), updated_at = NOW()
             WHERE id = $1 AND user_id = $2 AND status = 'claimed'",
        )
        .bind(agent_id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
        transaction.commit().await.map_err(CloudError::database)?;
        return Err(CloudError::new(
            StatusCode::GONE,
            "pairing_expired",
            "agent pairing has expired",
        ));
    }
    let sql = format!(
        "UPDATE cloud_agents SET status = 'active', confirmed_at = NOW(), updated_at = NOW()
         WHERE id = $1 AND user_id = $2 AND status = 'claimed'
         RETURNING {}",
        AGENT_SELECT
            .trim_start_matches("SELECT ")
            .trim_end_matches(" FROM cloud_agents")
    );
    let row = sqlx::query(&sql)
        .bind(agent_id)
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(CloudError::database)?
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::CONFLICT,
                "agent_not_claimed",
                "claimed agent is no longer available",
            )
        })?;
    sqlx::query(
        "UPDATE cloud_agent_pairings
         SET status = 'confirmed', confirmed_at = NOW()
         WHERE claimed_agent_id = $1 AND status = 'claimed'",
    )
    .bind(agent_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(agent_view(&row, Utc::now()))
}

async fn revoke_agent(
    Path(agent_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<StatusCode> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let result = sqlx::query(
        "UPDATE cloud_agents
         SET status = 'revoked', revoked_at = NOW(), updated_at = NOW()
         WHERE id = $1 AND user_id = $2 AND status != 'revoked'",
    )
    .bind(agent_id)
    .bind(user.user_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    if result.rows_affected() == 0 {
        return Err(CloudError::new(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "agent was not found or was already revoked",
        ));
    }
    sqlx::query(
        "UPDATE cloud_agent_pairings SET status = 'revoked'
         WHERE claimed_agent_id = $1 AND status IN ('claimed', 'confirmed')",
    )
    .bind(agent_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(StatusCode::NO_CONTENT)
}

fn agent_task_event_view(row: &sqlx::postgres::PgRow) -> AgentTaskEventView {
    AgentTaskEventView {
        seq: row.get("seq"),
        level: row.get("level"),
        message: row.get("message"),
        data: row.get("data"),
        created_at: row.get("created_at"),
    }
}

fn agent_task_checkpoint_view(row: &sqlx::postgres::PgRow) -> AgentTaskCheckpointView {
    AgentTaskCheckpointView {
        id: row.get("id"),
        task_id: row.get("task_id"),
        seq: row.get("seq"),
        checkpoint_key: row.get("checkpoint_key"),
        kind: row.get("kind"),
        resumable: row.get("resumable"),
        payload: row.get("payload"),
        created_at: row.get("created_at"),
    }
}

fn agent_task_checkpoint_metadata(row: &sqlx::postgres::PgRow) -> AgentTaskCheckpointMetadata {
    AgentTaskCheckpointMetadata {
        id: row.get("id"),
        seq: row.get("seq"),
        checkpoint_key: row.get("checkpoint_key"),
        kind: row.get("kind"),
        resumable: row.get("resumable"),
        created_at: row.get("created_at"),
    }
}

fn agent_task_is_terminal(status: &str) -> bool {
    ["succeeded", "failed", "cancelled"].contains(&status)
}

fn agent_task_view(
    row: &sqlx::postgres::PgRow,
    events: Vec<AgentTaskEventView>,
    latest_checkpoint: Option<AgentTaskCheckpointMetadata>,
    can_resume: bool,
) -> AgentTaskView {
    let cancel_requested_at = row.get("cancel_requested_at");
    AgentTaskView {
        id: row.get("id"),
        agent_id: row.get("agent_id"),
        team_id: row.get("team_id"),
        operation: row.get("operation"),
        required_permission: row.get("required_permission"),
        risk: row.get("risk"),
        input: row.get("input"),
        status: row.get("status"),
        idempotency_key: row.get("idempotency_key"),
        source_task_id: row.get("source_task_id"),
        approval_id: row.get("approval_id"),
        lineage_id: row.get("lineage_id"),
        attempt_no: row.get("attempt_no"),
        retry_of_task_id: row.get("retry_of_task_id"),
        execution_mode: row.get("execution_mode"),
        resume_checkpoint_id: row.get("resume_checkpoint_id"),
        rollback_source_task_id: row.get("rollback_source_task_id"),
        approved_by: row.get("approved_by"),
        approved_at: row.get("approved_at"),
        lease_expires_at: row.get("lease_expires_at"),
        leased_at: row.get("leased_at"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        cancelled_at: row.get("cancelled_at"),
        cancel_requested_at,
        cancel_requested_by: row.get("cancel_requested_by"),
        cancel_acknowledged_at: row.get("cancel_acknowledged_at"),
        cancel_requested: cancel_requested_at.is_some(),
        output: row.get("output"),
        error: row.get("error"),
        artifacts: row.get("artifacts"),
        rollback_available: row.get("rollback_available"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        latest_checkpoint,
        can_resume,
        events,
    }
}

async fn load_agent_task_checkpoint_state(
    cloud: &CloudState,
    row: &sqlx::postgres::PgRow,
) -> CloudResult<(Option<AgentTaskCheckpointMetadata>, bool)> {
    let task_id: Uuid = row.get("id");
    let latest = sqlx::query(
        "SELECT id, seq, checkpoint_key, kind, resumable, created_at
         FROM cloud_agent_task_checkpoints WHERE task_id = $1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .as_ref()
    .map(agent_task_checkpoint_metadata);
    let status: String = row.get("status");
    let operation: String = row.get("operation");
    let can_resume = if agent_task_is_terminal(&status) && operation != "task.rollback" {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
               SELECT 1 FROM cloud_agent_task_checkpoints c
               JOIN cloud_agent_tasks t ON t.id = c.task_id
               WHERE t.lineage_id = $1 AND t.attempt_no <= $2
                 AND c.kind = 'result' AND c.resumable
             )",
        )
        .bind(row.get::<Uuid, _>("lineage_id"))
        .bind(row.get::<i32, _>("attempt_no"))
        .fetch_one(&cloud.db)
        .await
        .map_err(CloudError::database)?
    } else {
        false
    };
    Ok((latest, can_resume))
}

async fn load_agent_task(
    cloud: &CloudState,
    user_id: Uuid,
    task_id: Uuid,
) -> CloudResult<AgentTaskView> {
    let sql = format!("{AGENT_TASK_SELECT} WHERE id = $1 AND user_id = $2");
    let row = sqlx::query(&sql)
        .bind(task_id)
        .bind(user_id)
        .fetch_optional(&cloud.db)
        .await
        .map_err(CloudError::database)?
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::NOT_FOUND,
                "agent_task_not_found",
                "agent task was not found",
            )
        })?;
    let events = sqlx::query(
        "SELECT seq, level, message, data, created_at
         FROM cloud_agent_task_events WHERE task_id = $1 ORDER BY seq",
    )
    .bind(task_id)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .iter()
    .map(agent_task_event_view)
    .collect();
    let (latest_checkpoint, can_resume) = load_agent_task_checkpoint_state(cloud, &row).await?;
    Ok(agent_task_view(&row, events, latest_checkpoint, can_resume))
}

async fn append_agent_task_event(
    transaction: &mut Transaction<'_, Postgres>,
    task_id: Uuid,
    level: &str,
    message: &str,
    data: Option<Value>,
) -> CloudResult<AgentTaskEventView> {
    let row = sqlx::query(
        "INSERT INTO cloud_agent_task_events (id, task_id, seq, level, message, data)
         SELECT $1, $2, COALESCE(MAX(seq), 0) + 1, $3, $4, $5
         FROM cloud_agent_task_events WHERE task_id = $2
         HAVING COUNT(*) < 2000
         RETURNING seq, level, message, data, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(task_id)
    .bind(level)
    .bind(message)
    .bind(data)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "agent_task_event_limit",
            "agent task already has the maximum number of events",
        )
    })?;
    Ok(agent_task_event_view(&row))
}

async fn lock_user_agent_for_task(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    agent_id: Uuid,
    permission: &str,
    additional_capability: Option<&str>,
) -> CloudResult<()> {
    let row = sqlx::query(
        "SELECT status, capabilities, permissions FROM cloud_agents
         WHERE id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(agent_id)
    .bind(user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "agent was not found",
        )
    })?;
    if row.get::<String, _>("status") != "active" {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_not_active",
            "agent must be active before tasks can be created",
        ));
    }
    let capabilities: Vec<String> =
        serde_json::from_value(row.get("capabilities")).unwrap_or_default();
    let permissions: Vec<String> =
        serde_json::from_value(row.get("permissions")).unwrap_or_default();
    if !capabilities.iter().any(|value| value == "tasks-v1") {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "agent_capability_missing",
            "agent does not advertise tasks-v1",
        ));
    }
    if additional_capability
        .is_some_and(|required| !capabilities.iter().any(|capability| capability == required))
    {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "agent_capability_missing",
            "agent does not advertise the capability required by this operation",
        ));
    }
    if !agent_permissions_allow(&permissions, permission) {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "agent_permission_missing",
            "agent does not have the permission required by this operation",
        ));
    }
    Ok(())
}

async fn list_agent_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Vec<AgentTaskView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    reconcile_expired_agent_tasks(&mut transaction, ExpiredAgentTaskScope::User(user.user_id))
        .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    let sql = format!("{AGENT_TASK_SELECT} WHERE user_id = $1 ORDER BY created_at DESC LIMIT 100");
    let rows = sqlx::query(&sql)
        .bind(user.user_id)
        .fetch_all(&cloud.db)
        .await
        .map_err(CloudError::database)?;
    let mut tasks = Vec::with_capacity(rows.len());
    for row in rows {
        let task_id: Uuid = row.get("id");
        let events = sqlx::query(
            "SELECT seq, level, message, data, created_at
             FROM cloud_agent_task_events WHERE task_id = $1 ORDER BY seq",
        )
        .bind(task_id)
        .fetch_all(&cloud.db)
        .await
        .map_err(CloudError::database)?
        .iter()
        .map(agent_task_event_view)
        .collect();
        let (latest_checkpoint, can_resume) =
            load_agent_task_checkpoint_state(&cloud, &row).await?;
        tasks.push(agent_task_view(&row, events, latest_checkpoint, can_resume));
    }
    Ok(Json(tasks))
}

async fn get_agent_task(
    Path(task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<AgentTaskView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    reconcile_expired_agent_tasks(&mut transaction, ExpiredAgentTaskScope::User(user.user_id))
        .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(load_agent_task(&cloud, user.user_id, task_id).await?))
}

fn agent_task_team_required_error() -> CloudError {
    CloudError::new(
        StatusCode::BAD_REQUEST,
        "agent_task_team_required",
        "high-risk agent tasks require an explicit team_id when the account belongs to zero or multiple teams",
    )
}

fn agent_task_approval_missing_error() -> CloudError {
    CloudError::new(
        StatusCode::CONFLICT,
        "agent_task_approval_missing",
        "this high-risk agent task has no linked team approval and cannot be approved; create a new task",
    )
}

fn agent_task_approval_invalid_error() -> CloudError {
    CloudError::new(
        StatusCode::FORBIDDEN,
        "agent_task_approval_invalid",
        "the linked team approval does not have a valid independent team decision",
    )
}

fn terminal_team_required_error() -> CloudError {
    CloudError::new(
        StatusCode::BAD_REQUEST,
        "terminal_team_required",
        "persistent terminal sessions require an explicit team_id when the account belongs to zero or multiple teams",
    )
}

async fn resolve_approval_team(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    requested_team_id: Option<Uuid>,
    approval_required: bool,
    required_error: fn() -> CloudError,
) -> CloudResult<Option<Uuid>> {
    if requested_team_id.is_none() && !approval_required {
        return Ok(None);
    }
    let memberships = sqlx::query_scalar::<_, Uuid>(
        "SELECT team_id FROM cloud_team_members
         WHERE user_id = $1 ORDER BY team_id FOR SHARE",
    )
    .bind(user_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    if let Some(team_id) = requested_team_id {
        if memberships.contains(&team_id) {
            return Ok(Some(team_id));
        }
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "team_access_denied",
            "the requester must be a member of the selected team",
        ));
    }
    if approval_required {
        return match memberships.as_slice() {
            [team_id] => Ok(Some(*team_id)),
            _ => Err(required_error()),
        };
    }
    Ok(None)
}

async fn create_agent_task_approval(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    team_id: Option<Uuid>,
    task_id: Uuid,
    agent_id: Uuid,
    operation: &str,
    required_permission: &str,
    risk: &str,
    input: &Value,
) -> CloudResult<Option<Uuid>> {
    let approval_risk = match risk {
        "high" | "critical" => "high",
        _ => return Ok(None),
    };
    let team_id = team_id.ok_or_else(agent_task_team_required_error)?;
    let member = sqlx::query(
        "SELECT role FROM cloud_team_members
         WHERE team_id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    if member.is_none() {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "team_access_denied",
            "the requester must be a member of the selected team",
        ));
    }

    let approval_id = Uuid::new_v4();
    let title = format!("Cloud Agent task: {operation}");
    let summary = format!(
        "Request to execute {operation} with {required_permission} permission; review the linked task input before deciding."
    );
    let payload = json!({
        "source": "cloud-agent-task",
        "task_id": task_id,
        "agent_id": agent_id,
        "operation": operation,
        "required_permission": required_permission,
        "risk": risk,
        "input": input,
    });
    sqlx::query(
        "INSERT INTO cloud_approvals
         (id, team_id, requested_by, title, summary, risk, payload, agent_task_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(approval_id)
    .bind(team_id)
    .bind(user_id)
    .bind(title)
    .bind(summary)
    .bind(approval_risk)
    .bind(payload)
    .bind(task_id)
    .execute(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    Ok(Some(approval_id))
}

async fn sync_agent_task_after_approval(
    transaction: &mut Transaction<'_, Postgres>,
    approval_id: Uuid,
    linked_task_id: Option<Uuid>,
    team_id: Uuid,
    requested_by: Uuid,
    decided_by: Uuid,
    decision: &str,
) -> CloudResult<()> {
    let Some(task_id) = linked_task_id else {
        return Ok(());
    };
    let task = sqlx::query(
        "SELECT id, user_id, team_id, risk, status, approval_id
         FROM cloud_agent_tasks
         WHERE id = $1 AND approval_id = $2 FOR UPDATE",
    )
    .bind(task_id)
    .bind(approval_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(agent_task_approval_invalid_error)?;
    if task.get::<Uuid, _>("user_id") != requested_by
        || task.get::<Option<Uuid>, _>("team_id") != Some(team_id)
        || task.get::<String, _>("risk") == "low"
        || task.get::<Option<Uuid>, _>("approval_id") != Some(approval_id)
        || task.get::<String, _>("status") != "awaiting_approval"
    {
        return Err(agent_task_approval_invalid_error());
    }
    match decision {
        "approved" => {
            sqlx::query(
                "UPDATE cloud_agent_tasks
                 SET status = 'queued', approved_by = $2, approved_at = NOW(), updated_at = NOW()
                 WHERE id = $1 AND status = 'awaiting_approval'",
            )
            .bind(task_id)
            .bind(decided_by)
            .execute(&mut **transaction)
            .await
            .map_err(CloudError::database)?;
            append_agent_task_event(
                transaction,
                task_id,
                "info",
                "Task approved",
                Some(json!({
                    "status": "queued",
                    "approval_id": approval_id,
                    "approved_by": decided_by
                })),
            )
            .await?;
        }
        "rejected" => {
            sqlx::query(
                "UPDATE cloud_agent_tasks
                 SET status = 'cancelled', cancelled_at = NOW(), completed_at = NOW(),
                     updated_at = NOW()
                 WHERE id = $1 AND status = 'awaiting_approval'",
            )
            .bind(task_id)
            .execute(&mut **transaction)
            .await
            .map_err(CloudError::database)?;
            append_agent_task_event(
                transaction,
                task_id,
                "warn",
                "Task approval rejected",
                Some(json!({
                    "status": "cancelled",
                    "approval_id": approval_id,
                    "rejected_by": decided_by
                })),
            )
            .await?;
        }
        _ => {
            return Err(CloudError::bad_request(
                "审批决定必须为 approved 或 rejected",
            ));
        }
    }
    Ok(())
}

async fn sync_terminal_after_approval(
    transaction: &mut Transaction<'_, Postgres>,
    approval_id: Uuid,
    linked_session_id: Option<Uuid>,
    team_id: Uuid,
    requested_by: Uuid,
    decided_by: Uuid,
    decision: &str,
) -> CloudResult<()> {
    let Some(session_id) = linked_session_id else {
        return Ok(());
    };
    let session = sqlx::query(
        "SELECT id, user_id, team_id, approval_id, status, cwd, cols, rows, next_command_seq
         FROM cloud_terminal_sessions
         WHERE id = $1 AND approval_id = $2 FOR UPDATE",
    )
    .bind(session_id)
    .bind(approval_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::FORBIDDEN,
            "terminal_approval_invalid",
            "the linked terminal approval does not match the session",
        )
    })?;
    if session.get::<Uuid, _>("user_id") != requested_by
        || session.get::<Option<Uuid>, _>("team_id") != Some(team_id)
        || session.get::<Option<Uuid>, _>("approval_id") != Some(approval_id)
        || session.get::<String, _>("status") != "awaiting_approval"
    {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "terminal_approval_invalid",
            "the linked terminal approval is no longer valid",
        ));
    }
    match decision {
        "approved" => {
            let seq = session.get::<i64, _>("next_command_seq") + 1;
            sqlx::query(
                "INSERT INTO cloud_terminal_commands (id, session_id, seq, kind, payload)
                 VALUES ($1, $2, $3, 'start', $4)",
            )
            .bind(Uuid::new_v4())
            .bind(session_id)
            .bind(seq)
            .bind(json!({
                "cwd": session.get::<Option<String>, _>("cwd"),
                "cols": session.get::<i32, _>("cols"),
                "rows": session.get::<i32, _>("rows"),
            }))
            .execute(&mut **transaction)
            .await
            .map_err(CloudError::database)?;
            sqlx::query(
                "UPDATE cloud_terminal_sessions
                 SET status = 'pending', approved_by = $2, approved_at = NOW(),
                     next_command_seq = $3, updated_at = NOW()
                 WHERE id = $1 AND status = 'awaiting_approval'",
            )
            .bind(session_id)
            .bind(decided_by)
            .bind(seq)
            .execute(&mut **transaction)
            .await
            .map_err(CloudError::database)?;
        }
        "rejected" => {
            sqlx::query(
                "UPDATE cloud_terminal_sessions
                 SET status = 'cancelled', exited_at = NOW(), updated_at = NOW()
                 WHERE id = $1 AND status = 'awaiting_approval'",
            )
            .bind(session_id)
            .execute(&mut **transaction)
            .await
            .map_err(CloudError::database)?;
        }
        _ => {
            return Err(CloudError::bad_request(
                "审批决定必须为 approved 或 rejected",
            ));
        }
    }
    Ok(())
}

struct CreatedAgentTask {
    id: Uuid,
    created: bool,
}

async fn create_agent_task_record(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    agent_id: Uuid,
    operation: &str,
    input: &Value,
    idempotency_key: Option<String>,
    team_id: Option<Uuid>,
) -> CloudResult<CreatedAgentTask> {
    let operation = operation.trim().to_ascii_lowercase();
    let spec = validate_agent_task_input(&operation, input, false)?;
    let team_id = resolve_approval_team(
        transaction,
        user_id,
        team_id,
        spec.approval_required,
        agent_task_team_required_error,
    )
    .await?;
    let idempotency_key = validate_idempotency_key(idempotency_key)?;
    if let Some(key) = &idempotency_key {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("{user_id}:{key}"))
            .execute(&mut **transaction)
            .await
            .map_err(CloudError::database)?;
        let existing = sqlx::query(
            "SELECT id, agent_id, team_id, operation, input, approval_id FROM cloud_agent_tasks
             WHERE user_id = $1 AND idempotency_key = $2 FOR UPDATE",
        )
        .bind(user_id)
        .bind(key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(CloudError::database)?;
        if let Some(existing) = existing {
            if existing.get::<Uuid, _>("agent_id") != agent_id
                || existing.get::<String, _>("operation") != operation
                || existing.get::<Value, _>("input") != *input
            {
                return Err(CloudError::new(
                    StatusCode::CONFLICT,
                    "idempotency_conflict",
                    "idempotency_key was already used for a different task",
                ));
            }
            if spec.approval_required {
                let existing_approval_id = existing.get::<Option<Uuid>, _>("approval_id");
                if existing.get::<Option<Uuid>, _>("team_id") != team_id {
                    return Err(CloudError::new(
                        StatusCode::CONFLICT,
                        "idempotency_conflict",
                        "idempotency_key was already used with a different approval team",
                    ));
                }
                let existing_team_id = if let Some(approval_id) = existing_approval_id {
                    sqlx::query_scalar::<_, Uuid>(
                        "SELECT team_id FROM cloud_approvals
                         WHERE id = $1 AND requested_by = $2 AND agent_task_id = $3",
                    )
                    .bind(approval_id)
                    .bind(user_id)
                    .bind(existing.get::<Uuid, _>("id"))
                    .fetch_optional(&mut **transaction)
                    .await
                    .map_err(CloudError::database)?
                } else {
                    None
                };
                if existing_team_id.is_none() {
                    return Err(agent_task_approval_missing_error());
                }
                if existing_team_id != team_id {
                    return Err(CloudError::new(
                        StatusCode::CONFLICT,
                        "idempotency_conflict",
                        "idempotency_key was already used with a different approval team",
                    ));
                }
            }
            return Ok(CreatedAgentTask {
                id: existing.get("id"),
                created: false,
            });
        }
    }
    lock_user_agent_for_task(
        transaction,
        user_id,
        agent_id,
        spec.permission,
        spec.additional_capability,
    )
    .await?;
    let task_id = Uuid::new_v4();
    let status = if spec.approval_required {
        "awaiting_approval"
    } else {
        "queued"
    };
    let approval_id = create_agent_task_approval(
        transaction,
        user_id,
        team_id,
        task_id,
        agent_id,
        &operation,
        spec.permission,
        spec.risk,
        input,
    )
    .await?;
    sqlx::query(
        "INSERT INTO cloud_agent_tasks
         (id, user_id, agent_id, team_id, operation, required_permission, risk, input, status, idempotency_key,
          approval_id, lineage_id, attempt_no, execution_mode)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $1, 1, 'original')",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(agent_id)
    .bind(team_id)
    .bind(&operation)
    .bind(spec.permission)
    .bind(spec.risk)
    .bind(input.clone())
    .bind(status)
    .bind(idempotency_key)
    .bind(approval_id)
    .execute(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    append_agent_task_event(
        transaction,
        task_id,
        "info",
        "Task created",
        Some(json!({ "status": status, "approval_id": approval_id })),
    )
    .await?;
    Ok(CreatedAgentTask {
        id: task_id,
        created: true,
    })
}

async fn create_agent_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateAgentTaskRequest>,
) -> CloudResult<(StatusCode, Json<AgentTaskView>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let task = create_agent_task_record(
        &mut transaction,
        user.user_id,
        request.agent_id,
        &request.operation,
        &request.input,
        request.idempotency_key,
        request.team_id,
    )
    .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((
        if task.created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(load_agent_task(&cloud, user.user_id, task.id).await?),
    ))
}

async fn approve_agent_task(
    Path(task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<AgentTaskView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let task = sqlx::query(
        "SELECT status, risk, user_id, team_id, approval_id FROM cloud_agent_tasks
         WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(task_id)
    .bind(user.user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "agent_task_not_found",
            "agent task was not found",
        )
    })?;
    let status: String = task.get("status");
    if status != "awaiting_approval" {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_task_not_awaiting_approval",
            "agent task is not awaiting approval",
        ));
    }
    let mut approved_by = user.user_id;
    if task.get::<String, _>("risk") != "low" {
        let approval_id = task
            .get::<Option<Uuid>, _>("approval_id")
            .ok_or_else(agent_task_approval_missing_error)?;
        let approval = sqlx::query(
            "SELECT team_id, requested_by, status, decided_by, agent_task_id
             FROM cloud_approvals WHERE id = $1 FOR UPDATE",
        )
        .bind(approval_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(CloudError::database)?
        .ok_or_else(agent_task_approval_missing_error)?;
        let requested_by: Uuid = approval.get("requested_by");
        if requested_by != task.get::<Uuid, _>("user_id") {
            return Err(agent_task_approval_invalid_error());
        }
        if approval.get::<Option<Uuid>, _>("agent_task_id") != Some(task_id) {
            return Err(agent_task_approval_invalid_error());
        }
        if task.get::<Option<Uuid>, _>("team_id") != Some(approval.get("team_id")) {
            return Err(agent_task_approval_invalid_error());
        }
        match approval.get::<String, _>("status").as_str() {
            "pending" => {
                return Err(CloudError::new(
                    StatusCode::CONFLICT,
                    "agent_task_approval_pending",
                    "the linked team approval is still pending",
                ));
            }
            "rejected" | "cancelled" => {
                return Err(CloudError::new(
                    StatusCode::CONFLICT,
                    "agent_task_approval_rejected",
                    "the linked team approval was not approved",
                ));
            }
            "approved" => {}
            _ => return Err(agent_task_approval_invalid_error()),
        }
        let decided_by = approval
            .get::<Option<Uuid>, _>("decided_by")
            .ok_or_else(agent_task_approval_invalid_error)?;
        if decided_by == requested_by {
            return Err(CloudError::new(
                StatusCode::FORBIDDEN,
                "agent_task_approval_self",
                "the task requester cannot be the team approval decision maker",
            ));
        }
        let role = sqlx::query_scalar::<_, String>(
            "SELECT role FROM cloud_team_members
             WHERE team_id = $1 AND user_id = $2 FOR SHARE",
        )
        .bind(approval.get::<Uuid, _>("team_id"))
        .bind(decided_by)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
        if !role
            .as_deref()
            .is_some_and(|role| ["owner", "admin", "approver"].contains(&role))
        {
            return Err(agent_task_approval_invalid_error());
        }
        approved_by = decided_by;
    }
    sqlx::query(
        "UPDATE cloud_agent_tasks
         SET status = 'queued', approved_by = $2, approved_at = NOW(), updated_at = NOW()
         WHERE id = $1 AND status = 'awaiting_approval'",
    )
    .bind(task_id)
    .bind(approved_by)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    append_agent_task_event(
        &mut transaction,
        task_id,
        "info",
        "Task approved",
        Some(json!({ "status": "queued", "approval_id": task.get::<Option<Uuid>, _>("approval_id"), "approved_by": approved_by })),
    )
    .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(load_agent_task(&cloud, user.user_id, task_id).await?))
}

async fn cancel_agent_task(
    Path(task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<AgentTaskView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let task = sqlx::query(
        "SELECT status, operation, cancel_requested_at FROM cloud_agent_tasks
         WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(task_id)
    .bind(user.user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "agent_task_not_found",
            "agent task was not found",
        )
    })?;
    let status: String = task.get("status");
    if status == "running" {
        if !agent_task_supports_running_cancellation(&task.get::<String, _>("operation")) {
            return Err(CloudError::new(
                StatusCode::CONFLICT,
                "agent_task_not_interruptible",
                "this running task does not support cooperative cancellation",
            ));
        }
        if task
            .get::<Option<DateTime<Utc>>, _>("cancel_requested_at")
            .is_none()
        {
            sqlx::query(
                "UPDATE cloud_agent_tasks
                 SET cancel_requested_at = NOW(), cancel_requested_by = $2, updated_at = NOW()
                 WHERE id = $1 AND status = 'running' AND cancel_requested_at IS NULL",
            )
            .bind(task_id)
            .bind(user.user_id)
            .execute(&mut *transaction)
            .await
            .map_err(CloudError::database)?;
            append_agent_task_event(
                &mut transaction,
                task_id,
                "warn",
                "Task cancellation requested",
                None,
            )
            .await?;
        }
        transaction.commit().await.map_err(CloudError::database)?;
        return Ok(Json(load_agent_task(&cloud, user.user_id, task_id).await?));
    }
    if !agent_task_is_cancellable(&status) {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_task_not_cancellable",
            "agent task is not cancellable",
        ));
    }
    sqlx::query(
        "UPDATE cloud_agent_tasks
         SET status = 'cancelled', cancelled_at = NOW(), completed_at = NOW(),
             lease_token_hash = NULL, lease_expires_at = NULL, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(task_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sqlx::query(
        "UPDATE cloud_approvals
         SET status = 'cancelled', decision_comment = '任务已取消', decided_at = NOW()
         WHERE agent_task_id = $1 AND status = 'pending'",
    )
    .bind(task_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    append_agent_task_event(
        &mut transaction,
        task_id,
        "warn",
        "Task cancelled by user",
        Some(json!({ "previous_status": status })),
    )
    .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(load_agent_task(&cloud, user.user_id, task_id).await?))
}

async fn rollback_agent_task(
    Path(source_task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<(StatusCode, Json<AgentTaskView>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let source = sqlx::query(
        "SELECT t.agent_id, t.team_id, t.status, t.rollback_available, t.rollback_source_task_id,
                t.approval_id,
                (SELECT a.team_id FROM cloud_approvals a
                 WHERE a.id = t.approval_id AND a.agent_task_id = t.id
                   AND a.requested_by = t.user_id AND a.status = 'approved'
                   AND a.decided_by = t.approved_by) AS approval_team_id
         FROM cloud_agent_tasks t
         WHERE t.id = $1 AND t.user_id = $2 FOR UPDATE",
    )
    .bind(source_task_id)
    .bind(user.user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "agent_task_not_found",
            "source agent task was not found",
        )
    })?;
    if source.get::<String, _>("status") != "succeeded"
        || !source.get::<bool, _>("rollback_available")
    {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "rollback_not_available",
            "source agent task is not rollback-capable",
        ));
    }
    let approval_team_id = source
        .get::<Option<Uuid>, _>("approval_team_id")
        .ok_or_else(agent_task_approval_missing_error)?;
    if source.get::<Option<Uuid>, _>("team_id") != Some(approval_team_id) {
        return Err(agent_task_approval_invalid_error());
    }
    let agent_id: Uuid = source.get("agent_id");
    let local_source_task_id: Uuid = source
        .get::<Option<Uuid>, _>("rollback_source_task_id")
        .unwrap_or(source_task_id);
    if let Some(existing_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM cloud_agent_tasks
         WHERE source_task_id = $1 AND operation = 'task.rollback'
           AND status NOT IN ('failed', 'cancelled')
         ORDER BY created_at DESC LIMIT 1 FOR UPDATE",
    )
    .bind(local_source_task_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    {
        transaction.commit().await.map_err(CloudError::database)?;
        return Ok((
            StatusCode::OK,
            Json(load_agent_task(&cloud, user.user_id, existing_id).await?),
        ));
    }
    lock_user_agent_for_task(&mut transaction, user.user_id, agent_id, "write", None).await?;
    let task_id = Uuid::new_v4();
    let input = json!({ "source_task_id": local_source_task_id });
    validate_agent_task_input("task.rollback", &input, true)?;
    let approval_id = create_agent_task_approval(
        &mut transaction,
        user.user_id,
        Some(approval_team_id),
        task_id,
        agent_id,
        "task.rollback",
        "write",
        "high",
        &input,
    )
    .await?
    .expect("rollback tasks always require team approval");
    sqlx::query(
        "INSERT INTO cloud_agent_tasks
         (id, user_id, agent_id, team_id, operation, required_permission, risk, input, status, source_task_id,
          approval_id, lineage_id, attempt_no, execution_mode)
         VALUES ($1, $2, $3, $4, 'task.rollback', 'write', 'high', $5, 'awaiting_approval', $6,
                 $7, $1, 1, 'original')",
    )
    .bind(task_id)
    .bind(user.user_id)
    .bind(agent_id)
    .bind(approval_team_id)
    .bind(input)
    .bind(local_source_task_id)
    .bind(approval_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    append_agent_task_event(
        &mut transaction,
        task_id,
        "warn",
        "Rollback task created and awaiting approval",
        Some(json!({
            "requested_task_id": source_task_id,
            "source_task_id": local_source_task_id,
            "approval_id": approval_id
        })),
    )
    .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(load_agent_task(&cloud, user.user_id, task_id).await?),
    ))
}

async fn retry_agent_task(
    Path(source_task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RetryAgentTaskRequest>,
) -> CloudResult<(StatusCode, Json<AgentTaskView>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mode = request.mode.trim().to_ascii_lowercase();
    if !["restart", "resume"].contains(&mode.as_str()) {
        return Err(CloudError::bad_request(
            "retry mode must be restart or resume",
        ));
    }
    let retry_key = validate_idempotency_key(Some(request.idempotency_key))?
        .expect("a required idempotency key remains present");
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let source = sqlx::query(
        "SELECT t.id, t.user_id, t.agent_id, t.team_id, t.operation, t.required_permission, t.risk,
                t.input, t.status, t.approval_id, t.lineage_id, t.attempt_no,
                (SELECT a.team_id FROM cloud_approvals a
                 WHERE a.id = t.approval_id AND a.agent_task_id = t.id
                   AND a.requested_by = t.user_id AND a.status = 'approved'
                   AND a.decided_by = t.approved_by) AS approval_team_id
         FROM cloud_agent_tasks t WHERE t.id = $1 AND t.user_id = $2 FOR UPDATE",
    )
    .bind(source_task_id)
    .bind(user.user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "agent_task_not_found",
            "source agent task was not found",
        )
    })?;
    let source_status: String = source.get("status");
    let operation: String = source.get("operation");
    let source_risk: String = source.get("risk");
    let approval_team_id = if source_risk == "low" {
        None
    } else {
        Some(
            source
                .get::<Option<Uuid>, _>("approval_team_id")
                .ok_or_else(agent_task_approval_missing_error)?,
        )
    };
    if source_risk != "low" && source.get::<Option<Uuid>, _>("team_id") != approval_team_id {
        return Err(agent_task_approval_invalid_error());
    }
    if !agent_task_is_terminal(&source_status) {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_task_not_terminal",
            "only terminal agent tasks can be retried",
        ));
    }
    if operation == "task.rollback" {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "rollback_retry_forbidden",
            "rollback tasks cannot be retried",
        ));
    }
    let lineage_id: Uuid = source.get("lineage_id");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("task-lineage:{lineage_id}"))
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    if let Some(existing) = sqlx::query(
        "SELECT id, execution_mode, approval_id FROM cloud_agent_tasks
         WHERE retry_of_task_id = $1 AND retry_request_key = $2 FOR UPDATE",
    )
    .bind(source_task_id)
    .bind(&retry_key)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    {
        if existing.get::<String, _>("execution_mode") != mode {
            return Err(CloudError::new(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "idempotency_key was already used with a different retry mode",
            ));
        }
        if source_risk != "low" && existing.get::<Option<Uuid>, _>("approval_id").is_none() {
            return Err(agent_task_approval_missing_error());
        }
        let existing_id: Uuid = existing.get("id");
        transaction.commit().await.map_err(CloudError::database)?;
        return Ok((
            StatusCode::OK,
            Json(load_agent_task(&cloud, user.user_id, existing_id).await?),
        ));
    }
    let agent_id: Uuid = source.get("agent_id");
    let input: Value = source.get("input");
    let spec = validate_agent_task_input(&operation, &input, false)?;
    let required_permission: String = source.get("required_permission");
    let risk: String = source.get("risk");
    lock_user_agent_for_task(
        &mut transaction,
        user.user_id,
        agent_id,
        spec.permission,
        spec.additional_capability,
    )
    .await?;
    if mode == "resume" {
        lock_user_agent_for_task(
            &mut transaction,
            user.user_id,
            agent_id,
            spec.permission,
            Some("task-checkpoints-v1"),
        )
        .await?;
    }
    let resume_checkpoint = if mode == "resume" {
        Some(
            sqlx::query(
                "SELECT c.id, c.task_id, c.kind, c.payload
                 FROM cloud_agent_task_checkpoints c
                 JOIN cloud_agent_tasks t ON t.id = c.task_id
                 WHERE t.lineage_id = $1 AND t.attempt_no <= $2
                   AND c.kind = 'result' AND c.resumable
                 ORDER BY t.attempt_no DESC, c.seq DESC LIMIT 1 FOR SHARE OF c",
            )
            .bind(lineage_id)
            .bind(source.get::<i32, _>("attempt_no"))
            .fetch_optional(&mut *transaction)
            .await
            .map_err(CloudError::database)?
            .ok_or_else(|| {
                CloudError::new(
                    StatusCode::CONFLICT,
                    "resume_checkpoint_unavailable",
                    "no resumable result checkpoint is available for this task",
                )
            })?,
        )
    } else {
        None
    };
    let attempt_no = sqlx::query_scalar::<_, i32>(
        "SELECT COALESCE(MAX(attempt_no), 0) + 1 FROM cloud_agent_tasks WHERE lineage_id = $1",
    )
    .bind(lineage_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    let task_id = Uuid::new_v4();
    let status = if spec.approval_required {
        "awaiting_approval"
    } else {
        "queued"
    };
    let resume_checkpoint_id = resume_checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.get::<Uuid, _>("id"));
    let approval_id = create_agent_task_approval(
        &mut transaction,
        user.user_id,
        approval_team_id,
        task_id,
        agent_id,
        &operation,
        &required_permission,
        &risk,
        &input,
    )
    .await?;
    sqlx::query(
        "INSERT INTO cloud_agent_tasks
         (id, user_id, agent_id, team_id, operation, required_permission, risk, input, status,
          lineage_id, attempt_no, retry_of_task_id, execution_mode, resume_checkpoint_id,
          retry_request_key, approval_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
    )
    .bind(task_id)
    .bind(user.user_id)
    .bind(agent_id)
    .bind(approval_team_id)
    .bind(&operation)
    .bind(&required_permission)
    .bind(&risk)
    .bind(input)
    .bind(status)
    .bind(lineage_id)
    .bind(attempt_no)
    .bind(source_task_id)
    .bind(&mode)
    .bind(resume_checkpoint_id)
    .bind(&retry_key)
    .bind(approval_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    append_agent_task_event(
        &mut transaction,
        task_id,
        "info",
        "Task retry created",
        Some(json!({
            "mode": mode,
            "retry_of_task_id": source_task_id,
            "lineage_id": lineage_id,
            "attempt_no": attempt_no,
            "resume_checkpoint_id": resume_checkpoint_id,
            "status": status,
            "approval_id": approval_id
        })),
    )
    .await?;

    let linked_conversation = sqlx::query_scalar::<_, Uuid>(
        "SELECT conversation_id FROM cloud_conversation_messages
         WHERE linked_task_id = $1 AND kind = 'plan'",
    )
    .bind(source_task_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    if let Some(conversation_id) = linked_conversation {
        let (last_seq, _) =
            lock_conversation_for_message(&mut transaction, user.user_id, conversation_id).await?;
        sqlx::query(
            "INSERT INTO cloud_conversation_messages
             (id, conversation_id, seq, role, content, kind, linked_task_id)
             VALUES ($1, $2, $3, 'assistant', $4, 'plan', $5)",
        )
        .bind(Uuid::new_v4())
        .bind(conversation_id)
        .bind(last_seq + 1)
        .bind(if mode == "resume" {
            "已从检查点创建恢复任务，请查看新任务状态并批准高风险操作。"
        } else {
            "已创建重新执行任务，请查看新任务状态并批准高风险操作。"
        })
        .bind(task_id)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
        sqlx::query(
            "UPDATE cloud_conversations SET next_message_seq = $2, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(conversation_id)
        .bind(last_seq + 1)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    }
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(load_agent_task(&cloud, user.user_id, task_id).await?),
    ))
}

fn agent_task_token_hash(headers: &HeaderMap) -> CloudResult<String> {
    let token = bearer(headers)?;
    if !valid_agent_token(token) {
        return Err(CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_agent_token",
            "agent token is invalid",
        ));
    }
    Ok(sha256_hex(token))
}

fn validate_lease_token(token: &str) -> CloudResult<String> {
    if !token.starts_with("scl_") || token.len() <= 40 || token.len() > 128 {
        return Err(CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_agent_lease",
            "agent task lease is invalid or expired",
        ));
    }
    Ok(sha256_hex(token))
}

async fn lock_active_agent_by_token(
    transaction: &mut Transaction<'_, Postgres>,
    token_hash: &str,
) -> CloudResult<(Uuid, Uuid)> {
    sqlx::query("SELECT id, user_id FROM cloud_agents WHERE token_hash = $1 AND status = 'active' FOR SHARE")
        .bind(token_hash)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(CloudError::database)?
        .map(|row| (row.get("id"), row.get("user_id")))
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_agent_token",
                "agent token is invalid or agent is not active",
            )
        })
}

enum ExpiredAgentTaskScope {
    Agent(Uuid),
    User(Uuid),
}

fn expired_running_task_outcome(cancellation_pending: bool) -> (&'static str, &'static str) {
    if cancellation_pending {
        (
            "cancellation was not acknowledged before agent lease expiry; final result unknown",
            "Task cancellation was not acknowledged before lease expiry; final result is unknown",
        )
    } else {
        (
            "agent lease expired",
            "Running task failed because its lease expired",
        )
    }
}

fn expired_leased_task_can_requeue(risk: &str, approval_enforced: bool) -> bool {
    risk == "low" || approval_enforced
}

async fn reconcile_expired_agent_tasks(
    transaction: &mut Transaction<'_, Postgres>,
    scope: ExpiredAgentTaskScope,
) -> CloudResult<()> {
    let (sql, owner_id) = match scope {
        ExpiredAgentTaskScope::Agent(agent_id) => (
            "SELECT id, status, risk, approval_enforced, cancel_requested_at
             FROM cloud_agent_tasks
             WHERE agent_id = $1 AND status IN ('leased', 'running')
               AND lease_expires_at <= NOW()
             ORDER BY lease_expires_at FOR UPDATE SKIP LOCKED",
            agent_id,
        ),
        ExpiredAgentTaskScope::User(user_id) => (
            "SELECT id, status, risk, approval_enforced, cancel_requested_at
             FROM cloud_agent_tasks
             WHERE user_id = $1 AND status IN ('leased', 'running')
               AND lease_expires_at <= NOW()
             ORDER BY lease_expires_at FOR UPDATE SKIP LOCKED",
            user_id,
        ),
    };
    let expired = sqlx::query(sql)
        .bind(owner_id)
        .fetch_all(&mut **transaction)
        .await
        .map_err(CloudError::database)?;
    for row in expired {
        let task_id: Uuid = row.get("id");
        if row.get::<String, _>("status") == "leased"
            && expired_leased_task_can_requeue(
                &row.get::<String, _>("risk"),
                row.get::<bool, _>("approval_enforced"),
            )
        {
            sqlx::query(
                "UPDATE cloud_agent_tasks
                 SET status = 'queued', lease_token_hash = NULL, lease_expires_at = NULL,
                     updated_at = NOW() WHERE id = $1",
            )
            .bind(task_id)
            .execute(&mut **transaction)
            .await
            .map_err(CloudError::database)?;
            append_agent_task_event(
                transaction,
                task_id,
                "warn",
                "Expired lease returned task to queue",
                None,
            )
            .await?;
        } else {
            let cancellation_pending = row
                .get::<Option<DateTime<Utc>>, _>("cancel_requested_at")
                .is_some();
            let (error, event_message) = if row.get::<String, _>("status") == "leased" {
                (
                    "legacy high-risk task lease expired; it was not requeued without a linked team approval",
                    "Legacy high-risk task lease expired and was not requeued without a linked team approval",
                )
            } else {
                expired_running_task_outcome(cancellation_pending)
            };
            sqlx::query(
                "UPDATE cloud_agent_tasks
                 SET status = 'failed', lease_token_hash = NULL, lease_expires_at = NULL,
                     completed_at = NOW(), error = $2, updated_at = NOW()
                 WHERE id = $1",
            )
            .bind(task_id)
            .bind(error)
            .execute(&mut **transaction)
            .await
            .map_err(CloudError::database)?;
            append_agent_task_event(transaction, task_id, "error", event_message, None).await?;
        }
    }
    Ok(())
}

async fn agent_task_approval_is_current_for_lease(
    transaction: &mut Transaction<'_, Postgres>,
    task_id: Uuid,
    team_id: Option<Uuid>,
    approval_id: Option<Uuid>,
    requested_by: Uuid,
    approved_by: Option<Uuid>,
) -> CloudResult<bool> {
    let (Some(team_id), Some(approval_id), Some(approved_by)) = (team_id, approval_id, approved_by)
    else {
        return Ok(false);
    };
    let Some(approval) = sqlx::query(
        "SELECT team_id, requested_by, agent_task_id, status, decided_by
         FROM cloud_approvals WHERE id = $1 FOR SHARE",
    )
    .bind(approval_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?
    else {
        return Ok(false);
    };
    let decided_by = approval.get::<Option<Uuid>, _>("decided_by");
    if approval.get::<Uuid, _>("team_id") != team_id
        || approval.get::<Uuid, _>("requested_by") != requested_by
        || approval.get::<Option<Uuid>, _>("agent_task_id") != Some(task_id)
        || approval.get::<String, _>("status") != "approved"
        || decided_by != Some(approved_by)
        || decided_by == Some(requested_by)
    {
        return Ok(false);
    }
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM cloud_team_members
         WHERE team_id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(team_id)
    .bind(approved_by)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    Ok(role
        .as_deref()
        .is_some_and(|role| ["owner", "admin", "approver"].contains(&role)))
}

async fn lease_agent_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_request): Json<AgentTaskLeaseRequest>,
) -> CloudResult<Response> {
    let cloud = cloud(&state)?;
    let token_hash = agent_task_token_hash(&headers)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (agent_id, _) = lock_active_agent_by_token(&mut transaction, &token_hash).await?;

    reconcile_expired_agent_tasks(&mut transaction, ExpiredAgentTaskScope::Agent(agent_id)).await?;

    let task = sqlx::query(
        "SELECT t.id, t.user_id, t.team_id, t.approval_id, t.approved_by,
                t.operation, t.input, t.required_permission, t.risk, t.execution_mode,
                c.id AS checkpoint_id, c.task_id AS checkpoint_task_id,
                c.kind AS checkpoint_kind, c.payload AS checkpoint_payload
         FROM cloud_agent_tasks t
         LEFT JOIN cloud_agent_task_checkpoints c ON c.id = t.resume_checkpoint_id
         WHERE t.agent_id = $1 AND t.status = 'queued'
           AND (
             t.risk = 'low'
             OR EXISTS (
               SELECT 1 FROM cloud_approvals a
               JOIN cloud_team_members m
                 ON m.team_id = a.team_id AND m.user_id = a.decided_by
               WHERE a.id = t.approval_id
                 AND a.agent_task_id = t.id
                 AND a.team_id = t.team_id
                 AND a.requested_by = t.user_id
                 AND a.status = 'approved'
                 AND a.decided_by IS NOT NULL
                 AND a.decided_by <> t.user_id
                 AND m.role IN ('owner', 'admin', 'approver')
                 AND t.approved_by = a.decided_by
             )
           )
         ORDER BY t.created_at FOR UPDATE OF t SKIP LOCKED LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    let Some(task) = task else {
        transaction.commit().await.map_err(CloudError::database)?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let task_id: Uuid = task.get("id");
    if task.get::<String, _>("risk") != "low"
        && !agent_task_approval_is_current_for_lease(
            &mut transaction,
            task_id,
            task.get("team_id"),
            task.get("approval_id"),
            task.get("user_id"),
            task.get("approved_by"),
        )
        .await?
    {
        transaction.commit().await.map_err(CloudError::database)?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let resume = if task.get::<String, _>("execution_mode") == "resume" {
        Some(AgentTaskResumeLeaseView {
            source_task_id: task.get("checkpoint_task_id"),
            checkpoint_id: task.get("checkpoint_id"),
            kind: task.get("checkpoint_kind"),
            payload: task.get("checkpoint_payload"),
        })
    } else {
        None
    };
    let lease_token = random_token("scl_");
    let lease_expires_at = Utc::now() + Duration::seconds(AGENT_TASK_LEASE_SECONDS);
    sqlx::query(
        "UPDATE cloud_agent_tasks
         SET status = 'leased', lease_token_hash = $2, lease_expires_at = $3,
             leased_at = NOW(), updated_at = NOW()
         WHERE id = $1 AND status = 'queued'",
    )
    .bind(task_id)
    .bind(sha256_hex(&lease_token))
    .bind(lease_expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    append_agent_task_event(
        &mut transaction,
        task_id,
        "info",
        "Task leased to agent",
        Some(json!({ "lease_expires_at": lease_expires_at })),
    )
    .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((
        StatusCode::OK,
        Json(AgentTaskLeaseResponse {
            lease_token,
            lease_expires_at,
            task: AgentTaskLeaseView {
                id: task_id,
                operation: task.get("operation"),
                input: task.get("input"),
                required_permission: task.get("required_permission"),
                risk: task.get("risk"),
                resume,
            },
        }),
    )
        .into_response())
}

async fn start_agent_task(
    Path(task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AgentTaskLeaseTokenRequest>,
) -> CloudResult<Json<Value>> {
    let cloud = cloud(&state)?;
    let token_hash = agent_task_token_hash(&headers)?;
    let lease_hash = validate_lease_token(&request.lease_token)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (agent_id, _) = lock_active_agent_by_token(&mut transaction, &token_hash).await?;
    let lease_expires_at = Utc::now() + Duration::seconds(AGENT_TASK_LEASE_SECONDS);
    let task = sqlx::query(
        "SELECT status, lease_expires_at FROM cloud_agent_tasks
         WHERE id = $1 AND agent_id = $2 AND lease_token_hash = $3 FOR UPDATE",
    )
    .bind(task_id)
    .bind(agent_id)
    .bind(lease_hash)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .filter(|row| {
        row.get::<DateTime<Utc>, _>("lease_expires_at") > Utc::now()
            && ["leased", "running"].contains(&row.get::<String, _>("status").as_str())
    })
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "invalid_agent_lease",
            "agent task lease is invalid, expired, or in the wrong state",
        )
    })?;
    let already_running = task.get::<String, _>("status") == "running";
    if already_running {
        sqlx::query(
            "UPDATE cloud_agent_tasks SET lease_expires_at = $2, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(task_id)
        .bind(lease_expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    } else {
        sqlx::query(
            "UPDATE cloud_agent_tasks
             SET status = 'running', started_at = NOW(), lease_expires_at = $2, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(task_id)
        .bind(lease_expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
        append_agent_task_event(&mut transaction, task_id, "info", "Task started", None).await?;
    }
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(json!({
        "task_id": task_id,
        "status": "running",
        "lease_expires_at": lease_expires_at
    })))
}

async fn control_agent_task(
    Path(task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AgentTaskLeaseTokenRequest>,
) -> CloudResult<Json<AgentTaskControlResponse>> {
    let cloud = cloud(&state)?;
    let token_hash = agent_task_token_hash(&headers)?;
    let lease_hash = validate_lease_token(&request.lease_token)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (agent_id, _) = lock_active_agent_by_token(&mut transaction, &token_hash).await?;
    let lease_expires_at = Utc::now() + Duration::seconds(AGENT_TASK_LEASE_SECONDS);
    let task = sqlx::query(
        "SELECT cancel_requested_at FROM cloud_agent_tasks
         WHERE id = $1 AND agent_id = $2 AND status = 'running'
           AND lease_token_hash = $3 AND lease_expires_at > NOW() FOR UPDATE",
    )
    .bind(task_id)
    .bind(agent_id)
    .bind(lease_hash)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "invalid_agent_lease",
            "agent task lease is invalid, expired, or not running",
        )
    })?;
    sqlx::query(
        "UPDATE cloud_agent_tasks SET lease_expires_at = $2, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(task_id)
    .bind(lease_expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    let cancel_requested = task
        .get::<Option<DateTime<Utc>>, _>("cancel_requested_at")
        .is_some();
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(AgentTaskControlResponse {
        task_id,
        lease_expires_at,
        cancel_requested,
    }))
}

async fn create_agent_task_event(
    Path(task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AgentTaskEventRequest>,
) -> CloudResult<(StatusCode, Json<AgentTaskEventView>)> {
    validate_task_event(&request.level, &request.message, &request.data)?;
    let cloud = cloud(&state)?;
    let token_hash = agent_task_token_hash(&headers)?;
    let lease_hash = validate_lease_token(&request.lease_token)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (agent_id, _) = lock_active_agent_by_token(&mut transaction, &token_hash).await?;
    let lease_expires_at = Utc::now() + Duration::seconds(AGENT_TASK_LEASE_SECONDS);
    let task = sqlx::query(
        "SELECT cancel_requested_at FROM cloud_agent_tasks
         WHERE id = $1 AND agent_id = $2 AND status = 'running'
           AND lease_token_hash = $3 AND lease_expires_at > NOW()
         FOR UPDATE",
    )
    .bind(task_id)
    .bind(agent_id)
    .bind(lease_hash)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    let Some(task) = task else {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "invalid_agent_lease",
            "agent task lease is invalid, expired, or not running",
        ));
    };
    let cancellation_requested = task
        .get::<Option<DateTime<Utc>>, _>("cancel_requested_at")
        .is_some();
    if agent_task_event_renews_lease(cancellation_requested) {
        sqlx::query(
            "UPDATE cloud_agent_tasks SET lease_expires_at = $2, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(task_id)
        .bind(lease_expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    }
    let event_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM cloud_agent_task_events WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    if event_count >= 1999 {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_task_event_limit",
            "agent task has reserved its final event slot for completion",
        ));
    }
    let event = append_agent_task_event(
        &mut transaction,
        task_id,
        &request.level,
        &request.message,
        request.data,
    )
    .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((StatusCode::CREATED, Json(event)))
}

async fn create_agent_task_checkpoint(
    Path(task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateAgentTaskCheckpointRequest>,
) -> CloudResult<(StatusCode, Json<AgentTaskCheckpointView>)> {
    validate_task_checkpoint(
        &request.checkpoint_key,
        &request.kind,
        request.resumable,
        &request.payload,
    )?;
    let cloud = cloud(&state)?;
    let token_hash = agent_task_token_hash(&headers)?;
    let lease_hash = validate_lease_token(&request.lease_token)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (agent_id, _) = lock_active_agent_by_token(&mut transaction, &token_hash).await?;
    let task = sqlx::query(
        "SELECT operation, required_permission, cancel_requested_at FROM cloud_agent_tasks
         WHERE id = $1 AND agent_id = $2 AND status = 'running'
           AND lease_token_hash = $3 AND lease_expires_at > NOW() FOR UPDATE",
    )
    .bind(task_id)
    .bind(agent_id)
    .bind(lease_hash)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "invalid_agent_lease",
            "agent task lease is invalid, expired, or not running",
        )
    })?;
    ensure_task_accepts_checkpoint(
        task.get::<Option<DateTime<Utc>>, _>("cancel_requested_at")
            .is_some(),
    )?;
    if request.kind == "result" {
        let result: ResultCheckpointPayload = serde_json::from_value(request.payload.clone())
            .map_err(|_| CloudError::bad_request("result checkpoint payload is invalid"))?;
        validate_task_rollback_result(
            &result.status,
            &task.get::<String, _>("required_permission"),
            &task.get::<String, _>("operation"),
            result.rollback_available,
        )?;
    }
    if let Some(existing) = sqlx::query(
        "SELECT id, task_id, seq, checkpoint_key, kind, resumable, payload, created_at
         FROM cloud_agent_task_checkpoints
         WHERE task_id = $1 AND checkpoint_key = $2",
    )
    .bind(task_id)
    .bind(&request.checkpoint_key)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    {
        if existing.get::<String, _>("kind") != request.kind
            || existing.get::<bool, _>("resumable") != request.resumable
            || existing.get::<Value, _>("payload") != request.payload
        {
            return Err(CloudError::new(
                StatusCode::CONFLICT,
                "checkpoint_conflict",
                "checkpoint_key was already used with different checkpoint data",
            ));
        }
        sqlx::query(
            "UPDATE cloud_agent_tasks
             SET lease_expires_at = NOW() + ($2 * INTERVAL '1 second'), updated_at = NOW()
             WHERE id = $1",
        )
        .bind(task_id)
        .bind(AGENT_TASK_LEASE_SECONDS)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
        let view = agent_task_checkpoint_view(&existing);
        transaction.commit().await.map_err(CloudError::database)?;
        return Ok((StatusCode::OK, Json(view)));
    }
    let checkpoint_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM cloud_agent_task_checkpoints WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    if checkpoint_count >= 1000 {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "agent_task_checkpoint_limit",
            "agent task already has the maximum number of checkpoints",
        ));
    }
    let row = sqlx::query(
        "INSERT INTO cloud_agent_task_checkpoints
         (id, task_id, seq, checkpoint_key, kind, resumable, payload)
         SELECT $1, $2, COALESCE(MAX(seq), 0) + 1, $3, $4, $5, $6
         FROM cloud_agent_task_checkpoints WHERE task_id = $2
         RETURNING id, task_id, seq, checkpoint_key, kind, resumable, payload, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(task_id)
    .bind(&request.checkpoint_key)
    .bind(&request.kind)
    .bind(request.resumable)
    .bind(request.payload)
    .fetch_one(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sqlx::query(
        "UPDATE cloud_agent_tasks
         SET lease_expires_at = NOW() + ($2 * INTERVAL '1 second'), updated_at = NOW()
         WHERE id = $1",
    )
    .bind(task_id)
    .bind(AGENT_TASK_LEASE_SECONDS)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    let view = agent_task_checkpoint_view(&row);
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn complete_agent_task(
    Path(task_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CompleteAgentTaskRequest>,
) -> CloudResult<Json<AgentTaskView>> {
    validate_task_completion_values(
        &request.status,
        &request.output,
        &request.error,
        &request.artifacts,
        true,
    )?;
    let cloud = cloud(&state)?;
    let token_hash = agent_task_token_hash(&headers)?;
    let lease_hash = validate_lease_token(&request.lease_token)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (agent_id, user_id) = lock_active_agent_by_token(&mut transaction, &token_hash).await?;
    let task = sqlx::query(
        "SELECT operation, required_permission, status, output, error, artifacts,
                rollback_available, execution_mode, resume_checkpoint_id,
                cancel_requested_at, cancel_acknowledged_at,
                (status = 'running' AND lease_token_hash = $3 AND lease_expires_at > NOW()) AS lease_valid
         FROM cloud_agent_tasks WHERE id = $1 AND agent_id = $2 FOR UPDATE",
    )
    .bind(task_id)
    .bind(agent_id)
    .bind(lease_hash)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "invalid_agent_lease",
            "agent task lease is invalid or does not belong to this agent",
        )
    })?;
    let existing_status: String = task.get("status");
    validate_task_cancellation_completion(
        &request.status,
        request.rollback_available,
        task.get::<Option<DateTime<Utc>>, _>("cancel_requested_at")
            .is_some(),
    )?;
    if ["succeeded", "failed", "cancelled"].contains(&existing_status.as_str()) {
        let same_completion = existing_status == request.status
            && task.get::<Option<Value>, _>("output") == request.output
            && task.get::<Option<String>, _>("error") == request.error
            && task.get::<Option<Value>, _>("artifacts") == request.artifacts
            && task.get::<bool, _>("rollback_available") == request.rollback_available;
        if !same_completion {
            return Err(CloudError::new(
                StatusCode::CONFLICT,
                "completion_conflict",
                "agent task was already completed with a different result",
            ));
        }
        transaction.commit().await.map_err(CloudError::database)?;
        return Ok(Json(load_agent_task(&cloud, user_id, task_id).await?));
    }
    if !task.get::<bool, _>("lease_valid") {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "invalid_agent_lease",
            "agent task lease is invalid, expired, or not running",
        ));
    }
    let latest_result_payload = sqlx::query_scalar::<_, Value>(
        "SELECT payload FROM cloud_agent_task_checkpoints
         WHERE task_id = $1 AND kind = 'result' ORDER BY seq DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    if let Some(payload) = latest_result_payload {
        let checkpoint: ResultCheckpointPayload = serde_json::from_value(payload)
            .map_err(|_| CloudError::bad_request("result checkpoint payload is invalid"))?;
        if checkpoint.status != request.status
            || checkpoint.output != request.output
            || checkpoint.error != request.error
            || checkpoint.artifacts != request.artifacts
            || checkpoint.rollback_available != request.rollback_available
        {
            return Err(CloudError::new(
                StatusCode::CONFLICT,
                "checkpoint_completion_conflict",
                "task completion does not match its latest result checkpoint",
            ));
        }
    }
    let operation: String = task.get("operation");
    let permission: String = task.get("required_permission");
    validate_task_rollback_result(
        &request.status,
        &permission,
        &operation,
        request.rollback_available,
    )?;
    let rollback_source_task_id = if request.rollback_available {
        if task.get::<String, _>("execution_mode") == "resume" {
            let checkpoint_id = task
                .get::<Option<Uuid>, _>("resume_checkpoint_id")
                .ok_or_else(|| {
                    CloudError::new(
                        StatusCode::CONFLICT,
                        "resume_checkpoint_unavailable",
                        "resume task no longer has its source checkpoint",
                    )
                })?;
            Some(
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT COALESCE(t.rollback_source_task_id, t.id)
                     FROM cloud_agent_task_checkpoints c
                     JOIN cloud_agent_tasks t ON t.id = c.task_id
                     WHERE c.id = $1 AND t.user_id = $2",
                )
                .bind(checkpoint_id)
                .bind(user_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(CloudError::database)?
                .ok_or_else(|| {
                    CloudError::new(
                        StatusCode::CONFLICT,
                        "resume_checkpoint_unavailable",
                        "resume checkpoint source task no longer exists",
                    )
                })?,
            )
        } else {
            Some(task_id)
        }
    } else {
        None
    };
    sqlx::query(
        "UPDATE cloud_agent_tasks
         SET status = $2, output = $3, error = $4, artifacts = $5,
             rollback_available = $6, completed_at = NOW(),
             rollback_source_task_id = $7,
             cancelled_at = CASE WHEN $2 = 'cancelled' THEN NOW() ELSE cancelled_at END,
             cancel_acknowledged_at = CASE WHEN $2 = 'cancelled' THEN NOW()
                                            ELSE cancel_acknowledged_at END,
             lease_token_hash = NULL, lease_expires_at = NULL, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(task_id)
    .bind(&request.status)
    .bind(request.output)
    .bind(request.error)
    .bind(request.artifacts)
    .bind(request.rollback_available)
    .bind(rollback_source_task_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    append_agent_task_event(
        &mut transaction,
        task_id,
        match request.status.as_str() {
            "succeeded" => "info",
            "cancelled" => "warn",
            _ => "error",
        },
        match request.status.as_str() {
            "succeeded" => "Task succeeded",
            "cancelled" => "Task cancellation acknowledged",
            _ => "Task failed",
        },
        Some(json!({ "status": request.status })),
    )
    .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(load_agent_task(&cloud, user_id, task_id).await?))
}

async fn agent_heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<AgentHeartbeat>> {
    let cloud = cloud(&state)?;
    let token = bearer(&headers)?;
    if !valid_agent_token(token) {
        return Err(CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_agent_token",
            "agent token is invalid",
        ));
    }
    let row = sqlx::query(
        "UPDATE cloud_agents SET last_seen_at = NOW(), updated_at = NOW()
         WHERE token_hash = $1 AND status IN ('claimed', 'active')
         RETURNING id, status, last_seen_at",
    )
    .bind(sha256_hex(token))
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_agent_token",
            "agent token is invalid or revoked",
        )
    })?;
    let status: String = row.get("status");
    let active = status == "active";
    let agent_id: Uuid = row.get("id");
    if active {
        let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
        reconcile_expired_agent_tasks(&mut transaction, ExpiredAgentTaskScope::Agent(agent_id))
            .await?;
        reconcile_expired_terminal_sessions(
            &mut transaction,
            ExpiredTerminalScope::Agent(agent_id),
        )
        .await?;
        transaction.commit().await.map_err(CloudError::database)?;
    }
    let commands_available = if active {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                SELECT 1
                FROM cloud_agent_tasks t
                WHERE t.agent_id = $1
                  AND (
                    t.status IN ('leased', 'running')
                    OR (
                        t.status = 'queued'
                        AND (
                            t.risk = 'low'
                            OR EXISTS (
                                SELECT 1
                                FROM cloud_approvals a
                                JOIN cloud_team_members m
                                  ON m.team_id = a.team_id AND m.user_id = a.decided_by
                                WHERE a.id = t.approval_id
                                  AND a.agent_task_id = t.id
                                  AND a.team_id = t.team_id
                                  AND a.requested_by = t.user_id
                                  AND a.status = 'approved'
                                  AND a.decided_by IS NOT NULL
                                  AND a.decided_by <> t.user_id
                                  AND m.role IN ('owner', 'admin', 'approver')
                                  AND t.approved_by = a.decided_by
                            )
                        )
                    )
                  )
                UNION ALL
                SELECT 1 FROM cloud_terminal_commands c
                JOIN cloud_terminal_sessions s ON s.id = c.session_id
                WHERE s.agent_id = $1 AND c.acknowledged_at IS NULL
                  AND (
                    (
                        c.kind = 'start'
                        AND s.status = 'pending'
                        AND EXISTS (
                            SELECT 1
                            FROM cloud_approvals a
                            JOIN cloud_team_members m
                              ON m.team_id = a.team_id AND m.user_id = a.decided_by
                            WHERE a.id = s.approval_id
                              AND a.terminal_session_id = s.id
                              AND a.team_id = s.team_id
                              AND a.requested_by = s.user_id
                              AND a.status = 'approved'
                              AND a.decided_by IS NOT NULL
                              AND a.decided_by <> s.user_id
                              AND m.role IN ('owner', 'admin', 'approver')
                              AND s.approved_by = a.decided_by
                        )
                    )
                    OR (
                        c.kind <> 'start'
                        AND s.status IN ('starting', 'running', 'terminating')
                    )
                  )
            )",
        )
        .bind(agent_id)
        .fetch_one(&cloud.db)
        .await
        .map_err(CloudError::database)?
    } else {
        false
    };
    Ok(Json(AgentHeartbeat {
        agent_id,
        status,
        active,
        commands_available,
        server_time: Utc::now(),
        next_heartbeat_seconds: 30,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTerminalSessionRequest {
    agent_id: Uuid,
    #[serde(default)]
    team_id: Option<Uuid>,
    title: Option<String>,
    cwd: Option<String>,
    cols: i32,
    rows: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalInputRequest {
    data_base64: String,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalResizeRequest {
    cols: i32,
    rows: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentTerminalCommandsRequest {
    instance_id: String,
    max_commands: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentTerminalEventInput {
    seq: i64,
    kind: String,
    #[serde(default)]
    data_base64: Option<String>,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentTerminalEventsRequest {
    instance_id: String,
    acknowledged_command_ids: Vec<Uuid>,
    events: Vec<AgentTerminalEventInput>,
}

#[derive(Deserialize)]
struct TerminalEventsQuery {
    after_seq: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize)]
struct TerminalSessionView {
    id: Uuid,
    agent_id: Uuid,
    team_id: Option<Uuid>,
    approval_id: Option<Uuid>,
    title: String,
    cwd: Option<String>,
    cols: i32,
    rows: i32,
    status: String,
    exit_code: Option<i32>,
    error: Option<String>,
    created_at: DateTime<Utc>,
    approved_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    last_seen_at: Option<DateTime<Utc>>,
    exited_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct TerminalCommandView {
    id: Uuid,
    session_id: Uuid,
    seq: i64,
    kind: String,
    payload: Value,
}

#[derive(Serialize)]
struct TerminalEventView {
    seq: i64,
    kind: String,
    data_base64: Option<String>,
    data: Option<Value>,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct TerminalEventsResponse {
    session: TerminalSessionView,
    events: Vec<TerminalEventView>,
}

const TERMINAL_SESSION_SELECT: &str =
    "SELECT id, agent_id, team_id, approval_id, title, cwd, cols, rows, status,
    exit_code, error, created_at, approved_at, started_at, last_seen_at, exited_at, updated_at
    FROM cloud_terminal_sessions";

fn validate_terminal_dimensions(cols: i32, rows: i32) -> CloudResult<()> {
    if !(20..=400).contains(&cols) || !(5..=200).contains(&rows) {
        return Err(CloudError::bad_request(
            "terminal dimensions are outside the allowed range",
        ));
    }
    Ok(())
}

fn validate_terminal_title(value: Option<String>) -> CloudResult<String> {
    let title = value.unwrap_or_else(|| "Terminal".into());
    bounded_agent_text(&title, "title", 1, 128)
}

fn validate_terminal_cwd(value: Option<String>) -> CloudResult<Option<String>> {
    value
        .map(|cwd| {
            if cwd.is_empty() || cwd.chars().count() > 1024 || cwd.contains('\0') {
                return Err(CloudError::bad_request(
                    "terminal cwd must contain 1-1024 characters",
                ));
            }
            Ok(cwd)
        })
        .transpose()
}

fn validate_terminal_instance_id(value: &str) -> CloudResult<String> {
    let value = value.trim();
    if value.len() < 8
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
    {
        return Err(CloudError::bad_request("terminal instance_id is invalid"));
    }
    Ok(value.to_string())
}

fn decode_terminal_base64(value: &str, maximum: usize) -> CloudResult<Vec<u8>> {
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| CloudError::bad_request("terminal data_base64 is invalid"))?;
    if decoded.is_empty() || decoded.len() > maximum {
        return Err(CloudError::bad_request(
            "terminal decoded data is outside the allowed size",
        ));
    }
    Ok(decoded)
}

const TERMINAL_SECRET_KEYS: &[&[u8]] = &[
    b"password",
    b"passwd",
    b"api_key",
    b"apikey",
    b"token",
    b"secret",
    b"authorization",
    b"cookie",
    b"credential",
    b"rcon_password",
    b"rcon-password",
];
const TERMINAL_TOKEN_PREFIXES: &[&[u8]] = &[
    b"sca_",
    b"scs_",
    b"sk-sc-",
    b"ghp_",
    b"github_pat_",
    b"AKIA",
];
const TERMINAL_REDACTION_OVERFLOW: &[u8] = b"[REDACTED: terminal output omitted]";

fn ascii_starts_with_ignore_case(value: &[u8], start: usize, needle: &[u8]) -> bool {
    value
        .get(start..start.saturating_add(needle.len()))
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(needle))
}

fn terminal_secret_boundary(value: Option<u8>) -> bool {
    value.is_none_or(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-'))
}

fn terminal_secret_value_end(value: &[u8], start: usize) -> usize {
    value[start..]
        .iter()
        .position(|byte| {
            byte.is_ascii_whitespace()
                || matches!(byte, b'\'' | b'"' | b'`' | b',' | b';' | b')' | b']' | b'}')
        })
        .map_or(value.len(), |offset| start + offset)
}

fn terminal_secret_value_start(value: &[u8], key_start: usize, key_end: usize) -> Option<usize> {
    let mut cursor = key_end;
    if matches!(value.get(cursor), Some(b'\'' | b'"' | b'`')) {
        cursor += 1;
    }
    while value.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if matches!(value.get(cursor), Some(b'=' | b':')) {
        cursor += 1;
        while value.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
    } else if key_start >= 2 && value[key_start - 2..key_start] == *b"--" && cursor > key_end {
        // Support the common CLI form `--password value` without treating
        // ordinary prose such as "password is required" as a credential.
    } else {
        return None;
    }
    if matches!(value.get(cursor), Some(b'\'' | b'"' | b'`')) {
        cursor += 1;
    }
    Some(cursor)
}

fn redact_terminal_output(value: &[u8]) -> Vec<u8> {
    let mut redacted = Vec::with_capacity(value.len());
    let mut copied_until = 0;
    let mut index = 0;
    while index < value.len() {
        let boundary =
            terminal_secret_boundary(index.checked_sub(1).and_then(|at| value.get(at)).copied());
        if !boundary {
            index += 1;
            continue;
        }

        if ascii_starts_with_ignore_case(value, index, b"bearer ") {
            let mut value_start = index + b"bearer ".len();
            while value.get(value_start).is_some_and(u8::is_ascii_whitespace) {
                value_start += 1;
            }
            if matches!(value.get(value_start), Some(b'\'' | b'"' | b'`')) {
                value_start += 1;
            }
            let value_end = terminal_secret_value_end(value, value_start);
            if value_end > value_start {
                redacted.extend_from_slice(&value[copied_until..value_start]);
                redacted.extend_from_slice(b"[REDACTED]");
                copied_until = value_end;
                index = value_end;
                continue;
            }
        }

        let mut key_redacted = false;
        for key in TERMINAL_SECRET_KEYS {
            let key_end = index + key.len();
            if !ascii_starts_with_ignore_case(value, index, key)
                || !terminal_secret_boundary(value.get(key_end).copied())
            {
                continue;
            }
            let Some(value_start) = terminal_secret_value_start(value, index, key_end) else {
                continue;
            };
            let value_end = terminal_secret_value_end(value, value_start);
            if value_end > value_start {
                redacted.extend_from_slice(&value[copied_until..value_start]);
                redacted.extend_from_slice(b"[REDACTED]");
                copied_until = value_end;
                index = value_end;
                key_redacted = true;
                break;
            }
        }
        if key_redacted {
            continue;
        }

        if let Some(prefix) = TERMINAL_TOKEN_PREFIXES.iter().find(|prefix| {
            value
                .get(index..)
                .is_some_and(|remaining| remaining.starts_with(prefix))
        }) {
            let token_end = value[index + prefix.len()..]
                .iter()
                .position(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-'))
                .map_or(value.len(), |offset| index + prefix.len() + offset);
            if token_end > index + prefix.len() {
                redacted.extend_from_slice(&value[copied_until..index]);
                redacted.extend_from_slice(b"[REDACTED]");
                copied_until = token_end;
                index = token_end;
                continue;
            }
        }
        index += 1;
    }
    redacted.extend_from_slice(&value[copied_until..]);
    if redacted.len() > 16 * 1024 {
        TERMINAL_REDACTION_OVERFLOW.to_vec()
    } else {
        redacted
    }
}

fn terminal_text_requires_redaction(value: &str) -> bool {
    redact_terminal_output(value.as_bytes()) != value.as_bytes()
}

fn terminal_hard_delimiter(byte: u8) -> bool {
    matches!(byte, b'\n' | b'\r' | b';' | b',' | b')' | b']' | b'}')
}

fn terminal_tail_after_hard_delimiter(value: &[u8]) -> &[u8] {
    value
        .iter()
        .rposition(|byte| terminal_hard_delimiter(*byte))
        .map_or(value, |position| &value[position + 1..])
}

fn terminal_marker_can_continue(tail: &[u8]) -> bool {
    const MARKERS: &[&[u8]] = &[
        b"password",
        b"passwd",
        b"api_key",
        b"apikey",
        b"token",
        b"secret",
        b"authorization",
        b"cookie",
        b"credential",
        b"rcon_password",
        b"rcon-password",
        b"bearer ",
        b"sca_",
        b"scs_",
        b"sk-sc-",
        b"ghp_",
        b"github_pat_",
        b"akia",
    ];
    MARKERS.iter().any(|marker| {
        let maximum = tail.len().min(marker.len());
        (3..=maximum)
            .any(|length| tail[tail.len() - length..].eq_ignore_ascii_case(&marker[..length]))
    })
}

fn terminal_key_waits_for_value(tail: &[u8]) -> bool {
    for index in 0..tail.len() {
        if !terminal_secret_boundary(index.checked_sub(1).and_then(|at| tail.get(at)).copied()) {
            continue;
        }
        for key in TERMINAL_SECRET_KEYS {
            let key_end = index + key.len();
            if !ascii_starts_with_ignore_case(tail, index, key)
                || !terminal_secret_boundary(tail.get(key_end).copied())
            {
                continue;
            }
            if terminal_secret_value_start(tail, index, key_end) == Some(tail.len()) {
                return true;
            }
        }
    }
    false
}

fn terminal_redaction_continues(value: &[u8]) -> bool {
    let tail = terminal_tail_after_hard_delimiter(value);
    !tail.is_empty()
        && (terminal_marker_can_continue(tail)
            || terminal_key_waits_for_value(tail)
            || redact_terminal_output(tail) != tail)
}

fn sanitize_terminal_output(
    event: &mut AgentTerminalEventInput,
    redaction_pending: bool,
) -> CloudResult<bool> {
    if event.kind != "output" {
        return Ok(redaction_pending);
    }
    let Some(data_base64) = event.data_base64.as_deref() else {
        return Ok(redaction_pending);
    };
    let decoded = decode_terminal_base64(data_base64, 16 * 1024)?;
    let continuation = terminal_redaction_continues(&decoded);
    use base64::Engine;
    let output = if redaction_pending {
        TERMINAL_REDACTION_OVERFLOW.to_vec()
    } else {
        redact_terminal_output(&decoded)
    };
    event.data_base64 = Some(base64::engine::general_purpose::STANDARD.encode(output));
    Ok(if redaction_pending {
        continuation || !decoded.iter().any(|byte| terminal_hard_delimiter(*byte))
    } else {
        continuation
    })
}

fn terminal_input_mac(master_key: &[u8; 32], data_base64: &str) -> String {
    use base64::Engine;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(master_key)
        .expect("SHA-256 HMAC accepts the fixed-length Cloud master key");
    mac.update(data_base64.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn encrypt_terminal_input_payload(master_key: &[u8; 32], data_base64: &str) -> CloudResult<Value> {
    use base64::Engine;
    let (ciphertext, nonce) = encrypt_api_key(master_key, data_base64)?;
    Ok(json!({
        "format": "encrypted-v1",
        "ciphertext_base64": base64::engine::general_purpose::STANDARD.encode(ciphertext),
        "nonce_base64": base64::engine::general_purpose::STANDARD.encode(nonce),
        "input_mac": terminal_input_mac(master_key, data_base64),
    }))
}

fn decrypt_terminal_input_payload(master_key: &[u8; 32], payload: &Value) -> CloudResult<Value> {
    if payload.get("format").and_then(Value::as_str) != Some("encrypted-v1") {
        // A release may be upgraded while a legacy command is still leased.
        // It is accepted only for delivery; acknowledgement immediately removes
        // its plaintext from the database.
        let legacy = payload
            .get("data_base64")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CloudError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "terminal_payload_invalid",
                    "terminal input payload cannot be decrypted",
                )
            })?;
        decode_terminal_base64(legacy, 8192)?;
        return Ok(json!({ "data_base64": legacy }));
    }
    use base64::Engine;
    let ciphertext = payload
        .get("ciphertext_base64")
        .and_then(Value::as_str)
        .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok())
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "terminal_payload_invalid",
                "terminal input payload cannot be decrypted",
            )
        })?;
    let nonce = payload
        .get("nonce_base64")
        .and_then(Value::as_str)
        .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok())
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "terminal_payload_invalid",
                "terminal input payload cannot be decrypted",
            )
        })?;
    let data_base64 = decrypt_api_key(master_key, &ciphertext, &nonce)?;
    decode_terminal_base64(&data_base64, 8192)?;
    Ok(json!({ "data_base64": data_base64 }))
}

fn terminal_input_payload_matches(existing: &Value, expected: &Value) -> bool {
    if existing == expected {
        return true;
    }
    let expected_mac = expected.get("input_mac").and_then(Value::as_str);
    let existing_mac = existing.get("input_mac").and_then(Value::as_str);
    matches!(
        (existing.get("format").and_then(Value::as_str), existing_mac, expected_mac),
        (Some("encrypted-v1" | "redacted-v1"), Some(existing_mac), Some(expected_mac))
            if existing_mac == expected_mac
    )
}

fn validate_terminal_event(event: &AgentTerminalEventInput) -> CloudResult<usize> {
    if event.seq <= 0 {
        return Err(CloudError::bad_request(
            "terminal event seq must be positive",
        ));
    }
    let object = event.data.as_ref().map(task_input_object).transpose()?;
    match event.kind.as_str() {
        "started" => {
            if event.data_base64.is_some() {
                return Err(CloudError::bad_request(
                    "started event cannot contain data_base64",
                ));
            }
            if let Some(object) = object {
                require_exact_task_keys(object, &[], &["pid"])?;
                if object.get("pid").is_some_and(|pid| {
                    pid.as_u64()
                        .is_none_or(|pid| pid == 0 || pid > u32::MAX as u64)
                }) {
                    return Err(CloudError::bad_request("terminal pid is invalid"));
                }
            }
            Ok(0)
        }
        "output" => {
            if event.data.is_some() {
                return Err(CloudError::bad_request(
                    "output event cannot contain structured data",
                ));
            }
            event
                .data_base64
                .as_deref()
                .ok_or_else(|| CloudError::bad_request("output event requires data_base64"))
                .and_then(|value| decode_terminal_base64(value, 16 * 1024))
                .map(|decoded| decoded.len())
        }
        "keepalive" => {
            if event.data_base64.is_some() {
                return Err(CloudError::bad_request(
                    "keepalive event cannot contain data_base64",
                ));
            }
            if let Some(object) = object {
                require_exact_task_keys(object, &[], &[])?;
            }
            Ok(0)
        }
        "exit" => {
            if event.data_base64.is_some() {
                return Err(CloudError::bad_request(
                    "exit event cannot contain data_base64",
                ));
            }
            let object =
                object.ok_or_else(|| CloudError::bad_request("exit event requires data"))?;
            require_exact_task_keys(object, &["exit_code"], &[])?;
            if !object["exit_code"].is_null()
                && object["exit_code"]
                    .as_i64()
                    .is_none_or(|code| !(i32::MIN as i64..=i32::MAX as i64).contains(&code))
            {
                return Err(CloudError::bad_request("terminal exit_code is invalid"));
            }
            Ok(0)
        }
        "error" => {
            if event.data_base64.is_some() {
                return Err(CloudError::bad_request(
                    "error event cannot contain data_base64",
                ));
            }
            let object =
                object.ok_or_else(|| CloudError::bad_request("error event requires data"))?;
            require_exact_task_keys(object, &["message"], &[])?;
            let message = object["message"].as_str().unwrap_or_default();
            if message.is_empty()
                || message.chars().count() > 4000
                || message.contains('\0')
                || task_text_contains_token(message)
                || terminal_text_requires_redaction(message)
            {
                return Err(CloudError::bad_request("terminal error message is invalid"));
            }
            Ok(0)
        }
        _ => Err(CloudError::bad_request(
            "terminal event kind is not allowed",
        )),
    }
}

fn terminal_session_view(row: &sqlx::postgres::PgRow) -> TerminalSessionView {
    TerminalSessionView {
        id: row.get("id"),
        agent_id: row.get("agent_id"),
        team_id: row.get("team_id"),
        approval_id: row.get("approval_id"),
        title: row.get("title"),
        cwd: row.get("cwd"),
        cols: row.get("cols"),
        rows: row.get("rows"),
        status: row.get("status"),
        exit_code: row.get("exit_code"),
        error: row.get("error"),
        created_at: row.get("created_at"),
        approved_at: row.get("approved_at"),
        started_at: row.get("started_at"),
        last_seen_at: row.get("last_seen_at"),
        exited_at: row.get("exited_at"),
        updated_at: row.get("updated_at"),
    }
}

fn terminal_event_view(row: &sqlx::postgres::PgRow) -> TerminalEventView {
    TerminalEventView {
        seq: row.get("seq"),
        kind: row.get("kind"),
        data_base64: row.get("data_base64"),
        data: row.get("data"),
        created_at: row.get("created_at"),
    }
}

enum ExpiredTerminalScope {
    Agent(Uuid),
    User(Uuid),
}

async fn reconcile_expired_terminal_sessions(
    transaction: &mut Transaction<'_, Postgres>,
    scope: ExpiredTerminalScope,
) -> CloudResult<()> {
    let (sql, owner_id) = match scope {
        ExpiredTerminalScope::Agent(agent_id) => (
            "UPDATE cloud_terminal_sessions
             SET status = 'failed', error = 'agent terminal lease expired', exited_at = NOW(),
                 instance_id = NULL, lease_expires_at = NULL, updated_at = NOW()
             WHERE agent_id = $1 AND status IN ('starting', 'running', 'terminating')
               AND lease_expires_at <= NOW()",
            agent_id,
        ),
        ExpiredTerminalScope::User(user_id) => (
            "UPDATE cloud_terminal_sessions
             SET status = 'failed', error = 'agent terminal lease expired', exited_at = NOW(),
                 instance_id = NULL, lease_expires_at = NULL, updated_at = NOW()
             WHERE user_id = $1 AND status IN ('starting', 'running', 'terminating')
               AND lease_expires_at <= NOW()",
            user_id,
        ),
    };
    sqlx::query(sql)
        .bind(owner_id)
        .execute(&mut **transaction)
        .await
        .map_err(CloudError::database)?;
    Ok(())
}

async fn lock_user_terminal_agent(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    agent_id: Uuid,
) -> CloudResult<()> {
    lock_user_agent_for_task(transaction, user_id, agent_id, "full", Some("shell-v1")).await?;
    let capabilities: Value = sqlx::query_scalar(
        "SELECT capabilities FROM cloud_agents WHERE id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(agent_id)
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    let capabilities: Vec<String> = serde_json::from_value(capabilities).unwrap_or_default();
    if !capabilities.iter().any(|value| value == "terminal-v1") {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "agent_capability_missing",
            "agent does not advertise terminal-v1",
        ));
    }
    Ok(())
}

async fn lock_active_terminal_agent_by_token(
    transaction: &mut Transaction<'_, Postgres>,
    token_hash: &str,
) -> CloudResult<(Uuid, Uuid)> {
    let (agent_id, user_id) = lock_active_agent_by_token(transaction, token_hash).await?;
    let row =
        sqlx::query("SELECT capabilities, permissions FROM cloud_agents WHERE id = $1 FOR SHARE")
            .bind(agent_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(CloudError::database)?;
    let capabilities: Vec<String> =
        serde_json::from_value(row.get("capabilities")).unwrap_or_default();
    let permissions: Vec<String> =
        serde_json::from_value(row.get("permissions")).unwrap_or_default();
    let has_capability = |required: &str| capabilities.iter().any(|value| value == required);
    if !["tasks-v1", "shell-v1", "terminal-v1"]
        .into_iter()
        .all(has_capability)
    {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "agent_capability_missing",
            "agent must advertise tasks-v1, shell-v1, and terminal-v1",
        ));
    }
    if !permissions.iter().any(|value| value == "full") {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "agent_permission_missing",
            "agent terminal access requires full permission",
        ));
    }
    Ok((agent_id, user_id))
}

async fn load_terminal_session(
    cloud: &CloudState,
    user_id: Uuid,
    session_id: Uuid,
) -> CloudResult<TerminalSessionView> {
    let sql = format!("{TERMINAL_SESSION_SELECT} WHERE id = $1 AND user_id = $2");
    sqlx::query(&sql)
        .bind(session_id)
        .bind(user_id)
        .fetch_optional(&cloud.db)
        .await
        .map_err(CloudError::database)?
        .as_ref()
        .map(terminal_session_view)
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::NOT_FOUND,
                "terminal_session_not_found",
                "terminal session was not found",
            )
        })
}

async fn list_terminal_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Vec<TerminalSessionView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    reconcile_expired_terminal_sessions(&mut transaction, ExpiredTerminalScope::User(user.user_id))
        .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    let sql =
        format!("{TERMINAL_SESSION_SELECT} WHERE user_id = $1 ORDER BY created_at DESC LIMIT 100");
    let sessions = sqlx::query(&sql)
        .bind(user.user_id)
        .fetch_all(&cloud.db)
        .await
        .map_err(CloudError::database)?
        .iter()
        .map(terminal_session_view)
        .collect();
    Ok(Json(sessions))
}

async fn create_terminal_session_approval(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    team_id: Uuid,
    session_id: Uuid,
    agent_id: Uuid,
    title: &str,
    cwd: &Option<String>,
    cols: i32,
    rows: i32,
) -> CloudResult<Uuid> {
    let approval_id = Uuid::new_v4();
    let approval_title = format!("Persistent terminal: {title}");
    let summary = "Request to start a persistent full-permission terminal session; review the target agent and working directory before deciding.";
    let payload = json!({
        "source": "cloud-terminal-session",
        "terminal_session_id": session_id,
        "agent_id": agent_id,
        "title": title,
        "cwd": cwd,
        "cols": cols,
        "rows": rows,
        "risk": "critical",
        "required_permission": "full",
    });
    sqlx::query(
        "INSERT INTO cloud_approvals
         (id, team_id, requested_by, title, summary, risk, payload, terminal_session_id)
         VALUES ($1, $2, $3, $4, $5, 'high', $6, $7)",
    )
    .bind(approval_id)
    .bind(team_id)
    .bind(user_id)
    .bind(approval_title)
    .bind(summary)
    .bind(payload)
    .bind(session_id)
    .execute(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    Ok(approval_id)
}

async fn create_terminal_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateTerminalSessionRequest>,
) -> CloudResult<(StatusCode, Json<TerminalSessionView>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let title = validate_terminal_title(request.title)?;
    let cwd = validate_terminal_cwd(request.cwd)?;
    validate_terminal_dimensions(request.cols, request.rows)?;
    let session_id = Uuid::new_v4();
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let team_id = resolve_approval_team(
        &mut transaction,
        user.user_id,
        request.team_id,
        true,
        terminal_team_required_error,
    )
    .await?
    .expect("terminal sessions always require an approval team");
    lock_user_terminal_agent(&mut transaction, user.user_id, request.agent_id).await?;
    let approval_id = create_terminal_session_approval(
        &mut transaction,
        user.user_id,
        team_id,
        session_id,
        request.agent_id,
        &title,
        &cwd,
        request.cols,
        request.rows,
    )
    .await?;
    sqlx::query(
        "INSERT INTO cloud_terminal_sessions
         (id, user_id, agent_id, team_id, approval_id, title, cwd, cols, rows, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'awaiting_approval')",
    )
    .bind(session_id)
    .bind(user.user_id)
    .bind(request.agent_id)
    .bind(team_id)
    .bind(approval_id)
    .bind(title)
    .bind(cwd)
    .bind(request.cols)
    .bind(request.rows)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(load_terminal_session(&cloud, user.user_id, session_id).await?),
    ))
}

async fn approve_terminal_session(
    Path(session_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<TerminalSessionView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let row = sqlx::query(
        "SELECT agent_id, team_id, approval_id, cwd, cols, rows, status, next_command_seq
         FROM cloud_terminal_sessions WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(session_id)
    .bind(user.user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "terminal_session_not_found",
            "terminal session was not found",
        )
    })?;
    if row.get::<String, _>("status") != "awaiting_approval" {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "terminal_not_awaiting_approval",
            "terminal session is not awaiting approval",
        ));
    }
    let agent_id: Uuid = row.get("agent_id");
    let team_id = row.get::<Option<Uuid>, _>("team_id").ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "terminal_approval_missing",
            "this terminal session has no linked team approval; create a new session",
        )
    })?;
    let approval_id = row.get::<Option<Uuid>, _>("approval_id").ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "terminal_approval_missing",
            "this terminal session has no linked team approval; create a new session",
        )
    })?;
    let approval = sqlx::query(
        "SELECT team_id, requested_by, terminal_session_id, status, decided_by
         FROM cloud_approvals WHERE id = $1 FOR UPDATE",
    )
    .bind(approval_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "terminal_approval_missing",
            "this terminal session has no linked team approval; create a new session",
        )
    })?;
    if approval.get::<Uuid, _>("team_id") != team_id
        || approval.get::<Uuid, _>("requested_by") != user.user_id
        || approval.get::<Option<Uuid>, _>("terminal_session_id") != Some(session_id)
    {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "terminal_approval_invalid",
            "the linked terminal approval does not match the session",
        ));
    }
    if approval.get::<String, _>("status") != "approved" {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "terminal_approval_pending",
            "the linked team approval is not approved yet",
        ));
    }
    let decided_by = approval
        .get::<Option<Uuid>, _>("decided_by")
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::FORBIDDEN,
                "terminal_approval_invalid",
                "the linked terminal approval has no independent decision maker",
            )
        })?;
    if decided_by == user.user_id {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "terminal_approval_self",
            "the terminal requester cannot approve their own session",
        ));
    }
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM cloud_team_members
         WHERE team_id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(team_id)
    .bind(decided_by)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    if !role
        .as_deref()
        .is_some_and(|role| ["owner", "admin", "approver"].contains(&role))
    {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "terminal_approval_invalid",
            "the terminal approval decision maker is no longer a team approver",
        ));
    }
    lock_user_terminal_agent(&mut transaction, user.user_id, agent_id).await?;
    let seq = row.get::<i64, _>("next_command_seq") + 1;
    sqlx::query(
        "INSERT INTO cloud_terminal_commands (id, session_id, seq, kind, payload)
         VALUES ($1, $2, $3, 'start', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(session_id)
    .bind(seq)
    .bind(json!({
        "cwd": row.get::<Option<String>, _>("cwd"),
        "cols": row.get::<i32, _>("cols"),
        "rows": row.get::<i32, _>("rows"),
    }))
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sqlx::query(
        "UPDATE cloud_terminal_sessions
         SET status = 'pending', approved_by = $2, approved_at = NOW(),
             next_command_seq = $3, updated_at = NOW() WHERE id = $1",
    )
    .bind(session_id)
    .bind(decided_by)
    .bind(seq)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(
        load_terminal_session(&cloud, user.user_id, session_id).await?,
    ))
}

async fn enqueue_terminal_command(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    session_id: Uuid,
    kind: &str,
    payload: Value,
    idempotency_key: Option<&str>,
) -> CloudResult<String> {
    let row = sqlx::query(
        "SELECT status, next_command_seq FROM cloud_terminal_sessions
         WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "terminal_session_not_found",
            "terminal session was not found",
        )
    })?;
    let status: String = row.get("status");
    if let Some(idempotency_key) = idempotency_key {
        let existing = sqlx::query(
            "SELECT kind, payload FROM cloud_terminal_commands
             WHERE session_id = $1 AND idempotency_key = $2",
        )
        .bind(session_id)
        .bind(idempotency_key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(CloudError::database)?;
        if let Some(existing) = existing {
            let existing_kind: String = existing.get("kind");
            let existing_payload: Value = existing.get("payload");
            let payload_matches = if kind == "input" && existing_kind == "input" {
                terminal_input_payload_matches(&existing_payload, &payload)
            } else {
                existing_payload == payload
            };
            if existing_kind != kind || !payload_matches {
                return Err(CloudError::new(
                    StatusCode::CONFLICT,
                    "idempotency_conflict",
                    "idempotency_key was already used for different terminal input",
                ));
            }
            return Ok(status);
        }
    }
    let allowed = match kind {
        "input" | "resize" => status == "running",
        "terminate" => ["starting", "running"].contains(&status.as_str()),
        _ => false,
    };
    if !allowed {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "terminal_state_conflict",
            "terminal session does not accept this command in its current state",
        ));
    }
    let seq = row.get::<i64, _>("next_command_seq") + 1;
    sqlx::query(
        "INSERT INTO cloud_terminal_commands
         (id, session_id, seq, kind, payload, idempotency_key)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(session_id)
    .bind(seq)
    .bind(kind)
    .bind(payload)
    .bind(idempotency_key)
    .execute(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    let next_status = if kind == "terminate" {
        "terminating"
    } else {
        status.as_str()
    };
    sqlx::query(
        "UPDATE cloud_terminal_sessions
         SET next_command_seq = $2, status = $3, updated_at = NOW() WHERE id = $1",
    )
    .bind(session_id)
    .bind(seq)
    .bind(next_status)
    .execute(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    Ok(next_status.to_string())
}

async fn terminal_input(
    Path(session_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TerminalInputRequest>,
) -> CloudResult<Json<TerminalSessionView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    decode_terminal_base64(&request.data_base64, 8192)?;
    let idempotency_key = validate_idempotency_key(request.idempotency_key)?;
    let payload = encrypt_terminal_input_payload(cloud.master_key.as_ref(), &request.data_base64)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    enqueue_terminal_command(
        &mut transaction,
        user.user_id,
        session_id,
        "input",
        payload,
        idempotency_key.as_deref(),
    )
    .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(
        load_terminal_session(&cloud, user.user_id, session_id).await?,
    ))
}

async fn terminal_resize(
    Path(session_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TerminalResizeRequest>,
) -> CloudResult<Json<TerminalSessionView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    validate_terminal_dimensions(request.cols, request.rows)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    enqueue_terminal_command(
        &mut transaction,
        user.user_id,
        session_id,
        "resize",
        json!({ "cols": request.cols, "rows": request.rows }),
        None,
    )
    .await?;
    sqlx::query(
        "UPDATE cloud_terminal_sessions SET cols = $2, rows = $3, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(session_id)
    .bind(request.cols)
    .bind(request.rows)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(
        load_terminal_session(&cloud, user.user_id, session_id).await?,
    ))
}

async fn terminate_terminal_session(
    Path(session_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<TerminalSessionView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM cloud_terminal_sessions
         WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(session_id)
    .bind(user.user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "terminal_session_not_found",
            "terminal session was not found",
        )
    })?;
    match status.as_str() {
        "awaiting_approval" | "pending" => {
            sqlx::query(
                "UPDATE cloud_terminal_sessions
                 SET status = 'cancelled', exited_at = NOW(), updated_at = NOW()
                 WHERE id = $1",
            )
            .bind(session_id)
            .execute(&mut *transaction)
            .await
            .map_err(CloudError::database)?;
            sqlx::query(
                "UPDATE cloud_approvals
                 SET status = 'cancelled', decision_comment = '终端会话已取消', decided_at = NOW()
                 WHERE terminal_session_id = $1 AND status = 'pending'",
            )
            .bind(session_id)
            .execute(&mut *transaction)
            .await
            .map_err(CloudError::database)?;
        }
        "starting" | "running" => {
            enqueue_terminal_command(
                &mut transaction,
                user.user_id,
                session_id,
                "terminate",
                json!({}),
                None,
            )
            .await?;
        }
        "terminating" | "exited" | "failed" | "cancelled" => {}
        _ => {
            return Err(CloudError::new(
                StatusCode::CONFLICT,
                "terminal_state_conflict",
                "terminal session cannot be terminated in its current state",
            ));
        }
    }
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(
        load_terminal_session(&cloud, user.user_id, session_id).await?,
    ))
}

async fn get_terminal_events(
    Path(session_id): Path<Uuid>,
    Query(query): Query<TerminalEventsQuery>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<TerminalEventsResponse>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let after_seq = query.after_seq.unwrap_or(0);
    if after_seq < 0 {
        return Err(CloudError::bad_request("after_seq cannot be negative"));
    }
    let limit = query.limit.unwrap_or(200);
    if !(1..=500).contains(&limit) {
        return Err(CloudError::bad_request("limit must be between 1 and 500"));
    }
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    reconcile_expired_terminal_sessions(&mut transaction, ExpiredTerminalScope::User(user.user_id))
        .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    let session = load_terminal_session(&cloud, user.user_id, session_id).await?;
    let events = sqlx::query(
        "SELECT seq, kind, data_base64, data, created_at FROM cloud_terminal_events
         WHERE session_id = $1 AND seq > $2 ORDER BY seq LIMIT $3",
    )
    .bind(session_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .iter()
    .map(terminal_event_view)
    .collect();
    Ok(Json(TerminalEventsResponse { session, events }))
}

async fn terminal_start_approval_is_current_for_lease(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    team_id: Option<Uuid>,
    approval_id: Option<Uuid>,
    requested_by: Uuid,
    approved_by: Option<Uuid>,
) -> CloudResult<bool> {
    let (Some(team_id), Some(approval_id), Some(approved_by)) = (team_id, approval_id, approved_by)
    else {
        return Ok(false);
    };
    let Some(approval) = sqlx::query(
        "SELECT team_id, requested_by, terminal_session_id, status, decided_by
         FROM cloud_approvals WHERE id = $1 FOR SHARE",
    )
    .bind(approval_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?
    else {
        return Ok(false);
    };
    let decided_by = approval.get::<Option<Uuid>, _>("decided_by");
    if approval.get::<Uuid, _>("team_id") != team_id
        || approval.get::<Uuid, _>("requested_by") != requested_by
        || approval.get::<Option<Uuid>, _>("terminal_session_id") != Some(session_id)
        || approval.get::<String, _>("status") != "approved"
        || decided_by != Some(approved_by)
        || decided_by == Some(requested_by)
    {
        return Ok(false);
    }
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM cloud_team_members
         WHERE team_id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(team_id)
    .bind(approved_by)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    Ok(role
        .as_deref()
        .is_some_and(|role| ["owner", "admin", "approver"].contains(&role)))
}

async fn lease_terminal_commands(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AgentTerminalCommandsRequest>,
) -> CloudResult<Response> {
    let cloud = cloud(&state)?;
    let token_hash = agent_task_token_hash(&headers)?;
    let instance_id = validate_terminal_instance_id(&request.instance_id)?;
    if request.max_commands == 0 || request.max_commands > TERMINAL_MAX_BATCH {
        return Err(CloudError::bad_request(
            "max_commands must be between 1 and 64",
        ));
    }
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (agent_id, _) = lock_active_terminal_agent_by_token(&mut transaction, &token_hash).await?;
    reconcile_expired_terminal_sessions(&mut transaction, ExpiredTerminalScope::Agent(agent_id))
        .await?;
    let rows = sqlx::query(
        "SELECT c.id, c.session_id, c.seq, c.kind, c.payload,
                s.user_id, s.team_id, s.approval_id, s.approved_by
         FROM cloud_terminal_commands c
         JOIN cloud_terminal_sessions s ON s.id = c.session_id
         WHERE s.agent_id = $1 AND c.acknowledged_at IS NULL
           AND (c.lease_expires_at IS NULL OR c.lease_expires_at <= NOW())
           AND (
             (c.kind = 'start' AND s.status = 'pending'
               AND EXISTS (
                 SELECT 1 FROM cloud_approvals a
                 JOIN cloud_team_members m
                   ON m.team_id = a.team_id AND m.user_id = a.decided_by
                 WHERE a.id = s.approval_id
                   AND a.terminal_session_id = s.id
                   AND a.team_id = s.team_id
                   AND a.requested_by = s.user_id
                   AND a.status = 'approved'
                   AND a.decided_by IS NOT NULL
                   AND a.decided_by <> s.user_id
                   AND m.role IN ('owner', 'admin', 'approver')
                   AND s.approved_by = a.decided_by
               )) OR
             (c.kind <> 'start' AND s.status IN ('starting', 'running', 'terminating')
               AND s.instance_id = $2)
           )
         ORDER BY c.created_at, c.seq
         LIMIT $3 FOR UPDATE OF c, s SKIP LOCKED",
    )
    .bind(agent_id)
    .bind(&instance_id)
    .bind(request.max_commands as i64)
    .fetch_all(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    for row in &rows {
        if row.get::<String, _>("kind") == "start"
            && !terminal_start_approval_is_current_for_lease(
                &mut transaction,
                row.get("session_id"),
                row.get("team_id"),
                row.get("approval_id"),
                row.get("user_id"),
                row.get("approved_by"),
            )
            .await?
        {
            transaction.commit().await.map_err(CloudError::database)?;
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
    }
    let mut commands = Vec::with_capacity(rows.len());
    for row in rows {
        let command_id: Uuid = row.get("id");
        let session_id: Uuid = row.get("session_id");
        let kind: String = row.get("kind");
        let stored_payload: Value = row.get("payload");
        let payload = if kind == "input" {
            decrypt_terminal_input_payload(cloud.master_key.as_ref(), &stored_payload)?
        } else {
            stored_payload
        };
        sqlx::query(
            "UPDATE cloud_terminal_commands
             SET lease_instance_id = $2,
                 lease_expires_at = NOW() + ($3 * INTERVAL '1 second')
             WHERE id = $1",
        )
        .bind(command_id)
        .bind(&instance_id)
        .bind(TERMINAL_COMMAND_LEASE_SECONDS)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
        if kind == "start" {
            sqlx::query(
                "UPDATE cloud_terminal_sessions
                 SET status = 'starting', instance_id = $2,
                     lease_expires_at = NOW() + ($3 * INTERVAL '1 second'),
                     last_seen_at = NOW(), updated_at = NOW()
                 WHERE id = $1 AND status = 'pending'",
            )
            .bind(session_id)
            .bind(&instance_id)
            .bind(TERMINAL_SESSION_LEASE_SECONDS)
            .execute(&mut *transaction)
            .await
            .map_err(CloudError::database)?;
        } else {
            sqlx::query(
                "UPDATE cloud_terminal_sessions
                 SET lease_expires_at = NOW() + ($2 * INTERVAL '1 second'),
                     last_seen_at = NOW(), updated_at = NOW()
                 WHERE id = $1 AND instance_id = $3
                   AND status IN ('starting', 'running', 'terminating')",
            )
            .bind(session_id)
            .bind(TERMINAL_SESSION_LEASE_SECONDS)
            .bind(&instance_id)
            .execute(&mut *transaction)
            .await
            .map_err(CloudError::database)?;
        }
        commands.push(TerminalCommandView {
            id: command_id,
            session_id,
            seq: row.get("seq"),
            kind,
            payload,
        });
    }
    transaction.commit().await.map_err(CloudError::database)?;
    if commands.is_empty() {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(Json(commands).into_response())
    }
}

#[derive(Serialize)]
struct AgentTerminalEventsResponse {
    session_id: Uuid,
    acknowledged_commands: usize,
    event_count: i32,
    status: String,
}

fn terminal_event_matches(row: &sqlx::postgres::PgRow, event: &AgentTerminalEventInput) -> bool {
    row.get::<String, _>("kind") == event.kind
        && row.get::<Option<Value>, _>("data") == event.data
        // Server-side redaction can intentionally make multiple raw output
        // chunks map to the same safe value. The sequence is immutable, so a
        // retried output cannot replace data and only needs a matching kind.
        && (event.kind == "output"
            || row.get::<Option<String>, _>("data_base64") == event.data_base64)
}

async fn create_terminal_events(
    Path(session_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut request): Json<AgentTerminalEventsRequest>,
) -> CloudResult<Json<AgentTerminalEventsResponse>> {
    let cloud = cloud(&state)?;
    let token_hash = agent_task_token_hash(&headers)?;
    let instance_id = validate_terminal_instance_id(&request.instance_id)?;
    if request.events.len() > TERMINAL_MAX_BATCH
        || request.acknowledged_command_ids.len() > TERMINAL_MAX_BATCH
    {
        return Err(CloudError::bad_request(
            "terminal event and acknowledgement batches cannot exceed 64 items",
        ));
    }
    let mut acknowledged_ids = request.acknowledged_command_ids.clone();
    acknowledged_ids.sort_unstable();
    acknowledged_ids.dedup();
    if acknowledged_ids.len() != request.acknowledged_command_ids.len() {
        return Err(CloudError::bad_request(
            "acknowledged_command_ids cannot contain duplicates",
        ));
    }
    let mut previous_seq = None;
    for event in &request.events {
        if previous_seq.is_some_and(|seq| event.seq <= seq) {
            return Err(CloudError::bad_request(
                "terminal events must have strictly increasing seq values",
            ));
        }
        previous_seq = Some(event.seq);
    }

    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (agent_id, _) = lock_active_terminal_agent_by_token(&mut transaction, &token_hash).await?;
    let session = sqlx::query(
        "SELECT status, instance_id, COALESCE(lease_expires_at > NOW(), FALSE) AS lease_valid,
                last_event_seq, event_count, output_bytes, terminal_redaction_pending
         FROM cloud_terminal_sessions WHERE id = $1 AND agent_id = $2 FOR UPDATE",
    )
    .bind(session_id)
    .bind(agent_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "terminal_session_not_found",
            "terminal session was not found for this agent",
        )
    })?;
    let mut status: String = session.get("status");
    let terminal = ["exited", "failed", "cancelled"].contains(&status.as_str());
    let mut redaction_pending: bool = session.get("terminal_redaction_pending");
    let mut output_sizes = Vec::with_capacity(request.events.len());
    for event in &mut request.events {
        // New agents redact before upload, but this server-side boundary also
        // protects sessions served by older or compromised agents. The small
        // state flag prevents a credential split across PTY chunks from being
        // reconstructed in persisted event rows.
        redaction_pending = sanitize_terminal_output(event, redaction_pending)?;
        output_sizes.push(validate_terminal_event(event)? as i64);
    }

    let acknowledgement_rows = if acknowledged_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query(
            "SELECT id, acknowledged_at, lease_instance_id,
                    acknowledged_at IS NOT NULL OR COALESCE(
                      lease_instance_id = $3 AND lease_expires_at > NOW(), FALSE) AS lease_valid
             FROM cloud_terminal_commands WHERE session_id = $1 AND id = ANY($2) FOR UPDATE",
        )
        .bind(session_id)
        .bind(&acknowledged_ids)
        .bind(&instance_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(CloudError::database)?
    };
    if acknowledgement_rows.len() != acknowledged_ids.len() {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "invalid_terminal_command_ack",
            "one or more terminal command acknowledgements are invalid",
        ));
    }

    if terminal {
        if acknowledgement_rows.iter().any(|row| {
            row.get::<Option<DateTime<Utc>>, _>("acknowledged_at")
                .is_none()
        }) {
            return Err(CloudError::new(
                StatusCode::CONFLICT,
                "terminal_state_conflict",
                "terminal session has already ended",
            ));
        }
        for event in &request.events {
            let existing = sqlx::query(
                "SELECT kind, data_base64, data FROM cloud_terminal_events
                 WHERE session_id = $1 AND seq = $2",
            )
            .bind(session_id)
            .bind(event.seq)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(CloudError::database)?;
            if existing
                .as_ref()
                .is_none_or(|row| !terminal_event_matches(row, event))
            {
                return Err(CloudError::new(
                    StatusCode::CONFLICT,
                    "terminal_event_conflict",
                    "terminal event conflicts with an existing terminal result",
                ));
            }
        }
        transaction.commit().await.map_err(CloudError::database)?;
        return Ok(Json(AgentTerminalEventsResponse {
            session_id,
            acknowledged_commands: acknowledged_ids.len(),
            event_count: session.get("event_count"),
            status,
        }));
    }

    let bound_instance: Option<String> = session.get("instance_id");
    if bound_instance.as_deref() != Some(instance_id.as_str())
        || !session.get::<bool, _>("lease_valid")
        || !["starting", "running", "terminating"].contains(&status.as_str())
    {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "invalid_terminal_lease",
            "terminal session lease is invalid or expired",
        ));
    }
    for row in &acknowledgement_rows {
        if row
            .get::<Option<DateTime<Utc>>, _>("acknowledged_at")
            .is_none()
        {
            let lease_instance: Option<String> = row.get("lease_instance_id");
            if lease_instance.as_deref() != Some(instance_id.as_str())
                || !row.get::<bool, _>("lease_valid")
            {
                return Err(CloudError::new(
                    StatusCode::CONFLICT,
                    "invalid_terminal_command_ack",
                    "terminal command lease is invalid or expired",
                ));
            }
            sqlx::query(
                "UPDATE cloud_terminal_commands SET acknowledged_at = NOW(),
                 lease_instance_id = NULL, lease_expires_at = NULL,
                 payload = CASE WHEN kind = 'input' THEN jsonb_build_object(
                   'format', 'redacted-v1', 'input_mac', payload -> 'input_mac'
                 ) ELSE payload END
                 WHERE id = $1",
            )
            .bind(row.get::<Uuid, _>("id"))
            .execute(&mut *transaction)
            .await
            .map_err(CloudError::database)?;
        }
    }

    let mut last_event_seq: i64 = session.get("last_event_seq");
    let mut event_count: i32 = session.get("event_count");
    let mut output_bytes: i64 = session.get("output_bytes");
    let mut exit_code = None;
    let mut error = None;
    let mut renew_lease = false;
    let mut started = false;
    let mut ended = false;
    for (event, output_size) in request.events.iter().zip(output_sizes) {
        let existing = sqlx::query(
            "SELECT kind, data_base64, data FROM cloud_terminal_events
             WHERE session_id = $1 AND seq = $2",
        )
        .bind(session_id)
        .bind(event.seq)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
        if let Some(existing) = existing {
            if !terminal_event_matches(&existing, event) {
                return Err(CloudError::new(
                    StatusCode::CONFLICT,
                    "terminal_event_conflict",
                    "terminal event seq was already used with different content",
                ));
            }
            continue;
        }
        if event.seq != last_event_seq + 1 {
            return Err(CloudError::new(
                StatusCode::CONFLICT,
                "terminal_event_sequence_gap",
                "terminal events must be appended without sequence gaps",
            ));
        }
        if event_count >= TERMINAL_MAX_EVENTS
            || output_bytes + output_size > TERMINAL_MAX_OUTPUT_BYTES
        {
            return Err(CloudError::new(
                StatusCode::CONFLICT,
                "terminal_event_limit",
                "terminal session reached its cumulative event or output limit",
            ));
        }
        match event.kind.as_str() {
            "started" if status == "starting" => {
                status = "running".into();
                started = true;
                renew_lease = true;
            }
            "started" if status == "terminating" => {
                started = true;
                renew_lease = true;
            }
            "output" | "keepalive" if ["running", "terminating"].contains(&status.as_str()) => {
                renew_lease = true;
            }
            "exit" if ["starting", "running", "terminating"].contains(&status.as_str()) => {
                status = "exited".into();
                exit_code = event
                    .data
                    .as_ref()
                    .and_then(|value| value["exit_code"].as_i64())
                    .map(|value| value as i32);
                ended = true;
            }
            "error" if ["starting", "running", "terminating"].contains(&status.as_str()) => {
                status = "failed".into();
                error = event
                    .data
                    .as_ref()
                    .and_then(|value| value["message"].as_str())
                    .map(ToString::to_string);
                ended = true;
            }
            _ => {
                return Err(CloudError::new(
                    StatusCode::CONFLICT,
                    "terminal_event_state_conflict",
                    "terminal event is invalid for the current session state",
                ));
            }
        }
        sqlx::query(
            "INSERT INTO cloud_terminal_events
             (id, session_id, seq, kind, data_base64, data, output_bytes)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(session_id)
        .bind(event.seq)
        .bind(&event.kind)
        .bind(&event.data_base64)
        .bind(&event.data)
        .bind(output_size as i32)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
        last_event_seq = event.seq;
        event_count += 1;
        output_bytes += output_size;
    }

    if ended {
        sqlx::query(
            "UPDATE cloud_terminal_sessions
             SET status = $2, exit_code = $3, error = $4, last_event_seq = $5,
                 event_count = $6, output_bytes = $7, last_seen_at = NOW(), exited_at = NOW(),
                 terminal_redaction_pending = FALSE, instance_id = NULL,
                 lease_expires_at = NULL, updated_at = NOW()
             WHERE id = $1",
        )
        .bind(session_id)
        .bind(&status)
        .bind(exit_code)
        .bind(error)
        .bind(last_event_seq)
        .bind(event_count)
        .bind(output_bytes)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    } else {
        sqlx::query(
            "UPDATE cloud_terminal_sessions
             SET status = $2, last_event_seq = $3, event_count = $4, output_bytes = $5,
                 terminal_redaction_pending = $6,
                 started_at = CASE WHEN $7 THEN COALESCE(started_at, NOW()) ELSE started_at END,
                 last_seen_at = CASE WHEN $8 THEN NOW() ELSE last_seen_at END,
                 lease_expires_at = CASE WHEN $8
                   THEN NOW() + ($9 * INTERVAL '1 second') ELSE lease_expires_at END,
                 updated_at = NOW() WHERE id = $1",
        )
        .bind(session_id)
        .bind(&status)
        .bind(last_event_seq)
        .bind(event_count)
        .bind(output_bytes)
        .bind(redaction_pending)
        .bind(started)
        .bind(renew_lease)
        .bind(TERMINAL_SESSION_LEASE_SECONDS)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    }
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(AgentTerminalEventsResponse {
        session_id,
        acknowledged_commands: acknowledged_ids.len(),
        event_count,
        status,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConversationRequest {
    title: Option<String>,
    agent_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConversationMessageRequest {
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConversationPlanRequest {
    content: String,
    agent_id: Uuid,
    operation: String,
    input: Value,
    idempotency_key: Option<String>,
    #[serde(default)]
    team_id: Option<Uuid>,
}

#[derive(Serialize)]
struct ConversationView {
    id: Uuid,
    title: String,
    agent_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct ConversationMessageView {
    id: Uuid,
    role: String,
    content: String,
    kind: String,
    linked_task_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct ConversationDetail {
    #[serde(flatten)]
    conversation: ConversationView,
    messages: Vec<ConversationMessageView>,
}

const CONVERSATION_SELECT: &str =
    "SELECT id, title, agent_id, created_at, updated_at FROM cloud_conversations";

fn conversation_view(row: &sqlx::postgres::PgRow) -> ConversationView {
    ConversationView {
        id: row.get("id"),
        title: row.get("title"),
        agent_id: row.get("agent_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn conversation_message_view(row: &sqlx::postgres::PgRow) -> ConversationMessageView {
    ConversationMessageView {
        id: row.get("id"),
        role: row.get("role"),
        content: row.get("content"),
        kind: row.get("kind"),
        linked_task_id: row.get("linked_task_id"),
        created_at: row.get("created_at"),
    }
}

fn validate_conversation_title(value: Option<String>) -> CloudResult<String> {
    bounded_agent_text(value.as_deref().unwrap_or("新对话"), "title", 1, 128)
}

fn validate_conversation_content(value: &str) -> CloudResult<String> {
    let value = value.trim();
    let length = value.chars().count();
    if length == 0
        || length > 20_000
        || value
            .chars()
            .any(|character| character.is_control() && !"\n\r\t".contains(character))
    {
        return Err(CloudError::bad_request(
            "conversation content must contain 1-20000 characters",
        ));
    }
    Ok(value.to_string())
}

async fn ensure_conversation_agent_owned(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    agent_id: Uuid,
) -> CloudResult<()> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM cloud_agents WHERE id = $1 AND user_id = $2)",
    )
    .bind(agent_id)
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(CloudError::database)?;
    if !exists {
        return Err(CloudError::new(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "agent was not found",
        ));
    }
    Ok(())
}

async fn load_conversation(
    cloud: &CloudState,
    user_id: Uuid,
    conversation_id: Uuid,
) -> CloudResult<ConversationDetail> {
    let sql = format!("{CONVERSATION_SELECT} WHERE id = $1 AND user_id = $2");
    let row = sqlx::query(&sql)
        .bind(conversation_id)
        .bind(user_id)
        .fetch_optional(&cloud.db)
        .await
        .map_err(CloudError::database)?
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::NOT_FOUND,
                "conversation_not_found",
                "conversation was not found",
            )
        })?;
    let messages = sqlx::query(
        "SELECT id, role, content, kind, linked_task_id, created_at
         FROM cloud_conversation_messages WHERE conversation_id = $1 ORDER BY seq",
    )
    .bind(conversation_id)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .iter()
    .map(conversation_message_view)
    .collect();
    Ok(ConversationDetail {
        conversation: conversation_view(&row),
        messages,
    })
}

async fn list_conversations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Vec<ConversationView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let sql =
        format!("{CONVERSATION_SELECT} WHERE user_id = $1 ORDER BY updated_at DESC LIMIT 100");
    let conversations = sqlx::query(&sql)
        .bind(user.user_id)
        .fetch_all(&cloud.db)
        .await
        .map_err(CloudError::database)?
        .iter()
        .map(conversation_view)
        .collect();
    Ok(Json(conversations))
}

async fn create_conversation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateConversationRequest>,
) -> CloudResult<(StatusCode, Json<ConversationDetail>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let title = validate_conversation_title(request.title)?;
    let conversation_id = Uuid::new_v4();
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    if let Some(agent_id) = request.agent_id {
        ensure_conversation_agent_owned(&mut transaction, user.user_id, agent_id).await?;
    }
    sqlx::query(
        "INSERT INTO cloud_conversations (id, user_id, title, agent_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(conversation_id)
    .bind(user.user_id)
    .bind(title)
    .bind(request.agent_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(load_conversation(&cloud, user.user_id, conversation_id).await?),
    ))
}

async fn get_conversation(
    Path(conversation_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<ConversationDetail>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    Ok(Json(
        load_conversation(&cloud, user.user_id, conversation_id).await?,
    ))
}

async fn lock_conversation_for_message(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    conversation_id: Uuid,
) -> CloudResult<(i64, Option<Uuid>)> {
    let row = sqlx::query(
        "SELECT next_message_seq, agent_id FROM cloud_conversations
         WHERE id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(conversation_id)
    .bind(user_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "conversation_not_found",
            "conversation was not found",
        )
    })?;
    let next_message_seq: i64 = row.get("next_message_seq");
    if next_message_seq >= 1000 {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "conversation_message_limit",
            "conversation already has the maximum number of messages",
        ));
    }
    Ok((next_message_seq, row.get("agent_id")))
}

async fn create_conversation_message(
    Path(conversation_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateConversationMessageRequest>,
) -> CloudResult<(StatusCode, Json<ConversationMessageView>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let content = validate_conversation_content(&request.content)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (last_seq, _) =
        lock_conversation_for_message(&mut transaction, user.user_id, conversation_id).await?;
    let row = sqlx::query(
        "INSERT INTO cloud_conversation_messages
         (id, conversation_id, seq, role, content, kind)
         VALUES ($1, $2, $3, 'user', $4, 'text')
         RETURNING id, role, content, kind, linked_task_id, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(conversation_id)
    .bind(last_seq + 1)
    .bind(content)
    .fetch_one(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sqlx::query(
        "UPDATE cloud_conversations SET next_message_seq = $2, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(conversation_id)
    .bind(last_seq + 1)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    let message = conversation_message_view(&row);
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((StatusCode::CREATED, Json(message)))
}

async fn create_conversation_plan(
    Path(conversation_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateConversationPlanRequest>,
) -> CloudResult<(StatusCode, Json<ConversationDetail>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let content = validate_conversation_content(&request.content)?;
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let (last_seq, conversation_agent_id) =
        lock_conversation_for_message(&mut transaction, user.user_id, conversation_id).await?;
    if conversation_agent_id.is_some_and(|agent_id| agent_id != request.agent_id) {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "conversation_agent_conflict",
            "conversation is already assigned to a different agent",
        ));
    }
    if last_seq > 998 {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "conversation_message_limit",
            "conversation does not have room for a plan exchange",
        ));
    }
    let user_message_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO cloud_conversation_messages
         (id, conversation_id, seq, role, content, kind)
         VALUES ($1, $2, $3, 'user', $4, 'text')",
    )
    .bind(user_message_id)
    .bind(conversation_id)
    .bind(last_seq + 1)
    .bind(content)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    let task = create_agent_task_record(
        &mut transaction,
        user.user_id,
        request.agent_id,
        &request.operation,
        &request.input,
        request.idempotency_key,
        request.team_id,
    )
    .await?;
    if !task.created {
        let linked_conversation = sqlx::query_scalar::<_, Uuid>(
            "SELECT conversation_id FROM cloud_conversation_messages
             WHERE linked_task_id = $1 AND kind = 'plan'",
        )
        .bind(task.id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
        if let Some(linked_conversation) = linked_conversation {
            if linked_conversation != conversation_id {
                return Err(CloudError::new(
                    StatusCode::CONFLICT,
                    "idempotency_conflict",
                    "idempotent task is already linked to another conversation",
                ));
            }
            sqlx::query("DELETE FROM cloud_conversation_messages WHERE id = $1")
                .bind(user_message_id)
                .execute(&mut *transaction)
                .await
                .map_err(CloudError::database)?;
            transaction.commit().await.map_err(CloudError::database)?;
            return Ok((
                StatusCode::OK,
                Json(load_conversation(&cloud, user.user_id, conversation_id).await?),
            ));
        }
    }
    sqlx::query(
        "INSERT INTO cloud_conversation_messages
         (id, conversation_id, seq, role, content, kind, linked_task_id)
         VALUES ($1, $2, $3, 'assistant', $4, 'plan', $5)",
    )
    .bind(Uuid::new_v4())
    .bind(conversation_id)
    .bind(last_seq + 2)
    .bind("执行计划已创建。请查看关联任务的状态并继续操作。")
    .bind(task.id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sqlx::query(
        "UPDATE cloud_conversations
         SET agent_id = COALESCE(agent_id, $2), next_message_seq = $3, updated_at = NOW()
         WHERE id = $1",
    )
    .bind(conversation_id)
    .bind(request.agent_id)
    .bind(last_seq + 2)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(load_conversation(&cloud, user.user_id, conversation_id).await?),
    ))
}

#[derive(Serialize)]
struct SyncedSettings {
    version: i64,
    payload: Value,
    updated_at: DateTime<Utc>,
    updated_by_device: Option<Uuid>,
}

#[derive(Deserialize)]
struct SyncSettingsRequest {
    base_version: i64,
    payload: Value,
}

const MAX_SYNC_PAYLOAD_BYTES: usize = 1_048_576;

fn payload_contains_secret(value: &Value) -> bool {
    match value {
        Value::Object(entries) => entries.iter().any(|(key, value)| {
            let normalized = key
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>();
            matches!(
                normalized.as_str(),
                "apikey"
                    | "accesstoken"
                    | "refreshtoken"
                    | "token"
                    | "authorization"
                    | "cookie"
                    | "session"
                    | "clientsecret"
                    | "rconpassword"
                    | "password"
                    | "secret"
                    | "privatekey"
                    | "credential"
                    | "command"
                    | "args"
                    | "path"
                    | "rootpath"
                    | "host"
            ) || payload_contains_secret(value)
        }),
        Value::Array(items) => items.iter().any(payload_contains_secret),
        _ => false,
    }
}

fn validate_sync_payload(payload: &Value) -> CloudResult<()> {
    if !payload.is_object() {
        return Err(CloudError::bad_request("同步设置必须是 JSON 对象"));
    }
    let encoded =
        serde_json::to_vec(payload).map_err(|_| CloudError::bad_request("同步设置无法序列化"))?;
    if encoded.len() > MAX_SYNC_PAYLOAD_BYTES {
        return Err(CloudError::bad_request("同步设置不能超过 1 MiB"));
    }
    if payload_contains_secret(payload) {
        return Err(CloudError::bad_request(
            "同步设置不能包含密码、API Key 或私钥；请使用加密凭据接口",
        ));
    }
    Ok(())
}

async fn get_synced_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<SyncedSettings>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let row = sqlx::query(
        "SELECT version, payload, updated_at, updated_by_device FROM cloud_settings WHERE user_id = $1",
    )
    .bind(user.user_id)
    .fetch_one(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(SyncedSettings {
        version: row.get("version"),
        payload: row.get("payload"),
        updated_at: row.get("updated_at"),
        updated_by_device: row.get("updated_by_device"),
    }))
}

async fn put_synced_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SyncSettingsRequest>,
) -> CloudResult<Json<SyncedSettings>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    validate_sync_payload(&request.payload)?;
    let row = sqlx::query(
        "UPDATE cloud_settings
         SET version = version + 1, payload = $3, updated_by_device = $4, updated_at = NOW()
         WHERE user_id = $1 AND version = $2
         RETURNING version, payload, updated_at, updated_by_device",
    )
    .bind(user.user_id)
    .bind(request.base_version)
    .bind(request.payload)
    .bind(user.device_id)
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "sync_conflict",
            "云端设置已经更新，请先拉取最新版本再同步",
        )
    })?;
    Ok(Json(SyncedSettings {
        version: row.get("version"),
        payload: row.get("payload"),
        updated_at: row.get("updated_at"),
        updated_by_device: row.get("updated_by_device"),
    }))
}

#[derive(Serialize)]
struct UserCredentialView {
    id: Uuid,
    name: String,
    base_url: String,
    api_key_masked: String,
    fingerprint: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct SaveUserCredentialRequest {
    name: String,
    base_url: String,
    api_key: String,
}

fn validate_credential_request(
    request: &SaveUserCredentialRequest,
) -> CloudResult<(String, String, String)> {
    let name = request.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(CloudError::bad_request("凭据名称需要为 1-64 个字符"));
    }
    let base_url = request.base_url.trim().trim_end_matches('/').to_string();
    let parsed = Url::parse(&base_url)
        .map_err(|_| CloudError::bad_request("Base URL 必须是合法的 HTTP(S) 地址"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(CloudError::bad_request("Base URL 仅支持 HTTP(S)"));
    }
    let api_key = request.api_key.trim().to_string();
    if api_key.len() < 8 || api_key.len() > 4096 {
        return Err(CloudError::bad_request("API Key 长度需要为 8-4096 个字符"));
    }
    Ok((name, base_url, api_key))
}

fn key_edges(value: &str) -> (String, String) {
    let characters = value.chars().collect::<Vec<_>>();
    let edge_length = if characters.len() >= 16 { 4 } else { 2 };
    let prefix = characters.iter().take(edge_length).collect::<String>();
    let suffix = characters
        .iter()
        .rev()
        .take(edge_length)
        .rev()
        .collect::<String>();
    (prefix, suffix)
}

fn credential_view(row: &sqlx::postgres::PgRow) -> UserCredentialView {
    let prefix: String = row.get("key_prefix");
    let suffix: String = row.get("key_suffix");
    let fingerprint: String = row.get("key_fingerprint");
    UserCredentialView {
        id: row.get("id"),
        name: row.get("name"),
        base_url: row.get("base_url"),
        api_key_masked: format!("{prefix}••••••••{suffix}"),
        fingerprint: fingerprint.chars().take(12).collect(),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

async fn list_user_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Vec<UserCredentialView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let rows = sqlx::query(
        "SELECT id, name, base_url, key_fingerprint, key_prefix, key_suffix,
                created_at, updated_at
         FROM cloud_user_api_credentials
         WHERE user_id = $1
         ORDER BY updated_at DESC",
    )
    .bind(user.user_id)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(rows.iter().map(credential_view).collect()))
}

async fn save_user_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveUserCredentialRequest>,
) -> CloudResult<Json<UserCredentialView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let (name, base_url, api_key) = validate_credential_request(&request)?;
    let fingerprint = sha256_hex(&api_key);
    let (prefix, suffix) = key_edges(&api_key);
    let (cipher, nonce) = encrypt_api_key(cloud.master_key.as_ref(), &api_key)?;
    let row = sqlx::query(
        "INSERT INTO cloud_user_api_credentials
            (id, user_id, name, base_url, api_key_cipher, api_key_nonce,
             key_fingerprint, key_prefix, key_suffix)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (user_id, key_fingerprint) DO UPDATE SET
            name = EXCLUDED.name,
            base_url = EXCLUDED.base_url,
            api_key_cipher = EXCLUDED.api_key_cipher,
            api_key_nonce = EXCLUDED.api_key_nonce,
            key_prefix = EXCLUDED.key_prefix,
            key_suffix = EXCLUDED.key_suffix,
            updated_at = NOW()
         RETURNING id, name, base_url, key_fingerprint, key_prefix, key_suffix,
                   created_at, updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(user.user_id)
    .bind(name)
    .bind(base_url)
    .bind(cipher)
    .bind(nonce)
    .bind(fingerprint)
    .bind(prefix)
    .bind(suffix)
    .fetch_one(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(credential_view(&row)))
}

async fn delete_user_credential(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<StatusCode> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let result =
        sqlx::query("DELETE FROM cloud_user_api_credentials WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.user_id)
            .execute(&cloud.db)
            .await
            .map_err(CloudError::database)?;
    if result.rows_affected() == 0 {
        return Err(CloudError::new(
            StatusCode::NOT_FOUND,
            "credential_not_found",
            "加密凭据不存在",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct TeamView {
    id: Uuid,
    name: String,
    slug: String,
    role: String,
    member_count: i64,
    pending_approvals: i64,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct TeamMemberView {
    id: Uuid,
    email: String,
    nickname: String,
    avatar_url: String,
    role: String,
    joined_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct CreateTeamRequest {
    name: String,
}

fn slugify(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if slug.is_empty() { "team" } else { &slug };
    format!("{base}-{}", &Uuid::new_v4().simple().to_string()[..6])
}

async fn list_teams(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Vec<TeamView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let rows = sqlx::query(
        "SELECT t.id, t.name, t.slug, tm.role, t.created_at,
                (SELECT COUNT(*) FROM cloud_team_members members WHERE members.team_id = t.id) AS member_count,
                (SELECT COUNT(*) FROM cloud_approvals approvals WHERE approvals.team_id = t.id AND approvals.status = 'pending') AS pending_approvals
         FROM cloud_team_members tm
         JOIN cloud_teams t ON t.id = tm.team_id
         WHERE tm.user_id = $1
         ORDER BY t.created_at DESC",
    )
    .bind(user.user_id)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(
        rows.iter()
            .map(|row| TeamView {
                id: row.get("id"),
                name: row.get("name"),
                slug: row.get("slug"),
                role: row.get("role"),
                member_count: row.get("member_count"),
                pending_approvals: row.get("pending_approvals"),
                created_at: row.get("created_at"),
            })
            .collect(),
    ))
}

async fn create_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateTeamRequest>,
) -> CloudResult<(StatusCode, Json<TeamView>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 48 {
        return Err(CloudError::bad_request("团队名称需要为 1-48 个字符"));
    }
    let id = Uuid::new_v4();
    let slug = slugify(name);
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let row = sqlx::query(
        "INSERT INTO cloud_teams (id, name, slug, owner_id) VALUES ($1, $2, $3, $4)
         RETURNING created_at",
    )
    .bind(id)
    .bind(name)
    .bind(&slug)
    .bind(user.user_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sqlx::query("INSERT INTO cloud_team_members (team_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(id)
        .bind(user.user_id)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(TeamView {
            id,
            name: name.to_string(),
            slug,
            role: "owner".into(),
            member_count: 1,
            pending_approvals: 0,
            created_at: row.get("created_at"),
        }),
    ))
}

async fn require_team_role(
    cloud: &CloudState,
    team_id: Uuid,
    user_id: Uuid,
    allowed: &[&str],
) -> CloudResult<String> {
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM cloud_team_members WHERE team_id = $1 AND user_id = $2",
    )
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::FORBIDDEN,
            "team_access_denied",
            "你不是该团队成员",
        )
    })?;
    if !allowed.contains(&role.as_str()) {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "team_role_denied",
            "当前团队角色无权执行该操作",
        ));
    }
    Ok(role)
}

async fn list_team_members(
    Path(team_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Vec<TeamMemberView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    require_team_role(
        &cloud,
        team_id,
        user.user_id,
        &["owner", "admin", "approver", "member"],
    )
    .await?;
    let rows = sqlx::query(
        "SELECT u.id, u.email, u.nickname, u.avatar_url, tm.role, tm.joined_at
         FROM cloud_team_members tm JOIN cloud_users u ON u.id = tm.user_id
         WHERE tm.team_id = $1
         ORDER BY CASE tm.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 WHEN 'approver' THEN 2 ELSE 3 END, tm.joined_at",
    )
    .bind(team_id)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(
        rows.iter()
            .map(|row| TeamMemberView {
                id: row.get("id"),
                email: row.get("email"),
                nickname: row.get("nickname"),
                avatar_url: row.get("avatar_url"),
                role: row.get("role"),
                joined_at: row.get("joined_at"),
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct InviteMemberRequest {
    email: String,
    role: String,
}

#[derive(Serialize)]
struct InvitationView {
    id: Uuid,
    email: String,
    role: String,
    invite_code: String,
    expires_at: DateTime<Utc>,
}

async fn invite_member(
    Path(team_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<InviteMemberRequest>,
) -> CloudResult<(StatusCode, Json<InvitationView>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    require_team_role(&cloud, team_id, user.user_id, &["owner", "admin"]).await?;
    let email = validate_email(&request.email)?;
    if !["admin", "approver", "member"].contains(&request.role.as_str()) {
        return Err(CloudError::bad_request("邀请角色无效"));
    }
    let id = Uuid::new_v4();
    let invite_code = random_token("sci_");
    let expires_at = Utc::now() + Duration::days(7);
    sqlx::query(
        "INSERT INTO cloud_team_invitations
         (id, team_id, email, role, token_hash, invited_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(team_id)
    .bind(&email)
    .bind(&request.role)
    .bind(sha256_hex(&invite_code))
    .bind(user.user_id)
    .bind(expires_at)
    .execute(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(InvitationView {
            id,
            email,
            role: request.role,
            invite_code,
            expires_at,
        }),
    ))
}

#[derive(Deserialize)]
struct AcceptInvitationRequest {
    invite_code: String,
}

async fn accept_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AcceptInvitationRequest>,
) -> CloudResult<Json<TeamView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let row = sqlx::query(
        "SELECT i.id, i.team_id, i.role, t.name, t.slug, t.created_at
         FROM cloud_team_invitations i JOIN cloud_teams t ON t.id = i.team_id
         WHERE i.token_hash = $1 AND LOWER(i.email) = LOWER($2)
           AND i.accepted_at IS NULL AND i.expires_at > NOW()",
    )
    .bind(sha256_hex(request.invite_code.trim()))
    .bind(&user.email)
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::NOT_FOUND,
            "invitation_not_found",
            "邀请码无效、已过期或与当前邮箱不匹配",
        )
    })?;
    let invitation_id: Uuid = row.get("id");
    let team_id: Uuid = row.get("team_id");
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    sqlx::query(
        "INSERT INTO cloud_team_members (team_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (team_id, user_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team_id)
    .bind(user.user_id)
    .bind(row.get::<String, _>("role"))
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sqlx::query("UPDATE cloud_team_invitations SET accepted_at = NOW() WHERE id = $1")
        .bind(invitation_id)
        .execute(&mut *transaction)
        .await
        .map_err(CloudError::database)?;
    transaction.commit().await.map_err(CloudError::database)?;
    Ok(Json(TeamView {
        id: team_id,
        name: row.get("name"),
        slug: row.get("slug"),
        role: row.get("role"),
        member_count: 0,
        pending_approvals: 0,
        created_at: row.get("created_at"),
    }))
}

#[derive(Serialize)]
struct ApprovalView {
    id: Uuid,
    team_id: Uuid,
    agent_task_id: Option<Uuid>,
    terminal_session_id: Option<Uuid>,
    team_name: String,
    requested_by: Uuid,
    requester_name: String,
    title: String,
    summary: String,
    risk: String,
    status: String,
    payload: Value,
    decision_comment: String,
    decided_by: Option<Uuid>,
    decided_by_name: Option<String>,
    decided_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

fn approval_from_row(row: &sqlx::postgres::PgRow) -> ApprovalView {
    ApprovalView {
        id: row.get("id"),
        team_id: row.get("team_id"),
        agent_task_id: row.get("agent_task_id"),
        terminal_session_id: row.get("terminal_session_id"),
        team_name: row.get("team_name"),
        requested_by: row.get("requested_by"),
        requester_name: row.get("requester_name"),
        title: row.get("title"),
        summary: row.get("summary"),
        risk: row.get("risk"),
        status: row.get("status"),
        payload: row.get("payload"),
        decision_comment: row.get("decision_comment"),
        decided_by: row.get("decided_by"),
        decided_by_name: row.get("decided_by_name"),
        decided_at: row.get("decided_at"),
        created_at: row.get("created_at"),
    }
}

#[derive(Deserialize)]
struct ApprovalQuery {
    team_id: Option<Uuid>,
    status: Option<String>,
}

const APPROVAL_SELECT: &str = "SELECT a.id, a.team_id, a.agent_task_id, a.terminal_session_id,
            t.name AS team_name, a.requested_by,
            requester.nickname AS requester_name, a.title, a.summary, a.risk, a.status,
            a.payload, a.decision_comment, a.decided_by, decider.nickname AS decided_by_name,
            a.decided_at, a.created_at
     FROM cloud_approvals a
     JOIN cloud_teams t ON t.id = a.team_id
     JOIN cloud_users requester ON requester.id = a.requested_by
     LEFT JOIN cloud_users decider ON decider.id = a.decided_by";

async fn list_approvals(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ApprovalQuery>,
) -> CloudResult<Json<Vec<ApprovalView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let sql = format!(
        "{APPROVAL_SELECT}
         JOIN cloud_team_members membership ON membership.team_id = a.team_id
         WHERE membership.user_id = $1
           AND ($2::uuid IS NULL OR a.team_id = $2)
           AND ($3::text IS NULL OR a.status = $3)
         ORDER BY CASE a.status WHEN 'pending' THEN 0 ELSE 1 END, a.created_at DESC
         LIMIT 100"
    );
    let rows = sqlx::query(&sql)
        .bind(user.user_id)
        .bind(query.team_id)
        .bind(query.status)
        .fetch_all(&cloud.db)
        .await
        .map_err(CloudError::database)?;
    Ok(Json(rows.iter().map(approval_from_row).collect()))
}

#[derive(Deserialize)]
struct CreateApprovalRequest {
    team_id: Uuid,
    title: String,
    #[serde(default)]
    summary: String,
    risk: String,
    #[serde(default)]
    payload: Value,
}

async fn create_approval(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateApprovalRequest>,
) -> CloudResult<(StatusCode, Json<ApprovalView>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    require_team_role(
        &cloud,
        request.team_id,
        user.user_id,
        &["owner", "admin", "approver", "member"],
    )
    .await?;
    let title = request.title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(CloudError::bad_request("审批标题需要为 1-120 个字符"));
    }
    if !["low", "medium", "high"].contains(&request.risk.as_str()) {
        return Err(CloudError::bad_request("风险等级无效"));
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO cloud_approvals
         (id, team_id, requested_by, title, summary, risk, payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(request.team_id)
    .bind(user.user_id)
    .bind(title)
    .bind(request.summary.trim())
    .bind(&request.risk)
    .bind(request.payload)
    .execute(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    let sql = format!("{APPROVAL_SELECT} WHERE a.id = $1");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_one(&cloud.db)
        .await
        .map_err(CloudError::database)?;
    Ok((StatusCode::CREATED, Json(approval_from_row(&row))))
}

#[derive(Deserialize)]
struct DecideApprovalRequest {
    decision: String,
    #[serde(default)]
    comment: String,
}

async fn decide_approval(
    Path(approval_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<DecideApprovalRequest>,
) -> CloudResult<Json<ApprovalView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    if !["approved", "rejected"].contains(&request.decision.as_str()) {
        return Err(CloudError::bad_request(
            "审批决定必须为 approved 或 rejected",
        ));
    }
    let mut transaction = cloud.db.begin().await.map_err(CloudError::database)?;
    let initial = sqlx::query(
        "SELECT team_id, requested_by, agent_task_id, terminal_session_id
         FROM cloud_approvals WHERE id = $1 AND status = 'pending'",
    )
    .bind(approval_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "approval_closed",
            "审批不存在或已经处理",
        )
    })?;
    let requested_by: Uuid = initial.get("requested_by");
    if requested_by == user.user_id {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "approval_self_forbidden",
            "审批请求人不能处理自己的审批",
        ));
    }

    // Lock linked resources before the approval row. Cancellation and the
    // compatibility task-approval endpoint use the same order, preventing a
    // task/approval deadlock under concurrent decisions.
    if let Some(task_id) = initial.get::<Option<Uuid>, _>("agent_task_id") {
        sqlx::query(
            "SELECT id FROM cloud_agent_tasks
             WHERE id = $1 AND approval_id = $2 FOR UPDATE",
        )
        .bind(task_id)
        .bind(approval_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(CloudError::database)?
        .ok_or_else(agent_task_approval_invalid_error)?;
    }
    if let Some(session_id) = initial.get::<Option<Uuid>, _>("terminal_session_id") {
        sqlx::query(
            "SELECT id FROM cloud_terminal_sessions
             WHERE id = $1 AND approval_id = $2 FOR UPDATE",
        )
        .bind(session_id)
        .bind(approval_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(CloudError::database)?
        .ok_or_else(|| {
            CloudError::new(
                StatusCode::FORBIDDEN,
                "terminal_approval_invalid",
                "the linked terminal approval does not match the session",
            )
        })?;
    }
    let approval = sqlx::query(
        "SELECT team_id, requested_by, agent_task_id, terminal_session_id, status
         FROM cloud_approvals WHERE id = $1 FOR UPDATE",
    )
    .bind(approval_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    if approval.get::<String, _>("status") != "pending"
        || approval.get::<Uuid, _>("requested_by") != requested_by
    {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "approval_closed",
            "该审批刚刚被其他成员处理",
        ));
    }
    let team_id: Uuid = approval.get("team_id");
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM cloud_team_members
         WHERE team_id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(team_id)
    .bind(user.user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    if !role
        .as_deref()
        .is_some_and(|role| ["owner", "admin", "approver"].contains(&role))
    {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "team_approval_required",
            "只有团队所有者、管理员或审批人可以处理审批",
        ));
    }
    sqlx::query(
        "UPDATE cloud_approvals
         SET status = $2, decision_comment = $3, decided_by = $4, decided_at = NOW()
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(approval_id)
    .bind(&request.decision)
    .bind(request.comment.trim())
    .bind(user.user_id)
    .execute(&mut *transaction)
    .await
    .map_err(CloudError::database)?;
    sync_agent_task_after_approval(
        &mut transaction,
        approval_id,
        approval.get("agent_task_id"),
        team_id,
        requested_by,
        user.user_id,
        &request.decision,
    )
    .await?;
    sync_terminal_after_approval(
        &mut transaction,
        approval_id,
        approval.get("terminal_session_id"),
        team_id,
        requested_by,
        user.user_id,
        &request.decision,
    )
    .await?;
    transaction.commit().await.map_err(CloudError::database)?;
    let sql = format!("{APPROVAL_SELECT} WHERE a.id = $1");
    let row = sqlx::query(&sql)
        .bind(approval_id)
        .fetch_one(&cloud.db)
        .await
        .map_err(CloudError::database)?;
    Ok(Json(approval_from_row(&row)))
}

#[derive(Serialize)]
struct ApiTokenView {
    id: Uuid,
    label: String,
    token_prefix: String,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    total_tokens: i64,
    request_count: i64,
}

#[derive(Serialize)]
struct ApiTokenCreated {
    token: String,
    item: ApiTokenView,
}

#[derive(Deserialize)]
struct CreateApiTokenRequest {
    label: String,
    #[serde(default)]
    expires_in_days: Option<i64>,
}

async fn list_api_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Vec<ApiTokenView>>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let rows = sqlx::query(
        "SELECT t.id, t.label, t.token_prefix, t.last_used_at, t.expires_at, t.created_at,
                COALESCE(SUM(u.total_tokens), 0)::BIGINT AS total_tokens,
                COUNT(u.id)::BIGINT AS request_count
         FROM cloud_api_tokens t
         LEFT JOIN cloud_api_usage u ON u.token_id = t.id
         WHERE t.user_id = $1 AND t.revoked_at IS NULL
         GROUP BY t.id
         ORDER BY t.created_at DESC",
    )
    .bind(user.user_id)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(
        rows.iter()
            .map(|row| ApiTokenView {
                id: row.get("id"),
                label: row.get("label"),
                token_prefix: row.get("token_prefix"),
                last_used_at: row.get("last_used_at"),
                expires_at: row.get("expires_at"),
                created_at: row.get("created_at"),
                total_tokens: row.get("total_tokens"),
                request_count: row.get("request_count"),
            })
            .collect(),
    ))
}

async fn create_api_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateApiTokenRequest>,
) -> CloudResult<(StatusCode, Json<ApiTokenCreated>)> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let label = request.label.trim();
    if label.is_empty() || label.chars().count() > 48 {
        return Err(CloudError::bad_request("Token 名称需要为 1-48 个字符"));
    }
    let expires_at = request
        .expires_in_days
        .map(|days| Utc::now() + Duration::days(days.clamp(1, 365)));
    let id = Uuid::new_v4();
    let token = random_token("sk-sc_");
    let token_prefix: String = token.chars().take(14).collect();
    let row = sqlx::query(
        "INSERT INTO cloud_api_tokens
         (id, user_id, label, token_prefix, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING created_at",
    )
    .bind(id)
    .bind(user.user_id)
    .bind(label)
    .bind(&token_prefix)
    .bind(sha256_hex(&token))
    .bind(expires_at)
    .fetch_one(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok((
        StatusCode::CREATED,
        Json(ApiTokenCreated {
            token,
            item: ApiTokenView {
                id,
                label: label.to_string(),
                token_prefix,
                last_used_at: None,
                expires_at,
                created_at: row.get("created_at"),
                total_tokens: 0,
                request_count: 0,
            },
        }),
    ))
}

async fn revoke_api_token(
    Path(token_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<StatusCode> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let result = sqlx::query(
        "UPDATE cloud_api_tokens SET revoked_at = NOW()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(token_id)
    .bind(user.user_id)
    .execute(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    if result.rows_affected() == 0 {
        return Err(CloudError::new(
            StatusCode::NOT_FOUND,
            "token_not_found",
            "Token 不存在或已经撤销",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct UsageQuery {
    days: Option<i32>,
}

#[derive(Serialize)]
struct UsageDay {
    day: NaiveDate,
    requests: i64,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
}

#[derive(Serialize)]
struct UsageSummary {
    days: i32,
    requests: i64,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    daily: Vec<UsageDay>,
}

async fn get_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UsageQuery>,
) -> CloudResult<Json<UsageSummary>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    let days = query.days.unwrap_or(30).clamp(1, 365);
    let summary = sqlx::query(
        "SELECT COUNT(*)::BIGINT AS requests,
                COALESCE(SUM(prompt_tokens), 0)::BIGINT AS prompt_tokens,
                COALESCE(SUM(completion_tokens), 0)::BIGINT AS completion_tokens,
                COALESCE(SUM(total_tokens), 0)::BIGINT AS total_tokens
         FROM cloud_api_usage
         WHERE user_id = $1 AND created_at >= NOW() - ($2 * INTERVAL '1 day')",
    )
    .bind(user.user_id)
    .bind(days)
    .fetch_one(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    let rows = sqlx::query(
        "SELECT created_at::date AS day, COUNT(*)::BIGINT AS requests,
                COALESCE(SUM(prompt_tokens), 0)::BIGINT AS prompt_tokens,
                COALESCE(SUM(completion_tokens), 0)::BIGINT AS completion_tokens,
                COALESCE(SUM(total_tokens), 0)::BIGINT AS total_tokens
         FROM cloud_api_usage
         WHERE user_id = $1 AND created_at >= NOW() - ($2 * INTERVAL '1 day')
         GROUP BY created_at::date ORDER BY day",
    )
    .bind(user.user_id)
    .bind(days)
    .fetch_all(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(UsageSummary {
        days,
        requests: summary.get("requests"),
        prompt_tokens: summary.get("prompt_tokens"),
        completion_tokens: summary.get("completion_tokens"),
        total_tokens: summary.get("total_tokens"),
        daily: rows
            .iter()
            .map(|row| UsageDay {
                day: row.get("day"),
                requests: row.get("requests"),
                prompt_tokens: row.get("prompt_tokens"),
                completion_tokens: row.get("completion_tokens"),
                total_tokens: row.get("total_tokens"),
            })
            .collect(),
    }))
}

fn require_admin(user: &AuthUser) -> CloudResult<()> {
    if user.role != "admin" {
        return Err(CloudError::new(
            StatusCode::FORBIDDEN,
            "admin_required",
            "仅 Sculk Cloud 管理员可以配置中转上游",
        ));
    }
    Ok(())
}

fn encrypt_api_key(master_key: &[u8; 32], value: &str) -> CloudResult<(Vec<u8>, Vec<u8>)> {
    let cipher = Aes256Gcm::new_from_slice(master_key).map_err(|_| {
        CloudError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "encryption_error",
            "无法初始化密钥加密器",
        )
    })?;
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), value.as_bytes())
        .map_err(|_| {
            CloudError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "encryption_error",
                "上游密钥加密失败",
            )
        })?;
    Ok((encrypted, nonce.to_vec()))
}

fn decrypt_api_key(master_key: &[u8; 32], encrypted: &[u8], nonce: &[u8]) -> CloudResult<String> {
    if nonce.len() != 12 {
        return Err(CloudError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "encryption_error",
            "上游密钥记录无效",
        ));
    }
    let cipher = Aes256Gcm::new_from_slice(master_key).map_err(|_| {
        CloudError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "encryption_error",
            "无法初始化密钥加密器",
        )
    })?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), encrypted)
        .map_err(|_| {
            CloudError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "encryption_error",
                "上游密钥解密失败，请由管理员重新保存配置",
            )
        })?;
    String::from_utf8(plain).map_err(|_| {
        CloudError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "encryption_error",
            "上游密钥编码无效",
        )
    })
}

#[derive(Serialize)]
struct ProviderView {
    configured: bool,
    name: String,
    base_url: String,
    api_key_masked: String,
    default_model: String,
    enabled: bool,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct UpdateProviderRequest {
    name: String,
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    default_model: String,
    enabled: bool,
}

async fn get_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<ProviderView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    require_admin(&user)?;
    let row = sqlx::query(
        "SELECT name, base_url, default_model, enabled, updated_at FROM cloud_relay_provider WHERE singleton = TRUE",
    )
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(match row {
        Some(row) => ProviderView {
            configured: true,
            name: row.get("name"),
            base_url: row.get("base_url"),
            api_key_masked: "sk-••••••••••••".into(),
            default_model: row.get("default_model"),
            enabled: row.get("enabled"),
            updated_at: Some(row.get("updated_at")),
        },
        None => ProviderView {
            configured: false,
            name: String::new(),
            base_url: String::new(),
            api_key_masked: String::new(),
            default_model: String::new(),
            enabled: false,
            updated_at: None,
        },
    }))
}

async fn update_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdateProviderRequest>,
) -> CloudResult<Json<ProviderView>> {
    let cloud = cloud(&state)?;
    let user = authenticate(&headers, &cloud).await?;
    require_admin(&user)?;
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(CloudError::bad_request("上游名称需要为 1-64 个字符"));
    }
    let base_url = request.base_url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(base_url)
        .map_err(|_| CloudError::bad_request("上游地址不是有效 URL"))?;
    if !["http", "https"].contains(&parsed.scheme()) {
        return Err(CloudError::bad_request("上游地址仅支持 HTTP 或 HTTPS"));
    }
    let existing = sqlx::query(
        "SELECT api_key_cipher, api_key_nonce FROM cloud_relay_provider WHERE singleton = TRUE",
    )
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    let (encrypted, nonce) = if request.api_key.trim().is_empty() {
        let row =
            existing.ok_or_else(|| CloudError::bad_request("首次配置必须填写上游 API Key"))?;
        (row.get("api_key_cipher"), row.get("api_key_nonce"))
    } else {
        encrypt_api_key(cloud.master_key.as_ref(), request.api_key.trim())?
    };
    let row = sqlx::query(
        "INSERT INTO cloud_relay_provider
         (singleton, name, base_url, api_key_cipher, api_key_nonce, default_model, enabled, updated_by)
         VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (singleton) DO UPDATE SET
           name = EXCLUDED.name, base_url = EXCLUDED.base_url,
           api_key_cipher = EXCLUDED.api_key_cipher, api_key_nonce = EXCLUDED.api_key_nonce,
           default_model = EXCLUDED.default_model, enabled = EXCLUDED.enabled,
           updated_by = EXCLUDED.updated_by, updated_at = NOW()
         RETURNING updated_at",
    )
    .bind(name)
    .bind(base_url)
    .bind(encrypted)
    .bind(nonce)
    .bind(request.default_model.trim())
    .bind(request.enabled)
    .bind(user.user_id)
    .fetch_one(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    Ok(Json(ProviderView {
        configured: true,
        name: name.to_string(),
        base_url: base_url.to_string(),
        api_key_masked: "sk-••••••••••••".into(),
        default_model: request.default_model.trim().to_string(),
        enabled: request.enabled,
        updated_at: Some(row.get("updated_at")),
    }))
}

struct RelayToken {
    id: Uuid,
    user_id: Uuid,
}

async fn authenticate_api_token(
    headers: &HeaderMap,
    cloud: &CloudState,
) -> CloudResult<RelayToken> {
    let token = bearer(headers)?;
    if !token.starts_with("sk-sc_") {
        return Err(CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_api_token",
            "API Token 无效",
        ));
    }
    let row = sqlx::query(
        "SELECT id, user_id FROM cloud_api_tokens
         WHERE token_hash = $1 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > NOW())",
    )
    .bind(sha256_hex(token))
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_api_token",
            "API Token 无效、已过期或已经撤销",
        )
    })?;
    Ok(RelayToken {
        id: row.get("id"),
        user_id: row.get("user_id"),
    })
}

async fn relay_chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut request): Json<Value>,
) -> CloudResult<Response> {
    let cloud = cloud(&state)?;
    let token = authenticate_api_token(&headers, &cloud).await?;
    if request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(CloudError::bad_request(
            "当前中转端点暂不支持 stream=true，请使用非流式响应",
        ));
    }

    let minute = Utc::now().timestamp() / 60;
    let rate_key = format!("sculk:relay:rate:{}:{minute}", token.id);
    let mut redis = cloud.redis.clone();
    let count: i64 = redis.incr(&rate_key, 1).await.map_err(|_| {
        CloudError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "rate_limit_unavailable",
            "限流服务暂时不可用",
        )
    })?;
    if count == 1 {
        let _: Result<(), _> = redis.expire(&rate_key, 90).await;
    }
    if count > cloud.rate_limit {
        return Err(CloudError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
            format!("每分钟最多调用 {} 次，请稍后重试", cloud.rate_limit),
        ));
    }

    let provider = sqlx::query(
        "SELECT base_url, api_key_cipher, api_key_nonce, default_model
         FROM cloud_relay_provider WHERE singleton = TRUE AND enabled = TRUE",
    )
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "relay_not_configured",
            "管理员尚未启用 API 中转上游",
        )
    })?;
    if request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .is_empty()
    {
        let default_model: String = provider.get("default_model");
        if default_model.is_empty() {
            return Err(CloudError::bad_request(
                "请求缺少 model，且管理员未配置默认模型",
            ));
        }
        request["model"] = Value::String(default_model);
    }
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let base_url: String = provider.get("base_url");
    let versioned_base = if base_url.ends_with("/v1") {
        base_url
    } else {
        format!("{base_url}/v1")
    };
    let api_key = decrypt_api_key(
        cloud.master_key.as_ref(),
        provider.get::<Vec<u8>, _>("api_key_cipher").as_slice(),
        provider.get::<Vec<u8>, _>("api_key_nonce").as_slice(),
    )?;
    let started = Instant::now();
    let upstream = cloud
        .http
        .post(format!("{versioned_base}/chat/completions"))
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .await
        .map_err(|error| {
            CloudError::new(
                StatusCode::BAD_GATEWAY,
                "upstream_unavailable",
                format!("无法连接中转上游：{error}"),
            )
        })?;
    let status = upstream.status();
    let bytes = upstream.bytes().await.map_err(|error| {
        CloudError::new(
            StatusCode::BAD_GATEWAY,
            "upstream_response_error",
            format!("读取上游响应失败：{error}"),
        )
    })?;
    let payload: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let prompt_tokens = payload
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .clamp(0, i32::MAX as i64) as i32;
    let completion_tokens = payload
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .clamp(0, i32::MAX as i64) as i32;
    let total_tokens = payload
        .pointer("/usage/total_tokens")
        .and_then(Value::as_i64)
        .unwrap_or((prompt_tokens + completion_tokens) as i64)
        .clamp(0, i32::MAX as i64) as i32;
    let latency_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    let _ = sqlx::query(
        "INSERT INTO cloud_api_usage
         (id, token_id, user_id, endpoint, model, prompt_tokens, completion_tokens, total_tokens, status_code, latency_ms)
         VALUES ($1, $2, $3, '/v1/chat/completions', $4, $5, $6, $7, $8, $9)",
    )
    .bind(Uuid::new_v4())
    .bind(token.id)
    .bind(token.user_id)
    .bind(model)
    .bind(prompt_tokens)
    .bind(completion_tokens)
    .bind(total_tokens)
    .bind(status.as_u16() as i32)
    .bind(latency_ms)
    .execute(&cloud.db)
    .await;
    let _ = sqlx::query("UPDATE cloud_api_tokens SET last_used_at = NOW() WHERE id = $1")
        .bind(token.id)
        .execute(&cloud.db)
        .await;

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(bytes))
        .map_err(|_| {
            CloudError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "response_error",
                "无法构造中转响应",
            )
        })
}

#[derive(Serialize)]
struct DeploymentCapability {
    available: bool,
    status: &'static str,
    api_version: &'static str,
    reserved_endpoints: [&'static str; 3],
}

async fn deployment_capability() -> Json<DeploymentCapability> {
    Json(DeploymentCapability {
        available: false,
        status: "planned",
        api_version: "2026-07-preview",
        reserved_endpoints: [
            "GET /api/cloud/deployments",
            "POST /api/cloud/deployments",
            "GET /api/cloud/deployments/{id}",
        ],
    })
}

async fn list_deployments(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Json<Value>> {
    let cloud = cloud(&state)?;
    let _ = authenticate(&headers, &cloud).await?;
    Ok(Json(
        json!({ "available": false, "items": [], "status": "planned" }),
    ))
}

async fn create_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> CloudResult<Response> {
    let cloud = cloud(&state)?;
    let _ = authenticate(&headers, &cloud).await?;
    Err(CloudError::new(
        StatusCode::NOT_IMPLEMENTED,
        "deployment_planned",
        "开服器云部署尚未开放，接口已预留",
    ))
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/cloud/status", get(status))
        .route("/api/cloud/auth/register", post(register))
        .route("/api/cloud/auth/login", post(login))
        .route("/api/cloud/auth/logout", post(logout))
        .route("/api/cloud/me", get(me).patch(update_profile))
        .route("/api/cloud/devices", get(list_devices))
        .route(
            "/api/cloud/devices/{id}",
            axum::routing::delete(revoke_device),
        )
        .route("/api/cloud/agent-pairings", post(create_agent_pairing))
        .route("/api/cloud/agent-bootstrap", post(create_agent_bootstrap))
        .route("/api/cloud/agent-pairings/claim", post(claim_agent_pairing))
        .route(
            "/api/cloud/agent-pairings/{id}/confirm",
            post(confirm_agent_pairing),
        )
        .route("/api/cloud/agents", get(list_agents))
        .route("/api/cloud/agents/{id}/confirm", post(confirm_agent))
        .route(
            "/api/cloud/agents/{id}",
            axum::routing::delete(revoke_agent),
        )
        .route(
            "/api/cloud/agent-tasks",
            get(list_agent_tasks).post(create_agent_task),
        )
        .route("/api/cloud/agent-tasks/{id}", get(get_agent_task))
        .route(
            "/api/cloud/agent-tasks/{id}/approve",
            post(approve_agent_task),
        )
        .route(
            "/api/cloud/agent-tasks/{id}/cancel",
            post(cancel_agent_task),
        )
        .route(
            "/api/cloud/agent-tasks/{id}/rollback",
            post(rollback_agent_task),
        )
        .route("/api/cloud/agent-tasks/{id}/retry", post(retry_agent_task))
        .route("/api/cloud/agent/heartbeat", post(agent_heartbeat))
        .route("/api/cloud/agent/tasks/lease", post(lease_agent_task))
        .route("/api/cloud/agent/tasks/{id}/start", post(start_agent_task))
        .route(
            "/api/cloud/agent/tasks/{id}/control",
            post(control_agent_task),
        )
        .route(
            "/api/cloud/agent/tasks/{id}/events",
            post(create_agent_task_event),
        )
        .route(
            "/api/cloud/agent/tasks/{id}/checkpoints",
            post(create_agent_task_checkpoint),
        )
        .route(
            "/api/cloud/agent/tasks/{id}/complete",
            post(complete_agent_task),
        )
        .route(
            "/api/cloud/terminal-sessions",
            get(list_terminal_sessions).post(create_terminal_session),
        )
        .route(
            "/api/cloud/terminal-sessions/{id}/approve",
            post(approve_terminal_session),
        )
        .route(
            "/api/cloud/terminal-sessions/{id}/input",
            post(terminal_input),
        )
        .route(
            "/api/cloud/terminal-sessions/{id}/resize",
            post(terminal_resize),
        )
        .route(
            "/api/cloud/terminal-sessions/{id}/terminate",
            post(terminate_terminal_session),
        )
        .route(
            "/api/cloud/terminal-sessions/{id}/events",
            get(get_terminal_events),
        )
        .route(
            "/api/cloud/agent/terminals/commands",
            post(lease_terminal_commands),
        )
        .route(
            "/api/cloud/agent/terminals/{id}/events",
            post(create_terminal_events),
        )
        .route(
            "/api/cloud/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route("/api/cloud/conversations/{id}", get(get_conversation))
        .route(
            "/api/cloud/conversations/{id}/messages",
            post(create_conversation_message),
        )
        .route(
            "/api/cloud/conversations/{id}/plans",
            post(create_conversation_plan),
        )
        .route(
            "/api/cloud/sync/settings",
            get(get_synced_settings).put(put_synced_settings),
        )
        .route(
            "/api/cloud/credentials",
            get(list_user_credentials).post(save_user_credential),
        )
        .route(
            "/api/cloud/credentials/{id}",
            axum::routing::delete(delete_user_credential),
        )
        .route("/api/cloud/teams", get(list_teams).post(create_team))
        .route("/api/cloud/teams/{id}/members", get(list_team_members))
        .route("/api/cloud/teams/{id}/invitations", post(invite_member))
        .route("/api/cloud/invitations/accept", post(accept_invitation))
        .route(
            "/api/cloud/approvals",
            get(list_approvals).post(create_approval),
        )
        .route("/api/cloud/approvals/{id}/decision", post(decide_approval))
        .route(
            "/api/cloud/tokens",
            get(list_api_tokens).post(create_api_token),
        )
        .route(
            "/api/cloud/tokens/{id}",
            axum::routing::delete(revoke_api_token),
        )
        .route("/api/cloud/usage", get(get_usage))
        .route(
            "/api/cloud/admin/relay-provider",
            get(get_provider).put(update_provider),
        )
        .route(
            "/api/cloud/v1/chat/completions",
            post(relay_chat_completions),
        )
        .route(
            "/api/cloud/deployments/capability",
            get(deployment_capability),
        )
        .route(
            "/api/cloud/deployments",
            get(list_deployments).post(create_deployment),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_account_fields() {
        assert!(validate_email("owner@example.com").is_ok());
        assert!(validate_email("broken@example").is_err());
        assert!(validate_password("long-enough").is_ok());
        assert!(validate_password("short").is_err());
    }

    #[test]
    fn master_secret_requires_a_non_placeholder_value() {
        assert!(validate_master_secret("").is_err());
        assert!(validate_master_secret("too-short").is_err());
        assert!(validate_master_secret("replace-with-a-long-random-production-secret").is_err());
        assert!(validate_master_secret("replace_with_a_long_random_production_secret").is_err());
        assert!(validate_master_secret("change-me-this-is-not-a-real-secret").is_err());
        assert!(validate_master_secret("example-secret-that-is-long-enough").is_err());
        assert_eq!(
            validate_master_secret("  a-unique-production-secret-with-32-bytes  ").unwrap(),
            "a-unique-production-secret-with-32-bytes"
        );
    }

    #[test]
    fn hashes_tokens_without_leaking_the_original() {
        let token = "sk-sc_example";
        let hash = sha256_hex(token);
        assert_eq!(hash.len(), 64);
        assert!(!hash.contains(token));
    }

    #[test]
    fn encrypts_and_decrypts_provider_keys() {
        let key = [7_u8; 32];
        let (encrypted, nonce) = encrypt_api_key(&key, "upstream-secret").unwrap();
        assert_ne!(encrypted, b"upstream-secret");
        assert_eq!(
            decrypt_api_key(&key, &encrypted, &nonce).unwrap(),
            "upstream-secret"
        );
    }

    #[test]
    fn accepts_workspace_sync_without_secrets() {
        let payload = json!({
            "schema_version": 2,
            "ui": { "language": "zh-CN" },
            "prompts": [{ "id": "repair", "title": "修复", "content": "分析报错" }],
            "skill_links": [{ "id": "docs", "name": "文档", "url": "https://example.com" }]
        });
        assert!(validate_sync_payload(&payload).is_ok());
    }

    #[test]
    fn rejects_nested_secrets_from_generic_sync() {
        let payload = json!({ "settings": { "provider": { "api_key": "sk-secret" } } });
        assert!(validate_sync_payload(&payload).is_err());
    }

    #[test]
    fn rejects_runtime_and_authentication_keys_from_generic_sync() {
        for key in [
            "token",
            "Authorization",
            "cookie",
            "session",
            "client_secret",
            "rcon-password",
            "command",
            "args",
            "path",
            "root_path",
            "host",
        ] {
            let payload = json!({ "portable": { (key): "unsafe" } });
            assert!(
                validate_sync_payload(&payload).is_err(),
                "accepted high-risk sync key {key}"
            );
        }
    }

    #[test]
    fn generic_sync_still_accepts_safe_urls_and_agent_metadata() {
        let payload = json!({
            "skill_links": [{ "url": "https://example.com/skill" }],
            "provider": { "base_url": "https://api.example.com/v1" },
            "agent": {
                "workspace_label": "production",
                "capabilities": ["heartbeat"],
                "permissions": ["read"]
            }
        });
        assert!(validate_sync_payload(&payload).is_ok());
    }

    #[test]
    fn agent_permissions_are_strictly_allowlisted() {
        assert_eq!(
            validate_permissions(&["read".into(), "process".into(), "full".into()]).unwrap(),
            vec!["read", "process", "full"]
        );
        assert!(validate_permissions(&["shell".into()]).is_err());
        assert!(validate_permissions(&["read".into(), "READ".into()]).is_err());
    }

    #[test]
    fn agent_claim_fields_and_capabilities_are_bounded() {
        let request = ClaimAgentRequest {
            pairing_code: "scp_example".into(),
            name: "survival-host".into(),
            platform: "linux-x86_64".into(),
            version: "0.1.0".into(),
            workspace_label: "production".into(),
            capabilities: vec!["heartbeat".into(), "server.status".into()],
            permissions: vec!["read".into()],
            fingerprint: "a1b2c3d4e5f6".into(),
        };
        let validated = validate_agent_claim(&request).unwrap();
        assert_eq!(validated.workspace_label, "production");
        assert!(
            validate_capabilities(&["bad capability".into()]).is_err(),
            "capabilities must use stable identifier characters"
        );
        assert!(bounded_agent_text(&"x".repeat(129), "workspace_label", 1, 128).is_err());
    }

    #[test]
    fn bootstrap_is_validated_and_uses_the_approved_agent_defaults() {
        let request = CreateAgentBootstrapRequest {
            platform: "windows-x86_64".into(),
            name: "survival-host".into(),
            workspace_label: "production".into(),
            workspace_root: "D:\\minecraft".into(),
        };
        let validated = validate_agent_bootstrap(&request).unwrap();
        assert_eq!(validated.platform, "windows-x86_64");
        assert_eq!(validated.workspace_root, "D:\\minecraft");
        assert_eq!(
            AGENT_BOOTSTRAP_CAPABILITIES,
            [
                "heartbeat",
                "tasks-v1",
                "shell-v1",
                "terminal-v1",
                "task-checkpoints-v1",
                "mcp-v1"
            ]
        );
        assert_eq!(
            AGENT_BOOTSTRAP_PERMISSIONS,
            ["read", "write", "process", "full"]
        );

        let invalid = CreateAgentBootstrapRequest {
            platform: "windows x86_64".into(),
            ..request
        };
        assert!(validate_agent_bootstrap(&invalid).is_err());
    }

    #[test]
    fn bootstrap_json_contains_only_pairing_material_and_agent_metadata() {
        let bootstrap = AgentBootstrap {
            schema_version: 1,
            permissions_granted_by_current_user: true,
            pairing_id: Uuid::nil(),
            pairing_code: "scp_temporary-code".into(),
            expires_at: Utc::now() + Duration::minutes(AGENT_PAIRING_MINUTES),
            cloud_url: "https://cloud.example.com".into(),
            platform: "linux-x86_64".into(),
            name: "survival-host".into(),
            workspace_label: "production".into(),
            workspace_root: "/srv/minecraft".into(),
            capabilities: AGENT_BOOTSTRAP_CAPABILITIES
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            permissions: AGENT_BOOTSTRAP_PERMISSIONS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        };
        let value = serde_json::to_value(bootstrap).unwrap();
        let object = value.as_object().unwrap();
        assert!(object.contains_key("pairing_code"));
        assert_eq!(
            object["permissions_granted_by_current_user"],
            Value::Bool(true)
        );
        assert!(!object.contains_key("token"));
        assert!(!object.contains_key("password"));
        assert!(!object.contains_key("session"));
    }

    #[test]
    fn bootstrap_cloud_url_is_https_or_loopback_http_without_credentials() {
        assert_eq!(
            normalize_bootstrap_cloud_url("https://cloud.example.com/api/").unwrap(),
            "https://cloud.example.com/api"
        );
        assert!(normalize_bootstrap_cloud_url("http://cloud.example.com").is_err());
        assert!(normalize_bootstrap_cloud_url("https://user:pass@cloud.example.com").is_err());
        assert!(normalize_bootstrap_cloud_url("https://cloud.example.com/?a=b").is_err());
        assert_eq!(
            normalize_bootstrap_cloud_url("http://127.0.0.1:8787/").unwrap(),
            "http://127.0.0.1:8787"
        );
    }

    #[test]
    fn full_agent_permission_includes_structured_operations() {
        let full = vec!["full".to_string()];
        assert!(agent_permissions_allow(&full, "read"));
        assert!(agent_permissions_allow(&full, "write"));
        assert!(agent_permissions_allow(&full, "full"));
        assert!(!agent_permissions_allow(&["read".into()], "write"));
    }

    #[test]
    fn pairing_expiration_uses_a_closed_deadline() {
        let now = Utc::now();
        assert!(pairing_is_expired(now, now));
        assert!(pairing_is_expired(now - Duration::seconds(1), now));
        assert!(!pairing_is_expired(now + Duration::seconds(1), now));
    }

    #[test]
    fn agent_token_prefix_and_online_window_are_enforced() {
        assert!(valid_agent_token("sca_abcdefghijklmnopqrstuvwxyz"));
        assert!(!valid_agent_token("scs_abcdefghijklmnopqrstuvwxyz"));
        let now = Utc::now();
        assert!(agent_is_online("active", Some(now), now));
        assert!(!agent_is_online("claimed", Some(now), now));
        assert!(!agent_is_online(
            "active",
            Some(now - Duration::seconds(AGENT_ONLINE_SECONDS + 1)),
            now
        ));
    }

    #[test]
    fn agent_task_operations_encode_permission_risk_and_approval() {
        let read = agent_task_operation("host.inspect", false).unwrap();
        assert_eq!(read.permission, "read");
        assert_eq!(read.risk, "low");
        assert!(!read.approval_required);

        let shell = agent_task_operation("shell.exec", false).unwrap();
        assert_eq!(shell.permission, "full");
        assert_eq!(shell.risk, "critical");
        assert_eq!(shell.additional_capability, Some("shell-v1"));
        assert!(shell.approval_required);

        let mcp_read = agent_task_operation("platform.mcp.read", false).unwrap();
        assert_eq!(mcp_read.permission, "read");
        assert_eq!(mcp_read.risk, "low");
        assert_eq!(mcp_read.additional_capability, Some("mcp-v1"));
        assert!(!mcp_read.approval_required);

        let mcp_reply = agent_task_operation("platform.mcp.reply", false).unwrap();
        assert_eq!(mcp_reply.permission, "write");
        assert_eq!(mcp_reply.risk, "high");
        assert_eq!(mcp_reply.additional_capability, Some("mcp-v1"));
        assert!(mcp_reply.approval_required);

        assert!(agent_task_operation("task.rollback", false).is_err());
        assert!(agent_task_operation("shell.raw", false).is_err());
    }

    #[test]
    fn platform_mcp_task_input_is_strict_and_separates_reply_risk() {
        let read = json!({
            "server": "douyin",
            "tool": "list_comments",
            "arguments": {
                "video_id": "item-1",
                "limit": 20
            }
        });
        assert!(validate_agent_task_input("platform.mcp.read", &read, false).is_ok());
        assert!(validate_agent_task_input("platform.mcp.reply", &read, false).is_err());

        let reply = json!({
            "server": "bilibili",
            "tool": "reply_comment",
            "arguments": {
                "video_id": "video-1",
                "comment_id": "comment-1",
                "content": "测试回复",
                "dry_run": true
            }
        });
        assert!(validate_agent_task_input("platform.mcp.reply", &reply, false).is_ok());
        assert!(validate_agent_task_input("platform.mcp.read", &reply, false).is_err());
    }

    #[test]
    fn shell_task_input_is_structurally_strict_but_command_agnostic() {
        let input = json!({
            "command": "if ($env:JAVA_HOME) { java -version } && echo '任意命令'",
            "cwd": "C:\\servers\\survival",
            "timeout_seconds": 1800
        });
        assert!(validate_agent_task_input("shell.exec", &input, false).is_ok());
        assert!(
            validate_agent_task_input(
                "shell.exec",
                &json!({ "command": "echo ok", "env": { "TOKEN": "unsafe" } }),
                false
            )
            .is_err()
        );
        assert!(
            validate_agent_task_input(
                "shell.exec",
                &json!({ "command": "echo ok", "shell": "pwsh" }),
                false
            )
            .is_err()
        );
        assert!(
            validate_agent_task_input(
                "shell.exec",
                &json!({ "command": "x", "timeout_seconds": 1801 }),
                false
            )
            .is_err()
        );
        assert!(validate_agent_task_input("shell.exec", &json!({ "command": "" }), false).is_err());
    }

    #[test]
    fn structured_task_paths_and_limits_reject_escape_attempts() {
        assert!(
            validate_agent_task_input(
                "workspace.list",
                &json!({ "path": ".", "max_entries": 500 }),
                false
            )
            .is_ok()
        );
        for path in ["../secret", "/etc", "C:\\Windows", "servers//world"] {
            assert!(
                validate_agent_task_input(
                    "workspace.list",
                    &json!({ "path": path, "max_entries": 10 }),
                    false
                )
                .is_err(),
                "accepted unsafe path {path}"
            );
        }
        assert!(
            validate_agent_task_input(
                "log.tail",
                &json!({ "path": "logs/latest.log", "lines": 1001, "max_bytes": 1024 }),
                false
            )
            .is_err()
        );
        for path in [
            ".env",
            "database.db",
            "logs/.env",
            "logs/world.sqlite3",
            "server.log",
        ] {
            assert!(
                validate_agent_task_input(
                    "log.tail",
                    &json!({ "path": path, "lines": 10, "max_bytes": 1024 }),
                    false
                )
                .is_err(),
                "accepted unsafe log path {path}"
            );
        }
        for path in ["logs/latest.log", "crash-reports/crash-1.log"] {
            assert!(
                validate_agent_task_input(
                    "log.tail",
                    &json!({ "path": path, "lines": 10, "max_bytes": 1024 }),
                    false
                )
                .is_ok(),
                "rejected allowed log path {path}"
            );
        }
    }

    #[test]
    fn server_properties_updates_are_allowlisted_and_typed() {
        assert!(
            validate_agent_task_input(
                "server.properties.update",
                &json!({
                    "path": "servers/survival/server.properties",
                    "changes": {
                        "motd": "Welcome",
                        "max-players": 60,
                        "difficulty": "hard",
                        "pvp": true,
                        "view-distance": 12
                    }
                }),
                false
            )
            .is_ok()
        );
        assert!(
            validate_agent_task_input(
                "server.properties.update",
                &json!({
                    "path": "servers/survival/server.properties",
                    "changes": { "online-mode": false }
                }),
                false
            )
            .is_err()
        );
        assert!(
            validate_agent_task_input(
                "server.properties.update",
                &json!({
                    "path": "servers/survival/other.properties",
                    "changes": { "pvp": true }
                }),
                false
            )
            .is_err()
        );
    }

    #[test]
    fn task_events_outputs_and_artifacts_are_bounded() {
        assert!(validate_task_event("info", "started", &Some(json!({ "percent": 20 }))).is_ok());
        assert!(validate_task_event("debug", "started", &None).is_err());
        assert!(validate_task_event("info", &"x".repeat(2001), &None).is_err());
        assert!(
            validate_task_event(
                "error",
                "Bearer sca_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG",
                &None
            )
            .is_err()
        );
        assert!(validate_task_event("info", "created directory sca_demo", &None).is_ok());
        assert!(
            validate_json_bounds(
                &json!({ "stdout": "x".repeat(AGENT_TASK_OUTPUT_BYTES) }),
                AGENT_TASK_OUTPUT_BYTES,
                8,
                4096
            )
            .is_err()
        );
        assert!(
            validate_task_artifacts(&Some(json!([{
                "name": "backup.zip",
                "path": "backups/backup.zip",
                "kind": "backup",
                "size_bytes": 1024
            }])))
            .is_ok()
        );
        assert!(
            validate_task_artifacts(&Some(json!([{
                "name": "external",
                "path": "backup.zip",
                "kind": "file",
                "url": "https://example.com/backup.zip"
            }])))
            .is_err()
        );
    }

    #[test]
    fn task_state_and_rollback_rules_are_closed() {
        assert!(agent_task_is_cancellable("awaiting_approval"));
        assert!(agent_task_is_cancellable("queued"));
        assert!(agent_task_is_cancellable("leased"));
        assert!(!agent_task_is_cancellable("running"));
        assert!(!agent_task_is_cancellable("succeeded"));
        assert!(
            validate_task_rollback_result("succeeded", "write", "server.properties.update", true)
                .is_ok()
        );
        assert!(validate_task_rollback_result("succeeded", "full", "shell.exec", true).is_err());
        assert!(
            validate_task_rollback_result("failed", "write", "workspace.create_directory", true)
                .is_err()
        );
        assert!(validate_task_rollback_result("succeeded", "read", "host.inspect", true).is_err());

        assert!(agent_task_supports_running_cancellation("shell.exec"));
        assert!(!agent_task_supports_running_cancellation("host.inspect"));
        assert!(validate_task_cancellation_completion("cancelled", false, true).is_ok());
        assert!(validate_task_cancellation_completion("cancelled", false, false).is_err());
        assert!(validate_task_cancellation_completion("cancelled", true, true).is_err());
        assert!(validate_task_cancellation_completion("succeeded", true, false).is_ok());
        assert!(validate_task_cancellation_completion("succeeded", false, true).is_err());
        assert!(validate_task_cancellation_completion("failed", false, true).is_err());
        assert!(agent_task_event_renews_lease(false));
        assert!(!agent_task_event_renews_lease(true));
        assert!(ensure_task_accepts_checkpoint(false).is_ok());
        assert!(ensure_task_accepts_checkpoint(true).is_err());
        let (expired_error, expired_event) = expired_running_task_outcome(true);
        assert!(expired_error.contains("not acknowledged"));
        assert!(expired_error.contains("result unknown"));
        assert!(expired_event.contains("not acknowledged"));
        assert!(expired_event.contains("result is unknown"));
        assert!(expired_leased_task_can_requeue("low", true));
        assert!(expired_leased_task_can_requeue("high", true));
        assert!(!expired_leased_task_can_requeue("high", false));
        assert!(!expired_leased_task_can_requeue("critical", false));
        assert!(validate_task_completion_values("cancelled", &None, &None, &None, true).is_ok());
        assert!(validate_task_completion_values("cancelled", &None, &None, &None, false).is_err());
    }

    #[test]
    fn task_checkpoints_apply_completion_security_and_resume_shape() {
        let result = json!({
            "status": "succeeded",
            "output": { "stdout": "done" },
            "error": null,
            "rollback_available": false,
            "artifacts": []
        });
        assert!(validate_task_checkpoint("result-v1", "result", true, &result).is_ok());
        assert!(
            validate_task_checkpoint(
                "failed-result-v1",
                "result",
                true,
                &json!({
                    "status": "failed",
                    "error": "execution failed",
                    "rollback_available": false
                })
            )
            .is_err()
        );
        assert!(
            validate_task_checkpoint(
                "result-v1",
                "result",
                true,
                &json!({
                    "status": "succeeded",
                    "output": { "token": "sca_abcdefghijklmnopqrstuvwxyz0123456789" },
                    "rollback_available": false
                })
            )
            .is_err()
        );
        assert!(
            validate_task_checkpoint("progress-v1", "progress", true, &json!({ "percent": 50 }))
                .is_err()
        );
        assert!(
            validate_task_checkpoint("progress-v1", "progress", false, &json!({ "percent": 50 }))
                .is_ok()
        );
        assert!(validate_task_checkpoint("bad key", "result", true, &result).is_err());
    }

    #[test]
    fn terminal_dimensions_and_payloads_are_bounded() {
        use base64::Engine;

        assert!(validate_terminal_dimensions(80, 24).is_ok());
        assert!(validate_terminal_dimensions(19, 24).is_err());
        assert!(validate_terminal_dimensions(80, 201).is_err());
        let maximum = base64::engine::general_purpose::STANDARD.encode(vec![b'x'; 8192]);
        assert_eq!(decode_terminal_base64(&maximum, 8192).unwrap().len(), 8192);
        let oversized = base64::engine::general_purpose::STANDARD.encode(vec![b'x'; 8193]);
        assert!(decode_terminal_base64(&oversized, 8192).is_err());
        assert!(decode_terminal_base64("not base64", 8192).is_err());
    }

    #[test]
    fn terminal_events_are_structurally_strict() {
        use base64::Engine;

        let output = AgentTerminalEventInput {
            seq: 1,
            kind: "output".into(),
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode("hello")),
            data: None,
        };
        assert_eq!(validate_terminal_event(&output).unwrap(), 5);
        assert!(
            validate_terminal_event(&AgentTerminalEventInput {
                seq: 2,
                kind: "exit".into(),
                data_base64: None,
                data: Some(json!({ "exit_code": 0 })),
            })
            .is_ok()
        );
        assert!(
            validate_terminal_event(&AgentTerminalEventInput {
                seq: 3,
                kind: "error".into(),
                data_base64: None,
                data: Some(
                    json!({ "message": "Bearer sca_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG" })
                ),
            })
            .is_err()
        );
        assert!(
            validate_terminal_event(&AgentTerminalEventInput {
                seq: 4,
                kind: "output".into(),
                data_base64: output.data_base64,
                data: Some(json!({})),
            })
            .is_err()
        );
        assert!(
            validate_terminal_event(&AgentTerminalEventInput {
                seq: 5,
                kind: "error".into(),
                data_base64: None,
                data: Some(json!({ "message": "password=hunter2" })),
            })
            .is_err()
        );
    }

    #[test]
    fn terminal_output_is_redacted_before_storage() {
        use base64::Engine;

        let mut output = AgentTerminalEventInput {
            seq: 1,
            kind: "output".into(),
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode(
                "password=hunter2 api_key: \"sk-test\" Bearer bearer-secret sca_abcdefghijklmnopqrstuvwxyz0123456789",
            )),
            data: None,
        };
        assert!(sanitize_terminal_output(&mut output, false).unwrap());
        let stored = String::from_utf8(
            decode_terminal_base64(output.data_base64.as_deref().unwrap(), 16 * 1024).unwrap(),
        )
        .unwrap();
        for secret in [
            "hunter2",
            "sk-test",
            "bearer-secret",
            "sca_abcdefghijklmnopqrstuvwxyz0123456789",
        ] {
            assert!(!stored.contains(secret), "leaked {secret}");
        }
        assert!(stored.contains("[REDACTED]"));
        assert_eq!(validate_terminal_event(&output).unwrap(), stored.len());
    }

    #[test]
    fn terminal_redaction_state_covers_split_credentials() {
        use base64::Engine;

        let mut key = AgentTerminalEventInput {
            seq: 1,
            kind: "output".into(),
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode("password=")),
            data: None,
        };
        let pending = sanitize_terminal_output(&mut key, false).unwrap();
        assert!(pending);
        let mut value = AgentTerminalEventInput {
            seq: 2,
            kind: "output".into(),
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode("hunter2\n")),
            data: None,
        };
        assert!(!sanitize_terminal_output(&mut value, pending).unwrap());
        let persisted = [key, value]
            .into_iter()
            .map(|event| {
                String::from_utf8(
                    decode_terminal_base64(event.data_base64.as_deref().unwrap(), 16 * 1024)
                        .unwrap(),
                )
                .unwrap()
            })
            .collect::<String>();
        assert!(!persisted.contains("hunter2"));
    }

    #[test]
    fn terminal_input_is_encrypted_and_keeps_idempotency_without_plaintext() {
        use base64::Engine;

        let key = [9_u8; 32];
        let input = base64::engine::general_purpose::STANDARD.encode("password=hunter2\n");
        let first = encrypt_terminal_input_payload(&key, &input).unwrap();
        assert!(first.get("data_base64").is_none());
        assert_ne!(first["ciphertext_base64"], input);
        assert_eq!(
            decrypt_terminal_input_payload(&key, &first).unwrap()["data_base64"],
            input
        );

        let retry = encrypt_terminal_input_payload(&key, &input).unwrap();
        assert!(terminal_input_payload_matches(&first, &retry));
        let different = encrypt_terminal_input_payload(
            &key,
            &base64::engine::general_purpose::STANDARD.encode("different"),
        )
        .unwrap();
        assert!(!terminal_input_payload_matches(&first, &different));
        let scrubbed = json!({
            "format": "redacted-v1",
            "input_mac": first["input_mac"].clone(),
        });
        assert!(terminal_input_payload_matches(&scrubbed, &retry));
    }

    #[test]
    fn conversation_fields_have_explicit_limits() {
        assert_eq!(validate_conversation_title(None).unwrap(), "新对话");
        assert_eq!(
            validate_conversation_content("  hello\nworld  ").unwrap(),
            "hello\nworld"
        );
        assert!(validate_conversation_content("").is_err());
        assert!(validate_conversation_content(&"x".repeat(20_001)).is_err());
        assert!(validate_conversation_content("secret\0value").is_err());
    }

    #[test]
    fn masks_credential_edges() {
        assert_eq!(
            key_edges("sk-example-secret"),
            ("sk-e".into(), "cret".into())
        );
        assert_eq!(key_edges("12345678"), ("12".into(), "78".into()));
    }
}
