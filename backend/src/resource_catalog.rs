use crate::AppState;
use axum::{
    Router,
    body::Body,
    extract::{Path, RawQuery},
    http::{Method, Response, StatusCode, header},
    routing::get,
};
use reqwest::{Client, Url, redirect::Policy};
use std::{net::IpAddr, sync::OnceLock, time::Duration};

const DEFAULT_RESOURCE_API_BASE: &str = "https://res.mcmy.love";
const DEFAULT_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MIN_MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PROXY_PATH_BYTES: usize = 1024;
const MAX_PROXY_QUERY_BYTES: usize = 4096;

type ProxyResult = Result<Response<Body>, (StatusCode, String)>;

pub(crate) fn router() -> Router<AppState> {
    Router::new().route(
        "/api/resource-catalog/{*path}",
        get(proxy_resource).head(proxy_resource),
    )
}

async fn proxy_resource(
    method: Method,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
) -> ProxyResult {
    let base = configured_resource_base()?;
    proxy_with_client(
        proxy_client(),
        &base,
        &method,
        &path,
        query.as_deref(),
        max_response_bytes(),
    )
    .await
}

fn proxy_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .redirect(Policy::none())
            .user_agent("Sculk-Catalyst-Resource-Catalog-Proxy/1.0")
            .build()
            .expect("resource catalog proxy client configuration must be valid")
    })
}

fn configured_resource_base() -> Result<Url, (StatusCode, String)> {
    let configured = std::env::var("SCULK_RESOURCE_API_BASE")
        .unwrap_or_else(|_| DEFAULT_RESOURCE_API_BASE.into());
    validate_resource_base(&configured).map_err(|message| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("resource catalog upstream is unavailable: {message}"),
        )
    })
}

fn validate_resource_base(value: &str) -> Result<Url, String> {
    let mut url = Url::parse(value.trim()).map_err(|_| "base URL is invalid".to_string())?;
    if url.username() != "" || url.password().is_some() {
        return Err("base URL cannot contain credentials".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("base URL cannot contain a query or fragment".into());
    }
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(&url) => {}
        _ => return Err("base URL must use HTTPS (loopback HTTP is allowed)".into()),
    }
    if url.cannot_be_a_base() || url.host_str().is_none() {
        return Err("base URL must be hierarchical and include a host".into());
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Ok(url)
}

fn is_loopback_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn max_response_bytes() -> usize {
    std::env::var("SCULK_RESOURCE_PROXY_MAX_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES)
        .clamp(MIN_MAX_RESPONSE_BYTES, MAX_MAX_RESPONSE_BYTES)
}

fn validate_public_path(path: &str) -> Result<Vec<&str>, (StatusCode, String)> {
    if path.is_empty() || path.len() > MAX_PROXY_PATH_BYTES {
        return Err((
            StatusCode::NOT_FOUND,
            "resource catalog path is not public".into(),
        ));
    }
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || *segment == "."
            || *segment == ".."
            || segment.len() > 256
            || segment.chars().any(char::is_control)
    }) {
        return Err((
            StatusCode::NOT_FOUND,
            "resource catalog path is not public".into(),
        ));
    }

    let allowed = match segments.as_slice() {
        ["api", "openapi.json"] => true,
        ["api", "catalog", "summary"] => true,
        ["api", "catalog", resource] => is_public_resource(resource),
        ["api", "catalog", resource, _slug] => is_public_resource(resource),
        ["api", "catalog", resource, _slug, "versions"] => is_public_resource(resource),
        ["api", "catalog", resource, _slug, "versions", _version] => is_public_resource(resource),
        ["api", "v1", "plugins", "search"] => true,
        ["api", "v1", "resolve"] => true,
        ["api", "v1", "download", kind, _project, _version] => is_public_kind(kind),
        _ => false,
    };
    if !allowed {
        return Err((
            StatusCode::NOT_FOUND,
            "resource catalog path is not public".into(),
        ));
    }
    Ok(segments)
}

fn is_public_resource(value: &str) -> bool {
    matches!(
        value,
        "cores" | "plugins" | "skins" | "bbmodels" | "ui-textures" | "skills" | "plugin-configs"
    )
}

fn is_public_kind(value: &str) -> bool {
    matches!(
        value,
        "core" | "plugin" | "skin" | "bbmodel" | "ui_texture" | "skill" | "plugin_config"
    )
}

