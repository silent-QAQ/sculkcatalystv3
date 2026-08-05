//! Safe inspection helpers for adopting an existing Minecraft server directory.
//!
//! This module deliberately does not execute anything and does not mutate the
//! inspected directory.  The API layer can use the result to ask the user to
//! choose a launch artifact before registering the workspace.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs as std_fs,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};
use tokio::fs;

/// Keep inspection bounded even when a user points the workbench at a large
/// directory or an accidentally mounted filesystem.
pub(crate) const MAX_INSPECTED_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct JarCandidate {
    /// A path relative to the selected server directory.
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) modified: Option<u64>,
    pub(crate) likely_core: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LaunchScriptCandidate {
    /// A path relative to the selected server directory.
    pub(crate) path: String,
    /// `bat`, `cmd`, `ps1`, or `sh`.
    pub(crate) kind: String,
    pub(crate) size: u64,
    pub(crate) modified: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ExistingServerInspection {
    /// Canonical absolute path.  Callers should persist this value instead of
    /// the user-supplied spelling so duplicate registrations can be detected.
    pub(crate) canonical_path: String,
    /// Parsed key/value pairs from the root `server.properties` file.
    pub(crate) properties: BTreeMap<String, String>,
    pub(crate) server_port: Option<u16>,
    pub(crate) max_players: Option<u32>,
    /// `None` means the file is absent or did not contain a valid `eula` key.
    pub(crate) eula_accepted: Option<bool>,
    pub(crate) jar_candidates: Vec<JarCandidate>,
    pub(crate) launch_scripts: Vec<LaunchScriptCandidate>,
    /// Set only when selection is unambiguous (or a root `server.jar` exists).
    pub(crate) recommended_jar: Option<String>,
    pub(crate) core_hint: Option<String>,
    pub(crate) minecraft_version_hint: Option<String>,
    pub(crate) warnings: Vec<String>,
}

/// Resolve an existing directory without following a symlink at the selected
/// root or in any existing parent component.  A caller may still need to
/// re-check the path immediately before opening it because a directory can be
/// replaced after this function returns.
pub(crate) fn canonicalize_existing_directory(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("已有服务器目录不能为空".into());
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("无法获取当前工作目录：{error}"))?
            .join(path)
    };
    reject_symlink_components(&absolute)?;

    let metadata = std_fs::symlink_metadata(&absolute)
        .map_err(|error| format!("无法读取服务器目录：{error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("选择的路径必须是非符号链接目录".into());
    }

    let canonical =
        std_fs::canonicalize(&absolute).map_err(|error| format!("无法解析服务器目录：{error}"))?;
    let canonical_metadata = std_fs::symlink_metadata(&canonical)
        .map_err(|error| format!("无法验证服务器目录：{error}"))?;
    if !canonical_metadata.is_dir() || canonical_metadata.file_type().is_symlink() {
        return Err("解析后的路径不是普通目录".into());
    }
    // Opening a filesystem root as a Minecraft workspace is almost certainly
    // an accidental selection and would make file browsing unnecessarily broad.
    if canonical.parent().is_none() {
        return Err("不能直接打开文件系统根目录".into());
    }
    Ok(canonical)
}

