//! Raw file sync for the offline Android client.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::error::{ApiError, fs_error};

/// Query for a raw sync download.
#[derive(Debug, Deserialize)]
pub struct SyncGetQuery {
    /// Vault-relative path.
    pub path: String,
}

/// Query for a raw sync upload/delete.
#[derive(Debug, Deserialize)]
pub struct SyncWriteQuery {
    /// Vault-relative path.
    pub path: String,
    /// SHA-256 revision guard.
    pub expected_revision: Option<String>,
}

/// `GET /api/sync/manifest`
pub async fn manifest(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let entries = state.vault.sync_manifest().map_err(|e| fs_error(&e))?;
    let files: Vec<Value> = entries
        .into_iter()
        .map(|entry| {
            json!({
                "path": entry.path,
                "revision": entry.revision,
                "size": entry.size,
                "mtime": entry.mtime
            })
        })
        .collect();
    Ok(Json(json!({ "files": files })))
}

/// `GET /api/sync/file?path=...`
pub async fn get_file(
    State(state): State<AppState>,
    Query(query): Query<SyncGetQuery>,
) -> Result<Response, ApiError> {
    let bytes = state
        .vault
        .sync_read(&query.path)
        .map_err(|e| fs_error(&e))?;
    let mime = mime_guess::from_path(&query.path).first_or_octet_stream();
    Ok((
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response())
}

/// `PUT /api/sync/file?path=...&expected_revision=...`
pub async fn put_file(
    State(state): State<AppState>,
    Query(query): Query<SyncWriteQuery>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let revision = state
        .vault
        .sync_write(&query.path, &body, query.expected_revision.as_deref())
        .map_err(|e| fs_error(&e))?;
    Ok(Json(json!({"path": query.path, "revision": revision.0})))
}

/// `DELETE /api/sync/file?path=...&expected_revision=...`
pub async fn delete_file(
    State(state): State<AppState>,
    Query(query): Query<SyncWriteQuery>,
) -> Result<StatusCode, ApiError> {
    state
        .vault
        .sync_delete(&query.path, query.expected_revision.as_deref())
        .map_err(|e| fs_error(&e))?;
    Ok(StatusCode::NO_CONTENT)
}
