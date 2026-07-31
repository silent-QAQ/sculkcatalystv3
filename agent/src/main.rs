// SPDX-License-Identifier: Apache-2.0

mod executor;
mod terminal;

use executor::{ExecutionContext, ExecutionResult, TaskArtifact, execute};
use rand::{RngCore, rngs::OsRng};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use terminal::{TerminalConfig, run as run_terminal_manager};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::mpsc,
    time::Instant,
};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const TASK_POLL_INTERVAL: Duration = Duration::from_secs(5);
const ALLOWED_PERMISSIONS: [&str; 4] = ["read", "write", "process", "full"];

#[derive(Serialize)]
struct ClaimRequest {
    pairing_code: String,
    name: String,
    platform: String,
    version: String,
    workspace_label: String,
    capabilities: Vec<String>,
    permissions: Vec<String>,
    fingerprint: String,
}

#[derive(Deserialize)]
struct ClaimResponse {
    agent_id: String,
    token: String,
    status: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentConfig {
    cloud_url: String,
    agent_id: String,
    token: String,
    name: String,
    workspace_label: String,
    fingerprint: String,
    capabilities: Vec<String>,
    permissions: Vec<String>,
    #[serde(default)]
    workspace_root: Option<PathBuf>,
}

/// A short-lived configuration distributed with a newly downloaded Agent.
///
/// It deliberately does not contain an Agent credential.  On the first `run`, the
/// pairing code is exchanged for a credential and this file is atomically replaced
/// with `AgentConfig`, so the pairing code is not retained after a successful claim.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapConfig {
    cloud_url: String,
    pairing_code: String,
    name: String,
    workspace_label: String,
    permissions: Vec<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    workspace_root: Option<PathBuf>,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

enum StoredConfig {
    Paired(AgentConfig),
    Bootstrap(BootstrapConfig),
}

#[derive(Deserialize)]
struct HeartbeatResponse {
    agent_id: String,
    status: String,
    active: bool,
    commands_available: bool,
}

#[derive(Deserialize)]
struct LeaseResponse {
    lease_token: String,
    task: LeasedTaskPayload,
}

#[derive(Deserialize)]
struct LeasedTaskPayload {
    id: String,
    operation: String,
    input: Value,
    #[serde(default)]
    resume: Option<LeasedTaskResumePayload>,
}

#[derive(Deserialize)]
struct LeasedTaskResumePayload {
    source_task_id: String,
    checkpoint_id: String,
    kind: String,
    payload: Value,
}

struct LeasedTask {
    id: String,
    operation: String,
    input: Value,
    lease_token: String,
    resume: Option<LeasedTaskResumePayload>,
}

#[derive(Serialize)]
struct LeaseTokenRequest<'a> {
    lease_token: &'a str,
}

#[derive(Serialize)]
struct TaskEventRequest<'a> {
    lease_token: &'a str,
    level: &'a str,
    message: &'a str,
    data: Value,
}

#[derive(Deserialize)]
struct TaskControlResponse {
    cancel_requested: bool,
}

#[derive(Serialize)]
struct TaskCheckpointRequest<'a> {
    lease_token: &'a str,
    checkpoint_key: &'a str,
    kind: &'a str,
    resumable: bool,
    payload: Value,
}

#[derive(Serialize)]
struct CompleteTaskRequest<'a> {
    lease_token: &'a str,
    status: &'a str,
    output: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    rollback_available: bool,
    artifacts: Vec<TaskArtifact>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedTaskResult {
    status: String,
    output: Value,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    error: String,
    rollback_available: bool,
    artifacts: Vec<TaskArtifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellExecInput {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default = "default_shell_timeout")]
    timeout_seconds: u64,
}

struct ShellChunk {
    stream: &'static str,
    text: String,
}

struct CapturedStream {
    bytes: Vec<u8>,
    truncated: bool,
}

fn default_shell_timeout() -> u64 {
    300
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

enum HeartbeatError {
    Terminal(String),
    Retryable(String),
}

#[derive(Debug)]
struct AgentApiError {
    message: String,
    retryable: bool,
}

impl std::fmt::Display for AgentApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Sculk Agent: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let options = parse_options(args.collect())?;
    match command.as_str() {
        "pair" => pair(options).await,
        "run" => run_agent(options).await,
        // Treat launching the executable without arguments as `run`. This makes a
        // bootstrap configuration usable by double-clicking the downloaded Agent.
        "" => run_agent(options).await,
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => Err(format!("unknown command '{command}'\n\n{}", usage())),
    }
}

fn parse_options(args: Vec<String>) -> Result<HashMap<String, String>, String> {
    let mut options = HashMap::new();
    let mut index = 0;
    while index < args.len() {
        let key = args[index]
            .strip_prefix("--")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("unexpected argument '{}'", args[index]))?;
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("--{key} requires a value"))?;
        if value.starts_with("--") {
            return Err(format!("--{key} requires a value"));
        }
        if options.insert(key.to_string(), value.clone()).is_some() {
            return Err(format!("--{key} was provided more than once"));
        }
        index += 2;
    }
    Ok(options)
}

fn required(options: &HashMap<String, String>, name: &str) -> Result<String, String> {
    options
        .get(name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("--{name} is required"))
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn validate_permissions(values: &[String]) -> Result<(), String> {
    if values.len() > ALLOWED_PERMISSIONS.len() {
        return Err("permissions are limited to read, write, process, and full".into());
    }
    let mut seen = Vec::new();
    for value in values {
        if !ALLOWED_PERMISSIONS.contains(&value.as_str()) || seen.contains(value) {
            return Err(
                "permissions are limited to unique read, write, process, and full values".into(),
            );
        }
        seen.push(value.clone());
    }
    Ok(())
}

fn validate_capabilities(values: &[String]) -> Result<(), String> {
    if values.is_empty() || values.len() > 32 {
        return Err("capabilities must contain between 1 and 32 entries".into());
    }
    let mut seen = Vec::new();
    for value in values {
        if value.is_empty()
            || value.len() > 64
            || !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
            || seen.contains(value)
        {
            return Err("capabilities contain an invalid or duplicate entry".into());
        }
        seen.push(value.clone());
    }
    Ok(())
}

fn normalize_cloud_url(value: &str) -> Result<String, String> {
    let mut url = Url::parse(value.trim()).map_err(|_| "--cloud must be a valid URL")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("--cloud cannot contain credentials, a query, or a fragment".into());
    }
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(
            "--cloud must use HTTPS (HTTP is allowed only for loopback development)".into(),
        );
    }
    let normalized_path = url.path().trim_end_matches('/').to_string();
    url.set_path(&normalized_path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn endpoint(cloud_url: &str, path: &str) -> String {
    format!("{}{}", cloud_url.trim_end_matches('/'), path)
}

fn default_config_path() -> PathBuf {
    if let Some(configured) = env::var_os("SCULK_AGENT_CONFIG").filter(|value| !value.is_empty()) {
        return PathBuf::from(configured);
    }
    #[cfg(windows)]
    {
        if let Some(root) = env::var_os("APPDATA") {
            return PathBuf::from(root).join("SculkCatalyst").join("agent.json");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(root)
                .join("sculk-catalyst")
                .join("agent.json");
        }
        if let Some(root) = env::var_os("HOME") {
            return PathBuf::from(root)
                .join(".config")
                .join("sculk-catalyst")
                .join("agent.json");
        }
    }
    PathBuf::from("sculk-agent.json")
}

fn sidecar_config_path() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let directory = executable.parent()?;
    let file_stem = executable.file_stem()?.to_str()?;
    Some(directory.join(format!("{file_stem}.json")))
}