fn build_upstream_url(
    base: &Url,
    path: &str,
    query: Option<&str>,
) -> Result<Url, (StatusCode, String)> {
    if query.is_some_and(|query| query.len() > MAX_PROXY_QUERY_BYTES) {
        return Err((
            StatusCode::URI_TOO_LONG,
            "resource catalog query is too long".into(),
        ));
    }
    let segments = validate_public_path(path)?;
    let mut url = base.clone();
    {
        let mut target = url.path_segments_mut().map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "resource catalog upstream base cannot contain path segments".into(),
            )
        })?;
        target.pop_if_empty();
        for segment in segments {
            target.push(segment);
        }
    }
    url.set_query(query.filter(|query| !query.is_empty()));
    Ok(url)
}

async fn proxy_with_client(
    client: &Client,
    base: &Url,
    method: &Method,
    path: &str,
    query: Option<&str>,
    max_bytes: usize,
) -> ProxyResult {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return Err((
            StatusCode::METHOD_NOT_ALLOWED,
            "resource catalog proxy is read-only".into(),
        ));
    }
    let upstream_url = build_upstream_url(base, path, query)?;
    let request = if *method == Method::HEAD {
        client.head(upstream_url.clone())
    } else {
        client.get(upstream_url.clone())
    };
    // Deliberately construct a fresh request without copying caller headers. In
    // particular, Authorization and Cookie must never reach the resource host.
    let mut upstream = request.send().await.map_err(|error| {
        (
            if error.is_timeout() {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::BAD_GATEWAY
            },
            format!("resource catalog upstream request failed: {error}"),
        )
    })?;
    let status = StatusCode::from_u16(upstream.status().as_u16()).map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            "upstream returned an invalid status".into(),
        )
    })?;
    if let Some(length) = upstream.content_length()
        && length > max_bytes as u64
    {
        return Err((
            StatusCode::BAD_GATEWAY,
            "resource catalog upstream response exceeds the configured limit".into(),
        ));
    }

    let mut builder = Response::builder().status(status);
    copy_safe_response_headers(&mut builder, upstream.headers(), &upstream_url);
    builder = builder.header("x-content-type-options", "nosniff");
    if *method == Method::HEAD {
        return builder
            .body(Body::empty())
            .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()));
    }

    let mut body = Vec::with_capacity(
        upstream
            .content_length()
            .unwrap_or_default()
            .min(max_bytes as u64) as usize,
    );
    while let Some(chunk) = upstream.chunk().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("resource catalog upstream body failed: {error}"),
        )
    })? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err((
                StatusCode::BAD_GATEWAY,
                "resource catalog upstream response exceeds the configured limit".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    builder
        .header(header::CONTENT_LENGTH, body.len())
        .body(Body::from(body))
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))
}

fn copy_safe_response_headers(
    builder: &mut axum::http::response::Builder,
    headers: &reqwest::header::HeaderMap,
    upstream_url: &Url,
) {
    for name in [
        header::CONTENT_TYPE,
        header::CACHE_CONTROL,
        header::ETAG,
        header::LAST_MODIFIED,
        header::CONTENT_DISPOSITION,
    ] {
        if let Some(value) = headers.get(name.as_str())
            && safe_content_header(&name, value)
        {
            *builder = std::mem::take(builder).header(name, value.as_bytes());
        }
    }
    if let Some(value) = headers.get(reqwest::header::LOCATION)
        && let Ok(value) = value.to_str()
        && let Ok(location) = upstream_url.join(value)
        && matches!(location.scheme(), "http" | "https")
        && location.username().is_empty()
        && location.password().is_none()
    {
        *builder = std::mem::take(builder).header(header::LOCATION, location.as_str());
    }
}

