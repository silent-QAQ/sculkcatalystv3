use crate::{
    AppState,
    catalog::{self, CatalogProject, CatalogVersion},
    persist,
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    middleware::from_fn,
    routing::{get, post},
};
use chrono::{DateTime, Local};
use reqwest::{
    Client, Response,
    header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, RANGE, RETRY_AFTER, USER_AGENT},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex, RwLock, Semaphore},
    task::JoinSet,
    time::Instant,
};
use url::Url;
use uuid::Uuid;

const DEFAULT_MSL_API_BASE: &str = "https://api.mslmc.cn/v4";
const DEFAULT_FASTMIRROR_API_BASE: &str = "https://download.fastmirror.net";
const DEFAULT_POLARS_API_BASE: &str = "https://mirror.polars.cc";
const DEFAULT_MSL_TARGET_VERSIONS: [&str; 9] = [
    "1.12.2", "1.16", "1.18", "1.20.1", "1.20.4", "1.21.1", "1.21.11", "26.1.2", "26.2",
];
const MSL_PROJECT_TAG: &str = "msl-mirror";
const MSL_RELEASE_PREFIX: &str = "MSL_AUTO_SYNC ";
const MSL_PLACEHOLDER_PREFIX: &str = "MSL_AUTO_SYNC pending;";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MslCoreSyncStatus {
    enabled: bool,
    base_url: String,
    target_versions: Vec<String>,
    interval_seconds: u64,
    mirror_sources: Vec<String>,
    running: bool,
    last_started_at: Option<String>,
    last_finished_at: Option<String>,
    last_error: Option<String>,
    core_types: usize,
    matching_core_types: usize,
    projects_created: usize,
    projects_refreshed: usize,
    versions_upserted: usize,
    versions_removed: usize,
    skipped_manual_versions: usize,
    placeholders_created: usize,
    versions_resolved: usize,
    sizes_backfilled: usize,
    pending_versions: usize,
    failures: Vec<String>,
}

impl Default for MslCoreSyncStatus {
    fn default() -> Self {
        Self {
            enabled: sync_enabled(),
            base_url: api_base(),
            target_versions: target_versions(),
            interval_seconds: sync_interval_seconds(),
            mirror_sources: mirror_source_names(),
            running: false,
            last_started_at: None,
            last_finished_at: None,
            last_error: None,
            core_types: 0,
            matching_core_types: 0,
            projects_created: 0,
            projects_refreshed: 0,
            versions_upserted: 0,
            versions_removed: 0,
            skipped_manual_versions: 0,
            placeholders_created: 0,
            versions_resolved: 0,
            sizes_backfilled: 0,
            pending_versions: 0,
            failures: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct MslEnvelope<T> {
    code: i64,
    #[serde(default)]
    message: String,
    data: T,
}

#[derive(Debug, Deserialize)]
struct MslCoreInfo {
    #[serde(default)]
    description: String,
    #[serde(default)]
    versions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MslDownload {
    url: String,
    #[serde(default)]
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct FastMirrorEnvelope<T> {
    success: bool,
    data: T,
}

#[derive(Debug, Deserialize)]
struct FastMirrorBuildList {
    #[serde(default)]
    builds: Vec<FastMirrorBuild>,
}

#[derive(Debug, Deserialize)]
struct FastMirrorBuild {
    core_version: String,
    #[serde(default)]
    update_time: String,
    #[serde(default)]
    sha1: String,
}

#[derive(Debug, Deserialize)]
struct FastMirrorDownload {
    core_version: String,
    #[serde(default)]
    update_time: String,
    #[serde(default)]
    sha1: String,
    #[serde(default)]
    filename: String,
    download_url: String,
}

#[derive(Clone, Debug, Deserialize)]
struct PolarsCoreEntry {
    #[serde(rename = "id")]
    _id: Value,
    name: String,
    #[serde(rename = "downloadUrl")]
    download_url: String,
    #[serde(rename = "syncTime", default)]
    sync_time: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MirrorSource {
    Msl,
    FastMirror,
    Polars,
}

#[derive(Debug)]
struct ImportedCore {
    project: CatalogProject,
    matches_target: bool,
    versions: Vec<CatalogVersion>,
    failures: Vec<String>,
    placeholders_created: usize,
    versions_resolved: usize,
    sizes_backfilled: usize,
}

#[derive(Debug, Default)]
struct SyncResult {
    core_types: usize,
    matching_core_types: usize,
    projects_created: usize,
    projects_refreshed: usize,
    versions_upserted: usize,
    versions_removed: usize,
    skipped_manual_versions: usize,
    failures: Vec<String>,
    placeholders_created: usize,
    versions_resolved: usize,
    sizes_backfilled: usize,
    pending_versions: usize,
}

static STATUS: OnceLock<RwLock<MslCoreSyncStatus>> = OnceLock::new();
static SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static REQUEST_GATE: OnceLock<Mutex<Instant>> = OnceLock::new();

fn status_store() -> &'static RwLock<MslCoreSyncStatus> {
    STATUS.get_or_init(|| RwLock::new(MslCoreSyncStatus::default()))
}

fn sync_lock() -> &'static Mutex<()> {
    SYNC_LOCK.get_or_init(|| Mutex::new(()))
}

fn request_gate() -> &'static Mutex<Instant> {
    REQUEST_GATE.get_or_init(|| Mutex::new(Instant::now() - msl_request_interval()))
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/catalog/admin/msl-core-status", get(get_sync_status))
        .route("/api/catalog/admin/sync-msl-cores", post(sync_now))
        .layer(from_fn(catalog::require_catalog_admin))
}

pub(crate) fn spawn_worker(state: AppState) {
    if !sync_enabled() {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            if let Err(error) = run_sync(&state).await {
                eprintln!("MSL core mirror sync failed: {error}");
            }
            tokio::time::sleep(Duration::from_secs(sync_interval_seconds())).await;
        }
    });
}

async fn get_sync_status() -> Json<MslCoreSyncStatus> {
    Json(status_store().read().await.clone())
}

async fn sync_now(
    State(state): State<AppState>,
) -> Result<Json<MslCoreSyncStatus>, (StatusCode, String)> {
    run_sync(&state)
        .await
        .map(Json)
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))
}

