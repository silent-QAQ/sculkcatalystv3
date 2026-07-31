// SPDX-License-Identifier: Apache-2.0

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use rand::{RngCore, rngs::OsRng};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::mpsc, time::Instant};

const COMMAND_POLL_IDLE: Duration = Duration::from_secs(2);
const COMMAND_POLL_ACTIVE: Duration = Duration::from_millis(250);
const EVENT_FLUSH_INTERVAL: Duration = Duration::from_millis(75);
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(1);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const EXIT_DRAIN_GRACE: Duration = Duration::from_millis(250);
const CLOUD_LEASE_GRACE: Duration = Duration::from_secs(75);
const LOCAL_EVENT_CHANNEL: usize = 128;
const MAX_ACTIVE_SESSIONS: usize = 8;
const MAX_PENDING_EVENTS: usize = 256;
const MAX_PENDING_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_CHUNK: usize = 16 * 1024;

#[derive(Clone)]
pub struct TerminalConfig {
    pub cloud_url: String,
    pub token: String,
    pub workspace_root: PathBuf,
}

#[derive(Serialize)]
struct CommandsRequest<'a> {
    instance_id: &'a str,
    max_commands: u16,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CommandsResponse {
    Wrapped { commands: Vec<TerminalCommand> },
    Direct(Vec<TerminalCommand>),
}

impl CommandsResponse {
    fn into_commands(self) -> Vec<TerminalCommand> {
        match self {
            Self::Wrapped { commands } => commands,
            Self::Direct(commands) => commands,
        }
    }
}

