use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{env, io::IsTerminal};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

mod login;

#[derive(Clone, Copy, Debug)]
pub enum Platform {
    Bilibili,
    Douyin,
}

impl Platform {
    fn id(self) -> &'static str {
        match self {
            Self::Bilibili => "bilibili",
            Self::Douyin => "douyin",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Bilibili => "B站评论适配器",
            Self::Douyin => "抖音评论适配器",
        }
    }

    fn env_prefix(self) -> &'static str {
        match self {
            Self::Bilibili => "SCULK_BILIBILI",
            Self::Douyin => "SCULK_DOUYIN",
        }
    }

    fn default_list_path(self) -> Option<&'static str> {
        match self {
            Self::Bilibili => None,
            Self::Douyin => Some("/item/comment/list/"),
        }
    }

    fn default_reply_path(self) -> Option<&'static str> {
        match self {
            Self::Bilibili => None,
            Self::Douyin => Some("/item/comment/reply/"),
        }
    }
}

#[derive(Clone)]
struct AdapterConfig {
    platform: Platform,
    api_base: Option<String>,
    access_token: Option<String>,
    list_path: Option<String>,
    reply_path: Option<String>,
}

impl AdapterConfig {
    fn from_env(platform: Platform) -> Self {
        let prefix = platform.env_prefix();
        let read = |suffix: &str| {
            env::var(format!("{prefix}_{suffix}"))
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let default_api_base = match platform {
            Platform::Douyin => Some("https://open.douyin.com".to_string()),
            Platform::Bilibili => None,
        };
        Self {
            platform,
            api_base: read("API_BASE").or(default_api_base),
            access_token: read("ACCESS_TOKEN")
                .or_else(|| login::access_token_from_local_account(platform)),
            list_path: read("LIST_PATH")
                .or_else(|| platform.default_list_path().map(ToString::to_string)),
            reply_path: read("REPLY_PATH")
                .or_else(|| platform.default_reply_path().map(ToString::to_string)),
        }
    }

    fn configured(&self) -> bool {
        self.api_base.is_some() && self.access_token.is_some()
    }

    fn status(&self) -> Value {
        json!({
            "platform": self.platform.id(),
            "display_name": self.platform.display_name(),
            "configured": self.configured(),
            "read_enabled": self.configured() && self.list_path.is_some(),
            "reply_enabled": self.configured() && self.reply_path.is_some(),
            "transport": "mcp-stdio",
            "credential_present": self.access_token.is_some(),
            "note": if self.platform.id() == "bilibili" {
                "B站必须配置经过授权的官方或合规连接器；未配置时只提供协议能力，不调用未知网页接口。"
            } else {
                "抖音使用 OAuth access-token；调用前需在开放平台申请并获得评论权限。"
            }
        })
    }
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl RpcResponse {
    fn result(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

pub fn run(platform: Platform) -> Result<(), String> {
    let login_mode =
        std::env::args().any(|argument| argument == "--login") || std::io::stdin().is_terminal();
    if login_mode {
        return login::run(platform);
    }
    run_mcp(platform)
}

fn run_mcp(platform: Platform) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|error| format!("初始化运行时失败：{error}"))?;
    runtime.block_on(run_server(platform))
}

async fn run_server(platform: Platform) -> Result<(), String> {
    let config = AdapterConfig::from_env(platform);
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| format!("初始化 HTTP 客户端失败：{error}"))?;
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut input = BufReader::new(stdin).lines();
    let mut output = tokio::io::BufWriter::new(stdout);

    while let Some(line) = input
        .next_line()
        .await
        .map_err(|error| format!("读取 MCP 输入失败：{error}"))?
    {
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(request) => request,
            Err(error) => {
                write_response(
                    &mut output,
                    RpcResponse::error(None, -32700, format!("无效 JSON：{error}")),
                )
                .await?;
                continue;
            }
        };
        let Some(id) = request.id.clone() else {
            continue;
        };
        let response = match request.method.as_str() {
            "initialize" => RpcResponse::result(
                Some(id),
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": {
                        "name": format!("sculk-{}", platform.id()),
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ),
            "tools/list" => RpcResponse::result(Some(id), tools_list()),
            "tools/call" => match call_tool(&client, &config, &request.params).await {
                Ok(value) => RpcResponse::result(Some(id), value),
                Err(error) => RpcResponse::result(
                    Some(id),
                    json!({
                        "content": [{ "type": "text", "text": error }],
                        "isError": true
                    }),
                ),
            },
            "notifications/initialized" => continue,
            _ => RpcResponse::error(Some(id), -32601, "不支持的 MCP 方法"),
        };
        write_response(&mut output, response).await?;
    }
    Ok(())
}