async fn run_sync(state: &AppState) -> Result<MslCoreSyncStatus, String> {
    let _guard = sync_lock()
        .try_lock()
        .map_err(|_| "MSL core mirror sync is already running".to_string())?;
    let started_at = Local::now().to_rfc3339();
    {
        let mut status = status_store().write().await;
        status.enabled = sync_enabled();
        status.base_url = api_base();
        status.target_versions = target_versions();
        status.interval_seconds = sync_interval_seconds();
        status.mirror_sources = mirror_source_names();
        status.running = true;
        status.last_started_at = Some(started_at);
        status.last_error = None;
        status.failures.clear();
    }

    match sync_catalog(state).await {
        Ok(result) => {
            let mut status = status_store().write().await;
            status.running = false;
            status.last_finished_at = Some(Local::now().to_rfc3339());
            status.core_types = result.core_types;
            status.matching_core_types = result.matching_core_types;
            status.projects_created = result.projects_created;
            status.projects_refreshed = result.projects_refreshed;
            status.versions_upserted = result.versions_upserted;
            status.versions_removed = result.versions_removed;
            status.skipped_manual_versions = result.skipped_manual_versions;
            status.placeholders_created = result.placeholders_created;
            status.versions_resolved = result.versions_resolved;
            status.sizes_backfilled = result.sizes_backfilled;
            status.pending_versions = result.pending_versions;
            let failure_count = result.failures.len();
            status.failures = result.failures.into_iter().take(20).collect();
            status.last_error =
                (failure_count > 0).then(|| format!("本轮有 {failure_count} 个缺失项未能补齐"));
            Ok(status.clone())
        }
        Err(error) => {
            let mut status = status_store().write().await;
            status.running = false;
            status.last_finished_at = Some(Local::now().to_rfc3339());
            status.last_error = Some(error.clone());
            Err(error)
        }
    }
}

async fn sync_catalog(state: &AppState) -> Result<SyncResult, String> {
    let base = api_base();
    Url::parse(&base).map_err(|error| format!("invalid MSL API base: {error}"))?;
    validate_https_api_base("FastMirror", &fastmirror_base())?;
    validate_https_api_base("Polars", &polars_base())?;
    let client = Client::builder()
        .user_agent(msl_user_agent())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()
        .map_err(|error| error.to_string())?;
    let classified: Value = get_json(&client, format!("{base}/mirrors")).await?;
    let entries = classified_core_entries(&classified)?;
    let core_types = entries.len();
    let targets = target_versions();
    let existing_versions = Arc::new({
        let data = state.inner.read().await;
        data.catalog
            .core_versions
            .iter()
            .map(|version| {
                (
                    (version.project.clone(), version.version.clone()),
                    version.clone(),
                )
            })
            .collect::<HashMap<_, _>>()
    });
    let download_quota_exhausted = Arc::new(AtomicBool::new(false));
    let semaphore = Arc::new(Semaphore::new(msl_concurrency()));
    let mut tasks = JoinSet::new();

    for (slug, category) in entries {
        let client = client.clone();
        let base = base.clone();
        let targets = targets.clone();
        let semaphore = semaphore.clone();
        let existing_versions = existing_versions.clone();
        let download_quota_exhausted = download_quota_exhausted.clone();
        tasks.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|error| error.to_string())?;
            let result = fetch_core(
                &client,
                &base,
                &slug,
                &category,
                &targets,
                &existing_versions,
                &download_quota_exhausted,
            )
            .await;
            Ok::<_, String>((slug, result))
        });
    }

    let mut imports = Vec::new();
    let mut failures = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok((_slug, Ok(imported)))) => imports.push(imported),
            Ok(Ok((slug, Err(error)))) => failures.push(format!("{slug}: {error}")),
            Ok(Err(error)) => failures.push(error),
            Err(error) => failures.push(format!("mirror task failed: {error}")),
        }
    }

    let mut result = SyncResult {
        core_types,
        matching_core_types: imports
            .iter()
            .filter(|imported| imported.matches_target)
            .count(),
        failures,
        ..SyncResult::default()
    };
    for imported in &imports {
        result.failures.extend(imported.failures.iter().cloned());
        result.placeholders_created += imported.placeholders_created;
        result.versions_resolved += imported.versions_resolved;
        result.sizes_backfilled += imported.sizes_backfilled;
    }

    let mut data = state.inner.write().await;

    for imported in imports {
        match data
            .catalog
            .core_projects
            .iter()
            .position(|project| project.slug == imported.project.slug)
        {
            Some(index) => {
                if is_msl_project(&data.catalog.core_projects[index]) {
                    data.catalog.core_projects[index] = imported.project.clone();
                    result.projects_refreshed += 1;
                }
            }
            None => {
                data.catalog.core_projects.push(imported.project.clone());
                result.projects_created += 1;
            }
        }

        for mut version in imported.versions {
            if let Some(index) = data.catalog.core_versions.iter().position(|current| {
                current.project == version.project && current.version == version.version
            }) {
                let current = &data.catalog.core_versions[index];
                if is_msl_managed_version(current) {
                    preserve_catalog_identity(current, &mut version);
                    data.catalog.core_versions[index] = version;
                    result.versions_upserted += 1;
                } else {
                    result.skipped_manual_versions += 1;
                }
            } else {
                data.catalog.core_versions.push(version);
                result.versions_upserted += 1;
            }
        }
    }
    data.catalog
        .core_projects
        .sort_by(|left, right| left.name.cmp(&right.name));
    result.pending_versions = count_pending_versions(&data.catalog.core_versions);
    persist(state, &data).await?;
    Ok(result)
}