fn safe_content_header(name: &header::HeaderName, value: &reqwest::header::HeaderValue) -> bool {
    if *name != header::CONTENT_TYPE {
        return true;
    }
    let Ok(value) = value.to_str() else {
        return false;
    };
    let media_type = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        media_type.as_str(),
        "application/json"
            | "application/octet-stream"
            | "application/java-archive"
            | "application/zip"
            | "text/plain"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        extract::Request,
        http::{HeaderMap, Uri},
        response::IntoResponse,
        routing::any,
    };
    use serde_json::json;
    use tokio::net::TcpListener;

    #[test]
    fn public_path_allowlist_excludes_admin_and_malformed_paths() {
        for path in [
            "api/openapi.json",
            "api/catalog/summary",
            "api/catalog/cores",
            "api/catalog/plugins/luckperms",
            "api/catalog/cores/paper/versions",
            "api/catalog/cores/paper/versions/1.21.4-232",
            "api/v1/plugins/search",
            "api/v1/resolve",
            "api/v1/download/core/paper/1.21.4-232",
        ] {
            assert!(
                validate_public_path(path).is_ok(),
                "{path} should be public"
            );
        }
        for path in [
            "api/catalog/admin/verify",
            "api/catalog/admin/upload",
            "api/catalog/unknown",
            "api/catalog/cores/paper/delete/all",
            "api/v1/download/admin/paper/latest",
            "api/cloud/me",
            "../api/catalog/cores",
        ] {
            assert!(
                validate_public_path(path).is_err(),
                "{path} must be rejected"
            );
        }
    }

    #[test]
    fn upstream_url_preserves_base_path_and_query_without_accepting_credentials() {
        let base = validate_resource_base("https://resources.example.test/root").unwrap();
        let url = build_upstream_url(
            &base,
            "api/catalog/plugins/luckperms/versions",
            Some("minecraft=1.21.4&channel=stable"),
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://resources.example.test/root/api/catalog/plugins/luckperms/versions?minecraft=1.21.4&channel=stable"
        );
        assert!(validate_resource_base("https://user:secret@example.test").is_err());
        assert!(validate_resource_base("http://example.test").is_err());
        assert!(validate_resource_base("http://127.0.0.1:8788").is_ok());
    }

    async fn mock_upstream() -> (Url, tokio::task::JoinHandle<()>) {
        async fn handler(request: Request) -> impl IntoResponse {
            let uri: Uri = request.uri().clone();
            let headers: HeaderMap = request.headers().clone();
            match uri.path() {
                "/api/catalog/plugins" => (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "application/json"),
                        (header::CACHE_CONTROL, "public, max-age=60"),
                    ],
                    json!({
                        "path": uri.path(),
                        "query": uri.query(),
                        "authorization": headers.contains_key(header::AUTHORIZATION),
                        "cookie": headers.contains_key(header::COOKIE),
                    })
                    .to_string(),
                )
                    .into_response(),
                "/api/v1/download/core/paper/latest" => (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, "/objects/paper.jar")],
                    Body::empty(),
                )
                    .into_response(),
                "/api/catalog/skins" => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    "this response is intentionally too large",
                )
                    .into_response(),
                _ => (StatusCode::NOT_FOUND, "missing").into_response(),
            }
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/{*path}", any(handler)))
                .await
                .unwrap();
        });
        (Url::parse(&format!("http://{address}")).unwrap(), task)
    }

    #[tokio::test]
    async fn proxy_preserves_public_response_and_never_sends_credentials() {
        let (base, task) = mock_upstream().await;
        let response = proxy_with_client(
            proxy_client(),
            &base,
            &Method::GET,
            "api/catalog/plugins",
            Some("search=paper"),
            4096,
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=60"
        );
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["path"], "/api/catalog/plugins");
        assert_eq!(payload["query"], "search=paper");
        assert_eq!(payload["authorization"], false);
        assert_eq!(payload["cookie"], false);
        task.abort();
    }

    #[tokio::test]
    async fn proxy_rewrites_relative_redirects_and_enforces_body_limit() {
        let (base, task) = mock_upstream().await;
        let redirect = proxy_with_client(
            proxy_client(),
            &base,
            &Method::GET,
            "api/v1/download/core/paper/latest",
            None,
            4096,
        )
        .await
        .unwrap();
        assert_eq!(redirect.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            redirect.headers()[header::LOCATION],
            format!("{}objects/paper.jar", base.as_str())
        );

        let oversized = proxy_with_client(
            proxy_client(),
            &base,
            &Method::GET,
            "api/catalog/skins",
            None,
            8,
        )
        .await
        .unwrap_err();
        assert_eq!(oversized.0, StatusCode::BAD_GATEWAY);
        task.abort();
    }

    #[test]
    fn non_read_methods_are_rejected_even_below_the_router_boundary() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let base = Url::parse("http://127.0.0.1:9").unwrap();
        let result = runtime.block_on(proxy_with_client(
            proxy_client(),
            &base,
            &Method::POST,
            "api/catalog/cores",
            None,
            1024,
        ));
        assert_eq!(result.unwrap_err().0, StatusCode::METHOD_NOT_ALLOWED);
    }
}