fn config_path(options: &HashMap<String, String>) -> PathBuf {
    if let Some(path) = options.get("config") {
        return PathBuf::from(path);
    }

    // An explicit environment override remains stronger than portable
    // sidecar discovery, which is important for service managers.
    if let Some(path) = env::var_os("SCULK_AGENT_CONFIG").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }

    // Prefer a sidecar next to the executable when present. This keeps a
    // downloaded Windows/Linux Agent portable and prevents an unrelated global
    // config from hijacking a freshly downloaded one-click bootstrap.
    if let Some(path) = sidecar_config_path().filter(|path| path.exists()) {
        return path;
    }

    let default = default_config_path();
    if default.exists() {
        return default;
    }

    // A downloaded Agent and its bootstrap JSON can be kept together. Once it
    // is paired, this same file becomes the long-lived protected configuration.
    sidecar_config_path().unwrap_or(default)
}

fn random_fingerprint() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let digest = Sha256::digest(bytes);
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn pair(options: HashMap<String, String>) -> Result<(), String> {
    const PAIR_OPTIONS: [&str; 10] = [
        "cloud",
        "code",
        "name",
        "workspace",
        "workspace-root",
        "permissions",
        "capabilities",
        "platform",
        "version",
        "config",
    ];
    for key in options.keys() {
        if !PAIR_OPTIONS.contains(&key.as_str()) {
            return Err(format!("pair does not support --{key}"));
        }
    }
    let cloud_url = normalize_cloud_url(&required(&options, "cloud")?)?;
    let pairing_code = required(&options, "code")?;
    let name = required(&options, "name")?;
    let workspace_label = required(&options, "workspace")?;
    let permissions = split_list(&required(&options, "permissions")?);
    validate_permissions(&permissions)?;
    let workspace_root = options
        .get("workspace-root")
        .map(PathBuf::from)
        .map(|path| {
            fs::canonicalize(&path).map_err(|error| {
                format!("workspace root {} is unavailable: {error}", path.display())
            })
        })
        .transpose()?;
    if workspace_root.as_ref().is_some_and(|path| !path.is_dir()) {
        return Err("--workspace-root must reference an existing directory".into());
    }
    if permissions.iter().any(|item| item == "full") && workspace_root.is_none() {
        return Err(
            "full shell mode requires --workspace-root as its default working directory".into(),
        );
    }
    let capabilities = options.get("capabilities").map_or_else(
        || {
            if workspace_root.is_some() {
                vec![
                    "heartbeat".into(),
                    "tasks-v1".into(),
                    "task-checkpoints-v1".into(),
                    "shell-v1".into(),
                    "terminal-v1".into(),
                ]
            } else {
                vec!["heartbeat".into()]
            }
        },
        |value| split_list(value),
    );
    validate_capabilities(&capabilities)?;
    let platform = options
        .get("platform")
        .cloned()
        .unwrap_or_else(|| format!("{}-{}", env::consts::OS, env::consts::ARCH));
    let version = options
        .get("version")
        .cloned()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let fingerprint = random_fingerprint();
    let path = config_path(&options);
    if path.exists() {
        return Err(format!(
            "configuration already exists at {}; use a different --config path before pairing",
            path.display()
        ));
    }

    let request = ClaimRequest {
        pairing_code,
        name: name.clone(),
        platform,
        version,
        workspace_label: workspace_label.clone(),
        capabilities: capabilities.clone(),
        permissions: permissions.clone(),
        fingerprint: fingerprint.clone(),
    };
    let response = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("failed to initialize HTTP client: {error}"))?
        .post(endpoint(&cloud_url, "/api/cloud/agent-pairings/claim"))
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("pairing request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    let claimed: ClaimResponse = response
        .json()
        .await
        .map_err(|error| format!("cloud returned an invalid pairing response: {error}"))?;
    if !claimed.token.starts_with("sca_") || claimed.status != "claimed" {
        return Err("cloud returned an invalid agent credential".into());
    }
    let config = AgentConfig {
        cloud_url,
        agent_id: claimed.agent_id,
        token: claimed.token,
        name,
        workspace_label,
        fingerprint,
        capabilities,
        permissions,
        workspace_root,
    };
    write_config(&path, &config)?;
    println!(
        "配对声明已提交并安全保存到 {}。\nAgent 指纹：{}\n请在 Cloud 控制台核对该指纹并确认；sca_ 凭据不会输出到日志。",
        path.display(),
        config.fingerprint
    );
    Ok(())
}

fn required_bootstrap_value(value: String, name: &str) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(format!("bootstrap config requires a non-empty {name}"));
    }
    Ok(value)
}

fn normalize_config_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn validate_bootstrap_config(config: BootstrapConfig) -> Result<BootstrapConfig, String> {
    let workspace_root = config
        .workspace_root
        .map(|path| {
            fs::canonicalize(&path).map_err(|error| {
                format!("workspace root {} is unavailable: {error}", path.display())
            })
        })
        .transpose()?;
    if workspace_root.as_ref().is_some_and(|path| !path.is_dir()) {
        return Err("bootstrap workspace_root must reference an existing directory".into());
    }

    let permissions = normalize_config_list(config.permissions);
    validate_permissions(&permissions)?;
    if permissions.iter().any(|item| item == "full") && workspace_root.is_none() {
        return Err(
            "full shell mode requires bootstrap workspace_root as its default working directory"
                .into(),
        );
    }
    let capabilities = if config.capabilities.is_empty() {
        if workspace_root.is_some() {
            vec![
                "heartbeat".into(),
                "tasks-v1".into(),
                "task-checkpoints-v1".into(),
                "shell-v1".into(),
                "terminal-v1".into(),
            ]
        } else {
            vec!["heartbeat".into()]
        }
    } else {
        normalize_config_list(config.capabilities)
    };
    validate_capabilities(&capabilities)?;

    Ok(BootstrapConfig {
        cloud_url: normalize_cloud_url(&config.cloud_url)?,
        pairing_code: required_bootstrap_value(config.pairing_code, "pairing_code")?,
        name: required_bootstrap_value(config.name, "name")?,
        workspace_label: required_bootstrap_value(config.workspace_label, "workspace_label")?,
        permissions,
        capabilities,
        workspace_root,
        platform: config
            .platform
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        version: config
            .version
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    })
}