async fn fetch_core(
    client: &Client,
    base: &str,
    slug: &str,
    category: &str,
    targets: &[String],
    existing_versions: &HashMap<(String, String), CatalogVersion>,
    download_quota_exhausted: &AtomicBool,
) -> Result<ImportedCore, String> {
    let info: MslCoreInfo = get_json(client, format!("{base}/mirrors/{slug}")).await?;
    let project = mirror_project(slug, category, &info.description);
    let matches_target = targets
        .iter()
        .any(|target| info.versions.iter().any(|version| version == target));
    let mut versions = Vec::new();
    let mut failures = Vec::new();
    let mut placeholders_created = 0;
    let mut versions_resolved = 0;
    let mut sizes_backfilled = 0;

    for target in targets {
        let existing = reusable_catalog_version(existing_versions, slug, target);
        if existing.is_some_and(automatic_version_needs_size_backfill) {
            let existing = existing.expect("existing version was checked");
            let (size, _) = inspect_download(client, &existing.download_url).await;
            if size > 0 {
                let mut backfilled = existing.clone();
                backfilled.size = size;
                versions.push(backfilled);
                sizes_backfilled += 1;
            }
            continue;
        }
        if !catalog_version_needs_resolution(existing) {
            continue;
        }
        if existing.is_none() && !info.versions.iter().any(|version| version == target) {
            continue;
        }
        let placeholder = existing
            .cloned()
            .unwrap_or_else(|| mirror_version_placeholder(base, slug, category, target));
        let placeholder_is_new = existing.is_none();
        match tokio::time::timeout(
            Duration::from_secs(90),
            resolve_latest_version(
                client,
                base,
                slug,
                category,
                target,
                download_quota_exhausted,
            ),
        )
        .await
        {
            Ok(Ok(version)) => {
                versions.push(version);
                versions_resolved += 1;
            }
            Ok(Err(error)) => {
                if placeholder_is_new {
                    versions.push(placeholder);
                    placeholders_created += 1;
                }
                failures.push(format!("{slug}/{target}: {error}"));
            }
            Err(_) => {
                if placeholder_is_new {
                    versions.push(placeholder);
                    placeholders_created += 1;
                }
                failures.push(format!(
                    "{slug}/{target}: mirror resolution exceeded 90 seconds"
                ));
            }
        }
    }
    Ok(ImportedCore {
        project,
        matches_target,
        versions,
        failures,
        placeholders_created,
        versions_resolved,
        sizes_backfilled,
    })
}

async fn resolve_latest_version(
    client: &Client,
    msl_base: &str,
    slug: &str,
    category: &str,
    minecraft: &str,
    download_quota_exhausted: &AtomicBool,
) -> Result<CatalogVersion, String> {
    let mut failures = Vec::new();
    for source in mirror_attempt_order(slug) {
        let resolved = match source {
            MirrorSource::Msl => {
                fetch_msl_version(
                    client,
                    msl_base,
                    slug,
                    category,
                    minecraft,
                    download_quota_exhausted,
                )
                .await
            }
            MirrorSource::FastMirror => {
                fetch_fastmirror_version(client, slug, category, minecraft).await
            }
            MirrorSource::Polars => fetch_polars_version(client, slug, category, minecraft).await,
        };
        match resolved {
            Ok(version) => return Ok(version),
            Err(error) => failures.push(format!("{}: {error}", source.name())),
        }
    }
    Err(failures.join(" | "))
}

async fn fetch_msl_version(
    client: &Client,
    base: &str,
    slug: &str,
    category: &str,
    minecraft: &str,
    download_quota_exhausted: &AtomicBool,
) -> Result<CatalogVersion, String> {
    let builds: Vec<String> =
        get_json(client, format!("{base}/mirrors/{slug}/{minecraft}")).await?;
    let build =
        latest_build(&builds).ok_or_else(|| "mirror returned no usable builds".to_string())?;
    if download_quota_exhausted.load(Ordering::Relaxed) {
        return Err("MSL download resolution quota is exhausted for this run".into());
    }
    let download: MslDownload = match get_json_with_query(
        client,
        format!("{base}/download/server/{slug}/{minecraft}"),
        &[("build", build.as_str())],
    )
    .await
    {
        Ok(download) => download,
        Err(error) => {
            if error.contains("HTTP 429") {
                download_quota_exhausted.store(true, Ordering::Relaxed);
            }
            return Err(error);
        }
    };
    let parsed = Url::parse(&download.url)
        .map_err(|error| format!("mirror returned an invalid download URL: {error}"))?;
    if parsed.scheme() != "https" {
        return Err("mirror download URL must use HTTPS".into());
    }
    let (size, header_filename) = inspect_download(client, &download.url).await;
    let fallback_filename = format!(
        "{}-{}-{}.jar",
        slug,
        minecraft,
        build.replace(['/', '\\'], "-")
    );
    let filename = safe_filename(
        header_filename
            .or_else(|| {
                parsed
                    .path_segments()
                    .and_then(|mut segments| segments.next_back())
                    .map(ToString::to_string)
            })
            .as_deref()
            .unwrap_or(&fallback_filename),
        &fallback_filename,
    );
    let sha256 = download.sha256.trim().to_ascii_lowercase();
    let sha256 = if sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        sha256
    } else {
        String::new()
    };
    Ok(CatalogVersion {
        id: Uuid::new_v4().to_string(),
        project: slug.into(),
        version: minecraft.into(),
        channel: "stable".into(),
        minecraft_versions: vec![minecraft.into()],
        loaders: loaders_for(category, slug),
        formats: Vec::new(),
        java_version: java_version_for(minecraft),
        filename,
        size,
        sha256,
        download_url: download.url,
        content: String::new(),
        release_notes: format!(
            "{MSL_RELEASE_PREFIX}source=MSL; build={build}; 本服务由MSL开服器提供；该 Minecraft 版本首次解析后固定此构建。"
        ),
        released_at: Local::now().to_rfc3339(),
        status: "published".into(),
        downloads: 0,
    })
}

