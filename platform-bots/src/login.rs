use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rand::RngCore;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{net::TcpListener, sync::Mutex};
use url::form_urlencoded::Serializer;

use super::Platform;

#[derive(Clone)]
struct LoginConfig {
    platform: Platform,
    auth_url: Option<String>,
    token_url: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    redirect_uri: String,
    scopes: String,
    port: u16,
}

impl LoginConfig {
    fn from_env(platform: Platform) -> Self {
        let prefix = platform.env_prefix();
        let read = |suffix: &str| {
            env::var(format!("{prefix}_{suffix}"))
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let port = read("LOGIN_PORT")
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| (1024..=65535).contains(value))
            .unwrap_or(match platform {
                Platform::Douyin => 18432,
                Platform::Bilibili => 18433,
            });
        let default_auth_url = match platform {
            Platform::Douyin => Some("https://open.douyin.com/platform/oauth/connect/".into()),
            Platform::Bilibili => None,
        };
        let default_token_url = match platform {
            Platform::Douyin => Some("https://open.douyin.com/oauth/access_token/".into()),
            Platform::Bilibili => None,
        };
        Self {
            platform,
            auth_url: read("AUTH_URL").or(default_auth_url),
            token_url: read("TOKEN_URL").or(default_token_url),
            client_id: read("CLIENT_KEY").or_else(|| read("CLIENT_ID")),
            client_secret: read("CLIENT_SECRET"),
            redirect_uri: read("REDIRECT_URI")
                .unwrap_or_else(|| format!("http://127.0.0.1:{port}/oauth/callback")),
            scopes: read("SCOPES").unwrap_or_else(|| {
                if matches!(platform, Platform::Douyin) {
                    "item.comment".into()
                } else {
                    String::new()
                }
            }),
            port,
        }
    }

    fn ready(&self) -> bool {
        self.auth_url.is_some()
            && self.token_url.is_some()
            && self.client_id.is_some()
            && self.client_secret.is_some()
    }

