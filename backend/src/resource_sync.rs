use crate::{AppState, ai, internal, persist};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::Local;
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use url::Url;
use uuid::Uuid;

type ApiError = (StatusCode, String);
type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ResourceSyncState {
    #[serde(default = "default_true")]
    pub(crate) auto_enabled: bool,
    #[serde(default)]
    pub(crate) connected: bool,
    #[serde(default)]
    pub(crate) last_scan_at: Option<String>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
    #[serde(default)]
    pub(crate) jobs: Vec<SkillBuildJob>,
}

impl Default for ResourceSyncState {
    fn default() -> Self {
        Self {
            auto_enabled: true,
            connected: false,
            last_scan_at: None,
            last_error: None,
            jobs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SkillBuildJob {
    id: Uuid,
    plugin_slug: String,
    plugin_name: String,
    repository: String,
    status: String,
    stage: String,
    detail: String,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    skill_slug: Option<String>,
    #[serde(default)]
    config_slug: Option<String>,
}

#[derive(Debug, Serialize)]
struct ResourceSyncView {
    configured: bool,
    base_url: Option<String>,
    state: ResourceSyncState,
    priority: [&'static str; 4],
}

#[derive(Debug, Deserialize)]
struct SyncSettingsInput {
    auto_enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct RemoteProject {
    slug: String,
    name: String,
    repository: String,
    #[serde(default)]
    published_versions: usize,
}

#[derive(Debug, Deserialize)]
struct GeneratedSkill {
    skill_md: String,
    openai_yaml: String,
    configuration_md: String,
    config_template: Value,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/resource-sync/status", get(get_status))
        .route("/api/resource-sync/settings", put(update_settings))
        .route("/api/resource-sync/scan", post(scan_now))
        .route("/api/resource-sync/run-next", post(run_next_now))
}

pub(crate) fn spawn_worker(state: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let interval_seconds = std::env::var("SCULK_RESOURCE_SYNC_INTERVAL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(300)
            .max(30);
        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
        loop {
            interval.tick().await;
            if let Err(error) = automatic_cycle(&state).await {
                set_sync_error(&state, error).await;
            }
        }
    });
}

/// 为聊天 AI 提供已经按资源库策略排序的插件候选；远程不可用时回退本地目录。
pub(crate) async fn plugin_context_for_ai(state: &AppState, query: &str) -> String {
    if let Some(base) = resource_base_url()
        && let Ok(client) = Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .user_agent("Sculk-Catalyst-AI-Plugin-Search/1.0")
            .build()
        && let Ok(mut url) = Url::parse(&format!("{base}/api/v1/plugins/search"))
    {
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("limit", "8");
        if let Ok(payload) = send_json::<Value>(
            resource_request(&client, client.get(url)),
            "AI 检索远程插件库",
        )
        .await
        {
            let context = payload["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| {
                    format!(
                        "- {} ({}) [{}]：{}",
                        item["name"].as_str().unwrap_or("未知插件"),
                        item["slug"].as_str().unwrap_or("unknown"),
                        item["plugin_category"].as_str().unwrap_or("standard"),
                        item["summary"].as_str().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !context.is_empty() {
                return context;
            }
        }
    }
    let data = state.inner.read().await;
    crate::catalog::plugin_search_context(&data.catalog, query, 8)
}

async fn get_status(State(state): State<AppState>) -> Json<ResourceSyncView> {
    status_view(&state).await
}

async fn update_settings(
    State(state): State<AppState>,
    Json(input): Json<SyncSettingsInput>,
) -> ApiResult<ResourceSyncView> {
    let mut data = state.inner.write().await;
    data.resource_sync.auto_enabled = input.auto_enabled;
    persist(&state, &data).await.map_err(internal)?;
    drop(data);
    Ok(Json(status_view(&state).await.0))
}

async fn scan_now(State(state): State<AppState>) -> ApiResult<ResourceSyncView> {
    scan_remote(&state).await.map_err(internal)?;
    Ok(Json(status_view(&state).await.0))
}

async fn run_next_now(State(state): State<AppState>) -> ApiResult<ResourceSyncView> {
    process_next(&state, true).await.map_err(internal)?;
    Ok(Json(status_view(&state).await.0))
}

async fn status_view(state: &AppState) -> Json<ResourceSyncView> {
    let base_url = resource_base_url();
    let data = state.inner.read().await;
    Json(ResourceSyncView {
        configured: base_url.is_some(),
        base_url,
        state: data.resource_sync.clone(),
        priority: ["mainstream", "open_source", "standard", "paid"],
    })
}

async fn automatic_cycle(state: &AppState) -> Result<(), String> {
    if resource_base_url().is_none() {
        return Ok(());
    }
    let auto_enabled = state.inner.read().await.resource_sync.auto_enabled;
    if !auto_enabled || !is_idle(state).await {
        return Ok(());
    }
    scan_remote(state).await?;
    process_next(state, false).await
}

async fn is_idle(state: &AppState) -> bool {
    let data = state.inner.read().await;
    let automation_busy = data
        .tasks
        .iter()
        .any(|task| task.status == "running" && task.kind != "skill_generation");
    let generator_busy = data.resource_sync.jobs.iter().any(|job| {
        matches!(
            job.status.as_str(),
            "analyzing_source" | "generating" | "uploading"
        )
    });
    !automation_busy && !generator_busy
}

async fn scan_remote(state: &AppState) -> Result<(), String> {
    let base = resource_base_url()
        .ok_or_else(|| "未配置 SCULK_RESOURCE_API_BASE，无法连接独立资源中心".to_string())?;
    let client = resource_client()?;
    let plugins: Vec<RemoteProject> = send_json(
        resource_request(
            &client,
            client.get(format!(
                "{base}/api/catalog/plugins?plugin_category=mainstream"
            )),
        ),
        "读取主流插件库",
    )
    .await?;

    let mut missing = Vec::new();
    for plugin in plugins {
        if plugin.repository.trim().is_empty() {
            continue;
        }
        let skills: Vec<RemoteProject> = send_json(
            resource_request(
                &client,
                client.get(format!(
                    "{base}/api/catalog/skills?target_plugin={}",
                    plugin.slug
                )),
            ),
            "查询插件专属 Skill",
        )
        .await?;
        if !skills.iter().any(|skill| skill.published_versions > 0) {
            missing.push(plugin);
        }
    }

    let mut data = state.inner.write().await;
    for plugin in missing {
        let already_known = data.resource_sync.jobs.iter().any(|job| {
            job.plugin_slug == plugin.slug && !matches!(job.status.as_str(), "failed" | "cancelled")
        });
        if already_known {
            continue;
        }
        let now = Local::now().to_rfc3339();
        data.resource_sync.jobs.push(SkillBuildJob {
            id: Uuid::new_v4(),
            plugin_slug: plugin.slug,
            plugin_name: plugin.name,
            repository: plugin.repository,
            status: "queued".into(),
            stage: "waiting_idle".into(),
            detail: "主流插件缺少专属配置 Skill，已进入空闲生成队列。".into(),
            created_at: now.clone(),
            updated_at: now,
            skill_slug: None,
            config_slug: None,
        });
    }
    data.resource_sync.jobs.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.plugin_slug.cmp(&right.plugin_slug))
    });
    data.resource_sync.jobs.truncate(100);
    data.resource_sync.connected = true;
    data.resource_sync.last_scan_at = Some(Local::now().to_rfc3339());
    data.resource_sync.last_error = None;
    persist(state, &data).await
}

async fn process_next(state: &AppState, force: bool) -> Result<(), String> {
    if !force && !is_idle(state).await {
        return Ok(());
    }
    let job = {
        let mut data = state.inner.write().await;
        let Some(job) = data.resource_sync.jobs.iter_mut().find(|job| {
            matches!(
                job.status.as_str(),
                "queued" | "waiting_ai" | "waiting_source"
            )
        }) else {
            return Ok(());
        };
        job.status = "analyzing_source".into();
        job.stage = "source_discovery".into();
        job.detail = "正在读取插件源码中的配置入口、默认配置和命令声明。".into();
        job.updated_at = Local::now().to_rfc3339();
        let cloned = job.clone();
        persist(state, &data).await?;
        cloned
    };

    let result = build_and_upload(state, &job).await;
    if let Err(error) = result {
        let waiting = if error.contains("未配置可用的") {
            "waiting_ai"
        } else if error.contains("源码") || error.contains("GitHub") {
            "waiting_source"
        } else {
            "failed"
        };
        update_job(state, job.id, waiting, "blocked", &error, None, None).await?;
        return Err(error);
    }
    Ok(())
}

async fn build_and_upload(state: &AppState, job: &SkillBuildJob) -> Result<(), String> {
    let source = load_source_context(&job.repository).await?;
    update_job(
        state,
        job.id,
        "generating",
        "ai_generation",
        "源码摘要已完成，正在生成标准 SKILL.md、界面元数据和配置参考。",
        None,
        None,
    )
    .await?;

    let ai_settings = state.inner.read().await.ai.clone();
    let generated = generate_skill(&ai_settings, job, &source).await?;
    validate_generated_skill(job, &generated)?;

    update_job(
        state,
        job.id,
        "uploading",
        "resource_upload",
        "Skill 已通过结构校验，正在上传 Skill 库与插件配置库。",
        None,
        None,
    )
    .await?;

    let skill_slug = bounded_slug(&format!("configure-{}", job.plugin_slug));
    let config_slug = bounded_slug(&format!("{}-config", job.plugin_slug));
    let skill_bundle = serde_json::to_string_pretty(&json!({
        "schema_version": 1,
        "plugin": job.plugin_slug,
        "files": {
            "SKILL.md": generated.skill_md,
            "agents/openai.yaml": generated.openai_yaml,
            "references/configuration.md": generated.configuration_md
        }
    }))
    .map_err(|error| error.to_string())?;
    let config_bundle = serde_json::to_string_pretty(&json!({
        "schema_version": 1,
        "plugin": job.plugin_slug,
        "source_repository": job.repository,
        "template": generated.config_template,
        "reference": generated.configuration_md
    }))
    .map_err(|error| error.to_string())?;

    upload_inline_resource(
        "skills",
        "skill",
        &skill_slug,
        job,
        "专属插件配置 Skill",
        "skill-bundle+json",
        &skill_bundle,
    )
    .await?;
    upload_inline_resource(
        "plugin-configs",
        "plugin_config",
        &config_slug,
        job,
        "插件配置模板与字段参考",
        "plugin-config+json",
        &config_bundle,
    )
    .await?;

    update_job(
        state,
        job.id,
        "completed",
        "published",
        "专属 Skill 与插件配置已发布到远程资源库。",
        Some(skill_slug),
        Some(config_slug),
    )
    .await
}

async fn generate_skill(
    settings: &ai::AiSettings,
    job: &SkillBuildJob,
    source: &str,
) -> Result<GeneratedSkill, String> {
    let skill_name = bounded_slug(&format!("configure-{}", job.plugin_slug));
    let system = r#"你是 Minecraft 插件配置 Skill 构建器。只输出一个 JSON 对象，不要 Markdown 围栏。Skill 必须简洁、可执行并符合 Codex Skill 规范：SKILL.md 仅包含 name 和 description 两个 frontmatter 字段；正文使用祈使式工作流；详细字段知识放 references/configuration.md；agents/openai.yaml 的字符串全部加引号，default_prompt 必须显式提及 $skill-name。禁止臆造源码中不存在的字段。"#;
    let user = format!(
        "插件：{} ({})\n仓库：{}\n目标 skill name：{}\n\n根据以下源码摘要生成：\n{{\"skill_md\":\"...\",\"openai_yaml\":\"...\",\"configuration_md\":\"...\",\"config_template\":{{}}}}\n\n源码摘要：\n{}",
        job.plugin_name, job.plugin_slug, job.repository, skill_name, source
    );
    let response = ai::complete_text(settings, "config", system, &user).await?;
    let payload = strip_json_fence(&response);
    serde_json::from_str(payload).map_err(|error| format!("AI Skill JSON 无法解析：{error}"))
}

fn validate_generated_skill(job: &SkillBuildJob, generated: &GeneratedSkill) -> Result<(), String> {
    let expected_name = bounded_slug(&format!("configure-{}", job.plugin_slug));
    let skill = generated.skill_md.trim();
    let mut sections = skill.splitn(3, "---");
    let _before = sections.next();
    let frontmatter = sections.next().unwrap_or_default();
    let body = sections.next().unwrap_or_default().trim();
    let fields: Vec<(&str, &str)> = frontmatter
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim(), value.trim().trim_matches('"')))
        .collect();
    let valid_name = fields
        .iter()
        .any(|(key, value)| *key == "name" && *value == expected_name);
    let has_description = fields
        .iter()
        .any(|(key, value)| *key == "description" && !value.is_empty());
    let only_allowed_fields = fields
        .iter()
        .all(|(key, _)| matches!(*key, "name" | "description"));
    if !skill.starts_with("---")
        || !valid_name
        || !has_description
        || !only_allowed_fields
        || fields.len() != 2
        || body.is_empty()
    {
        return Err("生成的 SKILL.md frontmatter 不符合规范".into());
    }
    if generated.configuration_md.trim().is_empty() {
        return Err("生成结果缺少 references/configuration.md".into());
    }
    if !generated.openai_yaml.contains(&format!("${expected_name}")) {
        return Err("agents/openai.yaml 的 default_prompt 未引用 Skill".into());
    }
    Ok(())
}

async fn upload_inline_resource(
    resource: &str,
    kind: &str,
    slug: &str,
    job: &SkillBuildJob,
    summary: &str,
    format: &str,
    content: &str,
) -> Result<(), String> {
    let base = resource_base_url().ok_or_else(|| "资源中心未配置".to_string())?;
    let client = resource_client()?;
    let project_url = format!("{base}/api/catalog/{resource}/{slug}");
    let existing = resource_request(&client, client.get(&project_url))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if existing.status() == reqwest::StatusCode::NOT_FOUND {
        let body = json!({
            "slug": slug,
            "name": format!("{} {}", job.plugin_name, if kind == "skill" { "配置 Skill" } else { "配置库" }),
            "summary": summary,
            "description": format!("根据 {} 源码生成，服务于 {} 的配置编写、校验与升级。", job.repository, job.plugin_name),
            "author": "Sculk Skill Builder",
            "homepage": job.repository,
            "repository": job.repository,
            "preview_url": "",
            "license": "Generated from upstream source; follow upstream license",
            "plugin_category": "",
            "target_plugin": job.plugin_slug,
            "tags": ["AI 生成", "插件配置", job.plugin_name.clone()],
            "color": if kind == "skill" { "#9c8cff" } else { "#32d5b0" },
            "featured": false
        });
        send_empty(
            resource_request(
                &client,
                client
                    .post(format!("{base}/api/catalog/{resource}"))
                    .json(&body),
            ),
            "创建远程资源项目",
        )
        .await?;
    } else if !existing.status().is_success() {
        return Err(format!("查询远程资源项目失败：HTTP {}", existing.status()));
    }

    let version = format!("0.1.0+{}", Local::now().format("%Y%m%d%H%M%S"));
    let size = content.len() as u64;
    let sha256 = format!("{:x}", Sha256::digest(content.as_bytes()));
    let body = json!({
        "version": version,
        "channel": "stable",
        "minecraft_versions": [],
        "loaders": [],
        "formats": [format],
        "java_version": null,
        "filename": format!("{slug}.json"),
        "size": size,
        "sha256": sha256,
        "download_url": "",
        "content": content,
        "release_notes": "总站空闲期间根据上游源码自动生成。",
        "released_at": Local::now().to_rfc3339(),
        "status": "published"
    });
    send_empty(
        resource_request(
            &client,
            client
                .post(format!("{base}/api/catalog/{resource}/{slug}/versions"))
                .json(&body),
        ),
        "上传远程资源版本",
    )
    .await
}

async fn load_source_context(repository: &str) -> Result<String, String> {
    let url = Url::parse(repository).map_err(|_| "插件源码仓库 URL 无效".to_string())?;
    if url.host_str() != Some("github.com") {
        return Err("当前自动源码分析支持 GitHub 仓库；其他 Git 源等待适配".into());
    }
    let segments: Vec<&str> = url
        .path_segments()
        .map(|segments| segments.filter(|value| !value.is_empty()).collect())
        .unwrap_or_default();
    if segments.len() < 2 {
        return Err("GitHub 源码仓库地址缺少 owner/repo".into());
    }
    let owner = segments[0];
    let repo = segments[1].trim_end_matches(".git");
    let client = Client::builder()
        .user_agent("Sculk-Catalyst-Skill-Builder/1.0")
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?;
    let repo_info: Value = send_json(
        github_request(client.get(format!("https://api.github.com/repos/{owner}/{repo}"))),
        "读取 GitHub 仓库信息",
    )
    .await?;
    let branch = repo_info["default_branch"]
        .as_str()
        .ok_or_else(|| "GitHub 仓库未返回默认分支".to_string())?;
    let tree: Value = send_json(
        github_request(client.get(format!(
            "https://api.github.com/repos/{owner}/{repo}/git/trees/{branch}?recursive=1"
        ))),
        "读取 GitHub 源码树",
    )
    .await?;
    let mut paths: Vec<String> = tree["tree"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "blob")
        .filter_map(|item| item["path"].as_str())
        .filter(|path| source_path_score(path) < 100)
        .map(str::to_string)
        .collect();
    paths.sort_by_key(|path| source_path_score(path));
    paths.truncate(12);
    if paths.is_empty() {
        return Err("源码中没有找到配置、插件声明或说明文档".into());
    }

    let mut context = String::new();
    for path in paths {
        if context.len() >= 120_000 {
            break;
        }
        let raw_url = format!(
            "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{}",
            path.replace(' ', "%20")
        );
        let response = github_request(client.get(raw_url))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            continue;
        }
        let text = response.text().await.map_err(|error| error.to_string())?;
        let remaining = 120_000usize.saturating_sub(context.len());
        context.push_str(&format!("\n\n===== {path} =====\n"));
        context.extend(text.chars().take(remaining));
    }
    if context.trim().is_empty() {
        return Err("GitHub 源码文件读取失败".into());
    }
    Ok(context)
}

