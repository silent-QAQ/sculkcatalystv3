use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

const MCP_CALL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MCP_PAYLOAD_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpServerConfig {
    pub(crate) id: String,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) enabled: bool,
}

pub(crate) async fn call_tool(
    servers: &[McpServerConfig],
    operation: &str,
    input: &Value,
) -> Result<Value, String> {
    let server_id = input
        .get("server")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 64)
        .ok_or_else(|| "MCP 任务缺少有效的 server".to_string())?;
    let tool = input
        .get("tool")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 64)
        .ok_or_else(|| "MCP 任务缺少有效的 tool".to_string())?;
    let arguments = input.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let payload_size = serde_json::to_vec(&arguments)
        .map_err(|error| format!("MCP arguments 无法编码：{error}"))?
        .len();
    if payload_size > MAX_MCP_PAYLOAD_BYTES {
        return Err("MCP arguments 超过大小限制".into());
    }
    validate_tool(operation, tool)?;
    let server = servers
        .iter()
        .find(|server| server.id == server_id && server.enabled)
        .ok_or_else(|| format!("MCP server 未启用或不存在：{server_id}"))?;
    let result = McpProcess::start(server).await?;
    result.call(tool, arguments).await
}

fn validate_tool(operation: &str, tool: &str) -> Result<(), String> {
    let allowed = match operation {
        "platform.mcp.read" => ["platform_status", "list_comments"].as_slice(),
        "platform.mcp.reply" => ["reply_comment"].as_slice(),
        _ => return Err("不是受支持的平台 MCP 操作".into()),
    };
    if allowed.contains(&tool) {
        Ok(())
    } else {
        Err(format!("{tool} 不允许通过 {operation} 调用"))
    }
}

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpProcess {
    async fn start(server: &McpServerConfig) -> Result<Self, String> {
        if server.command.trim().is_empty()
            || server.command.chars().count() > 1024
            || server.command.chars().any(char::is_control)
        {
            return Err("MCP command 无效".into());
        }
        let mut command = Command::new(&server.command);
        command
            .args(&server.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| format!("启动 MCP server 失败：{error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "MCP server 没有 stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "MCP server 没有 stdout".to_string())?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    async fn call(mut self, tool: &str, arguments: Value) -> Result<Value, String> {
        let initialize = self
            .request(
                1,
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "sculk-agent",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;
        if initialize.get("serverInfo").is_none() {
            return Err("MCP initialize 响应缺少 serverInfo".into());
        }
        self.notify("notifications/initialized", json!({})).await?;
        let result = self
            .request(
                2,
                "tools/call",
                json!({
                    "name": tool,
                    "arguments": arguments
                }),
            )
            .await?;
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if is_error {
            return Err(result
                .pointer("/content/0/text")
                .and_then(Value::as_str)
                .unwrap_or("MCP 工具调用失败")
                .to_string());
        }
        let output = result
            .get("structuredContent")
            .cloned()
            .or_else(|| result.get("content").cloned())
            .ok_or_else(|| "MCP tools/call 响应缺少结果".to_string())?;
        let _ = self.child.kill().await;
        Ok(output)
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.write_line(request).await
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.write_line(request).await?;
        loop {
            let mut line = String::new();
            let read = timeout(MCP_CALL_TIMEOUT, self.stdout.read_line(&mut line))
                .await
                .map_err(|_| "MCP server 响应超时".to_string())?
                .map_err(|error| format!("读取 MCP 响应失败：{error}"))?;
            if read == 0 {
                return Err("MCP server 提前退出".into());
            }
            let response: Value = serde_json::from_str(line.trim())
                .map_err(|error| format!("MCP server 返回无效 JSON：{error}"))?;
            if response.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(format!("MCP RPC 错误：{error}"));
            }
            return response
                .get("result")
                .cloned()
                .ok_or_else(|| "MCP 响应缺少 result".to_string());
        }
    }

    async fn write_line(&mut self, value: Value) -> Result<(), String> {
        let encoded =
            serde_json::to_vec(&value).map_err(|error| format!("编码 MCP 请求失败：{error}"))?;
        if encoded.len() > MAX_MCP_PAYLOAD_BYTES {
            return Err("MCP 请求超过大小限制".into());
        }
        self.stdin
            .write_all(&encoded)
            .await
            .map_err(|error| format!("写入 MCP 请求失败：{error}"))?;
        self.stdin
            .write_all(b"\n")
            .await
            .map_err(|error| format!("写入 MCP 换行失败：{error}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|error| format!("刷新 MCP 请求失败：{error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_and_reply_tools_are_separated() {
        assert!(validate_tool("platform.mcp.read", "list_comments").is_ok());
        assert!(validate_tool("platform.mcp.read", "reply_comment").is_err());
        assert!(validate_tool("platform.mcp.reply", "reply_comment").is_ok());
    }
}