/// Inspect an existing server directory.  This function only reads metadata
/// and small text files; it never writes `sculk.yml`, changes EULA, or starts a
/// process.
pub(crate) async fn inspect_existing_directory(
    path: &Path,
) -> Result<ExistingServerInspection, String> {
    let canonical = canonicalize_existing_directory(path)?;
    let properties_path = canonical.join("server.properties");
    let eula_path = canonical.join("eula.txt");

    let properties_bytes = read_optional_file(&properties_path).await?;
    let eula_bytes = read_optional_file(&eula_path).await?;
    let mut warnings = Vec::new();

    let properties = match properties_bytes {
        Some(bytes) => match parse_server_properties(&bytes) {
            Ok(properties) => properties,
            Err(error) => {
                warnings.push(format!("server.properties 无法解析：{error}"));
                BTreeMap::new()
            }
        },
        None => {
            warnings.push("未找到根目录 server.properties".into());
            BTreeMap::new()
        }
    };

    let server_port = parse_port(properties.get("server-port"), &mut warnings);
    let max_players = parse_max_players(properties.get("max-players"), &mut warnings);
    let eula_accepted = match eula_bytes {
        Some(bytes) => match parse_eula(&bytes) {
            Ok(value) => value,
            Err(error) => {
                warnings.push(format!("eula.txt 无法解析：{error}"));
                None
            }
        },
        None => {
            warnings.push("未找到根目录 eula.txt；启动前必须明确同意 EULA".into());
            None
        }
    };
    if eula_accepted == Some(false) {
        warnings.push("eula.txt 尚未同意 Minecraft EULA".into());
    }

    let (jar_candidates, launch_scripts) = scan_launch_artifacts(&canonical, &mut warnings).await?;
    let recommended_jar = recommend_jar(&jar_candidates);
    if jar_candidates.is_empty() {
        warnings.push("未找到根目录 JAR 核心文件；不会自动执行启动脚本".into());
    } else if recommended_jar.is_none() {
        warnings.push("检测到多个可能的核心 JAR，请在接入前明确选择".into());
    }

    let (core_hint, minecraft_version_hint) = recommended_jar
        .as_deref()
        .and_then(|jar| infer_jar_hints(jar))
        .unwrap_or((None, None));
    if !launch_scripts.is_empty() {
        warnings.push("检测到启动脚本；出于安全原因仅报告脚本，不会自动执行".into());
    }

    Ok(ExistingServerInspection {
        canonical_path: canonical.to_string_lossy().to_string(),
        properties,
        server_port,
        max_players,
        eula_accepted,
        jar_candidates,
        launch_scripts,
        recommended_jar,
        core_hint,
        minecraft_version_hint,
        warnings,
    })
}

/// Parse Minecraft's root `server.properties` file.  The parser intentionally
/// keeps unknown keys and values so the editor can round-trip the complete
/// configuration; only the first `=` or `:` is treated as a separator.
pub(crate) fn parse_server_properties(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "文件必须是 UTF-8 文本".to_string())?
        .trim_start_matches('\u{feff}');
    let mut values = BTreeMap::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let Some(separator) = line.find(['=', ':']) else {
            // Java properties permits a key with an empty value.  It is not a
            // useful Minecraft setting, but retaining it keeps one odd line
            // from preventing the normal server settings from being read.
            values.insert(line.to_string(), String::new());
            continue;
        };
        let key = line[..separator].trim();
        if key.is_empty() {
            return Err(format!("第 {} 行键名为空", index + 1));
        }
        let value = line[separator + 1..].trim().to_string();
        values.insert(key.to_string(), value);
    }
    Ok(values)
}

/// Parse an EULA file.  `None` indicates that no valid eula key was found.
pub(crate) fn parse_eula(bytes: &[u8]) -> Result<Option<bool>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "文件必须是 UTF-8 文本".to_string())?
        .trim_start_matches('\u{feff}');
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("eula") {
            return match value.trim().to_ascii_lowercase().as_str() {
                "true" => Ok(Some(true)),
                "false" => Ok(Some(false)),
                _ => Err("eula 必须是 true 或 false".into()),
            };
        }
    }
    Ok(None)
}

fn parse_port(value: Option<&String>, warnings: &mut Vec<String>) -> Option<u16> {
    let Some(value) = value else { return None };
    match value.parse::<u16>() {
        Ok(port) if port >= 1024 => Some(port),
        Ok(_) => {
            warnings.push("server-port 必须在 1024-65535 之间".into());
            None
        }
        Err(_) => {
            warnings.push("server-port 不是有效整数".into());
            None
        }
    }
}

fn parse_max_players(value: Option<&String>, warnings: &mut Vec<String>) -> Option<u32> {
    let Some(value) = value else { return None };
    match value.parse::<u32>() {
        Ok(players) if players > 0 => Some(players),
        Ok(_) => {
            warnings.push("max-players 必须是正整数".into());
            None
        }
        Err(_) => {
            warnings.push("max-players 不是有效整数".into());
            None
        }
    }
}

async fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("无法读取 {}：{error}", path.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} 必须是普通文件，不能是符号链接", path.display()));
    }
    if metadata.len() > MAX_INSPECTED_FILE_BYTES {
        return Err(format!("{} 超过 2 MiB 检查限制", path.display()));
    }
    fs::read(path)
        .await
        .map(Some)
        .map_err(|error| format!("无法读取 {}：{error}", path.display()))
}

