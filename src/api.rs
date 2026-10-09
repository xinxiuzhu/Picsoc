use std::{
    collections::BTreeSet,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Path, Query, Request, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use serde::Deserialize;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::oneshot,
};
use tokio_util::io::ReaderStream;

use crate::{
    db::Db,
    folders, i18n, media,
    models::{AssetBatch, AssetPatch, AssetQuery},
    scanner::{Scanner, ThumbnailJob},
};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub scanner: Scanner,
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    pub password: Option<Arc<str>>,
}

#[derive(rust_embed::RustEmbed)]
#[folder = "frontend/dist/"]
struct Frontend;

#[derive(Debug)]
pub struct ApiError(StatusCode, String);
impl ApiError {
    fn bad(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
    fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, "素材或目录不存在".into())
    }
    fn localized_response(self, english: bool) -> Response {
        let (message, code) = i18n::error_message(&self.1, english);
        let mut response = (self.0, Json(json!({"error":message,"code":code}))).into_response();
        response.headers_mut().insert(
            header::CONTENT_LANGUAGE,
            HeaderValue::from_static(if english { "en" } else { "zh-CN" }),
        );
        response
            .headers_mut()
            .insert(header::VARY, HeaderValue::from_static("Accept-Language"));
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        self.localized_response(false)
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        if error
            .downcast_ref::<rusqlite::Error>()
            .is_some_and(|e| matches!(e, rusqlite::Error::QueryReturnedNoRows))
        {
            return Self::missing();
        }
        tracing::error!(error=%error,"请求处理失败");
        Self(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(Into::into)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/stats", get(stats))
        .route("/api/directories", get(directories))
        .route("/api/libraries", get(libraries).post(add_library))
        .route("/api/libraries/{id}", axum::routing::delete(delete_library))
        .route("/api/libraries/{id}/scan", post(scan))
        .route("/api/libraries/{id}/scan/cancel", post(cancel_scan))
        .route("/api/libraries/{id}/folders", get(library_folders))
        .route("/api/assets", get(assets))
        .route("/api/assets/batch", post(batch_assets))
        .route("/api/assets/{id}", get(asset).patch(patch_asset))
        .route("/api/assets/{id}/thumbnail", get(thumbnail))
        .route("/api/assets/{id}/original", get(original))
        .route("/api/tags", get(tags))
        .fallback(frontend)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

async fn guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let english = i18n::wants_english(request.headers());
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if state.bind.ip().is_loopback() && !local_host(host, state.bind.port()) {
        return ApiError(
            StatusCode::FORBIDDEN,
            "本地服务仅接受 localhost 或回环地址访问".into(),
        )
        .localized_response(english);
    }
    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        let authority = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"));
        if authority.map(|s| s.trim_end_matches('/')) != Some(host) {
            return ApiError(StatusCode::FORBIDDEN, "拒绝跨站点请求".into())
                .localized_response(english);
        }
    }
    if let Some(password) = &state.password {
        let authorized = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Basic "))
            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
            .is_some_and(|v| constant_time_equal(&v, format!("picsoc:{password}").as_bytes()));
        if !authorized {
            let mut response = ApiError(
                StatusCode::UNAUTHORIZED,
                "需要用户名 picsoc 和设置的密码".into(),
            )
            .localized_response(english);
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"Picsoc\", charset=\"UTF-8\""),
            );
            return response;
        }
    }
    let mut response = next.run(request).await;
    // Axum's method/path rejections are plain text by default; keep all API failures in the same shape.
    if response.status().is_client_error()
        && !response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| v.as_bytes().starts_with(b"application/json"))
    {
        let status = response.status();
        let allow = response.headers().get(header::ALLOW).cloned();
        response = ApiError(
            status,
            match status {
                StatusCode::METHOD_NOT_ALLOWED => "请求方法无效",
                StatusCode::PAYLOAD_TOO_LARGE => "请求内容过大",
                StatusCode::NOT_FOUND => "接口不存在",
                _ => "请求参数无效",
            }
            .into(),
        )
        .into_response();
        if let Some(allow) = allow {
            response.headers_mut().insert(header::ALLOW, allow);
        }
    }
    response = localize_error_response(response, english).await;
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    response.headers_mut().insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );
    if response.headers().get(header::CACHE_CONTROL).is_none() {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

async fn localize_error_response(response: Response, english: bool) -> Response {
    if !(response.status().is_client_error() || response.status().is_server_error()) {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, 64 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => return ApiError(parts.status, "请求参数无效".into()).localized_response(english),
    };
    let mut value = serde_json::from_slice::<serde_json::Value>(&bytes)
        .unwrap_or_else(|_| json!({"error":"请求参数无效"}));
    if let Some(message) = value.get("error").and_then(serde_json::Value::as_str) {
        let (message, code) = i18n::error_message(message, english);
        value["error"] = message.into();
        value["code"] = code.into();
    }
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    parts.headers.insert(
        header::CONTENT_LANGUAGE,
        HeaderValue::from_static(if english { "en" } else { "zh-CN" }),
    );
    parts
        .headers
        .insert(header::VARY, HeaderValue::from_static("Accept-Language"));
    Response::from_parts(parts, Body::from(serde_json::to_vec(&value).unwrap()))
}

fn local_host(host: &str, port: u16) -> bool {
    [
        format!("localhost:{port}"),
        format!("127.0.0.1:{port}"),
        format!("[::1]:{port}"),
    ]
    .iter()
    .any(|allowed| host.eq_ignore_ascii_case(allowed))
        || (port == 80 && ["localhost", "127.0.0.1", "[::1]"].contains(&host))
}

fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for (i, byte) in b.iter().enumerate() {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ byte);
    }
    diff == 0
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"ok":true,"version":env!("CARGO_PKG_VERSION")}))
}

