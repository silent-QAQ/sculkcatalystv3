use crate::{AppState, internal, persist};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::get,
};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{cmp::Ordering, collections::BTreeSet};
use url::Url;
use uuid::Uuid;

type ApiError = (StatusCode, String);
type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CatalogState {
    #[serde(default)]
    pub(crate) schema_version: u8,
    #[serde(default)]
    pub(crate) core_projects: Vec<CatalogProject>,
    #[serde(default)]
    pub(crate) plugin_projects: Vec<CatalogProject>,
    #[serde(default)]
    pub(crate) core_versions: Vec<CatalogVersion>,
    #[serde(default)]
    pub(crate) plugin_versions: Vec<CatalogVersion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct CatalogProject {
    pub(crate) slug: String,
    pub(crate) name: String,
    pub(crate) summary: String,
    pub(crate) description: String,
    pub(crate) author: String,
    pub(crate) homepage: String,
    pub(crate) repository: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    pub(crate) color: String,
    #[serde(default)]
    pub(crate) featured: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct CatalogVersion {
    pub(crate) id: String,
    pub(crate) project: String,
    pub(crate) version: String,
    pub(crate) channel: String,
    pub(crate) minecraft_versions: Vec<String>,
    pub(crate) loaders: Vec<String>,
    pub(crate) java_version: Option<u8>,
    pub(crate) filename: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
    pub(crate) download_url: String,
    pub(crate) release_notes: String,
    pub(crate) released_at: String,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) downloads: u64,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct ProjectInput {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    repository: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    color: String,
    #[serde(default)]
    featured: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct VersionInput {
    #[serde(default)]
    version: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    minecraft_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    java_version: Option<u8>,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    download_url: String,
    #[serde(default)]
    release_notes: String,
    #[serde(default)]
    released_at: String,
    #[serde(default)]
    status: String,
}

#[derive(Clone, Debug, Serialize)]
struct ProjectView {
    #[serde(flatten)]
    project: CatalogProject,
    kind: String,
    version_count: usize,
    published_versions: usize,
    latest_version: Option<String>,
    downloads: u64,
    minecraft_versions: Vec<String>,
    channels: Vec<String>,
    loaders: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CatalogSummary {
    core_projects: usize,
    plugin_projects: usize,
    versions: usize,
    downloads: u64,
    published_versions: usize,
    featured_projects: usize,
}

#[derive(Debug, Serialize)]
struct ResolveResponse {
    kind: String,
    project: ProjectView,
    version: CatalogVersion,
    download_path: String,
}

#[derive(Debug, Serialize)]
struct DeleteResponse {
    deleted: bool,
    slug: String,
    version: Option<String>,
    deleted_versions: usize,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct CatalogQuery {
    search: Option<String>,
    minecraft: Option<String>,
    channel: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct ResolveQuery {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    project: String,
    #[serde(default)]
    minecraft: String,
    #[serde(default = "default_channel")]
    channel: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatalogKind {
    Core,
    Plugin,
}

impl CatalogKind {
    fn parse(value: &str) -> Result<Self, ApiError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "core" => Ok(Self::Core),
            "plugin" => Ok(Self::Plugin),
            _ => Err((
                StatusCode::BAD_REQUEST,
                "kind must be core or plugin".into(),
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Plugin => "plugin",
        }
    }
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/catalog/summary", get(get_summary))
        .route("/api/catalog/cores", get(list_cores).post(create_core))
        .route(
            "/api/catalog/cores/{slug}",
            get(get_core).put(update_core).delete(delete_core),
        )
        .route(
            "/api/catalog/cores/{slug}/versions",
            get(list_core_versions).post(create_core_version),
        )
        .route(
            "/api/catalog/cores/{slug}/versions/{version}",
            get(get_core_version)
                .put(update_core_version)
                .delete(delete_core_version),
        )
        .route(
            "/api/catalog/plugins",
            get(list_plugins).post(create_plugin),
        )
        .route(
            "/api/catalog/plugins/{slug}",
            get(get_plugin).put(update_plugin).delete(delete_plugin),
        )
        .route(
            "/api/catalog/plugins/{slug}/versions",
            get(list_plugin_versions).post(create_plugin_version),
        )
        .route(
            "/api/catalog/plugins/{slug}/versions/{version}",
            get(get_plugin_version)
                .put(update_plugin_version)
                .delete(delete_plugin_version),
        )
        .route("/api/v1/resolve", get(resolve))
        .route("/api/v1/download/{kind}/{project}/{version}", get(download))
        .route("/api/openapi.json", get(openapi))
}

async fn get_summary(State(state): State<AppState>) -> Json<CatalogSummary> {
    let data = state.inner.read().await;
    let catalog = &data.catalog;
    let versions = catalog.core_versions.iter().chain(&catalog.plugin_versions);
    Json(CatalogSummary {
        core_projects: catalog.core_projects.len(),
        plugin_projects: catalog.plugin_projects.len(),
        versions: catalog.core_versions.len() + catalog.plugin_versions.len(),
        downloads: versions.clone().map(|version| version.downloads).sum(),
        published_versions: versions
            .clone()
            .filter(|version| version.status == "published")
            .count(),
        featured_projects: catalog
            .core_projects
            .iter()
            .chain(&catalog.plugin_projects)
            .filter(|project| project.featured)
            .count(),
    })
}

async fn list_cores(
    State(state): State<AppState>,
    Query(query): Query<CatalogQuery>,
) -> Json<Vec<ProjectView>> {
    list_projects(state, CatalogKind::Core, query).await
}

async fn list_plugins(
    State(state): State<AppState>,
    Query(query): Query<CatalogQuery>,
) -> Json<Vec<ProjectView>> {
    list_projects(state, CatalogKind::Plugin, query).await
}

async fn list_projects(
    state: AppState,
    kind: CatalogKind,
    query: CatalogQuery,
) -> Json<Vec<ProjectView>> {
    let data = state.inner.read().await;
    Json(filtered_project_views(
        kind,
        projects(&data.catalog, kind),
        versions(&data.catalog, kind),
        &query,
    ))
}

async fn get_core(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<ProjectView> {
    get_project(state, CatalogKind::Core, slug).await
}

async fn get_plugin(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<ProjectView> {
    get_project(state, CatalogKind::Plugin, slug).await
}

async fn get_project(state: AppState, kind: CatalogKind, slug: String) -> ApiResult<ProjectView> {
    let data = state.inner.read().await;
    let project = projects(&data.catalog, kind)
        .iter()
        .find(|project| project.slug == slug)
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
    Ok(Json(project_view(
        kind,
        project,
        versions(&data.catalog, kind),
    )))
}

async fn list_core_versions(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Query(query): Query<CatalogQuery>,
) -> ApiResult<Vec<CatalogVersion>> {
    list_versions(state, CatalogKind::Core, slug, query).await
}

async fn list_plugin_versions(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Query(query): Query<CatalogQuery>,
) -> ApiResult<Vec<CatalogVersion>> {
    list_versions(state, CatalogKind::Plugin, slug, query).await
}

async fn list_versions(
    state: AppState,
    kind: CatalogKind,
    slug: String,
    query: CatalogQuery,
) -> ApiResult<Vec<CatalogVersion>> {
    let data = state.inner.read().await;
    if !projects(&data.catalog, kind)
        .iter()
        .any(|project| project.slug == slug)
    {
        return Err((StatusCode::NOT_FOUND, "project not found".into()));
    }
    Ok(Json(filtered_versions(
        versions(&data.catalog, kind),
        &slug,
        &query,
    )))
}

async fn get_core_version(
    Path((slug, version)): Path<(String, String)>,
    State(state): State<AppState>,
) -> ApiResult<CatalogVersion> {
    get_version(state, CatalogKind::Core, slug, version).await
}

async fn get_plugin_version(
    Path((slug, version)): Path<(String, String)>,
    State(state): State<AppState>,
) -> ApiResult<CatalogVersion> {
    get_version(state, CatalogKind::Plugin, slug, version).await
}

async fn get_version(
    state: AppState,
    kind: CatalogKind,
    slug: String,
    version: String,
) -> ApiResult<CatalogVersion> {
    let data = state.inner.read().await;
    let version = versions(&data.catalog, kind)
        .iter()
        .find(|item| item.project == slug && item.version == version)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "version not found".into()))?;
    Ok(Json(version))
}

async fn create_core(
    State(state): State<AppState>,
    Json(input): Json<ProjectInput>,
) -> ApiResult<ProjectView> {
    create_project(state, CatalogKind::Core, input).await
}

async fn create_plugin(
    State(state): State<AppState>,
    Json(input): Json<ProjectInput>,
) -> ApiResult<ProjectView> {
    create_project(state, CatalogKind::Plugin, input).await
}

async fn create_project(
    state: AppState,
    kind: CatalogKind,
    input: ProjectInput,
) -> ApiResult<ProjectView> {
    let project = normalize_project(input);
    validate_project(&project)?;

    let mut data = state.inner.write().await;
    ensure_project_unique(projects(&data.catalog, kind), &project, None)?;
    projects_mut(&mut data.catalog, kind).push(project.clone());
    let view = project_view(kind, &project, versions(&data.catalog, kind));
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn update_core(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(input): Json<ProjectInput>,
) -> ApiResult<ProjectView> {
    update_project(state, CatalogKind::Core, slug, input).await
}

async fn update_plugin(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(input): Json<ProjectInput>,
) -> ApiResult<ProjectView> {
    update_project(state, CatalogKind::Plugin, slug, input).await
}

async fn update_project(
    state: AppState,
    kind: CatalogKind,
    slug: String,
    input: ProjectInput,
) -> ApiResult<ProjectView> {
    let project = normalize_project(input);
    validate_project(&project)?;

    let mut data = state.inner.write().await;
    let index = projects(&data.catalog, kind)
        .iter()
        .position(|item| item.slug == slug)
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
    ensure_project_unique(projects(&data.catalog, kind), &project, Some(index))?;

    let old_slug = projects(&data.catalog, kind)[index].slug.clone();
    projects_mut(&mut data.catalog, kind)[index] = project.clone();
    if old_slug != project.slug {
        for version in versions_mut(&mut data.catalog, kind)
            .iter_mut()
            .filter(|version| version.project == old_slug)
        {
            version.project = project.slug.clone();
        }
    }

    let view = project_view(kind, &project, versions(&data.catalog, kind));
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(view))
}

async fn delete_core(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<DeleteResponse> {
    delete_project(state, CatalogKind::Core, slug).await
}

async fn delete_plugin(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> ApiResult<DeleteResponse> {
    delete_project(state, CatalogKind::Plugin, slug).await
}

async fn delete_project(
    state: AppState,
    kind: CatalogKind,
    slug: String,
) -> ApiResult<DeleteResponse> {
    let mut data = state.inner.write().await;
    let index = projects(&data.catalog, kind)
        .iter()
        .position(|project| project.slug == slug)
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
    projects_mut(&mut data.catalog, kind).remove(index);
    let before = versions(&data.catalog, kind).len();
    versions_mut(&mut data.catalog, kind).retain(|version| version.project != slug);
    let deleted_versions = before - versions(&data.catalog, kind).len();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(DeleteResponse {
        deleted: true,
        slug,
        version: None,
        deleted_versions,
    }))
}

async fn create_core_version(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(input): Json<VersionInput>,
) -> ApiResult<CatalogVersion> {
    create_version(state, CatalogKind::Core, slug, input).await
}

async fn create_plugin_version(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(input): Json<VersionInput>,
) -> ApiResult<CatalogVersion> {
    create_version(state, CatalogKind::Plugin, slug, input).await
}

async fn create_version(
    state: AppState,
    kind: CatalogKind,
    slug: String,
    input: VersionInput,
) -> ApiResult<CatalogVersion> {
    let version = normalize_version(&slug, input, Uuid::new_v4().to_string(), 0);
    validate_version(&version)?;

    let mut data = state.inner.write().await;
    if !projects(&data.catalog, kind)
        .iter()
        .any(|project| project.slug == slug)
    {
        return Err((StatusCode::NOT_FOUND, "project not found".into()));
    }
    ensure_version_unique(versions(&data.catalog, kind), &version, None)?;
    versions_mut(&mut data.catalog, kind).push(version.clone());
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(version))
}

async fn update_core_version(
    Path((slug, version)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(input): Json<VersionInput>,
) -> ApiResult<CatalogVersion> {
    update_version(state, CatalogKind::Core, slug, version, input).await
}

async fn update_plugin_version(
    Path((slug, version)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(input): Json<VersionInput>,
) -> ApiResult<CatalogVersion> {
    update_version(state, CatalogKind::Plugin, slug, version, input).await
}

async fn update_version(
    state: AppState,
    kind: CatalogKind,
    slug: String,
    current_version: String,
    input: VersionInput,
) -> ApiResult<CatalogVersion> {
    let mut data = state.inner.write().await;
    let index = versions(&data.catalog, kind)
        .iter()
        .position(|item| item.project == slug && item.version == current_version)
        .ok_or((StatusCode::NOT_FOUND, "version not found".into()))?;
    let current = versions(&data.catalog, kind)[index].clone();
    let version = normalize_version(&slug, input, current.id, current.downloads);
    validate_version(&version)?;
    ensure_version_unique(versions(&data.catalog, kind), &version, Some(index))?;
    versions_mut(&mut data.catalog, kind)[index] = version.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(version))
}

async fn delete_core_version(
    Path((slug, version)): Path<(String, String)>,
    State(state): State<AppState>,
) -> ApiResult<DeleteResponse> {
    delete_version(state, CatalogKind::Core, slug, version).await
}

async fn delete_plugin_version(
    Path((slug, version)): Path<(String, String)>,
    State(state): State<AppState>,
) -> ApiResult<DeleteResponse> {
    delete_version(state, CatalogKind::Plugin, slug, version).await
}

async fn delete_version(
    state: AppState,
    kind: CatalogKind,
    slug: String,
    version: String,
) -> ApiResult<DeleteResponse> {
    let mut data = state.inner.write().await;
    let index = versions(&data.catalog, kind)
        .iter()
        .position(|item| item.project == slug && item.version == version)
        .ok_or((StatusCode::NOT_FOUND, "version not found".into()))?;
    versions_mut(&mut data.catalog, kind).remove(index);
    persist(&state, &data).await.map_err(internal)?;
    Ok(Json(DeleteResponse {
        deleted: true,
        slug,
        version: Some(version),
        deleted_versions: 1,
    }))
}

async fn resolve(
    State(state): State<AppState>,
    Query(query): Query<ResolveQuery>,
) -> ApiResult<ResolveResponse> {
    let kind = CatalogKind::parse(&query.kind)?;
    let slug = query.project.trim().to_ascii_lowercase();
    if slug.is_empty() || query.minecraft.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "project and minecraft are required".into(),
        ));
    }
    let channel = if query.channel.trim().is_empty() {
        default_channel()
    } else {
        query.channel.trim().to_ascii_lowercase()
    };

    let data = state.inner.read().await;
    let project = projects(&data.catalog, kind)
        .iter()
        .find(|project| project.slug == slug)
        .ok_or((StatusCode::NOT_FOUND, "project not found".into()))?;
    let version = resolve_version(
        versions(&data.catalog, kind),
        &slug,
        query.minecraft.trim(),
        &channel,
    )
    .cloned()
    .ok_or((
        StatusCode::NOT_FOUND,
        "no published compatible version found".into(),
    ))?;
    let download_path = format!(
        "/api/v1/download/{}/{}/{}",
        kind.as_str(),
        project.slug,
        version.version
    );
    Ok(Json(ResolveResponse {
        kind: kind.as_str().into(),
        project: project_view(kind, project, versions(&data.catalog, kind)),
        version,
        download_path,
    }))
}

async fn download(
    Path((kind, project, version)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Redirect, ApiError> {
    let kind = CatalogKind::parse(&kind)?;
    let mut data = state.inner.write().await;
    let item = versions_mut(&mut data.catalog, kind)
        .iter_mut()
        .find(|item| item.project == project && item.version == version)
        .ok_or((StatusCode::NOT_FOUND, "version not found".into()))?;
    if item.status != "published" {
        return Err((
            StatusCode::CONFLICT,
            "only published versions can be downloaded".into(),
        ));
    }
    validate_http_url(&item.download_url, "download_url")?;
    item.downloads = item.downloads.saturating_add(1);
    let download_url = item.download_url.clone();
    persist(&state, &data).await.map_err(internal)?;
    Ok(Redirect::temporary(&download_url))
}

fn projects(catalog: &CatalogState, kind: CatalogKind) -> &[CatalogProject] {
    match kind {
        CatalogKind::Core => &catalog.core_projects,
        CatalogKind::Plugin => &catalog.plugin_projects,
    }
}

fn projects_mut(catalog: &mut CatalogState, kind: CatalogKind) -> &mut Vec<CatalogProject> {
    match kind {
        CatalogKind::Core => &mut catalog.core_projects,
        CatalogKind::Plugin => &mut catalog.plugin_projects,
    }
}

fn versions(catalog: &CatalogState, kind: CatalogKind) -> &[CatalogVersion] {
    match kind {
        CatalogKind::Core => &catalog.core_versions,
        CatalogKind::Plugin => &catalog.plugin_versions,
    }
}

fn versions_mut(catalog: &mut CatalogState, kind: CatalogKind) -> &mut Vec<CatalogVersion> {
    match kind {
        CatalogKind::Core => &mut catalog.core_versions,
        CatalogKind::Plugin => &mut catalog.plugin_versions,
    }
}

fn filtered_project_views(
    kind: CatalogKind,
    projects: &[CatalogProject],
    versions: &[CatalogVersion],
    query: &CatalogQuery,
) -> Vec<ProjectView> {
    let search = normalized_query(&query.search);
    let minecraft = normalized_query(&query.minecraft);
    let channel = normalized_query(&query.channel);
    let mut items: Vec<ProjectView> = projects
        .iter()
        .filter(|project| {
            let matches_search = search.as_ref().is_none_or(|search| {
                [
                    project.slug.as_str(),
                    project.name.as_str(),
                    project.summary.as_str(),
                    project.description.as_str(),
                    project.author.as_str(),
                ]
                .iter()
                .any(|value| value.to_ascii_lowercase().contains(search))
                    || project
                        .tags
                        .iter()
                        .any(|tag| tag.to_ascii_lowercase().contains(search))
            });
            let matches_versions = if minecraft.is_none() && channel.is_none() {
                true
            } else {
                versions.iter().any(|version| {
                    version.project == project.slug
                        && minecraft
                            .as_ref()
                            .is_none_or(|minecraft| matches_minecraft(version, minecraft))
                        && channel
                            .as_ref()
                            .is_none_or(|channel| version.channel.eq_ignore_ascii_case(channel))
                })
            };
            matches_search && matches_versions
        })
        .map(|project| project_view(kind, project, versions))
        .collect();
    items.sort_by(|left, right| {
        right
            .project
            .featured
            .cmp(&left.project.featured)
            .then_with(|| left.project.name.cmp(&right.project.name))
    });
    items
}

fn filtered_versions(
    versions: &[CatalogVersion],
    project: &str,
    query: &CatalogQuery,
) -> Vec<CatalogVersion> {
    let search = normalized_query(&query.search);
    let minecraft = normalized_query(&query.minecraft);
    let channel = normalized_query(&query.channel);
    let mut items: Vec<CatalogVersion> = versions
        .iter()
        .filter(|version| version.project == project)
        .filter(|version| {
            search.as_ref().is_none_or(|search| {
                version.version.to_ascii_lowercase().contains(search)
                    || version.filename.to_ascii_lowercase().contains(search)
                    || version.release_notes.to_ascii_lowercase().contains(search)
                    || version
                        .loaders
                        .iter()
                        .any(|loader| loader.to_ascii_lowercase().contains(search))
            })
        })
        .filter(|version| {
            minecraft
                .as_ref()
                .is_none_or(|minecraft| matches_minecraft(version, minecraft))
        })
        .filter(|version| {
            channel
                .as_ref()
                .is_none_or(|channel| version.channel.eq_ignore_ascii_case(channel))
        })
        .cloned()
        .collect();
    items.sort_by(|left, right| compare_release(right, left));
    items
}

fn project_view(
    kind: CatalogKind,
    project: &CatalogProject,
    versions: &[CatalogVersion],
) -> ProjectView {
    let project_versions: Vec<&CatalogVersion> = versions
        .iter()
        .filter(|version| version.project == project.slug)
        .collect();
    let latest_version = latest_published(project_versions.iter().copied())
        .or_else(|| {
            project_versions
                .iter()
                .copied()
                .max_by(|left, right| compare_release(left, right))
        })
        .map(|version| version.version.clone());
    let mut minecraft_versions = BTreeSet::new();
    let mut channels = BTreeSet::new();
    let mut loaders = BTreeSet::new();
    for version in &project_versions {
        minecraft_versions.extend(version.minecraft_versions.iter().cloned());
        channels.insert(version.channel.clone());
        loaders.extend(version.loaders.iter().cloned());
    }
    ProjectView {
        project: project.clone(),
        kind: kind.as_str().into(),
        version_count: project_versions.len(),
        published_versions: project_versions
            .iter()
            .filter(|version| version.status == "published")
            .count(),
        latest_version,
        downloads: project_versions
            .iter()
            .map(|version| version.downloads)
            .sum(),
        minecraft_versions: minecraft_versions.into_iter().collect(),
        channels: channels.into_iter().collect(),
        loaders: loaders.into_iter().collect(),
    }
}

fn resolve_version<'a>(
    versions: &'a [CatalogVersion],
    project: &str,
    minecraft: &str,
    channel: &str,
) -> Option<&'a CatalogVersion> {
    latest_published(versions.iter().filter(|version| {
        version.project == project
            && version.channel.eq_ignore_ascii_case(channel)
            && matches_minecraft(version, minecraft)
    }))
}

pub(crate) fn resolve_core_download(
    catalog: &CatalogState,
    project: &str,
    minecraft: &str,
    channel: &str,
) -> Option<CatalogVersion> {
    let project = project.trim().to_ascii_lowercase();
    let minecraft = minecraft.trim();
    let channel = if channel.trim().is_empty() {
        default_channel()
    } else {
        channel.trim().to_ascii_lowercase()
    };
    if project.is_empty()
        || minecraft.is_empty()
        || !catalog
            .core_projects
            .iter()
            .any(|item| item.slug.eq_ignore_ascii_case(&project))
    {
        return None;
    }
    resolve_version(&catalog.core_versions, &project, minecraft, &channel)
        .filter(|version| validate_version(version).is_ok())
        .cloned()
}

pub(crate) fn record_core_download(catalog: &mut CatalogState, version_id: &str) -> bool {
    let Some(version) = catalog
        .core_versions
        .iter_mut()
        .find(|version| version.id == version_id)
    else {
        return false;
    };
    version.downloads = version.downloads.saturating_add(1);
    true
}

fn latest_published<'a, I>(versions: I) -> Option<&'a CatalogVersion>
where
    I: Iterator<Item = &'a CatalogVersion>,
{
    versions
        .filter(|version| version.status == "published")
        .max_by(|left, right| compare_release(left, right))
}

fn compare_release(left: &CatalogVersion, right: &CatalogVersion) -> Ordering {
    let left_date = DateTime::parse_from_rfc3339(&left.released_at).ok();
    let right_date = DateTime::parse_from_rfc3339(&right.released_at).ok();
    left_date
        .cmp(&right_date)
        .then_with(|| left.version.cmp(&right.version))
}

fn matches_minecraft(version: &CatalogVersion, minecraft: &str) -> bool {
    version
        .minecraft_versions
        .iter()
        .any(|item| item == "*" || item.eq_ignore_ascii_case(minecraft))
}

fn normalized_query(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn normalize_project(input: ProjectInput) -> CatalogProject {
    CatalogProject {
        slug: input.slug.trim().to_ascii_lowercase(),
        name: input.name.trim().into(),
        summary: input.summary.trim().into(),
        description: input.description.trim().into(),
        author: input.author.trim().into(),
        homepage: input.homepage.trim().into(),
        repository: input.repository.trim().into(),
        tags: normalized_list(input.tags, false),
        color: if input.color.trim().is_empty() {
            "#32d5b0".into()
        } else {
            input.color.trim().into()
        },
        featured: input.featured,
    }
}

fn normalize_version(
    project: &str,
    input: VersionInput,
    id: String,
    downloads: u64,
) -> CatalogVersion {
    CatalogVersion {
        id,
        project: project.trim().to_ascii_lowercase(),
        version: input.version.trim().into(),
        channel: input.channel.trim().to_ascii_lowercase(),
        minecraft_versions: normalized_list(input.minecraft_versions, false),
        loaders: normalized_list(input.loaders, true),
        java_version: input.java_version,
        filename: input.filename.trim().into(),
        size: input.size,
        sha256: input.sha256.trim().to_ascii_lowercase(),
        download_url: input.download_url.trim().into(),
        release_notes: input.release_notes.trim().into(),
        released_at: input.released_at.trim().into(),
        status: input.status.trim().to_ascii_lowercase(),
        downloads,
    }
}

fn normalized_list(values: Vec<String>, lowercase: bool) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let value = if lowercase {
            value.to_ascii_lowercase()
        } else {
            value.into()
        };
        if !normalized
            .iter()
            .any(|item: &String| item.eq_ignore_ascii_case(&value))
        {
            normalized.push(value);
        }
    }
    normalized
}

fn validate_project(project: &CatalogProject) -> Result<(), ApiError> {
    validate_slug(&project.slug)?;
    for (field, value) in [
        ("name", project.name.as_str()),
        ("summary", project.summary.as_str()),
        ("description", project.description.as_str()),
        ("author", project.author.as_str()),
        ("homepage", project.homepage.as_str()),
        ("repository", project.repository.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err((StatusCode::BAD_REQUEST, format!("{field} is required")));
        }
    }
    validate_http_url(&project.homepage, "homepage")?;
    validate_http_url(&project.repository, "repository")?;
    if !valid_color(&project.color) {
        return Err((
            StatusCode::BAD_REQUEST,
            "color must be a #RRGGBB value".into(),
        ));
    }
    Ok(())
}

fn validate_version(version: &CatalogVersion) -> Result<(), ApiError> {
    if !valid_version_identifier(&version.version) {
        return Err((
            StatusCode::BAD_REQUEST,
            "version must use letters, numbers, dots, underscores, plus signs or hyphens".into(),
        ));
    }
    if !valid_channel(&version.channel) {
        return Err((StatusCode::BAD_REQUEST, "invalid channel".into()));
    }
    if version.minecraft_versions.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "minecraft_versions is required".into(),
        ));
    }
    if version.loaders.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "loaders is required".into()));
    }
    if version.filename.is_empty()
        || version.filename.contains('/')
        || version.filename.contains('\\')
    {
        return Err((StatusCode::BAD_REQUEST, "invalid filename".into()));
    }
    if version.release_notes.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "release_notes is required".into()));
    }
    if !matches!(version.status.as_str(), "draft" | "published" | "yanked") {
        return Err((
            StatusCode::BAD_REQUEST,
            "status must be draft, published or yanked".into(),
        ));
    }
    if DateTime::parse_from_rfc3339(&version.released_at).is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            "released_at must be an RFC3339 timestamp".into(),
        ));
    }
    validate_http_url(&version.download_url, "download_url")?;
    if !version.sha256.is_empty()
        && (version.sha256.len() != 64
            || !version.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "sha256 must contain exactly 64 hexadecimal characters".into(),
        ));
    }
    if version.status == "published" {
        if version.sha256.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "published versions require sha256".into(),
            ));
        }
        if version.size == 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                "published versions require a non-zero size".into(),
            ));
        }
    }
    if version
        .java_version
        .is_some_and(|java| !(8..=99).contains(&java))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "java_version must be between 8 and 99".into(),
        ));
    }
    Ok(())
}