async fn claim_bootstrap(bootstrap: BootstrapConfig) -> Result<AgentConfig, String> {
    let bootstrap = validate_bootstrap_config(bootstrap)?;
    let fingerprint = random_fingerprint();
    let request = ClaimRequest {
        pairing_code: bootstrap.pairing_code,
        name: bootstrap.name.clone(),
        platform: bootstrap
            .platform
            .unwrap_or_else(|| format!("{}-{}", env::consts::OS, env::consts::ARCH)),
        version: bootstrap
            .version
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        workspace_label: bootstrap.workspace_label.clone(),
        capabilities: bootstrap.capabilities.clone(),
        permissions: bootstrap.permissions.clone(),
        fingerprint: fingerprint.clone(),
    };
    let response = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("failed to initialize HTTP client: {error}"))?
        .post(endpoint(
            &bootstrap.cloud_url,
            "/api/cloud/agent-pairings/claim",
        ))
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("pairing request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    let claimed: ClaimResponse = response
        .json()
        .await
        .map_err(|error| format!("cloud returned an invalid pairing response: {error}"))?;
    if !claimed.token.starts_with("sca_") || claimed.status != "claimed" {
        return Err("cloud returned an invalid agent credential".into());
    }
    Ok(AgentConfig {
        cloud_url: bootstrap.cloud_url,
        agent_id: claimed.agent_id,
        token: claimed.token,
        name: bootstrap.name,
        workspace_label: bootstrap.workspace_label,
        fingerprint,
        capabilities: bootstrap.capabilities,
        permissions: bootstrap.permissions,
        workspace_root: bootstrap.workspace_root,
    })
}

fn write_config(path: &Path, config: &AgentConfig) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create config directory: {error}"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("agent.json");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("failed to encode agent config: {error}"))?;

    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("failed to create protected config: {error}"))?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("failed to commit agent config: {error}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to protect agent config: {error}"))?;
    }
    Ok(())
}

fn read_config(path: &Path) -> Result<StoredConfig, String> {
    let bytes = fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!(
                "no Agent configuration found at {}. Put a bootstrap JSON beside the executable, set SCULK_AGENT_CONFIG, or run `sculk-agent pair --cloud <url> --code <code> ...`",
                path.display()
            )
        } else {
            format!("failed to read config {}: {error}", path.display())
        }
    })?;

    match serde_json::from_slice::<AgentConfig>(&bytes) {
        Ok(config) => {
            if !config.token.starts_with("sca_") {
                return Err("agent config contains an invalid credential".into());
            }
            Ok(StoredConfig::Paired(config))
        }
        Err(paired_error) => serde_json::from_slice::<BootstrapConfig>(&bytes)
            .map(StoredConfig::Bootstrap)
            .map_err(|bootstrap_error| {
                format!(
                    "invalid Agent config {}: expected a paired credential or bootstrap pairing config ({paired_error}; {bootstrap_error})",
                    path.display()
                )
            }),
    }
}

async fn run_agent(options: HashMap<String, String>) -> Result<(), String> {
    for key in options.keys() {
        if key != "config" {
            return Err(format!("run does not support --{key}"));
        }
    }
    let path = config_path(&options);
    let mut config = match read_config(&path)? {
        StoredConfig::Paired(config) => config,
        StoredConfig::Bootstrap(bootstrap) => {
            println!("Claiming the one-time bootstrap pairing credential...");
            let config = claim_bootstrap(bootstrap).await?;
            // This replacement is the point at which the one-time pairing code is
            // removed. Do not log either code or the received Agent token.
            write_config(&path, &config)?;
            println!(
                "Agent paired and saved to {}. Fingerprint: {}",
                path.display(),
                config.fingerprint
            );
            config
        }
    };
    config.cloud_url = normalize_cloud_url(&config.cloud_url)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|error| format!("failed to initialize HTTP client: {error}"))?;
    let state_dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join("agent-state");
    let task_capable = config.capabilities.iter().any(|item| item == "tasks-v1");
    let shell_capable = config.capabilities.iter().any(|item| item == "shell-v1")
        && config.permissions.iter().any(|item| item == "full");
    let terminal_capable =
        shell_capable && config.capabilities.iter().any(|item| item == "terminal-v1");
    if task_capable && config.workspace_root.is_none() {
        return Err("tasks-v1 requires a configured workspace_root; pair this Agent again".into());
    }
    let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut task_interval = tokio::time::interval(TASK_POLL_INTERVAL);
    task_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut previous_status = String::new();
    let mut active = false;
    let mut task_handle: Option<tokio::task::JoinHandle<Result<(), String>>> = None;
    if terminal_capable {
        let terminal_config = TerminalConfig {
            cloud_url: config.cloud_url.clone(),
            token: config.token.clone(),
            workspace_root: config
                .workspace_root
                .clone()
                .ok_or_else(|| "terminal-v1 requires a configured workspace_root".to_string())?,
        };
        tokio::spawn(run_terminal_manager(client.clone(), terminal_config));
    }
    println!(
        "Sculk Agent 正在运行；仅建立出站连接，不监听任何入站端口。{}",
        if shell_capable {
            " Full Shell 已启用，命令权限等同于当前操作系统账号。"
        } else if task_capable {
            " 结构化任务执行器已启用，Full Shell 未启用。"
        } else {
            " 当前配置仅启用心跳。"
        }
    );
    loop {
        if let Some(mut handle) = task_handle.take() {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    handle.abort();
                    println!("Sculk Agent 已停止；正在执行的子进程已请求终止。");
                    return Ok(());
                }
                result = &mut handle => {
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => eprintln!("任务同步失败：{error}"),
                        Err(error) if error.is_cancelled() => {}
                        Err(error) => eprintln!("任务执行线程异常：{error}"),
                    }
                }
                _ = heartbeat_interval.tick() => {
                    active = handle_heartbeat(&client, &config, &mut previous_status).await?;
                    task_handle = Some(handle);
                }
            }
            continue;
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Sculk Agent 已停止。");
                return Ok(());
            }
            _ = heartbeat_interval.tick() => {
                active = handle_heartbeat(&client, &config, &mut previous_status).await?;
            }
            _ = task_interval.tick(), if active && task_capable => {
                match lease_task(&client, &config).await {
                    Ok(Some(task)) => {
                        let client = client.clone();
                        let config = config.clone();
                        let state_dir = state_dir.clone();
                        task_handle = Some(tokio::spawn(async move {
                            process_leased_task(&client, &config, &state_dir, task, shell_capable).await
                        }));
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("领取任务失败：{error}"),
                }
            }
        }
    }
}