    fn auth_url(&self, state: &str) -> Result<String, String> {
        let base = self
            .auth_url
            .as_deref()
            .ok_or_else(|| "未配置 OAuth 授权地址".to_string())?;
        let client_id = self
            .client_id
            .as_deref()
            .ok_or_else(|| "未配置平台 ClientKey/ClientID".to_string())?;
        let mut query = Serializer::new(String::new());
        if matches!(self.platform, Platform::Douyin) {
            query.append_pair("client_key", client_id);
        } else {
            query.append_pair("client_id", client_id);
        }
        query.append_pair("response_type", "code");
        query.append_pair("redirect_uri", &self.redirect_uri);
        query.append_pair("state", state);
        if !self.scopes.is_empty() {
            query.append_pair("scope", &self.scopes);
        }
        Ok(format!(
            "{}{}{}",
            base,
            if base.contains('?') { '&' } else { '?' },
            query.finish()
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredAccount {
    account_id: String,
    platform: String,
    open_id: String,
    display_name: Option<String>,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<u64>,
}

#[derive(Serialize)]
struct AccountSummary {
    account_id: String,
    platform: String,
    open_id: String,
    display_name: Option<String>,
    expires_at: Option<u64>,
}

impl From<&StoredAccount> for AccountSummary {
    fn from(account: &StoredAccount) -> Self {
        Self {
            account_id: account.account_id.clone(),
            platform: account.platform.clone(),
            open_id: account.open_id.clone(),
            display_name: account.display_name.clone(),
            expires_at: account.expires_at,
        }
    }
}

struct AccountStore {
    data_file: PathBuf,
    key: [u8; 32],
}

impl AccountStore {
    fn open(platform: Platform) -> Result<Self, String> {
        let root = env::var_os("SCULK_PLATFORM_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| env::var_os("APPDATA").map(PathBuf::from))
            .or_else(|| env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let directory = root
            .join("SculkCatalyst")
            .join("platform-bots")
            .join(platform.id());
        fs::create_dir_all(&directory).map_err(|error| format!("无法创建账号存储目录：{error}"))?;
        let key_file = directory.join("account.key");
        let key = if key_file.exists() {
            let bytes =
                fs::read(&key_file).map_err(|error| format!("无法读取账号密钥：{error}"))?;
            bytes
                .try_into()
                .map_err(|_| "账号密钥长度无效".to_string())?
        } else {
            let mut key = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            fs::write(&key_file, key).map_err(|error| format!("无法保存账号密钥：{error}"))?;
            key
        };
        Ok(Self {
            data_file: directory.join("accounts.enc"),
            key,
        })
    }

    fn load(&self) -> Result<Vec<StoredAccount>, String> {
        if !self.data_file.exists() {
            return Ok(Vec::new());
        }
        let encoded = fs::read_to_string(&self.data_file)
            .map_err(|error| format!("无法读取账号数据：{error}"))?;
        let blob = BASE64
            .decode(encoded.trim())
            .map_err(|error| format!("账号数据编码无效：{error}"))?;
        if blob.len() <= 12 {
            return Err("账号数据不完整".into());
        }
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|_| "账号加密密钥无效".to_string())?;
        let plaintext = cipher
            .decrypt(Nonce::from_slice(&blob[..12]), &blob[12..])
            .map_err(|_| "账号数据无法解密，可能需要重新登录".to_string())?;
        serde_json::from_slice(&plaintext).map_err(|error| format!("账号数据格式无效：{error}"))
    }

    fn save(&self, accounts: &[StoredAccount]) -> Result<(), String> {
        let plaintext =
            serde_json::to_vec(accounts).map_err(|error| format!("账号数据无法编码：{error}"))?;
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|_| "账号加密密钥无效".to_string())?;
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
            .map_err(|_| "账号数据加密失败".to_string())?;
        let mut blob = nonce.to_vec();
        blob.extend(ciphertext);
        let temporary = self.data_file.with_extension("enc.tmp");
        fs::write(&temporary, BASE64.encode(blob))
            .map_err(|error| format!("无法写入账号数据：{error}"))?;
        replace_file_atomically(&temporary, &self.data_file)?;
        Ok(())
    }
}

fn replace_file_atomically(temporary: &PathBuf, destination: &PathBuf) -> Result<(), String> {
    #[cfg(windows)]
    {
        if destination.exists() {
            let backup = destination.with_extension("enc.bak");
            let _ = fs::remove_file(&backup);
            fs::rename(destination, &backup)
                .map_err(|error| format!("无法替换账号数据：{error}"))?;
            if let Err(error) = fs::rename(temporary, destination) {
                let _ = fs::rename(&backup, destination);
                return Err(format!("无法提交账号数据：{error}"));
            }
            let _ = fs::remove_file(backup);
            return Ok(());
        }
    }
    fs::rename(temporary, destination).map_err(|error| format!("无法提交账号数据：{error}"))
}

pub(super) fn access_token_from_local_account(platform: Platform) -> Option<String> {
    let store = AccountStore::open(platform).ok()?;
    let selected_id = env::var(format!("{}_ACCOUNT_ID", platform.env_prefix()))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    store
        .load()
        .ok()?
        .into_iter()
        .filter(|account| {
            account
                .expires_at
                .map(|expires_at| expires_at > now_seconds())
                .unwrap_or(true)
        })
        .find(|account| {
            selected_id
                .as_deref()
                .map(|id| id == account.account_id)
                .unwrap_or(true)
        })
        .map(|account| account.access_token)
}

struct LoginState {
    config: LoginConfig,
    store: AccountStore,
    pending_state: Option<String>,
    last_result: Option<String>,
}

type SharedState = Arc<Mutex<LoginState>>;

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

pub(crate) fn run(platform: Platform) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("初始化登录运行时失败：{error}"))?;
    runtime.block_on(run_server(platform))
}

async fn run_server(platform: Platform) -> Result<(), String> {
    let config = LoginConfig::from_env(platform);
    let port = config.port;
    let state = Arc::new(Mutex::new(LoginState {
        config,
        store: AccountStore::open(platform)?,
        pending_state: None,
        last_result: None,
    }));
    let router = Router::new()
        .route("/", get(index))
        .route("/oauth/start", get(oauth_start))
        .route("/oauth/callback", get(oauth_callback))
        .route("/api/status", get(api_status))
        .route("/api/accounts", get(api_accounts))
        .route("/api/accounts/{id}/logout", post(api_logout))
        .with_state(state.clone());
    let address = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&address)
        .await
        .map_err(|error| format!("登录页面无法监听 {address}：{error}"))?;
    let url = format!("http://{address}/");
    eprintln!("{}扫码登录页面：{url}", platform.display_name());
    open_browser(&url);
    axum::serve(listener, router)
        .await
        .map_err(|error| format!("登录页面运行失败：{error}"))
}

async fn index(State(state): State<SharedState>) -> Html<String> {
    let (title, ready, result, accounts) = {
        let state = state.lock().await;
        let accounts = state.store.load().unwrap_or_default();
        (
            state.config.platform.display_name(),
            state.config.ready(),
            state.last_result.clone(),
            accounts
                .iter()
                .map(AccountSummary::from)
                .collect::<Vec<_>>(),
        )
    };
    let account_json = serde_json::to_string(&accounts).unwrap_or_else(|_| "[]".into());
    let action = if ready {
        r#"<a class="button" href="/oauth/start">开始扫码授权</a>"#
    } else {
        r#"<div class="warning">尚未配置 ClientKey/ClientSecret。请先配置本地环境变量，再重新启动。</div>"#
    };
    Html(format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>{title}</title>
<style>
body{{font-family:system-ui,sans-serif;max-width:760px;margin:48px auto;padding:0 24px;color:#20242c;background:#f7f8fa}}
main{{background:white;border:1px solid #e5e7eb;border-radius:16px;padding:28px;box-shadow:0 8px 30px #0000000d}}
h1{{margin-top:0}}.button{{display:inline-block;background:#2563eb;color:#fff;padding:11px 18px;border-radius:9px;text-decoration:none}}
.warning{{background:#fff7ed;border:1px solid #fdba74;padding:12px;border-radius:9px;color:#9a3412}}
.account{{display:flex;justify-content:space-between;border-top:1px solid #eee;padding:14px 0}}
code{{word-break:break-all}}
</style></head><body><main>
<h1>{title}</h1><p>平台账号授权只在本机完成，Token 会加密保存，不会显示在页面或日志中。</p>
{action}
<p>{result}</p>
<h2>已授权账号</h2><div id="accounts"></div>
<script>
const accounts={account_json};
const root=document.getElementById('accounts');
root.innerHTML=accounts.length?accounts.map(a=>'<div class="account"><span>'+(a.display_name||a.open_id)+'<br><small>'+a.account_id+'</small></span><button onclick="logout(''+encodeURIComponent(a.account_id)+'')">退出</button></div>').join(''):'<p>暂无账号</p>';
async function logout(id){{await fetch('/api/accounts/'+id+'/logout',{{method:'POST'}});location.reload();}}
</script></main></body></html>"#,
        title = html_escape(title),
        action = action,
        result = html_escape(result.as_deref().unwrap_or("")),
        account_json = account_json,
    ))
}

async fn oauth_start(State(state): State<SharedState>) -> impl IntoResponse {
    let mut state = state.lock().await;
    if !state.config.ready() {
        return Html(
            "<h2>未配置 OAuth 参数</h2><p>请配置 ClientKey/ClientSecret 后重启程序。</p>"
                .to_string(),
        )
        .into_response();
    }
    let nonce = random_id();
    state.pending_state = Some(nonce.clone());
    match state.config.auth_url(&nonce) {
        Ok(url) => Redirect::temporary(&url).into_response(),
        Err(error) => Html(format!(
            "<h2>无法开始授权</h2><p>{}</p>",
            html_escape(&error)
        ))
        .into_response(),
    }
}

async fn oauth_callback(
    State(state): State<SharedState>,
    Query(query): Query<CallbackQuery>,
) -> Html<String> {
    if let Some(error) = query.error {
        let description = query.error_description.unwrap_or_default();
        return Html(format!(
            "<h2>授权失败</h2><p>{}</p><p>{}</p><a href=\"/\">返回</a>",
            html_escape(&error),
            html_escape(&description)
        ));
    }
    let code = match query.code.filter(|value| !value.trim().is_empty()) {
        Some(value) => value,
        None => return Html("<h2>授权回调缺少 code</h2><a href=\"/\">返回</a>".into()),
    };
    let callback_state = match query.state {
        Some(value) => value,
        None => return Html("<h2>授权回调缺少 state</h2><a href=\"/\">返回</a>".into()),
    };
    let config = {
        let state = state.lock().await;
        if state.pending_state.as_deref() != Some(callback_state.as_str()) {
            return Html(
                "<h2>授权状态无效</h2><p>请重新开始扫码。</p><a href=\"/\">返回</a>".into(),
            );
        }
        state.config.clone()
    };
    match exchange_code(&config, &code).await {
        Ok(account) => {
            let mut state = state.lock().await;
            let mut accounts = match state.store.load() {
                Ok(accounts) => accounts,
                Err(error) => {
                    return Html(format!(
                        "<h2>读取账号存储失败</h2><p>{}</p>",
                        html_escape(&error)
                    ));
                }
            };
            accounts.retain(|item| item.account_id != account.account_id);
            accounts.push(account);
            if let Err(error) = state.store.save(&accounts) {
                return Html(format!(
                    "<h2>保存账号失败</h2><p>{}</p>",
                    html_escape(&error)
                ));
            }
            state.pending_state = None;
            state.last_result = Some("授权成功，账号已加密保存。".into());
            Html("<h2>授权成功</h2><p>账号已经保存，可以关闭此页面。</p><a href=\"/\">返回账号管理</a>".into())
        }
        Err(error) => Html(format!(
            "<h2>授权换取 Token 失败</h2><p>{}</p><a href=\"/\">返回</a>",
            html_escape(&error)
        )),
    }
}

async fn exchange_code(config: &LoginConfig, code: &str) -> Result<StoredAccount, String> {
    let token_url = config
        .token_url
        .as_deref()
        .ok_or_else(|| "未配置 Token 接口地址".to_string())?;
    let client_id = config
        .client_id
        .as_deref()
        .ok_or_else(|| "未配置 ClientKey/ClientID".to_string())?;
    let client_secret = config
        .client_secret
        .as_deref()
        .ok_or_else(|| "未配置 ClientSecret".to_string())?;
    let mut form = vec![
        ("code", code.to_string()),
        ("grant_type", "authorization_code".into()),
        ("client_secret", client_secret.to_string()),
    ];
    if matches!(config.platform, Platform::Douyin) {
        form.push(("client_key", client_id.to_string()));
    } else {
        form.push(("client_id", client_id.to_string()));
    }
    let value = Client::new()
        .post(token_url)
        .form(&form)
        .send()
        .await
        .map_err(|error| format!("请求 Token 接口失败：{error}"))?
        .json::<Value>()
        .await
        .map_err(|error| format!("Token 接口返回无效 JSON：{error}"))?;
    let data = value.get("data").unwrap_or(&value);
    if let Some(error) = data
        .get("description")
        .or_else(|| data.get("message"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        if data.get("access_token").is_none() {
            return Err(format!("平台授权失败：{error}"));
        }
    }
    let access_token = first_string(data, &["access_token", "accessToken"])
        .ok_or_else(|| "Token 接口没有返回 access_token".to_string())?;
    let refresh_token = first_string(data, &["refresh_token", "refreshToken"]);
    let open_id = first_string(data, &["open_id", "openId", "uid"])
        .unwrap_or_else(|| format!("{}-{}", config.platform.id(), random_id()));
    let expires_at = data
        .get("expires_in")
        .and_then(Value::as_u64)
        .map(|seconds| now_seconds().saturating_add(seconds));
    Ok(StoredAccount {
        account_id: format!("{}-{}", config.platform.id(), open_id),
        platform: config.platform.id().into(),
        open_id,
        display_name: first_string(data, &["nickname", "name", "screen_name"]),
        access_token,
        refresh_token,
        expires_at,
    })
}

async fn api_status(State(state): State<SharedState>) -> Json<Value> {
    let state = state.lock().await;
    let accounts = state.store.load().unwrap_or_default();
    Json(json!({
        "platform": state.config.platform.id(),
        "ready": state.config.ready(),
        "accounts": accounts.len(),
        "port": state.config.port
    }))
}

async fn api_accounts(State(state): State<SharedState>) -> Json<Vec<AccountSummary>> {
    let state = state.lock().await;
    let accounts = state.store.load().unwrap_or_default();
    Json(accounts.iter().map(AccountSummary::from).collect())
}

async fn api_logout(State(state): State<SharedState>, Path(id): Path<String>) -> Json<Value> {
    let state = state.lock().await;
    let mut accounts = state.store.load().unwrap_or_default();
    let before = accounts.len();
    accounts.retain(|account| account.account_id != id);
    let removed = before != accounts.len();
    if removed {
        let _ = state.store.save(&accounts);
    }
    Json(json!({ "removed": removed }))
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
}

fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default()
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn open_browser(url: &str) {
    if env::var("SCULK_PLATFORM_NO_BROWSER").ok().as_deref() == Some("1") {
        return;
    }
    #[cfg(windows)]
    {
        let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(url).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_url_contains_provider_parameters() {
        let config = LoginConfig {
            platform: Platform::Douyin,
            auth_url: Some("https://example.invalid/oauth".into()),
            token_url: Some("https://example.invalid/token".into()),
            client_id: Some("client-key".into()),
            client_secret: Some("client-secret".into()),
            redirect_uri: "http://127.0.0.1:18432/oauth/callback".into(),
            scopes: "item.comment".into(),
            port: 18432,
        };
        let url = config.auth_url("state-123").expect("授权地址应可生成");
        assert!(url.contains("client_key=client-key"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A18432%2Foauth%2Fcallback"));
        assert!(url.contains("state=state-123"));
        assert!(url.contains("scope=item.comment"));
    }

    #[test]
    fn account_store_round_trips_and_replaces() {
        let directory = env::temp_dir().join(format!("sculk-platform-login-{}", random_id()));
        fs::create_dir_all(&directory).expect("创建测试目录");
        let store = AccountStore {
            data_file: directory.join("accounts.enc"),
            key: [7u8; 32],
        };
        let account = StoredAccount {
            account_id: "douyin-open-1".into(),
            platform: "douyin".into(),
            open_id: "open-1".into(),
            display_name: Some("测试账号".into()),
            access_token: "access-token".into(),
            refresh_token: Some("refresh-token".into()),
            expires_at: None,
        };
        store
            .save(std::slice::from_ref(&account))
            .expect("首次保存");
        assert_eq!(
            store.load().expect("首次读取")[0].access_token,
            "access-token"
        );
        store.save(&[]).expect("覆盖保存");
        assert!(store.load().expect("覆盖读取").is_empty());
        let _ = fs::remove_dir_all(directory);
    }
}