#[derive(Deserialize)]
struct DirectoryQuery {
    path: Option<String>,
}

#[derive(serde::Serialize)]
struct DirectoryEntry {
    name: String,
    path: String,
}

#[derive(serde::Serialize)]
struct DirectoryList {
    path: Option<String>,
    parent: Option<String>,
    roots: Vec<DirectoryEntry>,
    directories: Vec<DirectoryEntry>,
    truncated: bool,
}

fn directory_roots() -> Vec<DirectoryEntry> {
    let mut roots = Vec::new();
    let home = std::env::var_os(if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    });
    if let Some(home) = home
        .and_then(|p| PathBuf::from(p).canonicalize().ok())
        .filter(|p| p.is_dir())
        && let Some(path) = home.to_str()
    {
        roots.push(DirectoryEntry {
            name: "用户目录".into(),
            path: path.into(),
        });
    }
    #[cfg(windows)]
    for letter in b'A'..=b'Z' {
        let path = format!("{}:\\", char::from(letter));
        if FsPath::new(&path).is_dir() {
            roots.push(DirectoryEntry {
                name: path.clone(),
                path,
            });
        }
    }
    #[cfg(not(windows))]
    roots.push(DirectoryEntry {
        name: "文件系统 /".into(),
        path: "/".into(),
    });
    roots
}

fn list_directories(path: Option<String>) -> Result<DirectoryList> {
    let roots = directory_roots();
    let Some(path) = path.filter(|s| !s.trim().is_empty()) else {
        return Ok(DirectoryList {
            path: None,
            parent: None,
            roots,
            directories: Vec::new(),
            truncated: false,
        });
    };
    anyhow::ensure!(path.len() <= 4096, "目录路径过长");
    let root = FsPath::new(&path);
    anyhow::ensure!(root.is_absolute(), "请选择绝对目录路径");
    let root = root
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("目录不可访问：{e}"))?;
    anyhow::ensure!(root.is_dir(), "路径必须是文件夹");
    let root_string = root
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("目录路径必须能转换为 Unicode"))?
        .to_owned();
    let mut directories = Vec::new();
    let mut truncated = false;
    for entry in std::fs::read_dir(&root)? {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if std::fs::read_dir(&path).is_err() {
            continue;
        }
        let Some(path) = path.to_str().map(str::to_owned) else {
            continue;
        };
        if directories.len() == 1000 {
            truncated = true;
            break;
        }
        directories.push(DirectoryEntry { name, path });
    }
    directories.sort_by(|a, b| a.name.cmp(&b.name));
    let parent = root.parent().and_then(|p| p.to_str()).map(str::to_owned);
    Ok(DirectoryList {
        path: Some(root_string),
        parent,
        roots,
        directories,
        truncated,
    })
}