async fn fetch_fastmirror_version(
    client: &Client,
    slug: &str,
    category: &str,
    minecraft: &str,
) -> Result<CatalogVersion, String> {
    let core = fastmirror_core(slug).ok_or_else(|| format!("core {slug} is not supported"))?;
    let mirror_minecraft = fastmirror_minecraft(slug, minecraft);
    let base = fastmirror_base();
    let list_url = url_with_segments(&base, &["api", "v3", core, &mirror_minecraft])?;
    let envelope: FastMirrorEnvelope<FastMirrorBuildList> =
        get_external_json_with_query(client, list_url, &[("offset", "0"), ("limit", "1")]).await?;
    if !envelope.success {
        return Err("build list request was not successful".into());
    }
    let build = envelope
        .data
        .builds
        .first()
        .ok_or_else(|| "mirror returned no builds".to_string())?;
    let build_id = build.core_version.trim();
    if build_id.is_empty() {
        return Err("mirror returned an empty build identifier".into());
    }
    let detail_url = url_with_segments(&base, &["api", "v3", core, &mirror_minecraft, build_id])?;
    let detail: FastMirrorEnvelope<FastMirrorDownload> =
        get_external_json_with_query(client, detail_url, &[]).await?;
    if !detail.success {
        return Err("download detail request was not successful".into());
    }
    let detail = detail.data;
    let parsed = secure_download_url(&detail.download_url, false)?;
    let download_url = parsed.to_string();
    let (size, header_filename) = inspect_download(client, &download_url).await;
    let fallback_filename = format!("{slug}-{minecraft}-{build_id}.jar");
    let filename = resolved_filename(
        header_filename,
        Some(detail.filename.as_str()),
        &parsed,
        &fallback_filename,
    );
    let update_time = if detail.update_time.trim().is_empty() {
        build.update_time.trim()
    } else {
        detail.update_time.trim()
    };
    let sha1 = if detail.sha1.trim().is_empty() {
        build.sha1.trim()
    } else {
        detail.sha1.trim()
    };
    Ok(resolved_catalog_version(
        slug,
        category,
        minecraft,
        filename,
        size,
        String::new(),
        download_url,
        format!(
            "{MSL_RELEASE_PREFIX}source=FastMirror; build={}; sha1={}; update_time={}; 首次解析后固定此构建。",
            detail.core_version.trim(),
            safe_note_value(sha1),
            safe_note_value(update_time)
        ),
    ))
}

async fn fetch_polars_version(
    client: &Client,
    slug: &str,
    category: &str,
    minecraft: &str,
) -> Result<CatalogVersion, String> {
    let (core_id, filename_prefix) =
        polars_core(slug).ok_or_else(|| format!("core {slug} is not supported"))?;
    let base = polars_base();
    let url = url_with_segments(
        &base,
        &["api", "query", "minecraft", "core", &core_id.to_string()],
    )?;
    let entries: Vec<PolarsCoreEntry> = get_external_json_with_query(client, url, &[]).await?;
    let entry = select_polars_entry(&entries, filename_prefix, minecraft)
        .ok_or_else(|| format!("no file strictly matches Minecraft {minecraft}"))?;
    let parsed = secure_download_url(&entry.download_url, true)?;
    let download_url = parsed.to_string();
    let (size, header_filename) = inspect_download(client, &download_url).await;
    let fallback_filename = format!("{slug}-{minecraft}-polars.jar");
    let filename = resolved_filename(
        header_filename,
        Some(entry.name.as_str()),
        &parsed,
        &fallback_filename,
    );
    Ok(resolved_catalog_version(
        slug,
        category,
        minecraft,
        filename,
        size,
        String::new(),
        download_url,
        format!(
            "{MSL_RELEASE_PREFIX}source=Polars; sync_time={}; 首次解析后固定此构建。",
            safe_note_value(&json_scalar_text(&entry.sync_time))
        ),
    ))
}

#[allow(clippy::too_many_arguments)]
fn resolved_catalog_version(
    slug: &str,
    category: &str,
    minecraft: &str,
    filename: String,
    size: u64,
    sha256: String,
    download_url: String,
    release_notes: String,
) -> CatalogVersion {
    CatalogVersion {
        id: Uuid::new_v4().to_string(),
        project: slug.into(),
        version: minecraft.into(),
        channel: "stable".into(),
        minecraft_versions: vec![minecraft.into()],
        loaders: loaders_for(category, slug),
        formats: Vec::new(),
        java_version: java_version_for(minecraft),
        filename,
        size,
        sha256,
        download_url,
        content: String::new(),
        release_notes,
        released_at: Local::now().to_rfc3339(),
        status: "published".into(),
        downloads: 0,
    }
}

fn mirror_version_placeholder(
    base: &str,
    slug: &str,
    category: &str,
    minecraft: &str,
) -> CatalogVersion {
    let fallback_filename = format!("{slug}-{minecraft}-pending.jar");
    CatalogVersion {
        id: Uuid::new_v4().to_string(),
        project: slug.into(),
        version: minecraft.into(),
        channel: "stable".into(),
        minecraft_versions: vec![minecraft.into()],
        loaders: loaders_for(category, slug),
        formats: Vec::new(),
        java_version: java_version_for(minecraft),
        filename: safe_filename(&fallback_filename, "server-pending.jar"),
        size: 0,
        sha256: String::new(),
        download_url: format!("{base}/mirrors/{slug}/{minecraft}"),
        content: String::new(),
        release_notes: format!(
            "{MSL_PLACEHOLDER_PREFIX} 等待镜像下载解析；本服务由MSL开服器提供。"
        ),
        released_at: Local::now().to_rfc3339(),
        status: "draft".into(),
        downloads: 0,
    }
}

async fn inspect_download(client: &Client, url: &str) -> (u64, Option<String>) {
    let head = client
        .head(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    if let Ok(response) = head
        && response.status().is_success()
    {
        let size = response_header_u64(&response, CONTENT_LENGTH);
        let filename = response
            .headers()
            .get(CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .and_then(content_disposition_filename);
        if size > 0 {
            return (size, filename);
        }
    }

    let Ok(response) = client
        .get(url)
        .header(RANGE, "bytes=0-0")
        .timeout(Duration::from_secs(5))
        .send()
        .await
    else {
        return (0, None);
    };
    if !response.status().is_success() {
        return (0, None);
    }
    let size = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(content_range_total)
        .unwrap_or_else(|| response_header_u64(&response, CONTENT_LENGTH));
    let filename = response
        .headers()
        .get(CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .and_then(content_disposition_filename);
    (size, filename)
}

fn response_header_u64(response: &Response, name: reqwest::header::HeaderName) -> u64 {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.parse::<u64>().ok()
}

async fn get_json<T: DeserializeOwned>(client: &Client, url: String) -> Result<T, String> {
    get_json_with_query(client, url, &[]).await
}

async fn get_json_with_query<T: DeserializeOwned>(
    client: &Client,
    url: String,
    query: &[(&str, &str)],
) -> Result<T, String> {
    const MAX_ATTEMPTS: u32 = 2;
    let mut last_error = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        wait_for_request_slot().await;
        let response = client
            .get(&url)
            .header(USER_AGENT, msl_user_agent())
            .query(query)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                last_error = format!("request failed for {url}: {error}");
                if attempt + 1 < MAX_ATTEMPTS {
                    tokio::time::sleep(transient_retry_delay(attempt)).await;
                    continue;
                }
                return Err(last_error);
            }
        };
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            last_error = format!("{url} returned HTTP {status}");
            if attempt + 1 < MAX_ATTEMPTS {
                tokio::time::sleep(response_retry_delay(&response, attempt)).await;
                continue;
            }
            return Err(last_error);
        }
        if status.is_server_error() {
            last_error = format!("{url} returned HTTP {status}");
            if attempt + 1 < MAX_ATTEMPTS {
                tokio::time::sleep(transient_retry_delay(attempt)).await;
                continue;
            }
            return Err(last_error);
        }
        if !status.is_success() {
            return Err(format!("{url} returned HTTP {status}"));
        }
        let envelope: MslEnvelope<T> = response
            .json()
            .await
            .map_err(|error| format!("invalid JSON from {url}: {error}"))?;
        if envelope.code != 200 {
            return Err(if envelope.message.is_empty() {
                format!("{url} returned API code {}", envelope.code)
            } else {
                envelope.message
            });
        }
        return Ok(envelope.data);
    }
    Err(last_error)
}