async fn handle_heartbeat(
    client: &Client,
    config: &AgentConfig,
    previous_status: &mut String,
) -> Result<bool, String> {
    match send_heartbeat(client, config).await {
        Ok(heartbeat) => {
            if heartbeat.agent_id != config.agent_id {
                return Err("cloud heartbeat identity does not match local config".into());
            }
            if heartbeat.status != *previous_status {
                if heartbeat.active {
                    println!(
                        "Agent 已由用户确认，心跳在线；任务队列状态：{}。",
                        if heartbeat.commands_available {
                            "有待处理任务"
                        } else {
                            "空闲"
                        }
                    );
                } else {
                    println!("Agent 已领取配对，正在等待用户确认。");
                }
                *previous_status = heartbeat.status;
            }
            Ok(heartbeat.active)
        }
        Err(HeartbeatError::Terminal(message)) => Err(message),
        Err(HeartbeatError::Retryable(message)) => {
            eprintln!("心跳暂时失败：{message}");
            Ok(false)
        }
    }
}

async fn lease_task(client: &Client, config: &AgentConfig) -> Result<Option<LeasedTask>, String> {
    let response = client
        .post(endpoint(&config.cloud_url, "/api/cloud/agent/tasks/lease"))
        .bearer_auth(&config.token)
        .json(&json!({}))
        .send()
        .await
        .map_err(|error| format!("任务租约请求失败：{error}"))?;
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    let response = response
        .json::<LeaseResponse>()
        .await
        .map_err(|error| format!("Cloud 返回了无效的任务租约：{error}"))?;
    let payload = response.task;
    Ok(Some(LeasedTask {
        id: payload.id,
        operation: payload.operation,
        input: payload.input,
        lease_token: response.lease_token,
        resume: payload.resume,
    }))
}

async fn process_leased_task(
    client: &Client,
    config: &AgentConfig,
    state_dir: &Path,
    task: LeasedTask,
    shell_capable: bool,
) -> Result<(), String> {
    start_task_reliably(client, config, &task).await?;
    let _ = send_task_event(
        client,
        config,
        &task,
        "info",
        "Agent 已开始执行任务",
        json!({ "operation": task.operation }),
    )
    .await;

    let cached = load_cached_result(state_dir, &task.id)?;
    let mut checkpoint_required = false;
    let result = if let Some(cached) = cached {
        let _ = send_task_event(
            client,
            config,
            &task,
            "warn",
            "检测到本机幂等结果，未重复执行任务",
            json!({}),
        )
        .await;
        checkpoint_required = cached.status != "cancelled";
        cached
    } else if let Some(resume) = &task.resume {
        let restored = restore_checkpoint_result(resume)?;
        let _ = send_task_event(
            client,
            config,
            &task,
            "info",
            "已从任务检查点恢复最终结果，未重复执行操作",
            json!({
                "source_task_id": resume.source_task_id,
                "checkpoint_id": resume.checkpoint_id,
            }),
        )
        .await;
        save_cached_result(state_dir, &task.id, &restored)?;
        restored
    } else {
        let result =
            execute_with_lease_watchdog(client, config, state_dir, &task, shell_capable).await?;
        save_cached_result(state_dir, &task.id, &result)?;
        checkpoint_required = result.status != "cancelled";
        result
    };

    if checkpoint_required
        && let Err(error) = submit_result_checkpoint_reliably(client, config, &task, &result).await
    {
        // A successful completion is itself durable. Do not discard a finished
        // operation merely because the optional pre-completion checkpoint could
        // not be persisted during a transient Cloud outage.
        eprintln!("提交任务检查点失败，将继续提交最终结果：{error}");
    }

    complete_task_reliably(client, config, &task, &result).await?;
    delete_cached_result(state_dir, &task.id);
    if let Some(resume) = &task.resume {
        delete_cached_result(state_dir, &resume.source_task_id);
    }
    Ok(())
}

fn restore_checkpoint_result(resume: &LeasedTaskResumePayload) -> Result<CachedTaskResult, String> {
    if resume.kind != "result" {
        return Err("Cloud 返回了不支持恢复的任务检查点类型".into());
    }
    let result: CachedTaskResult = serde_json::from_value(resume.payload.clone())
        .map_err(|error| format!("Cloud 任务检查点已损坏：{error}"))?;
    if result.status != "succeeded" {
        return Err("Cloud 任务检查点不是可恢复的成功结果".into());
    }
    Ok(result)
}

async fn execute_with_lease_watchdog(
    client: &Client,
    config: &AgentConfig,
    state_dir: &Path,
    task: &LeasedTask,
    shell_capable: bool,
) -> Result<CachedTaskResult, String> {
    let operation = execute_leased_operation(client, config, state_dir, task, shell_capable);
    tokio::pin!(operation);
    let mut control = tokio::time::interval_at(Instant::now(), Duration::from_secs(1));
    control.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut progress = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(20),
        Duration::from_secs(20),
    );
    progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_renewal = Instant::now();
    loop {
        tokio::select! {
            biased;
            result = &mut operation => return Ok(result),
            _ = control.tick() => {
                match poll_task_control(client, config, task).await {
                    Ok(response) => {
                        last_renewal = Instant::now();
                        if response.cancel_requested {
                            return Ok(cancelled_result());
                        }
                    }
                    Err(error) if !error.retryable => {
                        return Err(format!("任务租约或控制通道已失效，本机执行已终止：{error}"));
                    }
                    Err(error) => {
                        eprintln!("任务控制轮询暂时失败：{error}");
                        if last_renewal.elapsed() >= Duration::from_secs(50) {
                            return Err("任务超过 50 秒未能连接控制通道，本机执行已终止".into());
                        }
                    }
                }
            }
            _ = progress.tick() => {
                match send_task_event(
                    client,
                    config,
                    task,
                    "info",
                    "任务仍在执行",
                    json!({}),
                ).await {
                    Ok(()) => last_renewal = Instant::now(),
                    Err(error) if !error.retryable => {
                        return Err(format!("任务租约已失效，本机执行已终止：{error}"));
                    }
                    Err(error) => {
                        eprintln!("任务续租暂时失败：{error}");
                        if last_renewal.elapsed() >= Duration::from_secs(50) {
                            return Err("任务超过 50 秒未能续租，本机执行已终止".into());
                        }
                    }
                }
            }
        }
    }
}

