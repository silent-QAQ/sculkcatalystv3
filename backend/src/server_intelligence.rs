use crate::AppState;
use reqwest::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;
use zip::ZipArchive;

/// 为服务器规划请求构造证据上下文。
///
/// 顺序固定为：本机证据/内置目录 → res.mcmy.love → 公开互联网 → 明确缺失项。
/// 这里仅提供事实和来源，不替模型替用户确认高风险操作。
pub(crate) async fn context_for_request(state: &AppState, query: &str) -> String {
    let local = inspect_local_inputs(query);
    let terms = search_terms(query);
    let local_catalog = local_catalog_context(state, &terms).await;
    let resource_items = search_resource_center(&terms, query).await;
    let web_items = if resource_items.is_empty() {
        search_internet(&terms).await
    } else {
        Vec::new()
    };
    let java = if query.to_ascii_lowercase().contains("java")
        || query.contains("1.12")
        || query.contains("核心")
    {
        let info = crate::runtime::detect_java(&crate::runtime::data_root()).await;
        if info.java_installed {
            format!(
                "Java {}，可执行文件：{}",
                info.java_major
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "未知版本".into()),
                info.java_executable.unwrap_or_else(|| "未知路径".into())
            )
        } else {
            "本机未检测到可用 Java。".into()
        }
    } else {
        String::new()
    };

    let mut output = String::from("[服务器智能规划证据]\n");
    if local.is_empty() {
        output.push_str("本机输入：未发现可直接检查的文件路径。\n");
    } else {
        output.push_str("本机输入：\n");
        output.push_str(&local);
        output.push('\n');
    }
    if !java.is_empty() {
        output.push_str("运行环境：");
        output.push_str(&java);
        output.push('\n');
    }
    if !local_catalog.is_empty() {
        output.push_str("内置资源目录：\n");
        output.push_str(&local_catalog);
        output.push('\n');
    }
    if !resource_items.is_empty() {
        output.push_str("res.mcmy.love：\n");
        output.push_str(&resource_items.join("\n"));
        output.push('\n');
    } else if !web_items.is_empty() {
        output.push_str("res.mcmy.love：未找到匹配项；互联网公开来源：\n");
        output.push_str(&web_items.join("\n"));
        output.push('\n');
    } else {
        output.push_str("资源库与互联网：均未找到可验证匹配项。\n");
    }
    output.push_str(
        "规划决策：优先使用上述已验证事实；没有可靠来源时只询问改变核心选择、兼容性或安全性的最小缺失信息。用户明确表示不知道/不清楚时，才允许进入已配置 QQ 群协查。\n",
    );
    output
}

fn search_terms(query: &str) -> Vec<String> {
    let lower = query.to_ascii_lowercase();
    let candidates = [
        ("dragoncore", "DragonCore"),
        ("paper", "Paper"),
        ("leaves", "Leaves"),
        ("fabric", "Fabric"),
        ("carpet", "Carpet"),
        ("1.12.2", "Minecraft 1.12.2"),
        ("1.21", "Minecraft 1.21"),
    ];
    let mut terms = candidates
        .iter()
        .filter(|(needle, _)| lower.contains(needle))
        .map(|(_, value)| (*value).to_string())
        .collect::<Vec<_>>();
    if terms.is_empty() {
        let compact = query
            .lines()
            .next()
            .unwrap_or(query)
            .trim()
            .chars()
            .take(120)
            .collect::<String>();
        if !compact.is_empty() {
            terms.push(compact);
        }
    }
    terms.truncate(3);
    terms
}

async fn local_catalog_context(state: &AppState, terms: &[String]) -> String {
    let data = state.inner.read().await;
    let mut lines = Vec::new();
    for term in terms {
        let needle = term.to_ascii_lowercase();
        for project in data
            .catalog
            .core_projects
            .iter()
            .chain(data.catalog.plugin_projects.iter())
        {
            let haystack = format!(
                "{} {} {} {}",
                project.slug, project.name, project.summary, project.description
            )
            .to_ascii_lowercase();
            if haystack.contains(&needle) {
                lines.push(format!(
                    "- {} ({})：{}",
                    project.name, project.slug, project.summary
                ));
            }
        }
    }
    lines.sort();
    lines.dedup();
    lines.into_iter().take(8).collect::<Vec<_>>().join("\n")
}

