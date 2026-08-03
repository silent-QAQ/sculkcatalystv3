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
    let msl_items = if resource_items.is_empty() {
        search_msl_core(&terms, query).await
    } else {
        Vec::new()
    };
    let plugin_items = if resource_items.is_empty() && is_plugin_query(query) {
        search_plugin_sources(&terms, query).await
    } else {
        Vec::new()
    };
    let core_query = terms.iter().any(|term| {
        [
            "paper", "purpur", "leaves", "leaf", "fabric", "folia", "spigot",
        ]
        .iter()
        .any(|name| term.to_ascii_lowercase().contains(name))
    });
    let web_items = if resource_items.is_empty()
        && ((is_plugin_query(query) && plugin_items.is_empty())
            || (core_query && !is_plugin_query(query) && msl_items.is_empty())
            || (!core_query && !is_plugin_query(query)))
    {
        search_internet(&terms).await
    } else {
        Vec::new()
    };
    let system = crate::runtime::collect_system_info(&crate::runtime::data_root()).await;
    let expected_players = expected_player_count(query);
    let modded = contains_any_case_insensitive(query, &["模组", "modpack", "forge", "fabric"]);
    let recommended_memory = crate::runtime::recommended_server_memory_gb(
        system.total_memory_bytes,
        expected_players,
        modded,
    );
    let java = if system.java.java_installed {
        format!(
            "已检测 Java {}",
            system
                .java
                .java_major
                .map(|value| value.to_string())
                .unwrap_or_else(|| "未知版本".into())
        )
    } else if system.java_install_supported {
        "未检测到 Java；执行器可按 Minecraft 版本自动安装 Java 8/17/21".into()
    } else {
        "未检测到 Java，且当前平台不支持托管安装".into()
    };

    let mut output = String::from("[服务器智能规划证据]\n");
    if local.is_empty() {
        output.push_str("本机输入：未发现可直接检查的文件路径。\n");
    } else {
        output.push_str("本机输入：\n");
        output.push_str(&local);
        output.push('\n');
    }
    output.push_str(&format!(
        "部署机器（已自动检测，禁止再向用户询问）：{} {}，逻辑 CPU {}，总内存 {}，数据盘可用 {}，数据目录{}写；{}。\n",
        system.os,
        system.arch,
        system.logical_cpu_count,
        display_bytes(system.total_memory_bytes),
        display_bytes(system.data_dir_free_bytes),
        if system.data_dir_writable { "可" } else { "不可" },
        java
    ));
    output.push_str(&format!(
        "自动容量决策：{}玩家规模按 {} 人估算；初始分配 {} GB，端口从 25565 起自动避让，Java 按目标 Minecraft 版本自动选择并安装。\n",
        if modded { "模组服" } else { "插件/原版服" },
        expected_players.unwrap_or(12),
        recommended_memory
    ));
    if !local_catalog.is_empty() {
        output.push_str("内置资源目录：\n");
        output.push_str(&local_catalog);
        output.push('\n');
    }
    let mut has_evidence = false;
    if !resource_items.is_empty() {
        output.push_str("res.mcmy.love：\n");
        output.push_str(&resource_items.join("\n"));
        output.push('\n');
        has_evidence = true;
    }
    if !msl_items.is_empty() {
        output.push_str("MSL 国内镜像 API：\n");
        output.push_str(&msl_items.join("\n"));
        output.push('\n');
        has_evidence = true;
    }
    if !plugin_items.is_empty() {
        output.push_str("Modrinth / SpigotMC 官方插件源：\n");
        output.push_str(&plugin_items.join("\n"));
        output.push('\n');
        has_evidence = true;
    }
    if !web_items.is_empty() {
        output.push_str("资源库、MSL 与官方插件源未找到匹配项；互联网公开来源：\n");
        output.push_str(&web_items.join("\n"));
        output.push('\n');
        has_evidence = true;
    }
    if !has_evidence {
        output.push_str("资源库与互联网：均未找到可验证匹配项。\n");
    }
    output.push_str(
        "规划决策：优先使用上述已验证事实；资源中心没有答案才查询公开互联网。不得询问操作系统、CPU、内存、磁盘、Java、安装路径或端口，这些由本机检测和执行器处理。用户未指定版本时选择证据中可用的最新稳定版并注明可更改；未指定人数时按小型服务器默认值启动后再根据指标调整。只有玩法选择、已有世界迁移或会改变安全边界的信息既无法推断也无法默认时，才提出一个简短问题。用户明确表示不知道/不清楚时，才允许进入已配置 QQ 群协查。\n",
    );
    output
}