fn source_path_score(path: &str) -> u8 {
    let lower = path.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "src/main/resources/config.yml"
            | "src/main/resources/plugin.yml"
            | "src/main/resources/paper-plugin.yml"
    ) {
        return 0;
    }
    if lower.contains("config") && (lower.ends_with(".yml") || lower.ends_with(".yaml")) {
        return 1;
    }
    if lower.ends_with("plugin.yml") || lower.ends_with("paper-plugin.yml") {
        return 2;
    }
    if lower.starts_with("readme") || lower.contains("/docs/") {
        return 3;
    }
    if lower.contains("config")
        && [".java", ".kt", ".json", ".toml", ".properties"]
            .iter()
            .any(|extension| lower.ends_with(extension))
    {
        return 4;
    }
    100
}

async fn update_job(
    state: &AppState,
    id: Uuid,
    status: &str,
    stage: &str,
    detail: &str,
    skill_slug: Option<String>,
    config_slug: Option<String>,
) -> Result<(), String> {
    let mut data = state.inner.write().await;
    let job = data
        .resource_sync
        .jobs
        .iter_mut()
        .find(|job| job.id == id)
        .ok_or_else(|| "Skill 生成任务不存在".to_string())?;
    job.status = status.into();
    job.stage = stage.into();
    job.detail = detail.into();
    job.updated_at = Local::now().to_rfc3339();
    if skill_slug.is_some() {
        job.skill_slug = skill_slug;
    }
    if config_slug.is_some() {
        job.config_slug = config_slug;
    }
    persist(state, &data).await
}