async fn search_resource_center(terms: &[String], query: &str) -> Vec<String> {
    let Some(base) = crate::resource_sync::resource_base_url() else {
        return Vec::new();
    };
    let Ok(client) = Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .user_agent("Sculk-Catalyst-Server-Intelligence/1.0")
        .build()
    else {
        return Vec::new();
    };
    let minecraft = minecraft_version(query);
    let mut results = Vec::new();
    for term in terms.iter().take(2) {
        let lower = term.to_ascii_lowercase();
        let is_core_query = ["paper", "leaves", "fabric", "folia", "purpur"]
            .iter()
            .any(|name| lower.contains(name));
        let resources: &[&str] = if is_core_query {
            &["cores"]
        } else {
            &["plugins"]
        };
        for resource in resources {
            let mut url = match Url::parse(&format!("{base}/api/catalog/{resource}")) {
                Ok(url) => url,
                Err(_) => continue,
            };
            url.query_pairs_mut().append_pair("search", term);
            if let Some(version) = minecraft.as_deref() {
                url.query_pairs_mut().append_pair("minecraft", version);
            }
            let Ok(payload) = fetch_json(client.get(url)).await else {
                continue;
            };
            append_catalog_results(&mut results, &payload, resource);
        }
        if !is_core_query {
            let mut url = match Url::parse(&format!("{base}/api/v1/plugins/search")) {
                Ok(url) => url,
                Err(_) => continue,
            };
            url.query_pairs_mut()
                .append_pair("q", term)
                .append_pair("limit", "6");
            if let Some(version) = minecraft.as_deref() {
                url.query_pairs_mut().append_pair("minecraft", version);
            }
            if let Ok(payload) = fetch_json(client.get(url)).await {
                append_catalog_results(&mut results, &payload, "plugins");
            }
        }
        if !results.is_empty() {
            break;
        }
    }
    results.sort();
    results.dedup();
    results.into_iter().take(12).collect()
}

fn append_catalog_results(results: &mut Vec<String>, payload: &Value, resource: &str) {
    let items = payload
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| payload.as_array());
    let Some(items) = items else { return };
    for item in items {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| item.get("slug").and_then(Value::as_str));
        let Some(name) = name else { continue };
        let slug = item
            .get("slug")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let summary = item
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let latest = item
            .get("latest_version")
            .and_then(Value::as_str)
            .map(|value| format!("，最新版本 {value}"))
            .unwrap_or_default();
        let minecraft = item
            .get("minecraft_versions")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .take(6)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|value| !value.is_empty())
            .map(|value| format!("，Minecraft {value}"))
            .unwrap_or_default();
        results.push(format!(
            "- {name} ({slug}) [{resource}]：{summary}{latest}{minecraft}"
        ));
    }
}

fn minecraft_version(query: &str) -> Option<String> {
    query
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .find(|candidate| {
            candidate.matches('.').count() >= 2
                && candidate
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
        })
        .map(ToString::to_string)
}

async fn fetch_json(builder: reqwest::RequestBuilder) -> Result<Value, reqwest::Error> {
    builder
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await
}

async fn search_internet(terms: &[String]) -> Vec<String> {
    let Some(term) = terms.first() else {
        return Vec::new();
    };
    let Ok(client) = Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .user_agent("Sculk-Catalyst-Server-Intelligence/1.0")
        .build()
    else {
        return Vec::new();
    };
    let mut results = Vec::new();

    if let Ok(mut url) = Url::parse("https://api.modrinth.com/v2/search") {
        url.query_pairs_mut()
            .append_pair("query", term)
            .append_pair("limit", "5");
        if let Ok(payload) = fetch_json(client.get(url)).await {
            if let Some(items) = payload.get("hits").and_then(Value::as_array) {
                for item in items {
                    if let Some(title) = item.get("title").and_then(Value::as_str) {
                        let slug = item
                            .get("slug")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown");
                        results.push(format!(
                            "- Modrinth：{title} ({slug})，项目页：https://modrinth.com/"
                        ));
                    }
                }
            }
        }
    }

    if let Ok(mut url) = Url::parse("https://api.github.com/search/repositories") {
        url.query_pairs_mut()
            .append_pair("q", &format!("{term} Minecraft"))
            .append_pair("per_page", "5");
        if let Ok(payload) = fetch_json(client.get(url)).await {
            if let Some(items) = payload.get("items").and_then(Value::as_array) {
                for item in items {
                    let Some(name) = item.get("full_name").and_then(Value::as_str) else {
                        continue;
                    };
                    let html = item
                        .get("html_url")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    results.push(format!("- GitHub：{name}，项目页：{html}"));
                }
            }
        }
    }

    if let Some(endpoint) = std::env::var("SCULK_WEB_SEARCH_API_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        && let Ok(mut url) = Url::parse(&endpoint)
    {
        url.query_pairs_mut()
            .append_pair("q", term)
            .append_pair("limit", "5");
        if let Ok(payload) = fetch_json(client.get(url)).await {
            append_web_results(&mut results, &payload);
        }
    }
    results.sort();
    results.dedup();
    results.into_iter().take(12).collect()
}

fn append_web_results(results: &mut Vec<String>, payload: &Value) {
    let items = payload
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| payload.get("results").and_then(Value::as_array))
        .or_else(|| payload.as_array());
    let Some(items) = items else { return };
    for item in items {
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str));
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .or_else(|| item.get("link").and_then(Value::as_str));
        if let Some(title) = title {
            results.push(format!(
                "- 互联网搜索：{title}{}",
                url.map(|value| format!("，来源：{value}"))
                    .unwrap_or_default()
            ));
        }
    }
}