#[derive(Deserialize)]
struct TerminalCommand {
    id: String,
    session_id: String,
    seq: i64,
    kind: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartPayload {
    #[serde(default)]
    cwd: Option<String>,
    cols: u16,
    rows: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputPayload {
    data_base64: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizePayload {
    cols: u16,
    rows: u16,
}

#[derive(Clone, Serialize)]
struct PendingEvent {
    seq: u64,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl PendingEvent {
    fn output(seq: u64, bytes: &[u8]) -> Self {
        Self {
            seq,
            kind: "output",
            data_base64: Some(BASE64.encode(bytes)),
            data: None,
        }
    }

    fn structured(seq: u64, kind: &'static str, data: Value) -> Self {
        Self {
            seq,
            kind,
            data_base64: None,
            data: Some(data),
        }
    }

    fn output_len(&self) -> usize {
        self.data_base64
            .as_deref()
            .and_then(|value| BASE64.decode(value).ok())
            .map_or(0, |bytes| bytes.len())
    }
}

#[derive(Serialize)]
struct EventsRequest<'a> {
    instance_id: &'a str,
    acknowledged_command_ids: Vec<String>,
    events: Vec<PendingEvent>,
}

enum LocalEvent {
    Output { session_id: String, bytes: Vec<u8> },
    ReaderClosed { session_id: String },
    ReaderError { session_id: String, message: String },
    Exit { session_id: String, exit_code: i32 },
    WaitError { session_id: String, message: String },
}

struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    next_event_seq: u64,
    last_command_seq: i64,
    pending_events: VecDeque<PendingEvent>,
    pending_output_bytes: usize,
    pending_acks: VecDeque<String>,
    reader_closed: bool,
    pending_exit: Option<i32>,
    exit_observed_at: Option<Instant>,
    locally_finished: bool,
    failure_reported: bool,
    cloud_expired: bool,
    last_keepalive: Instant,
    last_cloud_success: Instant,
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

impl TerminalSession {
    fn next_seq(&mut self) -> u64 {
        self.next_event_seq += 1;
        self.next_event_seq
    }

    fn has_processed(&self, command_id: &str) -> bool {
        self.pending_acks.iter().any(|item| item == command_id)
    }

    fn acknowledge(&mut self, command_id: String, command_seq: i64) {
        if !self.has_processed(&command_id) {
            self.pending_acks.push_back(command_id);
        }
        self.last_command_seq = self.last_command_seq.max(command_seq);
    }

    fn push_structured(&mut self, kind: &'static str, data: Value) {
        let seq = self.next_seq();
        self.pending_events
            .push_back(PendingEvent::structured(seq, kind, data));
    }

    fn push_output(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.locally_finished || self.failure_reported {
            return Ok(());
        }
        if self.pending_events.len() >= MAX_PENDING_EVENTS.saturating_sub(2)
            || self.pending_output_bytes.saturating_add(bytes.len()) > MAX_PENDING_OUTPUT_BYTES
        {
            return Err("终端输出积压超过本地安全上限，已终止会话以防止内存无限增长".into());
        }
        let seq = self.next_seq();
        self.pending_events
            .push_back(PendingEvent::output(seq, bytes));
        self.pending_output_bytes += bytes.len();
        Ok(())
    }

    fn fail_locally(&mut self, message: String) {
        if self.locally_finished || self.failure_reported {
            return;
        }
        self.failure_reported = true;
        let _ = self.killer.kill();
        self.push_structured("error", json!({ "message": message }));
    }

    fn note_exit(&mut self, exit_code: i32) {
        if self.locally_finished {
            return;
        }
        self.pending_exit = Some(exit_code);
        self.exit_observed_at = Some(Instant::now());
        self.finish_if_reader_closed();
    }

    fn note_reader_closed(&mut self) {
        self.reader_closed = true;
        self.finish_if_reader_closed();
    }

    fn finish_if_reader_closed(&mut self) {
        if self.reader_closed {
            self.finish_exit();
        }
    }

    fn finish_exit(&mut self) {
        if self.locally_finished {
            return;
        }
        if let Some(exit_code) = self.pending_exit.take() {
            self.locally_finished = true;
            if !self.failure_reported {
                self.push_structured("exit", json!({ "exit_code": exit_code }));
            }
        }
    }
}

#[derive(Default)]
struct DetachedBatch {
    pending_events: VecDeque<PendingEvent>,
    pending_acks: VecDeque<String>,
}

struct Manager {
    client: Client,
    config: TerminalConfig,
    instance_id: String,
    event_sender: mpsc::Sender<LocalEvent>,
    sessions: HashMap<String, TerminalSession>,
    detached: HashMap<String, DetachedBatch>,
    last_warning: Option<Instant>,
}

struct PostEventsError {
    message: String,
    abandon_session: bool,
}

pub async fn run(client: Client, config: TerminalConfig) {
    let (event_sender, mut event_receiver) = mpsc::channel(LOCAL_EVENT_CHANNEL);
    let mut manager = Manager {
        client,
        config,
        instance_id: random_instance_id(),
        event_sender,
        sessions: HashMap::new(),
        detached: HashMap::new(),
        last_warning: None,
    };
    let mut next_poll = Instant::now();
    let mut next_flush = Instant::now();
    let mut next_maintenance = Instant::now() + MAINTENANCE_INTERVAL;

    loop {
        tokio::select! {
            event = event_receiver.recv() => {
                if let Some(event) = event {
                    manager.handle_local_event(event);
                }
            }
            _ = tokio::time::sleep_until(next_poll) => {
                if let Err(error) = manager.poll_commands().await {
                    manager.warn_throttled(&format!("终端命令同步暂时失败：{error}"));
                }
                next_poll = Instant::now() + if manager.sessions.is_empty() {
                    COMMAND_POLL_IDLE
                } else {
                    COMMAND_POLL_ACTIVE
                };
            }
            _ = tokio::time::sleep_until(next_flush) => {
                if let Err(error) = manager.flush_pending().await {
                    manager.warn_throttled(&format!("终端事件同步暂时失败：{error}"));
                }
                next_flush = Instant::now() + EVENT_FLUSH_INTERVAL;
            }
            _ = tokio::time::sleep_until(next_maintenance) => {
                manager.maintain();
                next_maintenance = Instant::now() + MAINTENANCE_INTERVAL;
            }
        }
    }
}

impl Manager {
    fn warn_throttled(&mut self, message: &str) {
        let now = Instant::now();
        if self
            .last_warning
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(30))
        {
            eprintln!("{message}");
            self.last_warning = Some(now);
        }
    }

    async fn poll_commands(&mut self) -> Result<(), String> {
        let response = self
            .client
            .post(endpoint(
                &self.config.cloud_url,
                "/api/cloud/agent/terminals/commands",
            ))
            .bearer_auth(&self.config.token)
            .json(&CommandsRequest {
                instance_id: &self.instance_id,
                max_commands: 64,
            })
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(());
        }
        if !response.status().is_success() {
            return Err(http_error(response).await);
        }
        let commands = response
            .json::<CommandsResponse>()
            .await
            .map_err(|error| format!("Cloud 返回了无效的终端命令：{error}"))?
            .into_commands();
        for command in commands {
            self.process_command(command).await;
        }
        Ok(())
    }

    async fn process_command(&mut self, command: TerminalCommand) {
        if command.id.is_empty() || command.session_id.is_empty() || command.seq <= 0 {
            self.warn_throttled("Cloud 返回了缺少标识或序号的终端命令");
            return;
        }
        if let Some(session) = self.sessions.get_mut(&command.session_id) {
            if session.has_processed(&command.id) {
                return;
            }
            if command.seq < session.last_command_seq {
                session.acknowledge(command.id, command.seq);
                return;
            }
        }

        match command.kind.as_str() {
            "start" => self.process_start(command),
            "input" => self.process_input(command).await,
            "resize" => self.process_resize(command),
            "terminate" => self.process_terminate(command),
            _ => self.detach_error(
                &command.session_id,
                command.id,
                format!("不支持的终端命令类型：{}", command.kind),
            ),
        }
    }

    fn process_start(&mut self, command: TerminalCommand) {
        if let Some(session) = self.sessions.get_mut(&command.session_id) {
            session.acknowledge(command.id, command.seq);
            return;
        }
        if self.sessions.len() >= MAX_ACTIVE_SESSIONS {
            self.detach_error(
                &command.session_id,
                command.id,
                format!("此 Agent 最多同时运行 {MAX_ACTIVE_SESSIONS} 个终端会话"),
            );
            return;
        }
        let payload = match serde_json::from_value::<StartPayload>(command.payload) {
            Ok(payload) => payload,
            Err(error) => {
                self.detach_error(
                    &command.session_id,
                    command.id,
                    format!("终端启动参数无效：{error}"),
                );
                return;
            }
        };
        let result = spawn_terminal(
            &command.session_id,
            &self.config.workspace_root,
            payload,
            self.event_sender.clone(),
        );
        match result {
            Ok((mut session, _shell, _cwd)) => {
                session.acknowledge(command.id, command.seq);
                session.push_structured("started", json!({}));
                self.sessions.insert(command.session_id, session);
            }
            Err(error) => self.detach_error(&command.session_id, command.id, error),
        }
    }

    async fn process_input(&mut self, command: TerminalCommand) {
        let payload = match serde_json::from_value::<InputPayload>(command.payload) {
            Ok(payload) => payload,
            Err(error) => {
                self.detach_error(
                    &command.session_id,
                    command.id,
                    format!("终端输入参数无效：{error}"),
                );
                return;
            }
        };
        let bytes = match BASE64.decode(payload.data_base64) {
            Ok(bytes) if (1..=8192).contains(&bytes.len()) => bytes,
            _ => {
                self.detach_error(
                    &command.session_id,
                    command.id,
                    "终端输入必须是 1 到 8192 字节的 Base64 数据".into(),
                );
                return;
            }
        };
        let Some(session) = self.sessions.get(&command.session_id) else {
            self.detach_ack(&command.session_id, command.id);
            return;
        };
        let writer = session.writer.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut writer = writer.lock().map_err(|_| "终端输入锁已损坏".to_string())?;
            writer
                .write_all(&bytes)
                .and_then(|_| writer.flush())
                .map_err(|error| format!("写入终端失败：{error}"))
        })
        .await
        .map_err(|error| format!("终端输入线程异常：{error}"))
        .and_then(|result| result);
        if let Some(session) = self.sessions.get_mut(&command.session_id) {
            session.acknowledge(command.id, command.seq);
            if let Err(error) = result {
                session.fail_locally(error);
            }
        }
    }

    fn process_resize(&mut self, command: TerminalCommand) {
        let payload = match serde_json::from_value::<ResizePayload>(command.payload) {
            Ok(payload)
                if (20..=400).contains(&payload.cols) && (5..=200).contains(&payload.rows) =>
            {
                payload
            }
            _ => {
                self.detach_error(
                    &command.session_id,
                    command.id,
                    "终端尺寸必须在 20×5 到 400×200 之间".into(),
                );
                return;
            }
        };
        let Some(session) = self.sessions.get_mut(&command.session_id) else {
            self.detach_ack(&command.session_id, command.id);
            return;
        };
        let result = session.master.resize(PtySize {
            rows: payload.rows,
            cols: payload.cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        session.acknowledge(command.id, command.seq);
        if let Err(error) = result {
            session.fail_locally(format!("同步终端尺寸失败：{error}"));
        }
    }

    fn process_terminate(&mut self, command: TerminalCommand) {
        let Some(session) = self.sessions.get_mut(&command.session_id) else {
            self.detach_ack(&command.session_id, command.id);
            return;
        };
        session.acknowledge(command.id, command.seq);
        if let Err(error) = session.killer.kill() {
            session.fail_locally(format!("终止终端进程失败：{error}"));
        }
    }

    fn detach_ack(&mut self, session_id: &str, command_id: String) {
        let batch = self.detached.entry(session_id.to_string()).or_default();
        if !batch.pending_acks.iter().any(|item| item == &command_id) {
            batch.pending_acks.push_back(command_id);
        }
    }

    fn detach_error(&mut self, session_id: &str, command_id: String, message: String) {
        let batch = self.detached.entry(session_id.to_string()).or_default();
        if !batch.pending_acks.iter().any(|item| item == &command_id) {
            batch.pending_acks.push_back(command_id);
        }
        if batch.pending_events.is_empty() {
            batch.pending_events.push_back(PendingEvent::structured(
                1,
                "error",
                json!({ "message": message }),
            ));
        }
    }

    fn handle_local_event(&mut self, event: LocalEvent) {
        match event {
            LocalEvent::Output { session_id, bytes } => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    if let Err(error) = session.push_output(&bytes) {
                        session.fail_locally(error);
                    }
                }
            }
            LocalEvent::ReaderClosed { session_id } => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.note_reader_closed();
                }
            }
            LocalEvent::ReaderError {
                session_id,
                message,
            } => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.note_reader_closed();
                    session.fail_locally(format!("读取终端输出失败：{message}"));
                }
            }
            LocalEvent::Exit {
                session_id,
                exit_code,
            } => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.note_exit(exit_code);
                }
            }
            LocalEvent::WaitError {
                session_id,
                message,
            } => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.fail_locally(format!("等待终端退出失败：{message}"));
                }
            }
        }
    }

    fn maintain(&mut self) {
        let now = Instant::now();
        for session in self.sessions.values_mut() {
            if session.pending_exit.is_some()
                && session
                    .exit_observed_at
                    .is_some_and(|observed| now.duration_since(observed) >= EXIT_DRAIN_GRACE)
            {
                session.finish_exit();
            }
            if !session.locally_finished
                && now.duration_since(session.last_keepalive) >= KEEPALIVE_INTERVAL
            {
                session.push_structured("keepalive", json!({}));
                session.last_keepalive = now;
            }
            if !session.locally_finished
                && !session.cloud_expired
                && now.duration_since(session.last_cloud_success) >= CLOUD_LEASE_GRACE
            {
                session.cloud_expired = true;
                session.fail_locally(
                    "Cloud 连续不可达，终端租约已过期；为避免失控进程，会话已终止".into(),
                );
            }
        }
    }

    async fn flush_pending(&mut self) -> Result<(), String> {
        let mut first_error = None;
        let session_ids = self.sessions.keys().cloned().collect::<Vec<_>>();
        for session_id in session_ids {
            let (events, acknowledgements) = {
                let Some(session) = self.sessions.get(&session_id) else {
                    continue;
                };
                (
                    session
                        .pending_events
                        .iter()
                        .take(64)
                        .cloned()
                        .collect::<Vec<_>>(),
                    session
                        .pending_acks
                        .iter()
                        .take(64)
                        .cloned()
                        .collect::<Vec<_>>(),
                )
            };
            if events.is_empty() && acknowledgements.is_empty() {
                continue;
            }
            match Self::post_events(
                &self.client,
                &self.config,
                &self.instance_id,
                &session_id,
                &events,
                &acknowledgements,
            )
            .await
            {
                Ok(()) => {
                    if let Some(session) = self.sessions.get_mut(&session_id) {
                        for _ in 0..events.len() {
                            if let Some(event) = session.pending_events.pop_front() {
                                session.pending_output_bytes = session
                                    .pending_output_bytes
                                    .saturating_sub(event.output_len());
                            }
                        }
                        for _ in 0..acknowledgements.len() {
                            session.pending_acks.pop_front();
                        }
                        session.last_cloud_success = Instant::now();
                    }
                }
                Err(error) => {
                    if error.abandon_session {
                        self.sessions.remove(&session_id);
                    }
                    first_error.get_or_insert(error.message);
                }
            }
        }

        let detached_ids = self.detached.keys().cloned().collect::<Vec<_>>();
        for session_id in detached_ids {
            let (events, acknowledgements) = {
                let Some(batch) = self.detached.get(&session_id) else {
                    continue;
                };
                (
                    batch.pending_events.iter().cloned().collect::<Vec<_>>(),
                    batch.pending_acks.iter().cloned().collect::<Vec<_>>(),
                )
            };
            match Self::post_events(
                &self.client,
                &self.config,
                &self.instance_id,
                &session_id,
                &events,
                &acknowledgements,
            )
            .await
            {
                Ok(()) => {
                    self.detached.remove(&session_id);
                }
                Err(error) => {
                    if error.abandon_session {
                        self.detached.remove(&session_id);
                    }
                    first_error.get_or_insert(error.message);
                }
            }
        }

        self.sessions.retain(|_, session| {
            !(session.locally_finished
                && session.pending_events.is_empty()
                && session.pending_acks.is_empty())
        });
        first_error.map_or(Ok(()), Err)
    }

    async fn post_events(
        client: &Client,
        config: &TerminalConfig,
        instance_id: &str,
        session_id: &str,
        events: &[PendingEvent],
        acknowledgements: &[String],
    ) -> Result<(), PostEventsError> {
        let response = client
            .post(endpoint(
                &config.cloud_url,
                &format!("/api/cloud/agent/terminals/{session_id}/events"),
            ))
            .bearer_auth(&config.token)
            .json(&EventsRequest {
                instance_id,
                acknowledged_command_ids: acknowledgements.to_vec(),
                events: events.to_vec(),
            })
            .send()
            .await
            .map_err(|error| PostEventsError {
                message: error.to_string(),
                abandon_session: false,
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            Err(PostEventsError {
                message: http_error(response).await,
                abandon_session: status.is_client_error()
                    && status != StatusCode::REQUEST_TIMEOUT
                    && status != StatusCode::TOO_MANY_REQUESTS,
            })
        }
    }
}