async fn directories(
    headers: HeaderMap,
    query: Result<Query<DirectoryQuery>, QueryRejection>,
) -> Result<Json<DirectoryList>, ApiError> {
    let Query(query) = query.map_err(|e| ApiError::bad(e.body_text()))?;
    let mut result = blocking(move || list_directories(query.path))
        .await
        .map_err(|e| ApiError::bad(e.1))?;
    if i18n::wants_english(&headers) {
        for root in &mut result.roots {
            root.name = match root.name.as_str() {
                "用户目录" => "Home".into(),
                "文件系统 /" => "File system /".into(),
                _ => format!("Drive {}", root.name),
            };
        }
    }
    Ok(Json(result))
}
async fn stats(State(state): State<AppState>) -> Result<Json<crate::models::Stats>, ApiError> {
    Ok(Json(blocking(move || state.db.stats()).await?))
}
async fn libraries(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let db = state.db.clone();
    let mut libraries = blocking(move || db.libraries()).await?;
    for library in &mut libraries {
        library.scan = state.scanner.status(library.id);
        if let Some(message) = library.scan.error.as_mut() {
            *message = i18n::error_message(message, i18n::wants_english(&headers)).0;
        }
    }
    Ok(Json(json!({"libraries":libraries})))
}

#[derive(Deserialize)]
struct NewLibrary {
    name: String,
    path: String,
}

async fn add_library(
    State(state): State<AppState>,
    body: Result<Json<NewLibrary>, JsonRejection>,
) -> Result<(StatusCode, Json<crate::models::Library>), ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad(e.body_text()))?;
    let name = body.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad("目录名称需要 1–100 个字符"));
    }
    if body.path.len() > 4096 {
        return Err(ApiError::bad("目录路径过长"));
    }
    let scanner = state.scanner.clone();
    let mut library = blocking(move || {
        let path = FsPath::new(body.path.trim());
        anyhow::ensure!(path.is_absolute(), "请填写此服务所在机器的绝对目录路径");
        let canonical = path
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("目录不存在或无权限：{e}"))?;
        anyhow::ensure!(canonical.is_dir(), "路径必须是文件夹");
        anyhow::ensure!(
            !canonical.starts_with(&state.data_dir),
            "不能把 Picsoc 数据目录作为素材目录"
        );
        std::fs::read_dir(&canonical).map_err(|e| anyhow::anyhow!("目录不可读取：{e}"))?;
        let path = canonical
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("目录路径必须能转换为 Unicode"))?;
        if state.db.libraries()?.iter().any(|l| l.path == path) {
            anyhow::bail!("该目录已经添加");
        }
        state.db.add_library(&name, path)
    })
    .await
    .map_err(|e| {
        if e.0 == StatusCode::INTERNAL_SERVER_ERROR {
            ApiError::bad(e.1)
        } else {
            e
        }
    })?;
    scanner.start(library.id);
    library.scan = scanner.status(library.id);
    Ok((StatusCode::CREATED, Json(library)))
}

async fn delete_library(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let db = state.db.clone();
    if !blocking(move || db.delete_library(id)).await? {
        return Err(ApiError::missing());
    }
    state.scanner.forget(id);
    let cache = state.scanner.cache_root().join(id.to_string());
    blocking(move || {
        if cache.exists() {
            std::fs::remove_dir_all(cache)?;
        }
        Ok(())
    })
    .await?;
    Ok(Json(json!({"ok":true})))
}

async fn scan(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let db = state.db.clone();
    blocking(move || {
        db.library(id)?;
        db.retry_failed_thumbnails(id)
    })
    .await?;
    state.scanner.start(id);
    Ok(Json(json!({"ok":true})))
}
async fn cancel_scan(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let db = state.db.clone();
    blocking(move || db.library(id)).await?;
    state.scanner.cancel(id);
    Ok(Json(json!({"ok":true})))
}

