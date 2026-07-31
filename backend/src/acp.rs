use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};

/// 极简 ACP（Agent Client Protocol）客户端：JSON-RPC 2.0 over stdio，按行分帧。
pub(crate) struct AcpClient {
    child: Child,
    stdin: ChildStdin,
    reader: Lines<BufReader<ChildStdout>>,
    next_id: i64,
}

impl AcpClient {
    pub(crate) async fn spawn(command: &str, args: &[String]) -> Result<Self, String> {
        if command.trim().is_empty() {
            return Err("Agent 启动命令为空".into());
        }
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("无法启动 Agent 进程 {command}：{error}"))?;
        let stdin = child.stdin.take().ok_or("无法接管 Agent stdin")?;
        let stdout = child.stdout.take().ok_or("无法接管 Agent stdout")?;
        Ok(Self {
            child,
            stdin,
            reader: BufReader::new(stdout).lines(),
            next_id: 0,
        })
    }

    async fn send(&mut self, value: &Value) -> Result<(), String> {
        let mut line = value.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|error| format!("向 Agent 写入失败：{error}"))
    }

    pub(crate) async fn send_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<i64, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        Ok(id)
    }

    pub(crate) async fn respond(&mut self, id: &Value, result: Value) -> Result<(), String> {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
            .await
    }

    pub(crate) async fn respond_error(
        &mut self,
        id: &Value,
        code: i64,
        message: &str,
    ) -> Result<(), String> {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
            .await
    }

    pub(crate) async fn next_message(&mut self, timeout: Duration) -> Result<Value, String> {
        loop {
            let line = tokio::time::timeout(timeout, self.reader.next_line())
                .await
                .map_err(|_| "Agent 响应超时".to_string())?
                .map_err(|error| format!("读取 Agent 输出失败：{error}"))?
                .ok_or_else(|| "Agent 进程已退出".to_string())?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                return Ok(value);
            }
        }
    }

    /// 等待指定请求的响应；期间拒绝 Agent 发来的其他请求（握手阶段不受理反向调用）。
    pub(crate) async fn wait_response(
        &mut self,
        id: i64,
        timeout: Duration,
    ) -> Result<Value, String> {
        loop {
            let message = self.next_message(timeout).await?;
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    let request_id = message["id"].clone();
                    let _ = self
                        .respond_error(&request_id, -32601, "method not supported")
                        .await;
                }
                continue;
            }
            if message.get("id").and_then(Value::as_i64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(error["message"]
                        .as_str()
                        .unwrap_or("Agent 返回错误")
                        .to_string());
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    pub(crate) async fn shutdown(&mut self) {
        let _ = self.child.kill().await;
    }
}