async fn write_response(
    output: &mut tokio::io::BufWriter<tokio::io::Stdout>,
    response: RpcResponse,
) -> Result<(), String> {
    let encoded =
        serde_json::to_vec(&response).map_err(|error| format!("编码 MCP 响应失败：{error}"))?;
    output
        .write_all(&encoded)
        .await
        .map_err(|error| format!("写入 MCP 响应失败：{error}"))?;
    output
        .write_all(b"\n")
        .await
        .map_err(|error| format!("写入 MCP 换行失败：{error}"))?;
    output
        .flush()
        .await
        .map_err(|error| format!("刷新 MCP 响应失败：{error}"))
}

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "platform_status",
                "description": "查看平台适配器配置状态，不会输出访问令牌。",
                "inputSchema": { "type": "object", "additionalProperties": false }
            },
            {
                "name": "list_comments",
                "description": "读取指定视频的评论。只返回平台连接器允许返回的数据。",
                "inputSchema": {
                    "type": "object",
                    "required": ["video_id"],
                    "properties": {
                        "video_id": { "type": "string", "minLength": 1, "maxLength": 256 },
                        "cursor": { "type": "string", "maxLength": 256 },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
                    },
                    "additionalProperties": false
                }
            },
            {
                "name": "reply_comment",
                "description": "回复一条评论。该操作会产生平台外部副作用，应由 Sculk Cloud 审批任务调用。",
                "inputSchema": {
                    "type": "object",
                    "required": ["video_id", "comment_id", "content"],
                    "properties": {
                        "video_id": { "type": "string", "minLength": 1, "maxLength": 256 },
                        "comment_id": { "type": "string", "minLength": 1, "maxLength": 256 },
                        "content": { "type": "string", "minLength": 1, "maxLength": 500 },
                        "dry_run": { "type": "boolean" }
                    },
                    "additionalProperties": false
                }
            }
        ]
    })
}

async fn call_tool(
    client: &Client,
    config: &AdapterConfig,
    params: &Value,
) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "tools/call 缺少 name".to_string())?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let value = match name {
        "platform_status" => config.status(),
        "list_comments" => list_comments(client, config, &arguments).await?,
        "reply_comment" => reply_comment(client, config, &arguments).await?,
        _ => return Err(format!("未知工具：{name}")),
    };
    let text =
        serde_json::to_string(&value).map_err(|error| format!("编码工具结果失败：{error}"))?;
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": value,
        "isError": false
    }))
}

async fn list_comments(
    client: &Client,
    config: &AdapterConfig,
    args: &Value,
) -> Result<Value, String> {
    let video_id = required_string(args, "video_id", 256)?;
    let path = config
        .list_path
        .as_deref()
        .ok_or_else(|| "当前平台未配置评论读取连接器".to_string())?;
    let mut request = client.get(join_url(config.api_base.as_deref(), path)?);
    request = request.query(&[
        ("item_id", video_id.as_str()),
        ("video_id", video_id.as_str()),
    ]);
    if let Some(cursor) = optional_string(args, "cursor", 256)? {
        request = request.query(&[("cursor", cursor)]);
    }
    if let Some(limit) = args.get("limit").and_then(Value::as_u64) {
        if !(1..=100).contains(&limit) {
            return Err("limit 必须在 1-100 之间".into());
        }
        request = request.query(&[("count", limit.to_string())]);
    }
    send_json_request(request, config).await
}