async fn assets(
    State(state): State<AppState>,
    query: Result<Query<AssetQuery>, QueryRejection>,
) -> Result<Json<crate::models::AssetList>, ApiError> {
    let Query(mut query) = query.map_err(|e| ApiError::bad(e.body_text()))?;
    if let Some(folder) = query.folder.as_mut() {
        if query.library_id.is_none() {
            return Err(ApiError::bad("按子目录筛选时必须指定素材库"));
        }
        *folder = folders::normalize_folder(folder).map_err(|e| ApiError::bad(e.to_string()))?;
    }
    if query.q.as_ref().is_some_and(|q| q.len() > 1024)
        || query.tag.as_ref().is_some_and(|q| q.len() > 200)
    {
        return Err(ApiError::bad("搜索内容过长"));
    }
    if query
        .sort
        .as_deref()
        .is_some_and(|s| !["name", "size", "modified"].contains(&s))
    {
        return Err(ApiError::bad("排序方式无效"));
    }
    if query
        .format
        .as_deref()
        .is_some_and(|s| !["jpg", "png", "gif", "webp", "bmp", "tiff"].contains(&s))
    {
        return Err(ApiError::bad("图片格式无效"));
    }
    Ok(Json(blocking(move || state.db.assets(&query)).await?))
}

#[derive(Deserialize)]
struct FolderQuery {
    parent: Option<String>,
}

async fn library_folders(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    query: Result<Query<FolderQuery>, QueryRejection>,
) -> Result<Json<crate::models::FolderList>, ApiError> {
    let Query(query) = query.map_err(|e| ApiError::bad(e.body_text()))?;
    let parent = folders::normalize_folder(query.parent.as_deref().unwrap_or_default())
        .map_err(|e| ApiError::bad(e.to_string()))?;
    Ok(Json(blocking(move || state.db.folders(id, &parent)).await?))
}

fn normalize_batch(mut batch: AssetBatch) -> Result<AssetBatch, ApiError> {
    batch.ids.sort_unstable();
    batch.ids.dedup();
    if batch.ids.is_empty() || batch.ids.len() > 500 || batch.ids.iter().any(|id| *id <= 0) {
        return Err(ApiError::bad("批量操作需要 1–500 个有效素材 ID"));
    }
    for tags in [&mut batch.add_tags, &mut batch.remove_tags]
        .into_iter()
        .flatten()
    {
        let mut unique = BTreeSet::new();
        for tag in tags.iter() {
            let tag = tag.trim();
            if tag.chars().count() > 50 || tag.contains(['\n', '\r', '\0']) {
                return Err(ApiError::bad("每个标签最多 50 个字符且不能包含换行"));
            }
            if !tag.is_empty() {
                unique.insert(tag.to_owned());
            }
        }
        *tags = unique.into_iter().collect();
    }
    if batch.favorite.is_none()
        && batch.add_tags.as_ref().is_none_or(|tags| tags.is_empty())
        && batch
            .remove_tags
            .as_ref()
            .is_none_or(|tags| tags.is_empty())
    {
        return Err(ApiError::bad("批量操作至少需要一项变更"));
    }
    Ok(batch)
}

async fn batch_assets(
    State(state): State<AppState>,
    body: Result<Json<AssetBatch>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad(e.body_text()))?;
    let batch = normalize_batch(body)?;
    let updated = blocking(move || state.db.batch_assets(&batch))
        .await
        .map_err(|e| {
            if e.1 == "批量操作后每张图片最多 50 个标签" {
                ApiError::bad(e.1)
            } else {
                e
            }
        })?;
    Ok(Json(json!({"updated":updated})))
}
async fn asset(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<crate::models::Asset>, ApiError> {
    Ok(Json(blocking(move || state.db.asset(id)).await?))
}
async fn patch_asset(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    body: Result<Json<AssetPatch>, JsonRejection>,
) -> Result<Json<crate::models::Asset>, ApiError> {
    let Json(patch) = body.map_err(|e| ApiError::bad(e.body_text()))?;
    if patch.tags.as_ref().is_some_and(|tags| {
        tags.len() > 50
            || tags
                .iter()
                .any(|t| t.chars().count() > 50 || t.contains(['\n', '\r', '\0']))
    }) {
        return Err(ApiError::bad(
            "每张图片最多 50 个标签，每个标签最多 50 个字符",
        ));
    }
    Ok(Json(
        blocking(move || state.db.patch_asset(id, &patch)).await?,
    ))
}
async fn tags(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(json!({"tags":blocking(move||state.db.tags()).await?})))
}