async fn set_sync_error(state: &AppState, error: String) {
    let mut data = state.inner.write().await;
    data.resource_sync.last_error = Some(error);
    let _ = persist(state, &data).await;
}

pub(crate) fn resource_base_url() -> Option<String> {
    let value = std::env::var("SCULK_RESOURCE_API_BASE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://res.mcmy.love".into());
    let value = value.trim().trim_end_matches('/').to_string();
    Url::parse(&value).is_ok().then_some(value)
}

fn resource_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(90))
        .user_agent("Sculk-Catalyst-Resource-Sync/1.0")
        .build()
        .map_err(|error| error.to_string())
}

fn resource_request(_client: &Client, builder: RequestBuilder) -> RequestBuilder {
    match std::env::var("SCULK_RESOURCE_API_TOKEN") {
        Ok(token) if !token.trim().is_empty() => builder.bearer_auth(token.trim()),
        _ => builder,
    }
}

fn github_request(builder: RequestBuilder) -> RequestBuilder {
    match std::env::var("GITHUB_TOKEN") {
        Ok(token) if !token.trim().is_empty() => builder.bearer_auth(token.trim()),
        _ => builder,
    }
}

async fn send_json<T: for<'de> Deserialize<'de>>(
    builder: RequestBuilder,
    action: &str,
) -> Result<T, String> {
    let response = builder.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("{action}失败：HTTP {status} {body}"));
    }
    response.json().await.map_err(|error| error.to_string())
}

async fn send_empty(builder: RequestBuilder, action: &str) -> Result<(), String> {
    let response = builder.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(format!("{action}失败：HTTP {status} {body}"))
}

fn strip_json_fence(value: &str) -> &str {
    let trimmed = value.trim();
    let trimmed = trimmed.strip_prefix("```json").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix("```").unwrap_or(trimmed);
    trimmed.strip_suffix("```").unwrap_or(trimmed).trim()
}

fn bounded_slug(value: &str) -> String {
    value
        .chars()
        .take(64)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string()
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_paths_prioritize_default_config_and_plugin_descriptors() {
        assert_eq!(source_path_score("src/main/resources/config.yml"), 0);
        assert!(source_path_score("src/main/java/demo/ConfigManager.java") < 100);
        assert_eq!(source_path_score("assets/logo.png"), 100);
    }

    #[test]
    fn bounded_skill_names_remain_valid_catalog_slugs() {
        let value = bounded_slug(&format!("configure-{}", "a".repeat(80)));
        assert_eq!(value.len(), 64);
        assert!(!value.ends_with('-'));
    }
}