async fn reply_comment(
    client: &Client,
    config: &AdapterConfig,
    args: &Value,
) -> Result<Value, String> {
    let video_id = required_string(args, "video_id", 256)?;
    let comment_id = required_string(args, "comment_id", 256)?;
    let content = required_string(args, "content", 500)?;
    if args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(json!({
            "platform": config.platform.id(),
            "dry_run": true,
            "video_id": video_id,
            "comment_id": comment_id,
            "content": content
        }));
    }
    let path = config
        .reply_path
        .as_deref()
        .ok_or_else(|| "当前平台未配置评论回复连接器".to_string())?;
    let body = json!({
        "item_id": video_id,
        "video_id": video_id,
        "comment_id": comment_id,
        "content": content
    });
    let request = client
        .post(join_url(config.api_base.as_deref(), path)?)
        .json(&body);
    send_json_request(request, config).await
}

async fn send_json_request(
    mut request: reqwest::RequestBuilder,
    config: &AdapterConfig,
) -> Result<Value, String> {
    if let Some(token) = config.access_token.as_deref() {
        request = request.header("access-token", token).bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("平台连接器请求失败：{error}"))?;
    let status = response.status();
    let value = response
        .json::<Value>()
        .await
        .map_err(|error| format!("平台连接器返回了无效 JSON（HTTP {status}）：{error}"))?;
    if !status.is_success() {
        return Err(format!("平台连接器返回 HTTP {status}"));
    }
    Ok(redact_sensitive_value(value))
}

fn join_url(base: Option<&str>, path: &str) -> Result<String, String> {
    let base = base.ok_or_else(|| "未配置平台连接器地址".to_string())?;
    if path.starts_with("http://") || path.starts_with("https://") {
        return Ok(path.to_string());
    }
    Ok(format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    ))
}

fn required_string(args: &Value, key: &str, max: usize) -> Result<String, String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} 必须是非空字符串"))?;
    if value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(format!("{key} 超出长度限制或包含控制字符"));
    }
    Ok(value.to_string())
}

fn optional_string(args: &Value, key: &str, max: usize) -> Result<Option<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    required_string(args, key, max).map(Some)
}

fn redact_sensitive_value(value: Value) -> Value {
    match value {
        Value::Object(mut object) => {
            for (key, item) in &mut object {
                let normalized = key.to_ascii_lowercase();
                if normalized.contains("token")
                    || normalized.contains("secret")
                    || normalized.contains("password")
                    || normalized.contains("cookie")
                {
                    *item = Value::String("[REDACTED]".into());
                } else {
                    *item = redact_sensitive_value(item.take());
                }
            }
            Value::Object(object)
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(redact_sensitive_value).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_does_not_expose_token() {
        let config = AdapterConfig {
            platform: Platform::Douyin,
            api_base: Some("https://example.invalid".into()),
            access_token: Some("secret".into()),
            list_path: Some("/list".into()),
            reply_path: Some("/reply".into()),
        };
        let value = config.status();
        assert_eq!(value["credential_present"], true);
        assert!(!value.to_string().contains("secret"));
    }

    #[test]
    fn required_string_rejects_control_characters() {
        let args = json!({ "content": "hello\nworld" });
        assert!(required_string(&args, "content", 500).is_err());
    }

    #[test]
    fn connector_results_redact_credentials() {
        let value = redact_sensitive_value(json!({
            "access_token": "secret",
            "items": [{ "cookie": "session" }]
        }));
        assert_eq!(value["access_token"], "[REDACTED]");
        assert_eq!(value["items"][0]["cookie"], "[REDACTED]");
    }
}