async fn execute_leased_operation(
    client: &Client,
    config: &AgentConfig,
    state_dir: &Path,
    task: &LeasedTask,
    shell_capable: bool,
) -> CachedTaskResult {
    if task.operation == "shell.exec" {
        if !shell_capable {
            return failed_result("该 Agent 未由主机安装者启用 Full Shell");
        }
        return execute_shell(client, config, task).await;
    }
    let Some(workspace_root) = config.workspace_root.as_deref() else {
        return failed_result("Agent 没有配置任务工作区");
    };
    let workspace_root = workspace_root.to_path_buf();
    let state_dir = state_dir.to_path_buf();
    let workspace_label = config.workspace_label.clone();
    let permissions = config.permissions.clone();
    let task_id = task.id.clone();
    let operation = task.operation.clone();
    let input = task.input.clone();
    let executed = tokio::task::spawn_blocking(move || {
        let context = ExecutionContext {
            workspace_root: &workspace_root,
            state_dir: &state_dir,
            workspace_label: &workspace_label,
            permissions: &permissions,
        };
        execute(&context, &task_id, &operation, input)
    })
    .await;
    let executed = match executed {
        Ok(result) => result,
        Err(error) => return failed_result(&format!("结构化任务执行线程异常：{error}")),
    };
    match executed {
        Ok(ExecutionResult {
            output,
            rollback_available,
            artifacts,
        }) => CachedTaskResult {
            status: "succeeded".into(),
            output,
            error: String::new(),
            rollback_available,
            artifacts,
        },
        Err(error) => failed_result(&error),
    }
}

fn failed_result(error: &str) -> CachedTaskResult {
    CachedTaskResult {
        status: "failed".into(),
        output: json!({}),
        error: truncate_text(&redact_sensitive_text(error), 4000),
        rollback_available: false,
        artifacts: vec![],
    }
}

fn cancelled_result() -> CachedTaskResult {
    CachedTaskResult {
        status: "cancelled".into(),
        output: json!({ "cancelled": true }),
        error: "任务已按用户请求终止".into(),
        rollback_available: false,
        artifacts: vec![],
    }
}

async fn start_task(
    client: &Client,
    config: &AgentConfig,
    task: &LeasedTask,
) -> Result<(), AgentApiError> {
    let response = client
        .post(endpoint(
            &config.cloud_url,
            &format!("/api/cloud/agent/tasks/{}/start", task.id),
        ))
        .bearer_auth(&config.token)
        .json(&LeaseTokenRequest {
            lease_token: &task.lease_token,
        })
        .send()
        .await
        .map_err(|error| AgentApiError {
            message: format!("启动任务失败：{error}"),
            retryable: true,
        })?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(agent_api_response_error(response).await)
    }
}