fn spawn_terminal(
    session_id: &str,
    workspace_root: &Path,
    payload: StartPayload,
    event_sender: mpsc::Sender<LocalEvent>,
) -> Result<(TerminalSession, String, PathBuf), String> {
    if !(20..=400).contains(&payload.cols) || !(5..=200).contains(&payload.rows) {
        return Err("终端尺寸必须在 20×5 到 400×200 之间".into());
    }
    let cwd = resolve_cwd(workspace_root, payload.cwd.as_deref())?;
    let shell = default_shell();
    let pty = native_pty_system()
        .openpty(PtySize {
            rows: payload.rows,
            cols: payload.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("创建 PTY 失败：{error}"))?;
    let mut command = CommandBuilder::new(&shell);
    command.cwd(&cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    let mut child = pty
        .slave
        .spawn_command(command)
        .map_err(|error| format!("启动交互式 Shell 失败：{error}"))?;
    let killer = child.clone_killer();
    let reader = pty
        .master
        .try_clone_reader()
        .map_err(|error| format!("连接 PTY 输出失败：{error}"))?;
    let writer = pty
        .master
        .take_writer()
        .map_err(|error| format!("连接 PTY 输入失败：{error}"))?;
    drop(pty.slave);

    let reader_session_id = session_id.to_string();
    let reader_sender = event_sender.clone();
    std::thread::Builder::new()
        .name(format!("sculk-pty-read-{}", short_id(session_id)))
        .spawn(move || read_pty(reader_session_id, reader, reader_sender))
        .map_err(|error| format!("启动 PTY 输出线程失败：{error}"))?;

    let wait_session_id = session_id.to_string();
    std::thread::Builder::new()
        .name(format!("sculk-pty-wait-{}", short_id(session_id)))
        .spawn(move || {
            let event = match child.wait() {
                Ok(status) => LocalEvent::Exit {
                    session_id: wait_session_id,
                    exit_code: status.exit_code() as i32,
                },
                Err(error) => LocalEvent::WaitError {
                    session_id: wait_session_id,
                    message: error.to_string(),
                },
            };
            let _ = event_sender.blocking_send(event);
        })
        .map_err(|error| format!("启动 PTY 等待线程失败：{error}"))?;

    let now = Instant::now();
    Ok((
        TerminalSession {
            master: pty.master,
            writer: Arc::new(Mutex::new(writer)),
            killer,
            next_event_seq: 0,
            last_command_seq: 0,
            pending_events: VecDeque::new(),
            pending_output_bytes: 0,
            pending_acks: VecDeque::new(),
            reader_closed: false,
            pending_exit: None,
            exit_observed_at: None,
            locally_finished: false,
            failure_reported: false,
            cloud_expired: false,
            last_keepalive: now,
            last_cloud_success: now,
        },
        shell,
        cwd,
    ))
}

fn read_pty(
    session_id: String,
    mut reader: Box<dyn Read + Send>,
    sender: mpsc::Sender<LocalEvent>,
) {
    let mut buffer = vec![0_u8; MAX_OUTPUT_CHUNK];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                let _ = sender.blocking_send(LocalEvent::ReaderClosed { session_id });
                return;
            }
            Ok(count) => {
                if sender
                    .blocking_send(LocalEvent::Output {
                        session_id: session_id.clone(),
                        bytes: buffer[..count].to_vec(),
                    })
                    .is_err()
                {
                    return;
                }
            }
            Err(error) => {
                let _ = sender.blocking_send(LocalEvent::ReaderError {
                    session_id,
                    message: error.to_string(),
                });
                return;
            }
        }
    }
}