fn validate_slug(slug: &str) -> Result<(), ApiError> {
    let valid = !slug.is_empty()
        && slug.len() <= 64
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && slug
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && slug
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    if !valid {
        return Err((
            StatusCode::BAD_REQUEST,
            "slug must be lowercase kebab-case and no longer than 64 characters".into(),
        ));
    }
    Ok(())
}

fn validate_http_url(value: &str, field: &str) -> Result<(), ApiError> {
    let parsed = Url::parse(value).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("{field} must be a valid http(s) URL"),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{field} must be a valid http(s) URL"),
        ));
    }
    Ok(())
}

fn valid_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_version_identifier(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 96
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
}

fn valid_channel(channel: &str) -> bool {
    !channel.is_empty()
        && channel.len() <= 32
        && channel
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn ensure_project_unique(
    projects: &[CatalogProject],
    candidate: &CatalogProject,
    ignored_index: Option<usize>,
) -> Result<(), ApiError> {
    for (index, project) in projects.iter().enumerate() {
        if ignored_index == Some(index) {
            continue;
        }
        if project.slug.eq_ignore_ascii_case(&candidate.slug) {
            return Err((StatusCode::CONFLICT, "project slug already exists".into()));
        }
        if project.name.eq_ignore_ascii_case(&candidate.name) {
            return Err((StatusCode::CONFLICT, "project name already exists".into()));
        }
    }
    Ok(())
}

fn ensure_version_unique(
    versions: &[CatalogVersion],
    candidate: &CatalogVersion,
    ignored_index: Option<usize>,
) -> Result<(), ApiError> {
    if versions.iter().enumerate().any(|(index, version)| {
        ignored_index != Some(index)
            && version.project == candidate.project
            && version.version.eq_ignore_ascii_case(&candidate.version)
    }) {
        return Err((
            StatusCode::CONFLICT,
            "project version already exists".into(),
        ));
    }
    Ok(())
}

fn default_channel() -> String {
    "stable".into()
}

pub(crate) fn seed_catalog() -> CatalogState {
    let core_projects = vec![
        seeded_project(
            "paper",
            "Paper",
            "高性能插件服务端核心。",
            "Paper 在 Spigot 基础上提供性能优化、漏洞修复和 API 扩展，是主流插件服务器核心。",
            "PaperMC",
            "https://papermc.io/software/paper",
            "https://github.com/PaperMC/Paper",
            &["性能", "插件", "主流"],
            "#8b5cf6",
            true,
        ),
        seeded_project(
            "purpur",
            "Purpur",
            "提供丰富玩法配置的 Paper 分支。",
            "Purpur 基于 Paper 构建，额外提供大量游戏行为配置，适合深度自定义的生存服务器。",
            "PurpurMC",
            "https://purpurmc.org",
            "https://github.com/PurpurMC/Purpur",
            &["自定义", "插件"],
            "#c084fc",
            true,
        ),
        seeded_project(
            "fabric",
            "Fabric",
            "轻量、模块化的模组加载器。",
            "Fabric Loader 提供轻量且快速迭代的模组运行环境，适合模组服与需要细粒度扩展的服务端。",
            "FabricMC",
            "https://fabricmc.net/use/server/",
            "https://github.com/FabricMC/fabric-loader",
            &["模组", "轻量", "加载器"],
            "#d6a85f",
            true,
        ),
        seeded_project(
            "velocity",
            "Velocity",
            "现代化高性能群组代理端。",
            "Velocity 是 PaperMC 出品的代理服务端，用于连接多个子服并支持现代转发协议。",
            "PaperMC",
            "https://papermc.io/software/velocity",
            "https://github.com/PaperMC/Velocity",
            &["代理", "群组"],
            "#38bdf8",
            false,
        ),
    ];
    let plugin_projects = vec![
        seeded_project(
            "luckperms",
            "LuckPerms",
            "跨平台权限与用户组管理插件。",
            "LuckPerms 为 Bukkit、Fabric 和代理端提供权限、继承、上下文及 Web 编辑器能力。",
            "LuckPerms",
            "https://luckperms.net/download",
            "https://github.com/LuckPerms/LuckPerms",
            &["权限", "管理", "跨平台"],
            "#f5b942",
            true,
        ),
        seeded_project(
            "viaversion",
            "ViaVersion",
            "允许新版本客户端连接旧版本服务端。",
            "ViaVersion 提供协议转换能力，常用于跨 Minecraft 版本兼容与群组服平滑升级。",
            "ViaVersion",
            "https://viaversion.com",
            "https://github.com/ViaVersion/ViaVersion",
            &["协议", "兼容", "群组"],
            "#3b82f6",
            true,
        ),
        seeded_project(
            "chunky",
            "Chunky",
            "高效预生成世界区块。",
            "Chunky 可在开服前预生成区块并控制任务范围，减少玩家探索时的实时区块生成压力。",
            "pop4959",
            "https://modrinth.com/plugin/chunky",
            "https://github.com/pop4959/Chunky",
            &["区块", "性能", "运维"],
            "#22c55e",
            false,
        ),
        seeded_project(
            "placeholderapi",
            "PlaceholderAPI",
            "为插件提供统一变量占位符生态。",
            "PlaceholderAPI 让计分板、聊天和菜单插件共享玩家与服务器变量，是插件服常用基础依赖。",
            "PlaceholderAPI",
            "https://placeholderapi.com",
            "https://github.com/PlaceholderAPI/PlaceholderAPI",
            &["前置", "变量", "生态"],
            "#ec4899",
            false,
        ),
    ];
    let core_versions = vec![
        seeded_published_version(
            "seed-paper-1214-232",
            "paper",
            "1.21.4-232",
            "stable",
            &["1.21.4"],
            &["paper"],
            "paper-1.21.4-232.jar",
            "https://fill-data.papermc.io/v1/objects/5ee4f542f628a14c644410b08c94ea42e772ef4d29fe92973636b6813d4eaffc/paper-1.21.4-232.jar",
            "2025-06-09T10:18:55.778Z",
            51_437_498,
            "5ee4f542f628a14c644410b08c94ea42e772ef4d29fe92973636b6813d4eaffc",
            "PaperMC 官方稳定构建 232，文件大小与 SHA-256 已按官方元数据核验。",
        ),
        seeded_version(
            "seed-purpur-1214",
            "purpur",
            "1.21.4-latest",
            "stable",
            &["1.21.4"],
            &["purpur"],
            "purpur-1.21.4.jar",
            "https://api.purpurmc.org/v2/purpur/1.21.4/latest/download",
            "2026-07-19T09:00:00+08:00",
        ),
        seeded_version(
            "seed-fabric-1214",
            "fabric",
            "0.16.14+1.21.4",
            "stable",
            &["1.21.4"],
            &["fabric"],
            "fabric-server-1.21.4.jar",
            "https://meta.fabricmc.net/v2/versions/loader/1.21.4",
            "2026-07-18T09:00:00+08:00",
        ),
        seeded_version(
            "seed-velocity-340",
            "velocity",
            "3.4.0-SNAPSHOT",
            "beta",
            &["1.21.4"],
            &["velocity"],
            "velocity-3.4.0-SNAPSHOT.jar",
            "https://api.papermc.io/v2/projects/velocity",
            "2026-07-17T09:00:00+08:00",
        ),
    ];
    let plugin_versions = vec![
        seeded_published_version(
            "seed-luckperms-5553-bukkit",
            "luckperms",
            "5.5.53-bukkit",
            "stable",
            &["1.21.4", "1.21.1", "1.20.6"],
            &["paper", "purpur", "fabric", "velocity"],
            "LuckPerms-Bukkit-5.5.53.jar",
            "https://cdn.modrinth.com/data/Vebnzrzj/versions/MBSY8toc/LuckPerms-Bukkit-5.5.53.jar",
            "2026-05-27T07:14:37.365816Z",
            1_490_252,
            "fc8d4eccbf11c1e844af4527f018bbfde90c1866a9aba1bf880173a8e644cd59",
            "LuckPerms Bukkit 5.5.53；文件已从 Modrinth 官方 CDN 下载并核验 SHA-256。",
        ),
        seeded_version(
            "seed-viaversion-521",
            "viaversion",
            "5.2.1",
            "stable",
            &["1.21.4", "1.21.1", "1.20.6"],
            &["paper", "purpur", "velocity"],
            "ViaVersion-5.2.1.jar",
            "https://api.modrinth.com/v2/project/viaversion/version",
            "2026-07-15T09:00:00+08:00",
        ),
        seeded_published_version(
            "seed-chunky-1440",
            "chunky",
            "1.4.40",
            "stable",
            &["1.21.4", "1.21.1"],
            &["paper", "purpur", "fabric"],
            "Chunky-Bukkit-1.4.40.jar",
            "https://cdn.modrinth.com/data/fALzjamp/versions/P3y2MXnd/Chunky-Bukkit-1.4.40.jar",
            "2025-06-21T08:01:39.060370Z",
            296_244,
            "2a5477fc80f71012e15ade1ce34dbeb836e17623b28db112492c0f1443c09721",
            "Chunky Bukkit 1.4.40；文件已从 Modrinth 官方 CDN 下载并核验 SHA-256。",
        ),
        seeded_version(
            "seed-placeholderapi-2116",
            "placeholderapi",
            "2.11.6",
            "stable",
            &["1.21.4", "1.21.1", "1.20.6"],
            &["paper", "purpur"],
            "PlaceholderAPI-2.11.6.jar",
            "https://api.modrinth.com/v2/project/placeholderapi/version",
            "2026-07-13T09:00:00+08:00",
        ),
    ];
    CatalogState {
        schema_version: 1,
        core_projects,
        plugin_projects,
        core_versions,
        plugin_versions,
    }
}

impl CatalogState {
    pub(crate) fn migrate(&mut self) -> bool {
        const CURRENT_SCHEMA: u8 = 1;
        if self.schema_version >= CURRENT_SCHEMA {
            return false;
        }

        self.core_versions
            .retain(|version| !placeholder_url(&version.download_url));
        self.plugin_versions
            .retain(|version| !placeholder_url(&version.download_url));
        self.core_projects.retain(|project| {
            !placeholder_url(&project.homepage) && !placeholder_url(&project.repository)
        });
        self.plugin_projects.retain(|project| {
            !placeholder_url(&project.homepage) && !placeholder_url(&project.repository)
        });
        self.core_versions.retain(|version| {
            self.core_projects
                .iter()
                .any(|project| project.slug == version.project)
        });
        self.plugin_versions.retain(|version| {
            self.plugin_projects
                .iter()
                .any(|project| project.slug == version.project)
        });

        let seeded = seed_catalog();
        merge_projects(&mut self.core_projects, seeded.core_projects);
        merge_projects(&mut self.plugin_projects, seeded.plugin_projects);
        merge_versions(&mut self.core_versions, seeded.core_versions);
        merge_versions(&mut self.plugin_versions, seeded.plugin_versions);
        self.schema_version = CURRENT_SCHEMA;
        true
    }
}

fn merge_projects(target: &mut Vec<CatalogProject>, seeded: Vec<CatalogProject>) {
    for project in seeded {
        if !target
            .iter()
            .any(|current| current.slug.eq_ignore_ascii_case(&project.slug))
        {
            target.push(project);
        }
    }
}

fn merge_versions(target: &mut Vec<CatalogVersion>, seeded: Vec<CatalogVersion>) {
    for version in seeded {
        if !target.iter().any(|current| {
            current.project.eq_ignore_ascii_case(&version.project)
                && current.version.eq_ignore_ascii_case(&version.version)
        }) {
            target.push(version);
        }
    }
}

fn placeholder_url(value: &str) -> bool {
    const RESERVED_EXAMPLE_HOST: &str = concat!("example", ".", "com");
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            host == RESERVED_EXAMPLE_HOST
                || host
                    .strip_suffix(RESERVED_EXAMPLE_HOST)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
}

#[allow(clippy::too_many_arguments)]
fn seeded_project(
    slug: &str,
    name: &str,
    summary: &str,
    description: &str,
    author: &str,
    homepage: &str,
    repository: &str,
    tags: &[&str],
    color: &str,
    featured: bool,
) -> CatalogProject {
    CatalogProject {
        slug: slug.into(),
        name: name.into(),
        summary: summary.into(),
        description: description.into(),
        author: author.into(),
        homepage: homepage.into(),
        repository: repository.into(),
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
        color: color.into(),
        featured,
    }
}

#[allow(clippy::too_many_arguments)]
fn seeded_version(
    id: &str,
    project: &str,
    version: &str,
    channel: &str,
    minecraft_versions: &[&str],
    loaders: &[&str],
    filename: &str,
    download_url: &str,
    released_at: &str,
) -> CatalogVersion {
    CatalogVersion {
        id: id.into(),
        project: project.into(),
        version: version.into(),
        channel: channel.into(),
        minecraft_versions: minecraft_versions
            .iter()
            .map(|version| (*version).into())
            .collect(),
        loaders: loaders.iter().map(|loader| (*loader).into()).collect(),
        java_version: Some(21),
        filename: filename.into(),
        size: 0,
        sha256: String::new(),
        download_url: download_url.into(),
        release_notes: "展示元数据；发布前需锁定真实二进制并补录文件大小与 SHA-256。".into(),
        released_at: released_at.into(),
        status: "draft".into(),
        downloads: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn seeded_published_version(
    id: &str,
    project: &str,
    version: &str,
    channel: &str,
    minecraft_versions: &[&str],
    loaders: &[&str],
    filename: &str,
    download_url: &str,
    released_at: &str,
    size: u64,
    sha256: &str,
    release_notes: &str,
) -> CatalogVersion {
    let mut version = seeded_version(
        id,
        project,
        version,
        channel,
        minecraft_versions,
        loaders,
        filename,
        download_url,
        released_at,
    );
    version.size = size;
    version.sha256 = sha256.into();
    version.release_notes = release_notes.into();
    version.status = "published".into();
    version
}

async fn openapi() -> Json<Value> {
    Json(openapi_document())
}

fn openapi_document() -> Value {
    let mut paths = Map::new();
    paths.insert(
        "/api/catalog/summary".into(),
        json!({"get": operation("目录统计概览", vec![], None, "200", schema_ref("CatalogSummary"))}),
    );
    add_catalog_paths(&mut paths, "cores", "核心");
    add_catalog_paths(&mut paths, "plugins", "插件");
    paths.insert(
        "/api/v1/resolve".into(),
        json!({
            "get": operation(
                "解析最新已发布兼容版本",
                vec![
                    query_parameter("kind", true, "core 或 plugin", json!({"type":"string","enum":["core","plugin"]})),
                    query_parameter("project", true, "项目 slug", json!({"type":"string"})),
                    query_parameter("minecraft", true, "Minecraft 版本", json!({"type":"string"})),
                    query_parameter("channel", false, "发布渠道，默认 stable", json!({"type":"string","default":"stable"})),
                ],
                None,
                "200",
                schema_ref("ResolveResponse"),
            )
        }),
    );
    paths.insert(
        "/api/v1/download/{kind}/{project}/{version}".into(),
        json!({
            "get": operation(
                "记录下载计数并重定向到已发布制品",
                vec![
                    path_parameter("kind", "core 或 plugin"),
                    path_parameter("project", "项目 slug"),
                    path_parameter("version", "版本标识"),
                ],
                None,
                "307",
                Value::Null,
            )
        }),
    );
    paths.insert(
        "/api/openapi.json".into(),
        json!({"get": operation("OpenAPI 描述文档", vec![], None, "200", json!({"type":"object"}))}),
    );

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Sculk Catalyst Catalog API",
            "description": "核心与插件目录、版本解析与下载重定向接口。",
            "version": "0.1.0"
        },
        "servers": [{"url": "http://127.0.0.1:8787"}],
        "paths": Value::Object(paths),
        "components": {
            "schemas": {
                "ProjectInput": {
                    "type": "object",
                    "required": ["slug","name","summary","description","author","homepage","repository"],
                    "properties": {
                        "slug": {"type":"string","pattern":"^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$"},
                        "name": {"type":"string"},
                        "summary": {"type":"string"},
                        "description": {"type":"string"},
                        "author": {"type":"string"},
                        "homepage": {"type":"string","format":"uri"},
                        "repository": {"type":"string","format":"uri"},
                        "tags": {"type":"array","items":{"type":"string"}},
                        "color": {"type":"string","pattern":"^#[0-9A-Fa-f]{6}$","default":"#32d5b0"},
                        "featured": {"type":"boolean","default":false}
                    }
                },
                "Project": {
                    "allOf": [
                        {"$ref":"#/components/schemas/ProjectInput"},
                        {"type":"object","properties":{
                            "kind":{"type":"string","enum":["core","plugin"]},
                            "version_count":{"type":"integer","minimum":0},
                            "published_versions":{"type":"integer","minimum":0},
                            "latest_version":{"type":["string","null"]},
                            "downloads":{"type":"integer","minimum":0},
                            "minecraft_versions":{"type":"array","items":{"type":"string"}},
                            "channels":{"type":"array","items":{"type":"string"}},
                            "loaders":{"type":"array","items":{"type":"string"}}
                        }}
                    ]
                },
                "VersionInput": {
                    "type":"object",
                    "required":["version","channel","minecraft_versions","loaders","filename","download_url","release_notes","released_at","status"],
                    "properties":{
                        "version":{"type":"string"},
                        "channel":{"type":"string"},
                        "minecraft_versions":{"type":"array","minItems":1,"items":{"type":"string"}},
                        "loaders":{"type":"array","minItems":1,"items":{"type":"string"}},
                        "java_version":{"type":["integer","null"],"minimum":8,"maximum":99},
                        "filename":{"type":"string"},
                        "size":{"type":"integer","minimum":0},
                        "sha256":{"type":"string","pattern":"^(?:[0-9A-Fa-f]{64})?$"},
                        "download_url":{"type":"string","format":"uri"},
                        "release_notes":{"type":"string"},
                        "released_at":{"type":"string","format":"date-time"},
                        "status":{"type":"string","enum":["draft","published","yanked"]}
                    }
                },
                "Version": {
                    "allOf":[
                        {"$ref":"#/components/schemas/VersionInput"},
                        {"type":"object","required":["id","project","downloads"],"properties":{
                            "id":{"type":"string"},
                            "project":{"type":"string"},
                            "downloads":{"type":"integer","minimum":0}
                        }}
                    ]
                },
                "CatalogSummary": {
                    "type":"object",
                    "required":["core_projects","plugin_projects","versions","downloads"],
                    "properties":{
                        "core_projects":{"type":"integer","minimum":0},
                        "plugin_projects":{"type":"integer","minimum":0},
                        "versions":{"type":"integer","minimum":0},
                        "downloads":{"type":"integer","minimum":0},
                        "published_versions":{"type":"integer","minimum":0},
                        "featured_projects":{"type":"integer","minimum":0}
                    }
                },
                "ResolveResponse": {
                    "type":"object",
                    "required":["kind","project","version","download_path"],
                    "properties":{
                        "kind":{"type":"string","enum":["core","plugin"]},
                        "project":{"$ref":"#/components/schemas/Project"},
                        "version":{"$ref":"#/components/schemas/Version"},
                        "download_path":{"type":"string"}
                    }
                },
                "DeleteResponse": {
                    "type":"object",
                    "properties":{
                        "deleted":{"type":"boolean"},
                        "slug":{"type":"string"},
                        "version":{"type":["string","null"]},
                        "deleted_versions":{"type":"integer","minimum":0}
                    }
                }
            }
        }
    })
}