async fn thumbnail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let db = state.db.clone();
    let asset = blocking(move || db.asset(id)).await?;
    let target = media::cache_path(state.scanner.cache_root(), &asset);
    if !target.is_file() && asset.thumbnail_error.is_none() {
        let (done, wait) = oneshot::channel();
        state
            .scanner
            .thumbnails
            .send(ThumbnailJob {
                asset: asset.clone(),
                complete: Some(done),
            })
            .await
            .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "缩略图服务暂不可用".into()))?;
        let _ = wait.await;
    }
    if target.is_file() {
        return stream_file(
            target,
            "image/png",
            &format!("\"{}\"", asset.cache_key()),
            &headers,
            true,
        )
        .await;
    }
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        media::placeholder(),
    )
        .into_response())
}

async fn original(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let db = state.db.clone();
    let asset = blocking(move || db.asset(id)).await?;
    let copy = asset.clone();
    let path =
        blocking(move || media::secure_path(FsPath::new(&copy.library_path), &copy.relative_path))
            .await
            .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.1))?;
    let mime = match asset.format.as_str() {
        "jpg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tiff" => "image/tiff",
        _ => "application/octet-stream",
    };
    stream_file(
        path,
        mime,
        &format!("\"{}\"", asset.cache_key()),
        &headers,
        false,
    )
    .await
}

async fn stream_file(
    path: PathBuf,
    mime: &str,
    etag: &str,
    headers: &HeaderMap,
    thumbnail: bool,
) -> Result<Response, ApiError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    let size = file
        .metadata()
        .await
        .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?
        .len();
    let cache = if thumbnail {
        "private, max-age=86400"
    } else {
        "private, max-age=0, must-revalidate"
    };
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag)
    {
        return Ok(Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::ETAG, etag)
            .header(header::CACHE_CONTROL, cache)
            .body(Body::empty())
            .unwrap());
    }
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .filter(|_| {
            headers
                .get(header::IF_RANGE)
                .is_none_or(|value| value.to_str().ok() == Some(etag))
        });
    let (start, length, status) = if let Some(range) = range {
        let Some((start, end)) = parse_range(range, size) else {
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{size}"))],
                Json(json!({"error":"请求的字节范围无效"})),
            )
                .into_response());
        };
        (start, end - start + 1, StatusCode::PARTIAL_CONTENT)
    } else {
        (0, size, StatusCode::OK)
    };
    if start > 0 {
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, length)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, cache);
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{}/{size}", start + length - 1),
        );
    }
    Ok(builder
        .body(Body::from_stream(ReaderStream::with_capacity(
            file.take(length),
            64 * 1024,
        )))
        .unwrap())
}

fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let range = value.strip_prefix("bytes=")?;
    if range.contains(',') {
        return None;
    }
    let (start, end) = range.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        Some((size.saturating_sub(suffix), size - 1))
    } else {
        let start = start.parse::<u64>().ok()?;
        let end = if end.is_empty() {
            size - 1
        } else {
            end.parse::<u64>().ok()?.min(size - 1)
        };
        (start <= end && start < size).then_some((start, end))
    }
}