async fn scan_launch_artifacts(
    directory: &Path,
    warnings: &mut Vec<String>,
) -> Result<(Vec<JarCandidate>, Vec<LaunchScriptCandidate>), String> {
    let mut entries = fs::read_dir(directory)
        .await
        .map_err(|error| format!("无法扫描服务器目录：{error}"))?;
    let mut jars = Vec::new();
    let mut scripts = Vec::new();
    let mut count = 0usize;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("扫描服务器目录失败：{error}"))?
    {
        count += 1;
        if count > MAX_DIRECTORY_ENTRIES {
            warnings.push(format!(
                "目录条目超过 {MAX_DIRECTORY_ENTRIES}，后续条目未扫描"
            ));
            break;
        }
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| format!("无法读取目录条目元数据：{error}"))?;
        if file_type.is_symlink() || !file_type.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        let metadata = entry
            .metadata()
            .await
            .map_err(|error| format!("无法读取 {} 元数据：{error}", name))?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());
        if lower.ends_with(".jar")
            && !lower.ends_with(".part.jar")
            && !lower.ends_with(".backup.jar")
            && !lower.ends_with(".tmp.jar")
        {
            jars.push(JarCandidate {
                path: name.clone(),
                size: metadata.len(),
                modified,
                likely_core: likely_core_jar(&lower),
            });
        }
        if let Some(kind) = script_kind(&lower) {
            scripts.push(LaunchScriptCandidate {
                path: name,
                kind: kind.into(),
                size: metadata.len(),
                modified,
            });
        }
    }
    jars.sort_by(|left, right| {
        (!left.likely_core, left.path.to_ascii_lowercase())
            .cmp(&(!right.likely_core, right.path.to_ascii_lowercase()))
    });
    scripts.sort_by(|left, right| {
        left.path
            .to_ascii_lowercase()
            .cmp(&right.path.to_ascii_lowercase())
    });
    Ok((jars, scripts))
}

fn likely_core_jar(name: &str) -> bool {
    if name == "server.jar" {
        return true;
    }
    [
        "paper",
        "purpur",
        "folia",
        "spigot",
        "bukkit",
        "fabric",
        "forge",
        "neoforge",
        "velocity",
        "leaves",
        "arclight",
        "catserver",
        "mohist",
        "sponge",
    ]
    .iter()
    .any(|marker| name.starts_with(marker) || name.contains(&format!("-{marker}")))
}

fn script_kind(name: &str) -> Option<&'static str> {
    if name.ends_with(".bat") {
        Some("bat")
    } else if name.ends_with(".cmd") {
        Some("cmd")
    } else if name.ends_with(".ps1") {
        Some("ps1")
    } else if name.ends_with(".sh") {
        Some("sh")
    } else {
        None
    }
}

fn recommend_jar(candidates: &[JarCandidate]) -> Option<String> {
    if let Some(server) = candidates
        .iter()
        .find(|candidate| candidate.path.eq_ignore_ascii_case("server.jar") && candidate.size > 0)
    {
        return Some(server.path.clone());
    }
    let nonempty = candidates
        .iter()
        .filter(|candidate| candidate.size > 0)
        .collect::<Vec<_>>();
    if nonempty.len() == 1 {
        return Some(nonempty[0].path.clone());
    }
    let likely = candidates
        .iter()
        .filter(|candidate| candidate.likely_core)
        .collect::<Vec<_>>();
    (likely.len() == 1 && likely[0].size > 0).then(|| likely[0].path.clone())
}

fn infer_jar_hints(name: &str) -> Option<(Option<String>, Option<String>)> {
    let lower = name.to_ascii_lowercase();
    let core = [
        ("neoforge", "NeoForge"),
        ("arclight", "Arclight"),
        ("purpur", "Purpur"),
        ("paper", "Paper"),
        ("folia", "Folia"),
        ("spigot", "Spigot"),
        ("fabric", "Fabric"),
        ("forge", "Forge"),
        ("velocity", "Velocity"),
        ("leaves", "Leaves"),
        ("mohist", "Mohist"),
        ("catserver", "CatServer"),
        ("sponge", "Sponge"),
    ]
    .iter()
    .find(|(marker, _)| lower.contains(marker))
    .map(|(_, label)| (*label).to_string());
    let version = lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '.')
        .find(|token| {
            let parts = token.split('.').collect::<Vec<_>>();
            parts.len() >= 2
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        })
        .map(ToString::to_string);
    (core.is_some() || version.is_some()).then_some((core, version))
}