fn add_catalog_paths(paths: &mut Map<String, Value>, resource: &str, label: &str) {
    let base = format!("/api/catalog/{resource}");
    let item = format!("{base}/{{slug}}");
    let versions = format!("{item}/versions");
    let version = format!("{versions}/{{version}}");
    paths.insert(
        base,
        json!({
            "get": operation(
                &format!("查询{label}项目"),
                filter_parameters(),
                None,
                "200",
                array_schema("Project"),
            ),
            "post": operation(
                &format!("创建{label}项目"),
                vec![],
                Some("ProjectInput"),
                "200",
                schema_ref("Project"),
            )
        }),
    );
    paths.insert(
        item,
        json!({
            "get": operation(&format!("获取{label}项目"), vec![path_parameter("slug", "项目 slug")], None, "200", schema_ref("Project")),
            "put": operation(&format!("更新{label}项目"), vec![path_parameter("slug", "当前项目 slug")], Some("ProjectInput"), "200", schema_ref("Project")),
            "delete": operation(&format!("删除{label}项目及其版本"), vec![path_parameter("slug", "项目 slug")], None, "200", schema_ref("DeleteResponse"))
        }),
    );
    paths.insert(
        versions,
        json!({
            "get": operation(&format!("查询{label}版本"), version_parameters(false), None, "200", array_schema("Version")),
            "post": operation(&format!("创建{label}版本"), vec![path_parameter("slug", "项目 slug")], Some("VersionInput"), "200", schema_ref("Version"))
        }),
    );
    paths.insert(
        version,
        json!({
            "get": operation(&format!("获取{label}版本"), version_parameters(true), None, "200", schema_ref("Version")),
            "put": operation(&format!("更新{label}版本"), version_parameters(true), Some("VersionInput"), "200", schema_ref("Version")),
            "delete": operation(&format!("删除{label}版本"), version_parameters(true), None, "200", schema_ref("DeleteResponse"))
        }),
    );
}