fn contains_any_case_insensitive(value: &str, needles: &[&str]) -> bool {
    let lower = value.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

pub(crate) fn expected_player_count(query: &str) -> Option<u32> {
    for marker in ["人", "players", "player"] {
        let lower = query.to_ascii_lowercase();
        let mut offset = 0;
        while let Some(index) = lower[offset..].find(marker) {
            let marker_at = offset + index;
            let prefix = &lower[..marker_at];
            let digits = prefix
                .chars()
                .rev()
                .skip_while(|character| character.is_whitespace() || *character == '约')
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
            if let Ok(players) = digits.parse::<u32>()
                && (1..=10_000).contains(&players)
            {
                return Some(players);
            }
            offset = marker_at + marker.len();
        }
    }
    None
}

fn display_bytes(value: Option<u64>) -> String {
    value
        .map(|bytes| format!("{:.1} GB", bytes as f64 / 1024_f64.powi(3)))
        .unwrap_or_else(|| "未知".into())
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
    if terms.is_empty()
        && (query.contains("插件") || query.contains("生存") || query.contains("开服"))
    {
        terms.push("Paper".into());
    }
    if terms.is_empty() {
        let compact = query
            .lines()
            .next()
            .unwrap_or(query)
            .split_whitespace()
            .filter(|value| {
                !value.contains(":\\")
                    && !value.contains("/Users/")
                    && !value.contains("/home/")
                    && !value.to_ascii_lowercase().contains("token")
                    && !value.to_ascii_lowercase().contains("password")
            })
            .collect::<Vec<_>>()
            .join(" ")
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
    crate::extract_minecraft_version(query)
}

async fn fetch_json(builder: reqwest::RequestBuilder) -> Result<Value, reqwest::Error> {
    builder
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await
}

fn is_plugin_query(query: &str) -> bool {
    let lower = query.to_ascii_lowercase();
    lower.contains("插件")
        || lower.contains("plugin")
        || lower.contains("luckperms")
        || lower.contains("coreprotect")
        || lower.contains("essentials")
}

async fn search_msl_core(terms: &[String], query: &str) -> Vec<String> {
    let Some(core) = terms.iter().find_map(|term| {
        let lower = term.to_ascii_lowercase();
        [
            ("paper", "paper"),
            ("purpur", "purpur"),
            ("leaves", "leaves"),
            ("leaf", "leaf"),
            ("fabric", "fabric"),
            ("folia", "folia"),
            ("spigot", "spigot"),
        ]
        .iter()
        .find(|(needle, _)| lower.contains(needle))
        .map(|(_, core)| *core)
    }) else {
        return Vec::new();
    };
    let Ok(client) = Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .user_agent("Sculk-Catalyst-MSL-Resolver/1.0")
        .build()
    else {
        return Vec::new();
    };
    let Some(version) = minecraft_version(query) else {
        let Ok(payload) =
            fetch_json(client.get(format!("https://api.mslmc.cn/v4/mirrors/{core}"))).await
        else {
            return Vec::new();
        };
        let versions = payload
            .get("data")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|value| !value.is_empty());
        return versions
            .map(|versions| vec![format!("- MSL：{core} 可用 Minecraft 版本：{versions}")])
            .unwrap_or_default();
    };
    let Ok(payload) = fetch_json(
        client
            .get(format!(
                "https://api.mslmc.cn/v4/download/server/{core}/{version}"
            ))
            .query(&[("build", "latest")]),
    )
    .await
    else {
        return Vec::new();
    };
    let Some(url) = payload
        .get("data")
        .and_then(|data| data.get("url"))
        .and_then(Value::as_str)
    else {
        return Vec::new();
    };
    let sha256 = payload
        .get("data")
        .and_then(|data| data.get("sha256"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    vec![format!(
        "- MSL：{core} {version}，下载地址：{url}，SHA-256：{sha256}"
    )]
}

async fn search_plugin_sources(terms: &[String], query: &str) -> Vec<String> {
    let term = terms
        .iter()
        .find(|term| {
            !["paper", "purpur", "leaves", "fabric", "folia"]
                .contains(&term.to_ascii_lowercase().as_str())
        })
        .or_else(|| terms.first());
    let Some(term) = term else { return Vec::new() };
    let Ok(client) = Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .user_agent("Sculk-Catalyst-Plugin-Resolver/1.0")
        .build()
    else {
        return Vec::new();
    };
    let minecraft = minecraft_version(query);
    let mut results = Vec::new();

    if let Ok(mut url) = Url::parse("https://api.modrinth.com/v2/search") {
        url.query_pairs_mut()
            .append_pair("query", term)
            .append_pair("limit", "5")
            .append_pair("facets", "[[\"project_type:plugin\"]]");
        if let Ok(payload) = fetch_json(client.get(url)).await
            && let Some(items) = payload.get("hits").and_then(Value::as_array)
        {
            for item in items {
                let Some(title) = item.get("title").and_then(Value::as_str) else {
                    continue;
                };
                let slug = item
                    .get("slug")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let project_id = item
                    .get("project_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let project_url = format!("https://modrinth.com/plugin/{slug}");
                let download = if project_id.is_empty() {
                    None
                } else {
                    modrinth_download_url(&client, project_id, minecraft.as_deref()).await
                };
                results.push(format!(
                    "- Modrinth：{title} ({slug})，项目页：{project_url}{}",
                    download
                        .map(|url| format!("，可下载文件：{url}"))
                        .unwrap_or_default()
                ));
            }
        }
    }

    if let Ok(mut url) = Url::parse("https://www.spigotmc.org/resources/") {
        url.query_pairs_mut().append_pair("search", term);
        let search_url = url.to_string();
        if let Ok(response) = client.get(url).send().await
            && let Ok(html) = response.text().await
        {
            for path in extract_spigot_resource_links(&html).into_iter().take(5) {
                let page = format!("https://www.spigotmc.org{path}");
                results.push(format!(
                    "- SpigotMC：项目页 {page}，下载入口：{page}download"
                ));
            }
        }
        if !results.iter().any(|item| item.contains("SpigotMC")) {
            results.push(format!("- SpigotMC 官方搜索：{search_url}"));
        }
    }
    results.sort();
    results.dedup();
    results.into_iter().take(12).collect()
}

async fn modrinth_download_url(
    client: &Client,
    project_id: &str,
    minecraft: Option<&str>,
) -> Option<String> {
    let mut url = Url::parse(&format!(
        "https://api.modrinth.com/v2/project/{project_id}/version"
    ))
    .ok()?;
    url.query_pairs_mut()
        .append_pair("loaders", "[\"paper\",\"spigot\",\"bukkit\"]");
    if let Some(minecraft) = minecraft {
        url.query_pairs_mut()
            .append_pair("game_versions", &format!("[\"{minecraft}\"]"));
    }
    let payload = fetch_json(client.get(url)).await.ok()?;
    for version in payload.as_array()? {
        let files = version.get("files")?.as_array()?;
        if let Some(url) = files.iter().find_map(|file| {
            file.get("url")
                .and_then(Value::as_str)
                .filter(|url| url.starts_with("https://"))
                .map(ToString::to_string)
        }) {
            return Some(url);
        }
    }
    None
}

fn extract_spigot_resource_links(html: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(start) = html[offset..].find("href=\"/resources/") {
        let start = offset + start + 6;
        let Some(end) = html[start..].find('"') else {
            break;
        };
        let path = &html[start..start + end];
        if path.ends_with('/') && !links.iter().any(|item| item == path) {
            links.push(path.to_string());
        }
        offset = start + end + 1;
    }
    links
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

    if let Ok(mut url) = Url::parse("https://api.github.com/search/repositories") {
        url.query_pairs_mut()
            .append_pair("q", &format!("{term} Minecraft"))
            .append_pair("per_page", "5");
        if let Ok(payload) = fetch_json(client.get(url)).await
            && let Some(items) = payload.get("items").and_then(Value::as_array)
        {
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
        assert_eq!(
            minecraft_version("Paper 26.2 插件生存"),
            Some("26.2".into())
        );
        assert_eq!(minecraft_version("Paper 最新版"), None);
    }

    #[test]
    fn extracts_expected_players_from_natural_language() {
        assert_eq!(expected_player_count("使用 26.2，10人左右"), Some(10));
        assert_eq!(expected_player_count("about 30 players"), Some(30));
        assert_eq!(expected_player_count("我不知道会有多少人"), None);
    }

    #[test]
    fn plugin_survival_defaults_to_paper_evidence() {
        assert_eq!(search_terms("我想和朋友玩插件生存服"), vec!["Paper"]);
    }

    #[test]
    fn plugin_source_detection_and_spigot_links_are_bounded() {
        assert!(is_plugin_query("安装 LuckPerms 插件"));
        assert!(!is_plugin_query("普通原版生存"));
        assert_eq!(
            extract_spigot_resource_links(
                r#"<a href="/resources/luckperms.28140/">LuckPerms</a><a href="/resources/coreprotect.8631/">CoreProtect</a>"#
            ),
            vec![
                "/resources/luckperms.28140/".to_string(),
                "/resources/coreprotect.8631/".to_string()
            ]
        );
    }
}
