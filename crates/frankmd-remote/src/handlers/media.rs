//! Media uploads, the media library, image search and YouTube.

use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use frankmd_media::api::{ApiError as MediaApiError, ApiStatus, Base64Upload};
use frankmd_media::import::ImportRequest;
use frankmd_media::library::{Library, LibraryError};
use frankmd_media::upload_storage::Upload;
use frankmd_media::{MediaKind, Param, youtube};

use super::AppState;
use crate::error::ApiError;

fn media_status(status: ApiStatus) -> StatusCode {
    match status {
        ApiStatus::BadRequest => StatusCode::BAD_REQUEST,
        ApiStatus::NotFound => StatusCode::NOT_FOUND,
        ApiStatus::Unprocessable => StatusCode::UNPROCESSABLE_ENTITY,
        ApiStatus::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn media_error(err: &MediaApiError) -> ApiError {
    ApiError::new(
        media_status(err.status),
        "media_error",
        err.message.as_str().to_owned(),
    )
}

fn library_error(err: &LibraryError) -> ApiError {
    match err {
        LibraryError::BadRequest(message) => {
            ApiError::bad_request("library_bad_request", message.clone())
        }
        LibraryError::NotFound => {
            ApiError::not_found("library.media_not_found", "Media file not found.")
        }
        LibraryError::DeleteFailed => {
            ApiError::unprocessable("library.delete_failed", "Could not delete this media file.")
        }
        LibraryError::RenameFailed => {
            ApiError::unprocessable("library.rename_failed", "Could not rename this media file.")
        }
        LibraryError::LoadFailed => {
            ApiError::unprocessable("library_load_failed", "Could not load the media library.")
        }
        LibraryError::UsageScanFailed => {
            ApiError::unavailable("library.usage_scan_failed", "Could not scan note usage.")
        }
        LibraryError::Media(media) => match media {
            frankmd_media::MediaError::NotConfigured(message) => {
                ApiError::unavailable("media_not_configured", message.clone())
            }
            other => ApiError::unprocessable("media_error", other.to_string()),
        },
    }
}

fn ruby_param(raw: Option<&str>) -> Param {
    raw.map_or(Param::Nil, |s| Param::Str(s.to_owned()))
}

fn limit_param(raw: Option<&str>) -> Param {
    raw.and_then(|s| s.trim().parse::<i64>().ok())
        .map_or(Param::Nil, Param::Int)
}

fn image_json(image: &frankmd_media::server_folder::ServerImage) -> Value {
    json!({
        "name": image.name,
        "path": image.path,
        "mtime": image.mtime,
        "size": image.size,
        "width": image.width,
        "height": image.height
    })
}

fn item_json(item: &frankmd_media::library::MediaItem) -> Value {
    json!({
        "name": item.name,
        "path": item.path,
        "size": item.size,
        "mtime": item.mtime,
        "preview_url": item.preview_url,
        "kind": match item.kind { MediaKind::Image => "image", MediaKind::Video => "video" },
        "s3_url": item.s3_url
    })
}

/// `GET /api/images/config`
pub async fn images_config(State(state): State<AppState>) -> Json<Value> {
    let info = state.media.api.images_config();
    Json(json!({
        "enabled": info.enabled,
        "s3_enabled": info.s3_enabled,
        "media_storage_provider": info.media_storage_provider,
        "media_storage_fallback": info.media_storage_fallback,
        "web_search_enabled": info.web_search_enabled,
        "google_enabled": info.google_enabled,
        "pinterest_enabled": info.pinterest_enabled,
        "image_extensions": info.image_extensions,
        "video_extensions": info.video_extensions
    }))
}

/// `?search=&limit=&offset=`
#[derive(Debug, Deserialize)]
pub struct ImagesQuery {
    /// Substring filter.
    pub search: Option<String>,
    /// Page size.
    pub limit: Option<String>,
    /// Offset.
    pub offset: Option<String>,
}

/// `GET /api/images`
pub async fn images(
    State(state): State<AppState>,
    Query(query): Query<ImagesQuery>,
) -> Result<Json<Value>, ApiError> {
    let index = state
        .media
        .api
        .index(
            query.search.as_deref(),
            &limit_param(query.limit.as_deref()),
            &limit_param(query.offset.as_deref()),
        )
        .map_err(|e| media_error(&e))?;
    Ok(Json(json!({
        "images": index.images.iter().map(image_json).collect::<Vec<_>>(),
        "total": index.total,
        "limit": index.limit,
        "offset": index.offset
    })))
}

/// `GET /api/images/preview/{*path}`
pub async fn image_preview(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Response, ApiError> {
    let path = state
        .media
        .api
        .preview(raw.trim_matches('/'))
        .map_err(|e| media_error(&e))?;
    let bytes = super::read_maybe_encrypted(&state, &path)?;
    let mime = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .to_string();
    Ok((StatusCode::OK, [(header::CONTENT_TYPE, mime)], bytes).into_response())
}

/// `DELETE /api/images/file/{*path}`
pub async fn image_delete(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .media
        .api
        .destroy(raw.trim_matches('/'))
        .map_err(|e| media_error(&e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Parsed multipart upload.
#[derive(Default)]
struct UploadForm {
    file: Option<Upload>,
    resize: Param,
    upload_to_s3: Param,
    s3_key: Option<String>,
    s3_prefix: Option<String>,
}

async fn read_upload(mut multipart: Multipart) -> Result<UploadForm, ApiError> {
    let mut form = UploadForm {
        resize: Param::Nil,
        upload_to_s3: Param::Nil,
        ..UploadForm::default()
    };
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request("invalid_multipart", e.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_owned();
        if name == "file" {
            let filename = field.file_name().unwrap_or("upload").to_owned();
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request("invalid_multipart", e.to_string()))?;
            form.file = Some(Upload::new(filename, bytes.to_vec()));
        } else {
            let value = field
                .text()
                .await
                .map_err(|e| ApiError::bad_request("invalid_multipart", e.to_string()))?;
            match name.as_str() {
                "resize" => form.resize = ruby_param(Some(&value)),
                "upload_to_s3" => form.upload_to_s3 = ruby_param(Some(&value)),
                "s3_key" => form.s3_key = Some(value),
                "s3_prefix" => form.s3_prefix = Some(value),
                _ => {}
            }
        }
    }
    Ok(form)
}

/// `POST /api/images/upload` (multipart `file`)
pub async fn image_upload(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let form = read_upload(multipart).await?;
    let url = state
        .media
        .api
        .upload(
            form.file,
            &form.resize,
            &form.upload_to_s3,
            form.s3_key.as_deref(),
        )
        .await
        .map_err(|e| media_error(&e))?;
    Ok(Json(json!({ "url": url })))
}

/// `POST /api/media/upload` (multipart `file`, videos)
pub async fn media_upload(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let form = read_upload(multipart).await?;
    let url = state
        .media
        .api
        .media_upload(form.file, &form.upload_to_s3, form.s3_prefix.as_deref())
        .await
        .map_err(|e| media_error(&e))?;
    Ok(Json(json!({ "url": url })))
}

/// `POST /api/images/upload_base64`
#[derive(Debug, Deserialize)]
pub struct Base64Body {
    /// Base64 data (or full data URI).
    pub data: String,
    /// MIME type.
    #[serde(default)]
    pub mime_type: String,
    /// Filename.
    #[serde(default)]
    pub filename: String,
    /// Legacy S3 flag.
    #[serde(default)]
    pub upload_to_s3: Option<bool>,
    /// S3 prefix.
    #[serde(default)]
    pub s3_prefix: Option<String>,
}

/// `POST /api/images/upload_base64`.
pub async fn image_upload_base64(
    State(state): State<AppState>,
    Json(body): Json<Base64Body>,
) -> Result<Json<Value>, ApiError> {
    let request = Base64Upload {
        data: body.data,
        mime_type: body.mime_type,
        filename: body.filename,
        upload_to_s3: body.upload_to_s3.map_or(Param::Nil, Param::Bool),
        s3_prefix: body.s3_prefix,
    };
    let url = state
        .media
        .api
        .upload_base64(&request)
        .await
        .map_err(|e| media_error(&e))?;
    Ok(Json(json!({ "url": url })))
}

/// `POST /api/images/import`
#[derive(Debug, Deserialize)]
pub struct ImportBody {
    /// External URL.
    #[serde(default)]
    pub url: Option<String>,
    /// Server image path.
    #[serde(default)]
    pub path: Option<String>,
    /// Resize ratio.
    #[serde(default)]
    pub resize: Option<f64>,
    /// S3 key.
    #[serde(default)]
    pub s3_key: Option<String>,
    /// S3 prefix.
    #[serde(default)]
    pub s3_prefix: Option<String>,
    /// Legacy S3 flag.
    #[serde(default)]
    pub upload_to_s3: Option<bool>,
}

/// `POST /api/images/import`.
pub async fn image_import(
    State(state): State<AppState>,
    Json(body): Json<ImportBody>,
) -> Result<Json<Value>, ApiError> {
    let request = ImportRequest {
        url: ruby_param(body.url.as_deref()),
        path: ruby_param(body.path.as_deref()),
        resize: body.resize,
        s3_key: body.s3_key,
        s3_prefix: body.s3_prefix,
        upload_to_s3: body.upload_to_s3.map_or(Param::Nil, Param::Bool),
    };
    let url = state
        .media
        .api
        .import(&request)
        .await
        .map_err(|e| media_error(&e))?;
    Ok(Json(json!({ "url": url })))
}

/// `POST /api/images/upload_to_s3`
#[derive(Debug, Deserialize)]
pub struct ToS3Body {
    /// Server image path.
    pub path: String,
    /// Resize ratio.
    #[serde(default)]
    pub resize: Option<f64>,
    /// Custom key.
    #[serde(default)]
    pub s3_key: Option<String>,
}

/// `POST /api/images/upload_to_s3`.
pub async fn image_to_s3(
    State(state): State<AppState>,
    Json(body): Json<ToS3Body>,
) -> Result<Json<Value>, ApiError> {
    let resize = body.resize.map_or(Param::Nil, Param::Float);
    let url = state
        .media
        .api
        .upload_to_s3(&body.path, &resize, body.s3_key.as_deref())
        .await
        .map_err(|e| media_error(&e))?;
    Ok(Json(json!({ "url": url })))
}

/// `POST /api/images/upload_external_to_s3`
#[derive(Debug, Deserialize)]
pub struct ExternalToS3Body {
    /// External URL.
    pub url: String,
    /// Resize ratio.
    #[serde(default)]
    pub resize: Option<f64>,
    /// S3 prefix.
    #[serde(default)]
    pub s3_prefix: Option<String>,
}

/// `POST /api/images/upload_external_to_s3`.
pub async fn image_external_to_s3(
    State(state): State<AppState>,
    Json(body): Json<ExternalToS3Body>,
) -> Result<Json<Value>, ApiError> {
    let resize = body.resize.map_or(Param::Nil, Param::Float);
    let url = state
        .media
        .api
        .upload_external_to_s3(&body.url, &resize, body.s3_prefix.as_deref())
        .await
        .map_err(|e| media_error(&e))?;
    Ok(Json(json!({ "url": url })))
}

/// `GET /api/library`
pub async fn library(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let listing = Library::new(&state.vault, &state.media.api.env)
        .with_recovery(&state.media.api.recovery)
        .list()
        .await
        .map_err(|e| library_error(&e))?;
    Ok(Json(json!({
        "images": listing.images.iter().map(item_json).collect::<Vec<_>>(),
        "videos": listing.videos.iter().map(item_json).collect::<Vec<_>>()
    })))
}

/// `DELETE /api/library/file/{*path}`
pub async fn library_delete(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<StatusCode, ApiError> {
    Library::new(&state.vault, &state.media.api.env)
        .with_recovery(&state.media.api.recovery)
        .delete(raw.trim_matches('/'))
        .await
        .map_err(|e| library_error(&e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /api/library/file/{*path}` (`{name}`)
#[derive(Debug, Deserialize)]
pub struct RenameItemBody {
    /// New basename.
    pub name: String,
}

/// `PATCH /api/library/file/{*path}` (`{name}`).
pub async fn library_rename(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    Json(body): Json<RenameItemBody>,
) -> Result<Json<Value>, ApiError> {
    let renamed = Library::new(&state.vault, &state.media.api.env)
        .with_recovery(&state.media.api.recovery)
        .rename(raw.trim_matches('/'), &body.name)
        .await
        .map_err(|e| library_error(&e))?;
    Ok(Json(json!({
        "name": renamed.name,
        "path": renamed.path,
        "url": renamed.url
    })))
}

/// `?path=`
#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    /// One media path.
    pub path: Option<String>,
}

/// `GET /api/library/usage`
pub async fn library_usage(
    State(state): State<AppState>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Value>, ApiError> {
    let usage = Library::new(&state.vault, &state.media.api.env)
        .with_recovery(&state.media.api.recovery)
        .usage(&state.media.usage, query.path.as_deref())
        .map_err(|e| library_error(&e))?;
    Ok(Json(match usage {
        frankmd_media::library::UsageResponse::Counts(counts) => {
            json!({ "usage_counts": counts })
        }
        frankmd_media::library::UsageResponse::Notes(notes) => {
            json!({ "usage_notes": notes })
        }
    }))
}

/// `GET /api/images/search_web?q=`
#[derive(Debug, Deserialize)]
pub struct WebSearchQuery {
    /// Query.
    pub q: String,
    /// Google `start`.
    pub start: Option<i64>,
}

fn search_results_json(results: &frankmd_media::search::SearchResults) -> Value {
    json!({
        "images": results.images.iter().map(|image| json!({
            "url": image.url,
            "thumbnail": image.thumbnail,
            "title": image.title,
            "source": image.source,
            "width": image.width,
            "height": image.height
        })).collect::<Vec<_>>(),
        "total": results.total,
        "next_start": results.next_start,
        "error": results.error,
        "note": results.note
    })
}

fn search_error(err: &frankmd_media::search::SearchError) -> ApiError {
    let status = match err {
        frankmd_media::search::SearchError::QueryRequired => StatusCode::BAD_REQUEST,
        frankmd_media::search::SearchError::NotConfigured(_) => StatusCode::SERVICE_UNAVAILABLE,
        frankmd_media::search::SearchError::Failed => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ApiError::new(status, err.i18n_key(), err.i18n_key())
}

/// `GET /api/images/search_web?q=`.
pub async fn search_web(
    State(state): State<AppState>,
    Query(query): Query<WebSearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let results = state
        .media
        .search
        .search_web(&query.q)
        .await
        .map_err(|e| search_error(&e))?;
    Ok(Json(search_results_json(&results)))
}

/// `GET /api/images/search_pinterest?q=`.
pub async fn search_pinterest(
    State(state): State<AppState>,
    Query(query): Query<WebSearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let results = state
        .media
        .search
        .search_pinterest(&query.q)
        .await
        .map_err(|e| search_error(&e))?;
    Ok(Json(search_results_json(&results)))
}

/// `GET /api/images/search_google?q=&start=`.
pub async fn search_google(
    State(state): State<AppState>,
    Query(query): Query<WebSearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let config = state.media_config();
    let results = state
        .media
        .search
        .search_google(&config, &query.q, query.start.unwrap_or(1))
        .await
        .map_err(|e| search_error(&e))?;
    Ok(Json(search_results_json(&results)))
}

/// `GET /api/youtube/config`
pub async fn youtube_config(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "enabled": youtube::enabled(&state.media_config()) }))
}

/// `GET /api/youtube/search?q=`
pub async fn youtube_search(
    State(state): State<AppState>,
    Query(query): Query<WebSearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let config = state.media_config();
    let results = state
        .media
        .youtube
        .search(&config, &query.q)
        .await
        .map_err(|e| search_error(&e))?;
    Ok(Json(json!({
        "videos": results.videos.iter().map(|video| json!({
            "id": video.id,
            "title": video.title,
            "channel": video.channel,
            "thumbnail": video.thumbnail
        })).collect::<Vec<_>>(),
        "error": results.error
    })))
}