async fn get_external_json_with_query<T: DeserializeOwned>(
    client: &Client,
    url: String,
    query: &[(&str, &str)],
) -> Result<T, String> {
    const MAX_ATTEMPTS: u32 = 2;
    let mut last_error = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        wait_for_request_slot().await;
        let response = client
            .get(&url)
            .header(USER_AGENT, msl_user_agent())
            .query(query)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                last_error = format!("request failed for {url}: {error}");
                if attempt + 1 < MAX_ATTEMPTS {
                    tokio::time::sleep(transient_retry_delay(attempt)).await;
                    continue;
                }
                return Err(last_error);
            }
        };
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            last_error = format!("{url} returned HTTP {status}");
            if attempt + 1 < MAX_ATTEMPTS {
                let delay = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    response_retry_delay(&response, attempt)
                } else {
                    transient_retry_delay(attempt)
                };
                tokio::time::sleep(delay).await;
                continue;
            }
            return Err(last_error);
        }
        if !status.is_success() {
            return Err(format!("{url} returned HTTP {status}"));
        }
        return response
            .json()
            .await
            .map_err(|error| format!("invalid JSON from {url}: {error}"));
    }
    Err(last_error)
}

async fn wait_for_request_slot() {
    let interval = msl_request_interval();
    let mut previous = request_gate().lock().await;
    let elapsed = previous.elapsed();
    if elapsed < interval {
        tokio::time::sleep(interval - elapsed).await;
    }
    *previous = Instant::now();
}

fn transient_retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(2_u64.saturating_pow(attempt + 1).min(30))
}

fn response_retry_delay(response: &Response, attempt: u32) -> Duration {
    if let Some(seconds) = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Duration::from_secs(seconds.clamp(1, 15));
    }
    if let Some(reset_at) = response
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if reset_at > now {
            return Duration::from_secs((reset_at - now + 1).clamp(1, 15));
        }
    }
    transient_retry_delay(attempt)
}

fn classified_core_entries(data: &Value) -> Result<Vec<(String, String)>, String> {
    let groups = data
        .as_object()
        .ok_or_else(|| "MSL classify response is not an object".to_string())?;
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for (category, values) in groups {
        let Some(values) = values.as_array() else {
            continue;
        };
        for value in values {
            let Some(slug) = value.as_str() else { continue };
            if seen.insert(slug.to_string()) {
                entries.push((slug.to_string(), category.to_string()));
            }
        }
    }
    if entries.is_empty() {
        return Err("MSL classify response contains no core types".into());
    }
    Ok(entries)
}

fn mirror_project(slug: &str, category: &str, description: &str) -> CatalogProject {
    let name = display_core_name(slug);
    let summary = if description.trim().is_empty() {
        format!("{name} 服务端核心的 MSL 加速镜像")
    } else {
        description.trim().to_string()
    };
    CatalogProject {
        slug: slug.into(),
        name: name.clone(),
        summary: summary.clone(),
        description: format!(
            "{summary}\n\n本项目由资源站自动同步。下载服务由 MSL 开服器提供：https://www.mslmc.cn/"
        ),
        author: "MSL 开服器".into(),
        homepage: "https://www.mslmc.cn/docs/msl/msl-mirrors/".into(),
        repository: String::new(),
        preview_url: String::new(),
        license: String::new(),
        plugin_category: String::new(),
        target_plugin: String::new(),
        tags: vec![MSL_PROJECT_TAG.into(), category.into(), "自动同步".into()],
        color: category_color(category).into(),
        featured: matches!(
            slug,
            "paper" | "purpur" | "fabric" | "forge" | "neoforge" | "vanilla" | "velocity"
        ),
    }
}

fn latest_build(builds: &[String]) -> Option<String> {
    builds
        .iter()
        .find(|build| !build.trim().is_empty())
        .cloned()
}

fn mirror_source_names() -> Vec<String> {
    ["MSL", "FastMirror", "Polars"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

impl MirrorSource {
    fn name(self) -> &'static str {
        match self {
            Self::Msl => "MSL",
            Self::FastMirror => "FastMirror",
            Self::Polars => "Polars",
        }
    }
}

fn mirror_attempt_order(slug: &str) -> Vec<MirrorSource> {
    let mut sources = vec![MirrorSource::Msl];
    if fastmirror_core(slug).is_some() {
        sources.push(MirrorSource::FastMirror);
    }
    if polars_core(slug).is_some() {
        sources.push(MirrorSource::Polars);
    }
    sources
}

fn fastmirror_core(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "paper" => "Paper",
        "purpur" => "Purpur",
        "folia" => "Folia",
        "forge" => "Forge",
        "fabric" => "Fabric",
        "vanilla" => "Vanilla",
        "velocity" => "Velocity",
        "bungeecord" => "BungeeCord",
        "spongeforge" => "SpongeForge",
        "spongevanilla" => "SpongeVanilla",
        "catserver" => "CatServer",
        "nukkitx" => "Nukkit",
        "arclight-forge" | "arclight-fabric" | "arclight-neoforge" => "Arclight",
        _ => return None,
    })
}