fn reject_symlink_components(path: &Path) -> Result<(), String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("无法获取当前工作目录：{error}"))?
            .join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => current.push(".."),
            Component::Normal(name) => current.push(name),
        }
        match std_fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "服务器路径包含不允许的符号链接：{}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("无法验证服务器路径：{error}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use uuid::Uuid;

    fn temp_directory(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4().simple()))
    }

    #[test]
    fn properties_parser_preserves_unknown_values_and_comments() {
        let parsed = parse_server_properties(
            "\u{feff}# comment\nserver-port=25570\nmax-players:24\nmotd=one=two\n! ignored\n"
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(parsed.get("server-port"), Some(&"25570".to_string()));
        assert_eq!(parsed.get("max-players"), Some(&"24".to_string()));
        assert_eq!(parsed.get("motd"), Some(&"one=two".to_string()));
    }

    #[test]
    fn eula_parser_requires_a_boolean_value() {
        assert_eq!(parse_eula(b"# generated\neula=true\n").unwrap(), Some(true));
        assert_eq!(parse_eula(b"eula=false\n").unwrap(), Some(false));
        assert!(parse_eula(b"eula=maybe\n").is_err());
        assert_eq!(parse_eula(b"# no value\n").unwrap(), None);
    }

    #[tokio::test]
    async fn inspection_detects_artifacts_and_derives_unambiguous_hints() {
        let root = temp_directory("sculk-import");
        fs::create_dir_all(&root).await.unwrap();
        fs::write(
            root.join("server.properties"),
            "server-port=25566\nmax-players=12\nmotd=Existing\n",
        )
        .await
        .unwrap();
        fs::write(root.join("eula.txt"), "eula=true\n")
            .await
            .unwrap();
        fs::write(root.join("paper-1.21.4-120.jar"), b"jar")
            .await
            .unwrap();
        fs::write(root.join("run.bat"), b"java -jar paper-1.21.4-120.jar")
            .await
            .unwrap();

        let result = inspect_existing_directory(&root).await.unwrap();
        assert_eq!(result.server_port, Some(25566));
        assert_eq!(result.max_players, Some(12));
        assert_eq!(result.eula_accepted, Some(true));
        assert_eq!(
            result.recommended_jar.as_deref(),
            Some("paper-1.21.4-120.jar")
        );
        assert_eq!(result.core_hint.as_deref(), Some("Paper"));
        assert_eq!(result.minecraft_version_hint.as_deref(), Some("1.21.4"));
        assert_eq!(result.launch_scripts[0].kind, "bat");
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("启动脚本"))
        );

        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn inspection_does_not_choose_between_multiple_core_jars() {
        let root = temp_directory("sculk-import-ambiguous");
        fs::create_dir_all(&root).await.unwrap();
        fs::write(root.join("paper-1.21.4.jar"), b"paper")
            .await
            .unwrap();
        fs::write(root.join("purpur-1.21.4.jar"), b"purpur")
            .await
            .unwrap();
        let result = inspect_existing_directory(&root).await.unwrap();
        assert_eq!(result.recommended_jar, None);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("多个"))
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[test]
    fn canonicalization_rejects_files_and_empty_paths() {
        assert!(canonicalize_existing_directory(Path::new("")).is_err());
        let file = temp_directory("sculk-import-file");
        let mut handle = std_fs::File::create(&file).unwrap();
        handle.write_all(b"not a directory").unwrap();
        assert!(canonicalize_existing_directory(&file).is_err());
        std_fs::remove_file(file).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonicalization_rejects_a_symlink_root() {
        use std::os::unix::fs::symlink;
        let real = temp_directory("sculk-import-real");
        let alias = temp_directory("sculk-import-alias");
        std_fs::create_dir_all(&real).unwrap();
        symlink(&real, &alias).unwrap();
        assert!(canonicalize_existing_directory(&alias).is_err());
        std_fs::remove_file(alias).unwrap();
        std_fs::remove_dir_all(real).unwrap();
    }
}