fn inspect_local_inputs(query: &str) -> String {
    extract_paths(query)
        .into_iter()
        .filter_map(|path| inspect_path(&path))
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_paths(query: &str) -> Vec<PathBuf> {
    let chars: Vec<char> = query.chars().collect();
    let mut paths = Vec::new();
    let mut index = 0;
    while index + 2 < chars.len() {
        let is_drive = chars[index].is_ascii_alphabetic()
            && chars[index + 1] == ':'
            && (chars[index + 2] == '\\' || chars[index + 2] == '/');
        if !is_drive {
            index += 1;
            continue;
        }
        let start = index;
        index += 3;
        while index < chars.len()
            && !matches!(
                chars[index],
                '\n' | '\r' | '`' | '"' | '\'' | '<' | '>' | ',' | '，' | '。' | ';' | '；'
            )
        {
            index += 1;
        }
        let raw: String = chars[start..index]
            .iter()
            .collect::<String>()
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    ' ' | ',' | '，' | '。' | ')' | '）' | ']' | '】' | ';' | '；'
                )
            })
            .to_string();
        if !raw.is_empty() && !paths.iter().any(|path: &PathBuf| path == Path::new(&raw)) {
            paths.push(PathBuf::from(raw));
        }
    }
    paths.into_iter().take(8).collect()
}

fn inspect_path(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let child = entry.path();
                let extension = child.extension()?.to_string_lossy().to_ascii_lowercase();
                if extension == "jar" || extension == "zip" {
                    Some(child.file_name()?.to_string_lossy().into_owned())
                } else {
                    None
                }
            })
            .take(12)
            .collect::<Vec<_>>();
        children.sort();
        return Some(format!(
            "- 目录 `{}` 存在；可用资源文件：{}",
            path.display(),
            if children.is_empty() {
                "未发现 JAR/ZIP".into()
            } else {
                children.join("、")
            }
        ));
    }
    let filename = path.file_name()?.to_string_lossy();
    let mut fact = format!("- 文件 `{}` 存在，大小 {} 字节", filename, metadata.len());
    if filename.to_ascii_lowercase().ends_with(".jar") {
        if let Some(descriptor) = inspect_plugin_jar(path) {
            fact.push_str("；插件描述：");
            fact.push_str(&descriptor);
        }
        if let Some(sha256) = sha256_file(path) {
            fact.push_str("；SHA-256 前 12 位：");
            fact.push_str(&sha256[..12]);
        }
    }
    Some(fact)
}

fn inspect_plugin_jar(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;
    let mut descriptor = String::new();
    for name in ["plugin.yml", "META-INF/MANIFEST.MF"] {
        let Ok(mut entry) = archive.by_name(name) else {
            continue;
        };
        entry.read_to_string(&mut descriptor).ok()?;
        break;
    }
    if descriptor.is_empty() {
        return None;
    }
    let fields = ["name", "version", "main", "api-version"]
        .iter()
        .filter_map(|field| {
            descriptor.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                (key.trim().eq_ignore_ascii_case(field))
                    .then(|| format!("{}={}", field, value.trim()))
            })
        })
        .collect::<Vec<_>>();
    (!fields.is_empty()).then(|| fields.join(", "))
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_windows_paths_without_trailing_sentence_punctuation() {
        let paths = extract_paths(
            "龙核位置 C:\\Users\\Administrator\\Desktop\\[插件]DragonCore-2.6.2.9.jar，这是已确认的文件。",
        );
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0].to_string_lossy(),
            r"C:\Users\Administrator\Desktop\[插件]DragonCore-2.6.2.9.jar"
        );
    }

    #[test]
    fn search_terms_prioritize_named_server_components() {
        let terms = search_terms("我要用 Leaves 1.21.4 和 DragonCore");
        assert!(terms.contains(&"DragonCore".into()));
        assert!(terms.contains(&"Leaves".into()));
        assert!(terms.contains(&"Minecraft 1.21".into()));
    }

    #[test]
    fn extracts_a_minecraft_version_for_compatibility_queries() {
        assert_eq!(minecraft_version("Paper 1.12.2 RPG"), Some("1.12.2".into()));
        assert_eq!(minecraft_version("Paper 最新版"), None);
    }
}