fn resolve_cwd(workspace_root: &Path, requested: Option<&str>) -> Result<PathBuf, String> {
    let candidate = requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace_root.join(path)
            }
        })
        .unwrap_or_else(|| workspace_root.to_path_buf());
    let canonical = fs::canonicalize(&candidate)
        .map_err(|error| format!("终端工作目录 {} 不可用：{error}", candidate.display()))?;
    if !canonical.is_dir() {
        return Err(format!("终端工作目录 {} 不是目录", canonical.display()));
    }
    Ok(shell_compatible_cwd(canonical))
}

#[cfg(windows)]
fn shell_compatible_cwd(path: PathBuf) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(stripped) = value.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{stripped}"));
    }
    if let Some(stripped) = value.strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }
    path
}

#[cfg(unix)]
fn shell_compatible_cwd(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
fn default_shell() -> String {
    std::env::var("COMSPEC")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "cmd.exe".into())
}

#[cfg(unix)]
fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|value| Path::new(value).is_absolute() && Path::new(value).is_file())
        .unwrap_or_else(|| "/bin/sh".into())
}

fn random_instance_id() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn short_id(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}

fn endpoint(cloud_url: &str, path: &str) -> String {
    format!("{}{}", cloud_url.trim_end_matches('/'), path)
}

