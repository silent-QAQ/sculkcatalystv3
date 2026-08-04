use serde::Serialize;
use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{OnceLock, RwLock},
    time::{Duration, Instant},
};
use tokio::{process::Command, time::timeout};

const DETECTION_CACHE_TTL: Duration = Duration::from_secs(30);
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Serialize)]
pub(crate) struct ReasoningEffortCapability {
    pub(crate) supported: bool,
    pub(crate) values: &'static [&'static str],
}

#[derive(Clone, Serialize)]
pub(crate) struct DetectedAgentCapabilities {
    pub(crate) reasoning_effort: ReasoningEffortCapability,
    pub(crate) acp: bool,
}

#[derive(Clone, Serialize)]
pub(crate) struct DetectedAgent {
    pub(crate) kind: &'static str,
    pub(crate) name: &'static str,
    pub(crate) installed: bool,
    pub(crate) available: bool,
    pub(crate) command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) version: Option<String>,
    pub(crate) transport: &'static str,
    pub(crate) capabilities: DetectedAgentCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

struct DetectionCache {
    at: Instant,
    agents: Vec<DetectedAgent>,
}

static DETECTION_CACHE: OnceLock<RwLock<Option<DetectionCache>>> = OnceLock::new();

pub(crate) const CODEX_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];
pub(crate) const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
pub(crate) const MODEL_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max"];

/// Detects native CLIs independently of the dashboard path. Results are briefly cached so
/// repeatedly opening AI settings never starts a process for every render.
pub(crate) async fn detected_agents() -> Vec<DetectedAgent> {
    let cache = DETECTION_CACHE.get_or_init(|| RwLock::new(None));
    if let Ok(cache) = cache.read()
        && let Some(cached) = cache.as_ref()
        && cached.at.elapsed() < DETECTION_CACHE_TTL
    {
        return cached.agents.clone();
    }

    let (codex, claude) = tokio::join!(
        detect("codex", "Codex CLI", "codex", CODEX_EFFORTS,),
        detect("claude-code", "Claude Code CLI", "claude", CLAUDE_EFFORTS,)
    );
    let agents = vec![codex, claude];
    if let Ok(mut cache) = cache.write() {
        *cache = Some(DetectionCache {
            at: Instant::now(),
            agents: agents.clone(),
        });
    }
    agents
}

pub(crate) fn cached_detected_agents() -> Vec<DetectedAgent> {
    DETECTION_CACHE
        .get()
        .and_then(|cache| cache.read().ok())
        .and_then(|cache| cache.as_ref().map(|cached| cached.agents.clone()))
        .unwrap_or_default()
}

async fn detect(
    kind: &'static str,
    name: &'static str,
    command: &'static str,
    efforts: &'static [&'static str],
) -> DetectedAgent {
    let Some(path) = find_on_path(command) else {
        return DetectedAgent {
            kind,
            name,
            installed: false,
            available: false,
            command,
            path: None,
            version: None,
            transport: "cli",
            capabilities: capabilities(efforts),
            reason: Some(format!("未在 PATH 中找到 {command}")),
        };
    };
    let path_text = path.to_string_lossy().into_owned();
    match probe_version(&path).await {
        Ok(version) => DetectedAgent {
            kind,
            name,
            installed: true,
            available: true,
            command,
            path: Some(path_text),
            version: Some(version),
            transport: "cli",
            capabilities: capabilities(efforts),
            reason: None,
        },
        Err(reason) => DetectedAgent {
            kind,
            name,
            installed: true,
            available: false,
            command,
            path: Some(path_text),
            version: None,
            transport: "cli",
            capabilities: capabilities(efforts),
            reason: Some(reason),
        },
    }
}

fn capabilities(efforts: &'static [&'static str]) -> DetectedAgentCapabilities {
    DetectedAgentCapabilities {
        reasoning_effort: ReasoningEffortCapability {
            supported: true,
            values: efforts,
        },
        acp: false,
    }
}

fn find_on_path(command: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        for candidate in executable_candidates(&directory, command) {
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn executable_candidates(directory: &Path, command: &str) -> Vec<PathBuf> {
    let direct = directory.join(command);
    #[cfg(windows)]
    {
        if Path::new(command).extension().is_some() {
            return vec![direct];
        }
        let extensions = env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|item| !item.trim().is_empty())
                    .map(|item| item.trim().to_ascii_lowercase())
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty())
            .unwrap_or_else(|| vec![".com".into(), ".exe".into(), ".bat".into(), ".cmd".into()]);
        extensions
            .into_iter()
            .map(|extension| directory.join(format!("{command}{extension}")))
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![direct]
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

async fn probe_version(path: &Path) -> Result<String, String> {
    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide_window(&mut command);
    let output = timeout(VERSION_TIMEOUT, command.output())
        .await
        .map_err(|_| "执行 --version 超时".to_string())?
        .map_err(|error| format!("无法执行 --version：{error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let version = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(256).collect::<String>());
    if !output.status.success() {
        return Err(version.unwrap_or_else(|| format!("--version 退出码 {}", output.status)));
    }
    version.ok_or_else(|| "--version 未返回版本信息".into())
}

pub(crate) async fn probe_command_version(command: &str) -> Result<String, String> {
    probe_version(Path::new(command)).await
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_include_windows_script_shims_or_unix_binary() {
        let candidates = executable_candidates(Path::new("tools"), "codex");
        assert!(candidates.iter().any(|path| {
            let name = path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            name == "codex" || name == "codex.cmd" || name == "codex.exe"
        }));
        #[cfg(windows)]
        assert!(!candidates.iter().any(|path| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("codex"))
        }));
    }

    #[test]
    fn capabilities_do_not_claim_acp_support() {
        let value = serde_json::to_value(capabilities(CODEX_EFFORTS)).unwrap();
        assert_eq!(value["acp"], false);
        assert_eq!(value["reasoning_effort"]["supported"], true);
        assert_eq!(value["reasoning_effort"]["values"][0], "minimal");
    }
}