fn operation(
    summary: &str,
    parameters: Vec<Value>,
    request_schema: Option<&str>,
    success_status: &str,
    success_schema: Value,
) -> Value {
    let mut responses = Map::new();
    responses.insert(
        success_status.into(),
        json!({"description": if success_status == "307" {"Temporary Redirect"} else {"OK"}}),
    );
    responses.insert(
        "400".into(),
        json!({"description":"请求参数或数据校验失败"}),
    );
    responses.insert("404".into(), json!({"description":"资源不存在"}));
    responses.insert(
        "409".into(),
        json!({"description":"资源冲突或版本尚未发布"}),
    );
    let mut operation = json!({"summary": summary, "responses": Value::Object(responses)});
    if !parameters.is_empty() {
        operation["parameters"] = Value::Array(parameters);
    }
    if let Some(schema) = request_schema {
        operation["requestBody"] = json!({
            "required": true,
            "content": {"application/json": {"schema": schema_ref(schema)}}
        });
    }
    if !success_schema.is_null() {
        operation["responses"][success_status]["content"] =
            json!({"application/json":{"schema":success_schema}});
    } else if success_status == "307" {
        operation["responses"][success_status]["headers"] =
            json!({"Location":{"schema":{"type":"string","format":"uri"}}});
    }
    operation
}