fn fastmirror_minecraft(slug: &str, minecraft: &str) -> String {
    match slug {
        "arclight-forge" => format!("{minecraft}-forge"),
        "arclight-fabric" => format!("{minecraft}-fabric"),
        "arclight-neoforge" => format!("{minecraft}-neoforge"),
        _ => minecraft.into(),
    }
}

fn polars_core(slug: &str) -> Option<(u8, &'static str)> {
    Some(match slug {
        "vanilla" => (1, "minecraft-server-"),
        "bukkit" => (2, "craftbukkit-"),
        "spigot" => (3, "spigot-"),
        "paper" => (4, "paper-"),
        "tuinity" => (5, "tuinity-"),
        "purpur" => (6, "purpur-"),
        "akarin" => (8, "akarin-"),
        "forge" => (11, "forge-"),
        "catserver" => (15, "catserver-"),
        "mohist" => (16, "mohist-"),
        "fabric" => (19, "fabric-"),
        "spongeforge" => (21, "spongeforge-"),
        _ => return None,
    })
}

fn filename_strictly_matches_minecraft(filename: &str, minecraft: &str) -> bool {
    filename.match_indices(minecraft).any(|(start, matched)| {
        let before = filename[..start].chars().next_back();
        let suffix = &filename[start + matched.len()..];
        let after = suffix.chars().next();
        before.is_none_or(|value| !value.is_ascii_digit() && value != '.')
            && (suffix.eq_ignore_ascii_case(".jar")
                || after.is_none_or(|value| !value.is_ascii_digit() && value != '.'))
    })
}

fn select_polars_entry<'a>(
    entries: &'a [PolarsCoreEntry],
    filename_prefix: &str,
    minecraft: &str,
) -> Option<&'a PolarsCoreEntry> {
    entries
        .iter()
        .filter(|entry| {
            entry.name.to_ascii_lowercase().starts_with(filename_prefix)
                && filename_strictly_matches_minecraft(&entry.name, minecraft)
        })
        .max_by_key(|entry| polars_sync_time_key(&entry.sync_time))
}

fn polars_sync_time_key(value: &Value) -> i128 {
    if let Some(value) = value.as_i64() {
        return i128::from(value);
    }
    if let Some(value) = value.as_u64() {
        return i128::from(value);
    }
    let Some(value) = value.as_str() else {
        return i128::MIN;
    };
    value
        .parse::<i128>()
        .ok()
        .or_else(|| {
            DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|timestamp| i128::from(timestamp.timestamp_millis()))
        })
        .unwrap_or(i128::MIN)
}

fn secure_download_url(value: &str, allow_polars_cdn_upgrade: bool) -> Result<Url, String> {
    let mut parsed = Url::parse(value)
        .map_err(|error| format!("mirror returned an invalid download URL: {error}"))?;
    if parsed.scheme() == "http"
        && allow_polars_cdn_upgrade
        && parsed
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("cdn.polars.cc"))
    {
        parsed
            .set_scheme("https")
            .map_err(|_| "could not upgrade the Polars CDN URL to HTTPS".to_string())?;
    }
    if parsed.scheme() != "https" {
        return Err("mirror download URL must use HTTPS".into());
    }
    Ok(parsed)
}

fn url_with_segments(base: &str, segments: &[&str]) -> Result<String, String> {
    let mut url = Url::parse(base).map_err(|error| format!("invalid mirror API base: {error}"))?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| "mirror API base cannot contain path segments".to_string())?;
        path.pop_if_empty();
        path.extend(segments.iter().copied());
    }
    Ok(url.to_string())
}

fn resolved_filename(
    header_filename: Option<String>,
    api_filename: Option<&str>,
    parsed_url: &Url,
    fallback: &str,
) -> String {
    let filename = header_filename
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            api_filename
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            parsed_url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .map(str::to_string)
        });
    safe_filename(filename.as_deref().unwrap_or(fallback), fallback)
}

fn safe_note_value(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| if character == ';' { ',' } else { character })
        .take(120)
        .collect()
}

fn json_scalar_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn content_disposition_filename(value: &str) -> Option<String> {
    value.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix("filename=")
            .map(|value| value.trim_matches(['"', '\'']).to_string())
            .filter(|value| !value.is_empty())
    })
}

fn safe_filename(value: &str, fallback: &str) -> String {
    let mut filename: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '+') {
                character
            } else {
                '_'
            }
        })
        .take(180)
        .collect();
    if filename.is_empty() || !filename.contains('.') {
        filename = fallback.into();
    }
    filename
}

fn is_msl_project(project: &CatalogProject) -> bool {
    project.tags.iter().any(|tag| tag == MSL_PROJECT_TAG)
}

fn is_msl_managed_version(version: &CatalogVersion) -> bool {
    version.release_notes.starts_with(MSL_RELEASE_PREFIX)
}

fn reusable_catalog_version<'a>(
    existing_versions: &'a HashMap<(String, String), CatalogVersion>,
    slug: &str,
    target: &str,
) -> Option<&'a CatalogVersion> {
    existing_versions.get(&(slug.to_string(), target.to_string()))
}

fn catalog_version_needs_resolution(existing: Option<&CatalogVersion>) -> bool {
    existing.is_none_or(is_msl_placeholder)
}

fn is_msl_placeholder(version: &CatalogVersion) -> bool {
    version.release_notes.starts_with(MSL_PLACEHOLDER_PREFIX)
}

fn automatic_version_needs_size_backfill(version: &CatalogVersion) -> bool {
    is_msl_managed_version(version)
        && !is_msl_placeholder(version)
        && version.status == "published"
        && version.size == 0
        && version.content.is_empty()
}

fn preserve_catalog_identity(current: &CatalogVersion, replacement: &mut CatalogVersion) {
    replacement.id = current.id.clone();
    replacement.downloads = current.downloads;
}

fn count_pending_versions(versions: &[CatalogVersion]) -> usize {
    versions
        .iter()
        .filter(|version| is_msl_placeholder(version))
        .count()
}

