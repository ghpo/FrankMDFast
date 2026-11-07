//! HTTP handlers, grouped by area.

pub mod ai;
pub mod config;
pub mod events;
pub mod media;
pub mod notes;
pub mod search;

use std::sync::Arc;

use serde::Deserialize;

use crate::error::ApiError;
use crate::state::ServerState;

/// State type used by every handler.
pub type AppState = Arc<ServerState>;

/// A `{path, new_path}` body.
#[derive(Debug, Deserialize)]
pub struct RenameBody {
    /// Current vault-relative path.
    pub path: String,
    /// New vault-relative path.
    pub new_path: String,
}

/// A note body.
#[derive(Debug, Deserialize)]
pub struct NoteBody {
    /// Vault-relative path.
    pub path: String,
    /// Full text (`None` = empty on create, required on update).
    #[serde(default)]
    pub content: Option<String>,
    /// Optimistic concurrency token (`sha256` hex).
    #[serde(default)]
    pub expected_revision: Option<String>,
}

/// A single-folder body.
#[derive(Debug, Deserialize)]
pub struct FolderBody {
    /// Vault-relative folder path.
    pub path: String,
}

/// `?path=` style query.
#[derive(Debug, Deserialize)]
pub struct PathQuery {
    /// Vault-relative path.
    pub path: Option<String>,
    /// Optional revision guard.
    pub expected_revision: Option<String>,
}

/// Normalize a request note path and reject the config file.
pub fn note_path(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("path_required", "path is required"));
    }
    if frankmd_core::path::is_config_file(trimmed) {
        return Err(ApiError::forbidden(
            "reserved_path",
            "the .fed configuration file cannot be edited over the remote API",
        ));
    }
    Ok(frankmd_core::path::normalize_note_path(trimmed))
}

/// Normalize a folder path (no `.md` appended).
pub fn folder_path(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("path_required", "path is required"));
    }
    if frankmd_core::path::is_config_file(trimmed) {
        return Err(ApiError::forbidden(
            "reserved_path",
            "the .fed configuration file cannot be edited over the remote API",
        ));
    }
    Ok(trimmed.to_owned())
}