async fn frontend(request: Request) -> Response {
    let path = request.uri().path().trim_start_matches('/');
    if path.starts_with("api/") || path == "api" {
        return ApiError(StatusCode::NOT_FOUND, "接口不存在".into()).into_response();
    }
    if !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    ) {
        return ApiError(StatusCode::METHOD_NOT_ALLOWED, "请求方法无效".into()).into_response();
    }
    let candidate = if path.is_empty() { "index.html" } else { path };
    let (file, mime) = if let Some(file) = Frontend::get(candidate) {
        (
            file,
            mime_guess::from_path(candidate)
                .first_or_octet_stream()
                .to_string(),
        )
    } else if !path.contains('.') {
        let Some(file) = Frontend::get("index.html") else {
            return ApiError(StatusCode::SERVICE_UNAVAILABLE, "前端尚未构建".into())
                .into_response();
        };
        (file, "text/html".into())
    } else {
        return ApiError(StatusCode::NOT_FOUND, "文件不存在".into()).into_response();
    };
    let size = file.data.len();
    let mut response = Response::new(if request.method() == axum::http::Method::HEAD {
        Body::empty()
    } else {
        Body::from(file.data.into_owned())
    });
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_str(&mime).unwrap());
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&size.to_string()).unwrap(),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if candidate.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_supports_explicit_open_and_suffix() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=0-200", 100), Some((0, 99)));
        for input in [
            "bytes=100-",
            "bytes=-0",
            "bytes=9-1",
            "bytes=0-1,5-6",
            "foo",
        ] {
            assert_eq!(parse_range(input, 100), None);
        }
    }
    #[test]
    fn authentication_checks_full_value() {
        assert!(constant_time_equal(b"picsoc:password", b"picsoc:password"));
        assert!(!constant_time_equal(b"picsoc:pass", b"picsoc:password"));
        assert!(!constant_time_equal(
            b"picsoc:passwordextra",
            b"picsoc:password"
        ));
    }

    #[test]
    fn directory_picker_only_lists_visible_folders() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["中文素材", "alpha", ".hidden"] {
            std::fs::create_dir(temp.path().join(name)).unwrap();
        }
        std::fs::write(temp.path().join("image.png"), b"image").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(temp.path().join("alpha"), temp.path().join("linked")).unwrap();
        let list = list_directories(Some(temp.path().to_str().unwrap().to_owned())).unwrap();
        assert_eq!(
            list.directories
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "中文素材"]
        );
        assert_eq!(
            list.path,
            Some(
                temp.path()
                    .canonicalize()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned()
            )
        );
        assert!(list.parent.is_some());
        assert!(!list.truncated);
        assert!(list_directories(Some("relative/folder".into())).is_err());
        assert!(!list_directories(None).unwrap().roots.is_empty());
    }

    #[tokio::test]
    async fn server_rejects_rebinding_cross_origin_and_wrong_password() {
        use tower::ServiceExt;
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().canonicalize().unwrap();
        let db = Db::open(&data.join("test.sqlite")).unwrap();
        let scanner = Scanner::new(db.clone(), data.clone(), 1);
        let app = router(AppState {
            db,
            scanner,
            data_dir: data,
            bind: "127.0.0.1:3210".parse().unwrap(),
            password: Some(Arc::from("测试密码")),
        });
        for (host, origin, expected) in [
            ("evil.example:3210", None, StatusCode::FORBIDDEN),
            (
                "127.0.0.1:3210",
                Some("http://evil.example"),
                StatusCode::FORBIDDEN,
            ),
            ("127.0.0.1:3210", None, StatusCode::UNAUTHORIZED),
        ] {
            let mut builder = Request::builder()
                .uri("/api/health")
                .header(header::HOST, host);
            if let Some(origin) = origin {
                builder = builder.header(header::ORIGIN, origin);
            }
            let response = app
                .clone()
                .oneshot(builder.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
        let token = base64::engine::general_purpose::STANDARD.encode("picsoc:测试密码");
        let request = Request::builder()
            .uri("/api/health")
            .header(header::HOST, "127.0.0.1:3210")
            .header(header::AUTHORIZATION, format!("Basic {token}"))
            .body(Body::empty())
            .unwrap();
        assert_eq!(app.oneshot(request).await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn api_localizes_errors_and_directory_labels_per_request() {
        use tower::ServiceExt;
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().canonicalize().unwrap();
        let db = Db::open(&data.join("test.sqlite")).unwrap();
        let scanner = Scanner::new(db.clone(), data.clone(), 1);
        let state = AppState {
            db,
            scanner,
            data_dir: data,
            bind: "127.0.0.1:3210".parse().unwrap(),
            password: None,
        };
        let app = router(state.clone());
        for (language, expected) in [
            ("en-US", "Please select an absolute directory path."),
            ("zh-CN", "请选择绝对目录路径"),
        ] {
            let request = Request::builder()
                .uri("/api/directories?path=relative")
                .header(header::HOST, "127.0.0.1:3210")
                .header(header::ACCEPT_LANGUAGE, language)
                .body(Body::empty())
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let value: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 8192)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(value["error"], expected);
            assert_eq!(value["code"], "absolute_path_required");
        }
        let request = Request::builder()
            .uri("/api/directories")
            .header(header::HOST, "127.0.0.1:3210")
            .header(header::ACCEPT_LANGUAGE, "en")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 16384)
                .await
                .unwrap(),
        )
        .unwrap();
        for root in value["roots"].as_array().unwrap() {
            let name = root["name"].as_str().unwrap();
            assert!(name == "Home" || name == "File system /" || name.starts_with("Drive "));
        }
        let mut state = state;
        state.password = Some(Arc::from("secret-not-in-response"));
        let request = Request::builder()
            .uri("/api/health")
            .header(header::HOST, "127.0.0.1:3210")
            .header(header::ACCEPT_LANGUAGE, "en")
            .body(Body::empty())
            .unwrap();
        let response = router(state).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            response
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("Basic")
        );
        assert_eq!(
            response.headers().get(header::CONTENT_LANGUAGE).unwrap(),
            "en"
        );
        let value: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 8192)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["code"], "authentication_required");
        assert_eq!(
            value["error"],
            "Sign in with username picsoc and your configured password."
        );
        assert!(!value.to_string().contains("secret-not-in-response"));
    }

    #[test]
    fn batches_validate_and_normalize_ids_and_tags() {
        let batch = normalize_batch(AssetBatch {
            ids: vec![2, 1, 2],
            favorite: None,
            add_tags: Some(vec![" 标签 ".into(), "标签".into(), "".into()]),
            remove_tags: None,
        })
        .unwrap();
        assert_eq!(batch.ids, vec![1, 2]);
        assert_eq!(batch.add_tags.unwrap(), vec!["标签"]);
        assert!(
            normalize_batch(AssetBatch {
                ids: (1..=501).collect(),
                favorite: Some(true),
                add_tags: None,
                remove_tags: None
            })
            .is_err()
        );
        assert!(
            normalize_batch(AssetBatch {
                ids: vec![1],
                favorite: None,
                add_tags: Some(vec![" ".into()]),
                remove_tags: None
            })
            .is_err()
        );
        assert!(
            normalize_batch(AssetBatch {
                ids: vec![1],
                favorite: None,
                add_tags: Some(vec!["x".repeat(51)]),
                remove_tags: None
            })
            .is_err()
        );
        assert!(
            normalize_batch(AssetBatch {
                ids: vec![1],
                favorite: Some(false),
                add_tags: None,
                remove_tags: None
            })
            .is_ok()
        );
    }

    #[tokio::test]
    async fn batch_and_folder_errors_use_stable_localized_codes() {
        use tower::ServiceExt;
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().canonicalize().unwrap();
        let db = Db::open(&data.join("test.sqlite")).unwrap();
        let scanner = Scanner::new(db.clone(), data.clone(), 1);
        let app = router(AppState {
            db,
            scanner,
            data_dir: data,
            bind: "127.0.0.1:3210".parse().unwrap(),
            password: None,
        });
        for (url, body, code, message) in [
            (
                "/api/assets?folder=abc",
                None,
                "folder_library_required",
                "Select a library before filtering by folder.",
            ),
            (
                "/api/assets?library_id=1&folder=..",
                None,
                "invalid_folder_path",
                "The relative library folder path is invalid.",
            ),
            (
                "/api/libraries/1/folders?parent=%2Fabsolute",
                None,
                "invalid_folder_path",
                "The relative library folder path is invalid.",
            ),
            (
                "/api/assets/batch",
                Some(json!({"ids":[],"favorite":true})),
                "invalid_batch_ids",
                "Batch operations require 1–500 valid asset IDs.",
            ),
            (
                "/api/assets/batch",
                Some(json!({"ids":[1]})),
                "empty_batch_change",
                "A batch operation must include at least one change.",
            ),
        ] {
            let mut builder = Request::builder()
                .uri(url)
                .header(header::HOST, "127.0.0.1:3210")
                .header(header::ACCEPT_LANGUAGE, "en");
            let body = if let Some(body) = body {
                builder = builder
                    .method("POST")
                    .header(header::CONTENT_TYPE, "application/json");
                Body::from(body.to_string())
            } else {
                Body::empty()
            };
            let response = app
                .clone()
                .oneshot(builder.body(body).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let value: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 8192)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(value["code"], code);
            assert_eq!(value["error"], message);
        }
        let request = Request::builder()
            .uri("/api/assets?folder=abc")
            .header(header::HOST, "127.0.0.1:3210")
            .header(header::ACCEPT_LANGUAGE, "zh-CN")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 8192)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["error"], "按子目录筛选时必须指定素材库");
        assert_eq!(value["code"], "folder_library_required");
    }
}