fn loaders_for(category: &str, slug: &str) -> Vec<String> {
    match category {
        "pluginsCore" => vec!["bukkit".into(), "spigot".into(), slug.into()],
        "pluginsAndModsCore_Forge" => vec!["forge".into(), "bukkit".into()],
        "pluginsAndModsCore_Fabric" => vec!["fabric".into(), "bukkit".into()],
        "modsCore_Forge" => vec![slug.into()],
        "modsCore_Fabric" => vec![slug.into()],
        "vanillaCore" => vec!["vanilla".into()],
        "bedrockCore" => vec!["bedrock".into()],
        "proxyCore" => vec!["proxy".into()],
        _ => vec![slug.into()],
    }
}

fn java_version_for(version: &str) -> Option<u8> {
    if version.starts_with("26.") || version.starts_with("1.21") {
        Some(21)
    } else if version.starts_with("1.18") || version.starts_with("1.20") {
        Some(17)
    } else {
        Some(8)
    }
}

fn display_core_name(slug: &str) -> String {
    let known: HashMap<&str, &str> = HashMap::from([
        ("paper", "Paper"),
        ("purpur", "Purpur"),
        ("folia", "Folia"),
        ("fabric", "Fabric"),
        ("forge", "Forge"),
        ("neoforge", "NeoForge"),
        ("quilt", "Quilt"),
        ("vanilla", "Vanilla"),
        ("velocity", "Velocity"),
        ("bungeecord", "BungeeCord"),
        ("spigot", "Spigot"),
        ("bukkit", "Bukkit"),
        ("spongeforge", "SpongeForge"),
        ("spongevanilla", "SpongeVanilla"),
        ("nukkitx", "NukkitX"),
    ]);
    known.get(slug).copied().unwrap_or(slug).to_string()
}

fn category_color(category: &str) -> &'static str {
    match category {
        "pluginsCore" => "#32d5b0",
        "pluginsAndModsCore_Forge" | "modsCore_Forge" => "#e39a55",
        "pluginsAndModsCore_Fabric" | "modsCore_Fabric" => "#8bc8e8",
        "vanillaCore" => "#79b66a",
        "bedrockCore" => "#6e9ee8",
        "proxyCore" => "#a78bfa",
        _ => "#32d5b0",
    }
}