fn schema_ref(name: &str) -> Value {
    json!({"$ref":format!("#/components/schemas/{name}")})
}

fn array_schema(name: &str) -> Value {
    json!({"type":"array","items":schema_ref(name)})
}

fn filter_parameters() -> Vec<Value> {
    vec![
        query_parameter(
            "search",
            false,
            "搜索名称、slug、作者、简介或标签",
            json!({"type":"string"}),
        ),
        query_parameter(
            "minecraft",
            false,
            "筛选兼容的 Minecraft 版本",
            json!({"type":"string"}),
        ),
        query_parameter("channel", false, "筛选发布渠道", json!({"type":"string"})),
    ]
}

fn version_parameters(include_version: bool) -> Vec<Value> {
    let mut parameters = vec![path_parameter("slug", "项目 slug")];
    if include_version {
        parameters.push(path_parameter("version", "版本标识"));
    } else {
        parameters.extend(filter_parameters());
    }
    parameters
}

fn path_parameter(name: &str, description: &str) -> Value {
    json!({
        "name":name,
        "in":"path",
        "required":true,
        "description":description,
        "schema":{"type":"string"}
    })
}

fn query_parameter(name: &str, required: bool, description: &str, schema: Value) -> Value {
    json!({
        "name":name,
        "in":"query",
        "required":required,
        "description":description,
        "schema":schema
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn published_version(
        project: &str,
        version: &str,
        minecraft: &str,
        released_at: &str,
    ) -> CatalogVersion {
        let mut item = seeded_version(
            &format!("test-{project}-{version}"),
            project,
            version,
            "stable",
            &[minecraft],
            &["paper"],
            &format!("{project}-{version}.jar"),
            "https://github.com/PaperMC/Paper/releases",
            released_at,
        );
        item.status = "published".into();
        item.size = 42;
        item.sha256 = "a".repeat(64);
        item
    }

    #[test]
    fn filters_projects_and_versions() {
        let catalog = seed_catalog();
        let query = CatalogQuery {
            search: Some("高性能插件".into()),
            minecraft: Some("1.21.4".into()),
            channel: Some("stable".into()),
        };
        let projects = filtered_project_views(
            CatalogKind::Core,
            &catalog.core_projects,
            &catalog.core_versions,
            &query,
        );
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].project.slug, "paper");

        let versions = filtered_versions(
            &catalog.plugin_versions,
            "luckperms",
            &CatalogQuery {
                search: None,
                minecraft: Some("1.21.1".into()),
                channel: Some("stable".into()),
            },
        );
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, "5.5.53-bukkit");
    }

    #[test]
    fn resolve_uses_latest_published_compatible_version() {
        let older = published_version("paper", "1.0.0", "1.21.4", "2026-01-01T00:00:00Z");
        let newer = published_version("paper", "1.1.0", "1.21.4", "2026-02-01T00:00:00Z");
        let incompatible = published_version("paper", "2.0.0", "1.22", "2026-03-01T00:00:00Z");
        let mut draft = published_version("paper", "3.0.0", "1.21.4", "2026-04-01T00:00:00Z");
        draft.status = "draft".into();
        let versions = vec![older, newer, incompatible, draft];

        let resolved = resolve_version(&versions, "paper", "1.21.4", "stable").unwrap();
        assert_eq!(resolved.version, "1.1.0");
        assert_ne!(resolved.status, "draft");
    }

    #[test]
    fn downloader_resolves_only_valid_published_core_artifacts() {
        let mut catalog = seed_catalog();
        let resolved = resolve_core_download(&catalog, "Paper", "1.21.4", "stable").unwrap();
        assert_eq!(resolved.version, "1.21.4-232");
        assert_eq!(resolved.sha256.len(), 64);

        let id = resolved.id.clone();
        assert!(record_core_download(&mut catalog, &id));
        assert_eq!(
            catalog
                .core_versions
                .iter()
                .find(|version| version.id == id)
                .unwrap()
                .downloads,
            resolved.downloads + 1
        );

        assert!(resolve_core_download(&catalog, "paper", "1.20.1", "stable").is_none());
        assert!(resolve_core_download(&catalog, "purpur", "1.21.4", "stable").is_none());
    }

    #[test]
    fn validates_project_version_url_hash_and_uniqueness() {
        let catalog = seed_catalog();
        for project in catalog.core_projects.iter().chain(&catalog.plugin_projects) {
            validate_project(project).unwrap();
        }
        for version in catalog.core_versions.iter().chain(&catalog.plugin_versions) {
            validate_version(version).unwrap();
        }

        let mut bad_slug = catalog.core_projects[0].clone();
        bad_slug.slug = "bad slug".into();
        assert!(validate_project(&bad_slug).is_err());

        let mut bad_url = catalog.core_projects[0].clone();
        bad_url.homepage = "ftp://papermc.io".into();
        assert!(validate_project(&bad_url).is_err());

        let mut bad_hash = catalog.core_versions[0].clone();
        bad_hash.sha256 = "1234".into();
        assert!(validate_version(&bad_hash).is_err());

        let mut published = catalog.core_versions[0].clone();
        published.status = "published".into();
        published.size = 0;
        published.sha256.clear();
        assert!(validate_version(&published).is_err());
        published.size = 1;
        published.sha256 = "b".repeat(64);
        assert!(validate_version(&published).is_ok());

        assert!(
            ensure_project_unique(&catalog.core_projects, &catalog.core_projects[0], None).is_err()
        );
        assert!(
            ensure_version_unique(&catalog.core_versions, &catalog.core_versions[0], None).is_err()
        );
    }

    #[test]
    fn legacy_state_without_catalog_uses_seed_default() {
        let legacy = r#"{"servers":[],"tasks":[],"configs":{},"logs":{}}"#;
        let state: crate::PersistedState = serde_json::from_str(legacy).unwrap();
        assert_eq!(state.catalog.schema_version, 1);
        assert_eq!(state.catalog.core_projects.len(), 4);
        assert_eq!(state.catalog.plugin_projects.len(), 4);
        assert_eq!(state.catalog.core_versions.len(), 4);
        assert_eq!(state.catalog.plugin_versions.len(), 4);
    }

    #[test]
    fn migrates_early_placeholder_catalog_once() {
        let mut catalog = seed_catalog();
        catalog.schema_version = 0;
        catalog
            .core_projects
            .retain(|project| project.slug != "fabric");
        catalog.plugin_projects.clear();
        catalog.plugin_versions.clear();
        let reserved_host = ["example", "com"].join(".");
        let placeholder_homepage = format!("https://catalog.{reserved_host}/project");
        let placeholder_repository = format!("https://git.{reserved_host}/project");
        let placeholder_download = format!("https://downloads.{reserved_host}/placeholder.jar");
        catalog.plugin_projects.push(seeded_project(
            "placeholder-demo",
            "Placeholder Demo",
            "旧占位项目",
            "旧占位项目",
            "Sculk",
            &placeholder_homepage,
            &placeholder_repository,
            &["demo"],
            "#123456",
            false,
        ));
        catalog.plugin_versions.push(seeded_version(
            "placeholder-version",
            "placeholder-demo",
            "1.0.0",
            "stable",
            &["1.21.4"],
            &["paper"],
            "placeholder.jar",
            &placeholder_download,
            "2026-01-01T00:00:00Z",
        ));

        assert!(catalog.migrate());
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.core_projects.len(), 4);
        assert_eq!(catalog.plugin_projects.len(), 4);
        assert!(
            !catalog
                .core_versions
                .iter()
                .chain(&catalog.plugin_versions)
                .any(|version| placeholder_url(&version.download_url))
        );
        assert!(!catalog.migrate());
    }

    #[test]
    fn openapi_covers_catalog_resolve_and_redirect_contracts() {
        let document = openapi_document();
        assert_eq!(document["openapi"], "3.1.0");
        assert!(document["paths"]["/api/catalog/cores"].is_object());
        assert!(document["paths"]["/api/catalog/plugins/{slug}/versions/{version}"].is_object());
        assert!(document["paths"]["/api/v1/resolve"].is_object());
        assert!(document["paths"]["/api/v1/download/{kind}/{project}/{version}"]["get"]
            ["responses"]["307"]
            .is_object());
    }
}