async fn start_task_reliably(
    client: &Client,
    config: &AgentConfig,
    task: &LeasedTask,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(50);
    loop {
        match start_task(client, config, task).await {
            Ok(()) => return Ok(()),
            Err(error) if error.retryable && Instant::now() < deadline => {
                eprintln!("启动任务请求暂时失败，正在重试：{error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

async fn poll_task_control(
    client: &Client,
    config: &AgentConfig,
    task: &LeasedTask,
) -> Result<TaskControlResponse, AgentApiError> {
    let response = client
        .post(endpoint(
            &config.cloud_url,
            &format!("/api/cloud/agent/tasks/{}/control", task.id),
        ))
        .bearer_auth(&config.token)
        .json(&LeaseTokenRequest {
            lease_token: &task.lease_token,
        })
        .send()
        .await
        .map_err(|error| AgentApiError {
            message: format!("轮询任务控制状态失败：{error}"),
            retryable: true,
        })?;
    if !response.status().is_success() {
        return Err(agent_api_response_error(response).await);
    }
    response
        .json::<TaskControlResponse>()
        .await
        .map_err(|error| AgentApiError {
            message: format!("Cloud 返回了无效的任务控制状态：{error}"),
            retryable: false,
        })
}

async fn send_task_event(
    client: &Client,
    config: &AgentConfig,
    task: &LeasedTask,
    level: &str,
    message: &str,
    data: Value,
) -> Result<(), AgentApiError> {
    let response = client
        .post(endpoint(
            &config.cloud_url,
            &format!("/api/cloud/agent/tasks/{}/events", task.id),
        ))
        .bearer_auth(&config.token)
        .json(&TaskEventRequest {
            lease_token: &task.lease_token,
            level,
            message,
            data,
        })
        .send()
        .await
        .map_err(|error| AgentApiError {
            message: format!("回传任务事件失败：{error}"),
            retryable: true,
        })?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(agent_api_response_error(response).await)
    }
}

async fn submit_result_checkpoint(
    client: &Client,
    config: &AgentConfig,
    task: &LeasedTask,
    result: &CachedTaskResult,
) -> Result<(), AgentApiError> {
    let payload = serde_json::to_value(result).map_err(|error| AgentApiError {
        message: format!("编码任务检查点失败：{error}"),
        retryable: false,
    })?;
    let response = client
        .post(endpoint(
            &config.cloud_url,
            &format!("/api/cloud/agent/tasks/{}/checkpoints", task.id),
        ))
        .bearer_auth(&config.token)
        .json(&TaskCheckpointRequest {
            lease_token: &task.lease_token,
            checkpoint_key: "result-v1",
            kind: "result",
            resumable: result.status == "succeeded",
            payload,
        })
        .send()
        .await
        .map_err(|error| AgentApiError {
            message: format!("提交任务检查点失败：{error}"),
            retryable: true,
        })?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(agent_api_response_error(response).await)
    }
}

async fn submit_result_checkpoint_reliably(
    client: &Client,
    config: &AgentConfig,
    task: &LeasedTask,
    result: &CachedTaskResult,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match submit_result_checkpoint(client, config, task, result).await {
            Ok(()) => return Ok(()),
            Err(error) if error.retryable && Instant::now() < deadline => {
                eprintln!("任务检查点暂时无法提交，正在重试：{error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

async fn complete_task(
    client: &Client,
    config: &AgentConfig,
    task: &LeasedTask,
    result: &CachedTaskResult,
) -> Result<(), AgentApiError> {
    let response = client
        .post(endpoint(
            &config.cloud_url,
            &format!("/api/cloud/agent/tasks/{}/complete", task.id),
        ))
        .bearer_auth(&config.token)
        .json(&CompleteTaskRequest {
            lease_token: &task.lease_token,
            status: &result.status,
            output: result.output.clone(),
            error: (!result.error.is_empty()).then_some(result.error.as_str()),
            rollback_available: result.rollback_available,
            artifacts: result.artifacts.clone(),
        })
        .send()
        .await
        .map_err(|error| AgentApiError {
            message: format!("提交任务结果失败：{error}"),
            retryable: true,
        })?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(agent_api_response_error(response).await)
    }
}

async fn complete_task_reliably(
    client: &Client,
    config: &AgentConfig,
    task: &LeasedTask,
    result: &CachedTaskResult,
) -> Result<(), String> {
    let mut last_renewal = Instant::now();
    let mut first_attempt = true;
    loop {
        if !first_attempt {
            match send_task_event(
                client,
                config,
                task,
                "info",
                "正在重试提交任务最终结果",
                json!({}),
            )
            .await
            {
                Ok(()) => last_renewal = Instant::now(),
                Err(error) => eprintln!("最终结果重试续租失败：{error}"),
            }
        }
        first_attempt = false;
        match complete_task(client, config, task, result).await {
            Ok(()) => return Ok(()),
            Err(error) if error.retryable && last_renewal.elapsed() < Duration::from_secs(50) => {
                eprintln!("提交任务最终结果暂时失败，正在重试：{error}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

async fn execute_shell(
    client: &Client,
    config: &AgentConfig,
    task: &LeasedTask,
) -> CachedTaskResult {
    let input: ShellExecInput = match serde_json::from_value(task.input.clone()) {
        Ok(input) => input,
        Err(error) => return failed_result(&format!("Shell 任务参数无效：{error}")),
    };
    if input.command.trim().is_empty()
        || input.command.chars().count() > 32_768
        || input.command.contains('\0')
    {
        return failed_result("Shell 命令为空、过长或包含 NUL 字符");
    }
    if !(1..=1800).contains(&input.timeout_seconds) {
        return failed_result("Shell 超时需要在 1-1800 秒之间");
    }
    let Some(default_root) = config.workspace_root.as_ref() else {
        return failed_result("Full Shell 没有配置默认工作目录");
    };
    let requested_cwd = input
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested_cwd.is_some_and(|value| value.len() > 1024 || value.contains('\0')) {
        return failed_result("Shell 工作目录无效或过长");
    }
    let cwd = requested_cwd.map_or_else(
        || fs::canonicalize(default_root),
        |value| {
            let path = PathBuf::from(value);
            fs::canonicalize(if path.is_absolute() {
                path
            } else {
                default_root.join(path)
            })
        },
    );
    let cwd = match cwd {
        Ok(cwd) if cwd.is_dir() => cwd,
        Ok(_) => return failed_result("Shell 工作目录不是目录"),
        Err(error) => return failed_result(&format!("无法访问 Shell 工作目录：{error}")),
    };

    let mut command = shell_command(&input.command);
    command
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return failed_result(&format!("无法启动 Shell：{error}")),
    };
    let Some(pid) = child.id() else {
        return failed_result("无法取得 Shell 进程 ID");
    };
    let mut process_guard = ProcessTreeGuard::attach(pid, &child);
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => return failed_result("无法捕获 Shell 标准输出"),
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => return failed_result("无法捕获 Shell 标准错误"),
    };
    let (sender, mut receiver) = mpsc::channel(64);
    let stdout_reader = tokio::spawn(read_shell_stream(stdout, "stdout", sender.clone()));
    let stderr_reader = tokio::spawn(read_shell_stream(stderr, "stderr", sender.clone()));
    drop(sender);

    let deadline = Instant::now() + Duration::from_secs(input.timeout_seconds);
    let mut exit_status = None;
    let mut timed_out = false;
    let mut forced_cleanup_at = None;
    let mut event_count = 0_usize;
    while exit_status.is_none() || !receiver.is_closed() || !receiver.is_empty() {
        tokio::select! {
            chunk = receiver.recv(), if !receiver.is_closed() || !receiver.is_empty() => {
                if let Some(chunk) = chunk
                    && event_count < 1900
                {
                    let text = truncate_text(&redact_sensitive_text(&chunk.text), 1800);
                    let level = if chunk.stream == "stderr" { "warn" } else { "info" };
                    match send_task_event(
                        client,
                        config,
                        task,
                        level,
                        &text,
                        json!({ "stream": chunk.stream }),
                    ).await {
                        Ok(()) => event_count += 1,
                        Err(error) => eprintln!("Shell 输出事件回传失败：{error}"),
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if exit_status.is_none() {
                    match child.try_wait() {
                        Ok(Some(status)) => exit_status = Some(status),
                        Ok(None) => {}
                        Err(error) => {
                            process_guard.terminate();
                            let _ = child.kill().await;
                            return failed_result(&format!("无法等待 Shell 进程：{error}"));
                        }
                    }
                }
                let now = Instant::now();
                if !timed_out && now >= deadline {
                    timed_out = true;
                    process_guard.terminate();
                    if exit_status.is_none() {
                        let _ = child.kill().await;
                        exit_status = child.wait().await.ok();
                    }
                    forced_cleanup_at = Some(now);
                } else if timed_out
                    && forced_cleanup_at.is_some_and(|started| now.duration_since(started) >= Duration::from_secs(2))
                    && (!receiver.is_closed() || !receiver.is_empty())
                {
                    stdout_reader.abort();
                    stderr_reader.abort();
                    break;
                }
            }
        }
    }
    if !timed_out {
        process_guard.disarm();
    }
    let stdout = match stdout_reader.await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return failed_result(&error),
        Err(error) if timed_out && error.is_cancelled() => CapturedStream {
            bytes: vec![],
            truncated: true,
        },
        Err(error) => return failed_result(&format!("读取 Shell 标准输出失败：{error}")),
    };
    let stderr = match stderr_reader.await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return failed_result(&error),
        Err(error) if timed_out && error.is_cancelled() => CapturedStream {
            bytes: vec![],
            truncated: true,
        },
        Err(error) => return failed_result(&format!("读取 Shell 标准错误失败：{error}")),
    };
    let exit_code = exit_status.and_then(|status| status.code());
    let stdout_text = redact_sensitive_text(&String::from_utf8_lossy(&stdout.bytes));
    let stderr_text = redact_sensitive_text(&String::from_utf8_lossy(&stderr.bytes));
    let output = json!({
        "exit_code": exit_code,
        "timed_out": timed_out,
        "stdout": stdout_text,
        "stderr": stderr_text,
        "stdout_truncated": stdout.truncated,
        "stderr_truncated": stderr.truncated,
    });
    let success = !timed_out && exit_status.is_some_and(|status| status.success());
    CachedTaskResult {
        status: if success { "succeeded" } else { "failed" }.into(),
        output,
        error: if success {
            String::new()
        } else if timed_out {
            format!("Shell 命令超过 {} 秒并已终止", input.timeout_seconds)
        } else {
            format!(
                "Shell 命令退出码为 {}",
                exit_code.map_or_else(|| "未知".into(), |code| code.to_string())
            )
        },
        rollback_available: false,
        artifacts: vec![],
    }
}

fn shell_command(script: &str) -> Command {
    #[cfg(windows)]
    {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ]);
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = Command::new("/bin/sh");
        command.args(["-lc", script]);
        command
    }
}

async fn read_shell_stream<R>(
    mut reader: R,
    stream: &'static str,
    sender: mpsc::Sender<ShellChunk>,
) -> Result<CapturedStream, String>
where
    R: AsyncRead + Unpin,
{
    // Keep the final JSON comfortably below the Cloud 1 MiB output limit even
    // when control characters need JSON escaping. The event stream remains the
    // primary source for long command output.
    const CAPTURE_LIMIT: usize = 80_000;
    let mut captured = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取 Shell {stream} 失败：{error}"))?;
        if read == 0 {
            break;
        }
        if captured.len() < CAPTURE_LIMIT {
            let remaining = CAPTURE_LIMIT - captured.len();
            captured.extend_from_slice(&buffer[..read.min(remaining)]);
            if read > remaining {
                truncated = true;
            }
            let _ = sender
                .send(ShellChunk {
                    stream,
                    text: String::from_utf8_lossy(&buffer[..read]).into_owned(),
                })
                .await;
        } else {
            truncated = true;
        }
    }
    Ok(CapturedStream {
        bytes: captured,
        truncated,
    })
}

struct ProcessTreeGuard {
    pid: u32,
    armed: bool,
    #[cfg(windows)]
    job: isize,
}

impl ProcessTreeGuard {
    fn attach(pid: u32, child: &tokio::process::Child) -> Self {
        #[cfg(not(windows))]
        let _ = child;
        Self {
            pid,
            armed: true,
            #[cfg(windows)]
            job: attach_windows_job(child),
        }
    }

    fn terminate(&mut self) {
        if self.armed {
            #[cfg(unix)]
            terminate_process_tree(self.pid);
            #[cfg(windows)]
            terminate_windows_tree(self.pid, self.job);
            self.armed = false;
        }
    }

    fn disarm(&mut self) {
        if self.armed {
            #[cfg(windows)]
            release_windows_job(self.job);
            self.armed = false;
        }
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(unix)]
fn terminate_process_tree(pid: u32) {
    // The shell is placed in its own process group before spawn.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

#[cfg(windows)]
fn attach_windows_job(child: &tokio::process::Child) -> isize {
    use std::{mem, ptr};
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
    };
    unsafe {
        let job = CreateJobObjectW(ptr::null(), ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &information as *const _ as *const _,
            mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) != 0;
        let assigned = child
            .raw_handle()
            .is_some_and(|handle| configured && AssignProcessToJobObject(job, handle) != 0);
        if !assigned {
            CloseHandle(job);
            return 0;
        }
        job as isize
    }
}

#[cfg(windows)]
fn terminate_windows_tree(pid: u32, job: isize) {
    use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::TerminateJobObject};
    if job != 0 {
        unsafe {
            let handle = job as *mut std::ffi::c_void;
            TerminateJobObject(handle, 1);
            CloseHandle(handle);
        }
        return;
    }
    let _ = std::process::Command::new("taskkill.exe")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(windows)]
fn release_windows_job(job: isize) {
    use std::mem;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::JobObjects::{
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
    };
    if job == 0 {
        return;
    }
    unsafe {
        let handle = job as *mut std::ffi::c_void;
        let information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &information as *const _ as *const _,
            mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        CloseHandle(handle);
    }
}

fn truncate_text(value: &str, maximum_chars: usize) -> String {
    value.chars().take(maximum_chars).collect()
}

fn redact_sensitive_text(value: &str) -> String {
    let mut redacted = value.replace("Bearer ", "Authorization-redacted ");
    for prefix in ["sca_", "scs_", "sk-sc_"] {
        while let Some(start) = redacted.find(prefix) {
            let tail = &redacted[start..];
            let end = tail
                .char_indices()
                .skip(prefix.chars().count())
                .find_map(|(index, character)| {
                    character.is_whitespace().then_some(index).or_else(|| {
                        ['\'', '"', '`', ',', ';', ')', ']', '}']
                            .contains(&character)
                            .then_some(index)
                    })
                })
                .unwrap_or(tail.len());
            redacted.replace_range(start..start + end, "[REDACTED]");
        }
    }
    redacted
}

fn cached_result_path(state_dir: &Path, task_id: &str) -> Result<PathBuf, String> {
    let task_id = task_id.trim();
    if task_id.len() < 8
        || task_id.len() > 64
        || !task_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("Cloud 返回了无效的任务 ID".into());
    }
    Ok(state_dir.join("results").join(format!("{task_id}.json")))
}

fn load_cached_result(state_dir: &Path, task_id: &str) -> Result<Option<CachedTaskResult>, String> {
    let path = cached_result_path(state_dir, task_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|error| format!("无法读取本机任务幂等记录：{error}"))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("本机任务幂等记录已损坏：{error}"))
}

fn save_cached_result(
    state_dir: &Path,
    task_id: &str,
    result: &CachedTaskResult,
) -> Result<(), String> {
    let path = cached_result_path(state_dir, task_id)?;
    let parent = path
        .parent()
        .ok_or_else(|| "任务记录缺少父目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建任务记录目录：{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("无法保护任务记录目录：{error}"))?;
    }
    let bytes = serde_json::to_vec(result).map_err(|error| format!("无法编码任务结果：{error}"))?;
    if bytes.len() > 1_048_576 {
        return Err("任务结果超过 1 MiB，已拒绝提交".into());
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("无法创建任务幂等记录：{error}"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("无法保存任务幂等记录：{error}"))
}

fn delete_cached_result(state_dir: &Path, task_id: &str) {
    if let Ok(path) = cached_result_path(state_dir, task_id) {
        let _ = fs::remove_file(path);
    }
}

async fn send_heartbeat(
    client: &Client,
    config: &AgentConfig,
) -> Result<HeartbeatResponse, HeartbeatError> {
    let response = client
        .post(endpoint(&config.cloud_url, "/api/cloud/agent/heartbeat"))
        .bearer_auth(&config.token)
        .send()
        .await
        .map_err(|error| HeartbeatError::Retryable(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let message = response_error(response).await;
        return if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            Err(HeartbeatError::Terminal(message))
        } else {
            Err(HeartbeatError::Retryable(message))
        };
    }
    response
        .json()
        .await
        .map_err(|error| HeartbeatError::Retryable(format!("invalid heartbeat response: {error}")))
}

async fn response_error(response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    serde_json::from_str::<ErrorEnvelope>(&body)
        .map(|envelope| format!("HTTP {status}: {}", envelope.error.message))
        .unwrap_or_else(|_| format!("HTTP {status}"))
}

async fn agent_api_response_error(response: reqwest::Response) -> AgentApiError {
    let status = response.status();
    AgentApiError {
        message: response_error(response).await,
        retryable: status.is_server_error()
            || status == StatusCode::REQUEST_TIMEOUT
            || status == StatusCode::TOO_MANY_REQUESTS,
    }
}

fn usage() -> &'static str {
    "Usage:\n  sculk-agent pair --cloud <url> --code <code> --name <name> --workspace <label> --workspace-root <path> --permissions full [--capabilities <list>] [--config <path>]\n  sculk-agent run [--config <path>]"
}

fn print_usage() {
    println!("{}", usage());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_are_allowlisted() {
        assert!(validate_permissions(&split_list("read,write,process")).is_ok());
        assert!(validate_permissions(&split_list("full")).is_ok());
        assert!(validate_permissions(&split_list("read,shell")).is_err());
        assert!(validate_permissions(&split_list("read,READ")).is_err());
    }

    #[test]
    fn successful_completion_omits_an_empty_error() {
        let request = CompleteTaskRequest {
            lease_token: "scl_test",
            status: "succeeded",
            output: json!({ "ok": true }),
            error: None,
            rollback_available: false,
            artifacts: vec![],
        };
        let encoded = serde_json::to_value(request).unwrap();
        assert!(encoded.get("error").is_none());
    }

    #[test]
    fn result_checkpoint_restores_without_reexecuting_the_operation() {
        let resume = LeasedTaskResumePayload {
            source_task_id: "11111111-1111-1111-1111-111111111111".into(),
            checkpoint_id: "22222222-2222-2222-2222-222222222222".into(),
            kind: "result".into(),
            payload: json!({
                "status": "succeeded",
                "output": { "stdout": "done" },
                "error": "",
                "rollback_available": false,
                "artifacts": [],
            }),
        };
        let restored = restore_checkpoint_result(&resume).unwrap();
        assert_eq!(restored.status, "succeeded");
        assert_eq!(restored.output["stdout"], "done");
    }

    #[test]
    fn failed_result_checkpoint_cannot_be_resumed() {
        let resume = LeasedTaskResumePayload {
            source_task_id: "11111111-1111-1111-1111-111111111111".into(),
            checkpoint_id: "22222222-2222-2222-2222-222222222222".into(),
            kind: "result".into(),
            payload: json!({
                "status": "failed",
                "output": {},
                "error": "command failed",
                "rollback_available": false,
                "artifacts": [],
            }),
        };
        assert!(restore_checkpoint_result(&resume).is_err());
    }

    #[test]
    fn cancelled_result_is_terminal_and_not_rollback_capable() {
        let result = cancelled_result();
        assert_eq!(result.status, "cancelled");
        assert_eq!(result.output["cancelled"], true);
        assert!(!result.rollback_available);
        assert!(result.artifacts.is_empty());
    }

    #[test]
    fn cloud_url_requires_tls_except_for_loopback() {
        assert_eq!(
            normalize_cloud_url("https://cloud.example.com/").unwrap(),
            "https://cloud.example.com"
        );
        assert!(normalize_cloud_url("http://127.0.0.1:8788").is_ok());
        assert!(normalize_cloud_url("http://cloud.example.com").is_err());
        assert!(normalize_cloud_url("https://user:secret@cloud.example.com").is_err());
    }

    #[test]
    fn bootstrap_config_defaults_capabilities_without_retaining_a_token() {
        let bootstrap = validate_bootstrap_config(BootstrapConfig {
            cloud_url: "https://cloud.example.com/".into(),
            pairing_code: "scp_once".into(),
            name: "host".into(),
            workspace_label: "minecraft".into(),
            permissions: vec!["read".into()],
            capabilities: vec![],
            workspace_root: None,
            platform: None,
            version: None,
        })
        .unwrap();

        assert_eq!(bootstrap.cloud_url, "https://cloud.example.com");
        assert_eq!(bootstrap.capabilities, vec!["heartbeat"]);
        let encoded = serde_json::to_value(bootstrap).unwrap();
        assert!(encoded.get("pairing_code").is_some());
        assert!(encoded.get("token").is_none());
    }

    #[test]
    fn paired_config_rejects_bootstrap_pairing_code() {
        let config = json!({
            "cloud_url": "https://cloud.example.com",
            "agent_id": "agent_123",
            "token": "sca_secret",
            "name": "host",
            "workspace_label": "minecraft",
            "fingerprint": "abcdef",
            "capabilities": ["heartbeat"],
            "permissions": ["read"],
            "pairing_code": "scp_once"
        });
        assert!(serde_json::from_value::<AgentConfig>(config).is_err());
    }

    #[test]
    fn claiming_replaces_the_bootstrap_code_on_disk() {
        let directory =
            std::env::temp_dir().join(format!("sculk-agent-test-{}", random_fingerprint()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("agent.json");
        let bootstrap = BootstrapConfig {
            cloud_url: "https://cloud.example.com".into(),
            pairing_code: "scp_once".into(),
            name: "host".into(),
            workspace_label: "minecraft".into(),
            permissions: vec!["read".into()],
            capabilities: vec!["heartbeat".into()],
            workspace_root: None,
            platform: None,
            version: None,
        };
        fs::write(&path, serde_json::to_vec(&bootstrap).unwrap()).unwrap();

        write_config(
            &path,
            &AgentConfig {
                cloud_url: "https://cloud.example.com".into(),
                agent_id: "agent_123".into(),
                token: "sca_secret".into(),
                name: "host".into(),
                workspace_label: "minecraft".into(),
                fingerprint: "abcdef".into(),
                capabilities: vec!["heartbeat".into()],
                permissions: vec!["read".into()],
                workspace_root: None,
            },
        )
        .unwrap();

        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved.get("pairing_code").is_none());
        assert_eq!(
            saved.get("token").and_then(Value::as_str),
            Some("sca_secret")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