fn sync_enabled() -> bool {
    std::env::var("SCULK_MSL_CORE_SYNC_ENABLED")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn api_base() -> String {
    std::env::var("SCULK_MSL_API_BASE")
        .unwrap_or_else(|_| DEFAULT_MSL_API_BASE.into())
        .trim()
        .trim_end_matches('/')
        .to_string()
}

fn fastmirror_base() -> String {
    mirror_api_base("SCULK_FASTMIRROR_API_BASE", DEFAULT_FASTMIRROR_API_BASE)
}

fn polars_base() -> String {
    mirror_api_base("SCULK_POLARS_API_BASE", DEFAULT_POLARS_API_BASE)
}

fn mirror_api_base(variable: &str, default: &str) -> String {
    std::env::var(variable)
        .unwrap_or_else(|_| default.into())
        .trim()
        .trim_end_matches('/')
        .to_string()
}

fn validate_https_api_base(name: &str, base: &str) -> Result<(), String> {
    let parsed = Url::parse(base).map_err(|error| format!("invalid {name} API base: {error}"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(format!("{name} API base must be an absolute HTTPS URL"));
    }
    Ok(())
}

fn target_versions() -> Vec<String> {
    let configured = std::env::var("SCULK_MSL_TARGET_VERSIONS").unwrap_or_default();
    let values: Vec<String> = if configured.trim().is_empty() {
        DEFAULT_MSL_TARGET_VERSIONS
            .iter()
            .map(ToString::to_string)
            .collect()
    } else {
        configured
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect()
    };
    let mut unique = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

fn sync_interval_seconds() -> u64 {
    std::env::var("SCULK_MSL_SYNC_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(7_200)
        .max(300)
}

fn msl_concurrency() -> usize {
    std::env::var("SCULK_MSL_SYNC_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(12)
        .clamp(1, 12)
}

fn msl_request_interval() -> Duration {
    let milliseconds = std::env::var("SCULK_MSL_REQUEST_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(750)
        .clamp(250, 5_000);
    Duration::from_millis(milliseconds)
}

fn msl_user_agent() -> String {
    std::env::var("SCULK_MSL_USER_AGENT")
        .unwrap_or_else(|_| "Sculk-Catalyst-Resource-Sync/1.0 (+https://res.mcmy.love)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_version(project: &str, version: &str) -> CatalogVersion {
        CatalogVersion {
            id: "managed-version".into(),
            project: project.into(),
            version: version.into(),
            channel: "stable".into(),
            minecraft_versions: vec![version.into()],
            loaders: vec![project.into()],
            formats: Vec::new(),
            java_version: None,
            filename: "server.jar".into(),
            size: 1,
            sha256: "a".repeat(64),
            download_url: "https://example.com/server.jar".into(),
            content: String::new(),
            release_notes: "手工上传版本".into(),
            released_at: "2026-07-29T00:00:00+08:00".into(),
            status: "published".into(),
            downloads: 0,
        }
    }

    fn polars_entry(name: &str, sync_time: i64) -> PolarsCoreEntry {
        PolarsCoreEntry {
            _id: Value::Null,
            name: name.into(),
            download_url: format!("http://cdn.polars.cc/{name}"),
            sync_time: Value::from(sync_time),
        }
    }

    #[test]
    fn latest_build_uses_the_first_non_empty_mirror_build() {
        assert_eq!(
            latest_build(&["14.23.5.2864".into(), "14.23.5.2863".into()]),
            Some("14.23.5.2864".into())
        );
        assert_eq!(latest_build(&["latest".into()]), Some("latest".into()));
    }

    #[test]
    fn filenames_and_java_versions_are_normalized() {
        assert_eq!(
            content_disposition_filename("attachment; filename=\"paper.jar\""),
            Some("paper.jar".into())
        );
        assert_eq!(
            safe_filename("jar", "fabric-1.16-latest.jar"),
            "fabric-1.16-latest.jar"
        );
        assert_eq!(java_version_for("1.12.2"), Some(8));
        assert_eq!(java_version_for("1.20.4"), Some(17));
        assert_eq!(java_version_for("26.2"), Some(21));
        assert_eq!(content_range_total("bytes 0-0/83876241"), Some(83_876_241));
        assert_eq!(content_range_total("bytes */83876241"), Some(83_876_241));
        assert_eq!(content_range_total("invalid"), None);
    }

    #[test]
    fn existing_catalog_version_is_reused_only_for_the_exact_project_and_target() {
        let existing = catalog_version("paper", "1.20.4");
        let versions = HashMap::from([(
            (existing.project.clone(), existing.version.clone()),
            existing.clone(),
        )]);

        assert_eq!(
            reusable_catalog_version(&versions, "paper", "1.20.4").map(|entry| &entry.id),
            Some(&existing.id)
        );
        assert!(reusable_catalog_version(&versions, "paper", "1.21.1").is_none());
        assert!(reusable_catalog_version(&versions, "purpur", "1.20.4").is_none());
    }

    #[test]
    fn only_missing_versions_and_automatic_placeholders_need_resolution() {
        let manual = catalog_version("paper", "1.20.4");
        let mut complete_msl = manual.clone();
        complete_msl.release_notes = format!("{MSL_RELEASE_PREFIX}build=123");

        assert!(catalog_version_needs_resolution(None));
        assert!(!catalog_version_needs_resolution(Some(&manual)));
        assert!(!catalog_version_needs_resolution(Some(&complete_msl)));

        let placeholder =
            mirror_version_placeholder(DEFAULT_MSL_API_BASE, "paper", "pluginsCore", "1.20.4");
        assert!(catalog_version_needs_resolution(Some(&placeholder)));
    }

    #[test]
    fn placeholder_is_persistable_but_not_downloadable() {
        let placeholder =
            mirror_version_placeholder(DEFAULT_MSL_API_BASE, "paper", "pluginsCore", "1.20.4");

        assert_eq!(placeholder.status, "draft");
        assert_eq!(placeholder.size, 0);
        assert!(placeholder.sha256.is_empty());
        assert!(placeholder.content.is_empty());
        assert!(placeholder.filename.ends_with("-pending.jar"));
        assert!(is_msl_managed_version(&placeholder));
        assert!(is_msl_placeholder(&placeholder));
        assert_eq!(
            Url::parse(&placeholder.download_url)
                .expect("placeholder URL should be valid")
                .scheme(),
            "https"
        );
    }

    #[test]
    fn completed_resolution_keeps_the_placeholder_identity() {
        let mut placeholder =
            mirror_version_placeholder(DEFAULT_MSL_API_BASE, "paper", "pluginsCore", "1.20.4");
        placeholder.downloads = 7;
        let mut completed = catalog_version("paper", "1.20.4");
        completed.release_notes = format!("{MSL_RELEASE_PREFIX}build=123");

        preserve_catalog_identity(&placeholder, &mut completed);

        assert_eq!(completed.id, placeholder.id);
        assert_eq!(completed.downloads, 7);
        assert_eq!(completed.status, "published");
    }

    #[test]
    fn mirror_order_and_core_mappings_are_explicit() {
        assert_eq!(
            mirror_attempt_order("paper"),
            vec![
                MirrorSource::Msl,
                MirrorSource::FastMirror,
                MirrorSource::Polars
            ]
        );
        assert_eq!(
            mirror_attempt_order("folia"),
            vec![MirrorSource::Msl, MirrorSource::FastMirror]
        );
        assert_eq!(mirror_attempt_order("unknown"), vec![MirrorSource::Msl]);
        assert_eq!(fastmirror_core("arclight-forge"), Some("Arclight"));
        assert_eq!(
            fastmirror_minecraft("arclight-forge", "1.20.4"),
            "1.20.4-forge"
        );
        assert_eq!(
            fastmirror_minecraft("arclight-fabric", "1.20.4"),
            "1.20.4-fabric"
        );
        assert_eq!(
            fastmirror_minecraft("arclight-neoforge", "1.20.4"),
            "1.20.4-neoforge"
        );
        assert_eq!(polars_core("spongeforge"), Some((21, "spongeforge-")));
        assert_eq!(polars_core("spongevanilla"), None);
    }

    #[test]
    fn polars_selection_requires_core_prefix_and_exact_version_boundary() {
        let entries = vec![
            polars_entry("paper-1.21.11-10.jar", 300),
            polars_entry("purpur-1.21.1-99.jar", 400),
            polars_entry("paper-1.21.1-10.jar", 100),
            polars_entry("paper-1.21.1-11.jar", 200),
        ];

        assert!(!filename_strictly_matches_minecraft(
            "paper-1.21.11.jar",
            "1.21.1"
        ));
        assert!(filename_strictly_matches_minecraft(
            "paper-1.16.jar",
            "1.16"
        ));
        assert!(!filename_strictly_matches_minecraft(
            "paper-1.16.5.jar",
            "1.16"
        ));
        assert_eq!(
            select_polars_entry(&entries, "paper-", "1.21.1").map(|entry| entry.name.as_str()),
            Some("paper-1.21.1-11.jar")
        );
        assert!(select_polars_entry(&entries, "fabric-", "1.21.1").is_none());
    }

    #[test]
    fn only_the_polars_cdn_can_be_upgraded_from_http() {
        assert_eq!(
            secure_download_url("http://cdn.polars.cc/paper.jar", true)
                .expect("Polars CDN should be upgraded")
                .as_str(),
            "https://cdn.polars.cc/paper.jar"
        );
        assert!(secure_download_url("http://cdn.polars.cc/paper.jar", false).is_err());
        assert!(secure_download_url("http://example.com/paper.jar", true).is_err());
        assert!(secure_download_url("https://example.com/paper.jar", false).is_ok());
    }

    #[test]
    fn pending_and_size_backfill_state_are_counted_without_treating_manual_versions_as_automatic() {
        let placeholder =
            mirror_version_placeholder(DEFAULT_MSL_API_BASE, "paper", "pluginsCore", "1.20.4");
        let manual = catalog_version("paper", "1.21.1");
        let mut automatic = catalog_version("paper", "1.18");
        automatic.release_notes = format!("{MSL_RELEASE_PREFIX}source=FastMirror; build=1");
        automatic.size = 0;

        assert_eq!(
            count_pending_versions(&[placeholder.clone(), manual.clone(), automatic.clone()]),
            1
        );
        assert!(automatic_version_needs_size_backfill(&automatic));
        assert!(!automatic_version_needs_size_backfill(&placeholder));
        assert!(!automatic_version_needs_size_backfill(&manual));
    }
}
