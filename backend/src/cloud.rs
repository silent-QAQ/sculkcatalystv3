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
use rand::{RngCore, rngs::OsRng};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{env, sync::Arc, time::Instant};
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
    pub(crate) async fn from_env() -> Self {
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
        let master_secret = match env::var("SCULK_MASTER_KEY") {
            Ok(value) if value.len() >= 24 => value,
            _ => {
                return Self {
                    inner: None,
                    message: "SCULK_MASTER_KEY 至少需要 24 个字符".into(),
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
    features: [&'static str; 5],
}

async fn status(State(state): State<AppState>) -> Json<CloudStatus> {
    Json(CloudStatus {
        available: state.cloud.inner.is_some(),
        message: state.cloud.message.clone(),
        features: [
            "sync",
            "teams",
            "approvals",
            "api-relay",
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
    if !request.payload.is_object() {
        return Err(CloudError::bad_request("同步设置必须是 JSON 对象"));
    }
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
    team_name: String,
    requested_by: Uuid,
    requester_name: String,
    title: String,
    summary: String,
    risk: String,
    status: String,
    payload: Value,
    decision_comment: String,
    decided_by_name: Option<String>,
    decided_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

fn approval_from_row(row: &sqlx::postgres::PgRow) -> ApprovalView {
    ApprovalView {
        id: row.get("id"),
        team_id: row.get("team_id"),
        team_name: row.get("team_name"),
        requested_by: row.get("requested_by"),
        requester_name: row.get("requester_name"),
        title: row.get("title"),
        summary: row.get("summary"),
        risk: row.get("risk"),
        status: row.get("status"),
        payload: row.get("payload"),
        decision_comment: row.get("decision_comment"),
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

const APPROVAL_SELECT: &str = "SELECT a.id, a.team_id, t.name AS team_name, a.requested_by,
            requester.nickname AS requester_name, a.title, a.summary, a.risk, a.status,
            a.payload, a.decision_comment, decider.nickname AS decided_by_name,
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
    let team_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT team_id FROM cloud_approvals WHERE id = $1 AND status = 'pending'",
    )
    .bind(approval_id)
    .fetch_optional(&cloud.db)
    .await
    .map_err(CloudError::database)?
    .ok_or_else(|| {
        CloudError::new(
            StatusCode::CONFLICT,
            "approval_closed",
            "审批不存在或已经处理",
        )
    })?;
    require_team_role(
        &cloud,
        team_id,
        user.user_id,
        &["owner", "admin", "approver"],
    )
    .await?;
    let result = sqlx::query(
        "UPDATE cloud_approvals
         SET status = $2, decision_comment = $3, decided_by = $4, decided_at = NOW()
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(approval_id)
    .bind(&request.decision)
    .bind(request.comment.trim())
    .bind(user.user_id)
    .execute(&cloud.db)
    .await
    .map_err(CloudError::database)?;
    if result.rows_affected() == 0 {
        return Err(CloudError::new(
            StatusCode::CONFLICT,
            "approval_closed",
            "该审批刚刚被其他成员处理",
        ));
    }
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
        .route(
            "/api/cloud/sync/settings",
            get(get_synced_settings).put(put_synced_settings),
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
}