async fn http_error(response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if body.trim().is_empty() {
        format!("HTTP {status}")
    } else {
        format!(
            "HTTP {status}: {}",
            body.chars().take(512).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_envelope_accepts_wrapped_and_direct_shapes() {
        let wrapped: CommandsResponse = serde_json::from_value(json!({ "commands": [] })).unwrap();
        let direct: CommandsResponse = serde_json::from_value(json!([])).unwrap();
        assert!(wrapped.into_commands().is_empty());
        assert!(direct.into_commands().is_empty());
    }

    #[test]
    fn cwd_defaults_to_workspace_and_accepts_relative_directories() {
        let root = std::env::temp_dir().join(format!("sculk-terminal-{}", random_instance_id()));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            resolve_cwd(&root, None).unwrap(),
            shell_compatible_cwd(fs::canonicalize(&root).unwrap())
        );
        assert_eq!(
            resolve_cwd(&root, Some("nested")).unwrap(),
            shell_compatible_cwd(fs::canonicalize(&nested).unwrap())
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pending_output_is_bounded() {
        let event = PendingEvent::output(1, b"hello");
        assert_eq!(event.output_len(), 5);
        assert_eq!(event.kind, "output");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pty_accepts_input_resize_and_returns_output() {
        let root = fs::canonicalize(std::env::temp_dir()).unwrap();
        let (sender, mut receiver) = mpsc::channel(LOCAL_EVENT_CHANNEL);
        let (session, _, _) = spawn_terminal(
            "terminal-integration-test",
            &root,
            StartPayload {
                cwd: None,
                cols: 80,
                rows: 24,
            },
            sender,
        )
        .unwrap();
        session
            .master
            .resize(PtySize {
                rows: 32,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        {
            let mut writer = session.writer.lock().unwrap();
            #[cfg(windows)]
            writer
                .write_all(b"\x1b[1;1Recho SCULK_PTY_READY\rexit\r")
                .unwrap();
            #[cfg(unix)]
            writer.write_all(b"echo SCULK_PTY_READY\nexit\n").unwrap();
            writer.flush().unwrap();
        }
        let observed = tokio::time::timeout(Duration::from_secs(10), async {
            let mut output = Vec::new();
            let mut exited = false;
            while let Some(event) = receiver.recv().await {
                match event {
                    LocalEvent::Output { bytes, .. } => output.extend(bytes),
                    LocalEvent::Exit { .. } => exited = true,
                    LocalEvent::ReaderClosed { .. } if exited => break,
                    _ => {}
                }
                if exited && String::from_utf8_lossy(&output).contains("SCULK_PTY_READY") {
                    break;
                }
            }
            (output, exited)
        })
        .await
        .expect("PTY did not exit in time");
        assert!(observed.1);
        assert!(String::from_utf8_lossy(&observed.0).contains("SCULK_PTY_READY"));
        drop(session);
    }
}
