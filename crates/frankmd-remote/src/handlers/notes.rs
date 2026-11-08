//! Notes, folders, Hugo posts, previews, backlinks and the tree.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path as FsPath, PathBuf};

use frankmd_core::hugo::{self, HugoPathStyle};
use frankmd_core::path::normalize_note_path;
use frankmd_fs::NotesService;
use frankmd_markdown::{RenderOptions, html, parse};

use super::{AppState, FolderBody, NoteBody, PathQuery, RenameBody, folder_path, note_path};
use crate::error::{ApiError, fs_error};

/// `GET /api/tree`
pub async fn tree(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let tree = state.notes().list_tree().map_err(|e| fs_error(&e))?;
    Ok(Json(json!({ "tree": tree })))
}

/// `GET /api/note?path=...`
pub async fn get_note(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let raw = query
        .path
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("path_required", "path is required"))?;
    let path = note_path(raw)?;
    let file = state.notes().read(&path).map_err(|e| fs_error(&e))?;
    let (content, valid_utf8) = match String::from_utf8(file.bytes) {
        Ok(text) => (text, true),
        Err(err) => (String::from_utf8_lossy(err.as_bytes()).into_owned(), false),
    };
    Ok(Json(json!({
        "path": path,
        "content": content,
        "revision": file.revision.0,
        "mtime": file.mtime,
        "valid_utf8": valid_utf8,
        "read_only": !valid_utf8,
        "encrypted": file.encrypted,
        "exists": true
    })))
}

/// `POST /api/note` (`{path, content?}`), no-clobber.
pub async fn create_note(
    State(state): State<AppState>,
    Json(body): Json<NoteBody>,
) -> Result<Response, ApiError> {
    let path = note_path(&body.path)?;
    let content = body.content.unwrap_or_default();
    let revision = state
        .notes()
        .create(&path, content.as_bytes())
        .map_err(|e| fs_error(&e))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "path": path, "revision": revision.0 })),
    )
        .into_response())
}

/// `PATCH /api/note` (`{path, content, expected_revision?}`), never creates.
pub async fn update_note(
    State(state): State<AppState>,
    Json(body): Json<NoteBody>,
) -> Result<Json<Value>, ApiError> {
    let path = note_path(&body.path)?;
    let content = body
        .content
        .ok_or_else(|| ApiError::bad_request("content_required", "content is required"))?;
    let service = state.notes();
    if let Some(expected) = body.expected_revision.as_deref() {
        let current = service.read(&path).map_err(|e| fs_error(&e))?;
        if current.revision.0 != expected {
            return Err(ApiError::conflict(
                "revision_conflict",
                format!("note changed since it was loaded: {path}"),
            ));
        }
    }
    let revision = service
        .update(&path, content.as_bytes())
        .map_err(|e| fs_error(&e))?;
    Ok(Json(json!({ "path": path, "revision": revision.0 })))
}

/// `DELETE /api/note?path=...&expected_revision=...` (moves to the trash).
pub async fn delete_note(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
) -> Result<StatusCode, ApiError> {
    let raw = query
        .path
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("path_required", "path is required"))?;
    let path = note_path(raw)?;
    state
        .settings
        .with_store(|store| {
            NotesService::new(&state.vault)
                .with_recents(&*store)
                .trash(&path, query.expected_revision.as_deref())
        })
        .map_err(|e| fs_error(&e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/note/rename` (`{path, new_path}`).
pub async fn rename_note(
    State(state): State<AppState>,
    Json(body): Json<RenameBody>,
) -> Result<Json<Value>, ApiError> {
    let from = note_path(&body.path)?;
    let to = note_path(&body.new_path)?;
    state
        .settings
        .with_store(|store| {
            NotesService::new(&state.vault)
                .with_recents(&*store)
                .rename(&from, &to)
        })
        .map_err(|e| fs_error(&e))?;
    Ok(Json(json!({ "path": to })))
}

/// `POST /api/folder` (`{path}`).
pub async fn create_folder(
    State(state): State<AppState>,
    Json(body): Json<FolderBody>,
) -> Result<StatusCode, ApiError> {
    let path = folder_path(&body.path)?;
    state
        .notes()
        .create_folder(&path)
        .map_err(|e| fs_error(&e))?;
    Ok(StatusCode::CREATED)
}

/// `DELETE /api/folder?path=...` (moves the folder to the trash).
pub async fn delete_folder(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
) -> Result<StatusCode, ApiError> {
    let raw = query
        .path
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("path_required", "path is required"))?;
    let path = folder_path(raw)?;
    state.notes().trash(&path, None).map_err(|e| fs_error(&e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/folder/rename` (`{path, new_path}`), rewrites the Hugo slug.
pub async fn rename_folder(
    State(state): State<AppState>,
    Json(body): Json<RenameBody>,
) -> Result<Json<Value>, ApiError> {
    let from = folder_path(&body.path)?;
    let to = folder_path(&body.new_path)?;
    state
        .notes()
        .rename_folder(&from, &to)
        .map_err(|e| fs_error(&e))?;
    Ok(Json(json!({ "path": to })))
}

/// Hugo post creation body.
#[derive(Debug, Deserialize)]
pub struct HugoBody {
    /// Parent folder (`""`/null = vault root).
    #[serde(default)]
    pub parent: Option<String>,
    /// Title.
    pub title: String,
    /// Optional explicit slug (slugified; blank → from the title).
    #[serde(default)]
    pub slug: Option<String>,
}

/// `POST /api/hugo` (`{parent?, title, slug?}`).
pub async fn create_hugo(
    State(state): State<AppState>,
    Json(body): Json<HugoBody>,
) -> Result<Response, ApiError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("title_required", "title is required"));
    }
    let slug = match body.slug.as_deref().map(str::trim) {
        Some(slug) if !slug.is_empty() => frankmd_core::slug::slugify(slug),
        _ => frankmd_core::slug::slugify(title),
    };
    if slug.is_empty() {
        return Err(ApiError::bad_request(
            "empty_slug",
            "the title or slug produces an empty path",
        ));
    }
    let parent = body
        .parent
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty());
    if let Some(parent) = parent {
        folder_path(parent)?;
    }
    let resolved = state.resolved();
    let style = HugoPathStyle::from_config(
        resolved
            .value(frankmd_config::Key::HugoPathStyle)
            .as_ref()
            .and_then(frankmd_config::Value::as_str),
    );
    let now = jiff::Zoned::now();
    let relative = match style {
        HugoPathStyle::Flat => format!("{slug}.md"),
        HugoPathStyle::Dated => format!("{}/{slug}/index.md", hugo::date_path(&now)),
    };
    let path = match parent {
        Some(parent) => format!("{parent}/{relative}"),
        None => relative,
    };
    let service = state.notes();
    let template = service
        .hugo_template()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let content = hugo::render_template(
        template.as_deref().unwrap_or(hugo::DEFAULT_TEMPLATE),
        title,
        &slug,
        &hugo::format_date(&now),
    );
    let revision = service
        .create(&path, content.as_bytes())
        .map_err(|e| fs_error(&e))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "path": path, "revision": revision.0 })),
    )
        .into_response())
}

/// Preview body (`{content}` or `{path}`).
#[derive(Debug, Deserialize)]
pub struct PreviewBody {
    /// Markdown source (wins over `path`).
    #[serde(default)]
    pub content: Option<String>,
    /// Note to read when `content` is absent.
    #[serde(default)]
    pub path: Option<String>,
}

/// `POST /api/preview` → sanitized HTML.
pub async fn preview(
    State(state): State<AppState>,
    Json(body): Json<PreviewBody>,
) -> Result<Json<Value>, ApiError> {
    let source = match body.content {
        Some(content) => content,
        None => {
            let raw = body.path.as_deref().ok_or_else(|| {
                ApiError::bad_request("content_required", "content or path is required")
            })?;
            let path = note_path(raw)?;
            let file = state.notes().read(&path).map_err(|e| fs_error(&e))?;
            String::from_utf8_lossy(&file.bytes).into_owned()
        }
    };
    let doc = parse::render(&source, &RenderOptions::default());
    Ok(Json(json!({ "html": html::to_html(&doc) })))
}

/// `GET /api/backlinks?path=...`
pub async fn backlinks(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>, ApiError> {
    let raw = query
        .path
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("path_required", "path is required"))?;
    let path = normalize_note_path(raw.trim().trim_matches('/'));
    let hits = state
        .notes()
        .backlinks(&path, 3)
        .map_err(|e| fs_error(&e))?;
    Ok(Json(json!({ "backlinks": hits, "count": hits.len() })))
}

/// `GET /api/asset/{*path}`: raw file inside the vault (images, attachments).
pub async fn get_asset(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Response, ApiError> {
    let contained = state
        .vault
        .safe_path(raw.trim_matches('/'), true)
        .map_err(|e| fs_error(&e))?;
    let bytes = super::read_maybe_encrypted(&state, contained.abs())?;
    let mime = mime_guess::from_path(contained.abs())
        .first_or_octet_stream()
        .to_string();
    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, mime)],
        bytes,
    )
        .into_response())
}

/// `POST /api/note/encryption` (`{path, encrypted}`).
#[derive(Debug, Deserialize)]
pub struct NoteEncryptionBody {
    /// Note path.
    pub path: String,
    /// `true` to encrypt, `false` to decrypt.
    pub encrypted: bool,
}

/// `{path, encrypted, attachments}`.
pub async fn set_note_encryption(
    State(state): State<AppState>,
    Json(body): Json<NoteEncryptionBody>,
) -> Result<Json<Value>, ApiError> {
    if !state.encryption.is_unlocked() {
        return Err(ApiError::new(
            StatusCode::LOCKED,
            "locked",
            "Unlock encryption before changing note encryption.",
        ));
    }
    let path = note_path(&body.path)?;
    let service = state.notes();
    let note = service.read(&path).map_err(|e| fs_error(&e))?;
    let source = String::from_utf8_lossy(&note.bytes).into_owned();
    let attachments = local_attachments(&path, &source, state.vault.root());

    if body.encrypted {
        let mut changed = Vec::new();
        for attachment in &attachments {
            match encrypt_media_file(&state.encryption, attachment) {
                Ok(true) => changed.push(attachment.clone()),
                Ok(false) => {}
                Err(err) => {
                    rollback_media(&state.encryption, &changed);
                    return Err(ApiError::internal(err));
                }
            }
        }
        if !note.encrypted
            && let Err(err) = service.set_encrypted(&path, true)
        {
            rollback_media(&state.encryption, &changed);
            return Err(fs_error(&err));
        }
    } else {
        if note.encrypted
            && let Err(err) = service.set_encrypted(&path, false)
        {
            return Err(fs_error(&err));
        }
        let shared = match encrypted_note_attachments(&state, &service, &path) {
            Ok(shared) => shared,
            Err(err) => {
                tracing::warn!(%err, "could not verify encrypted notes; keeping attachments encrypted");
                return Ok(Json(json!({
                    "path": path,
                    "encrypted": false,
                    "attachments": attachments.len(),
                    "warning": "attachments kept encrypted: could not verify other encrypted notes"
                })));
            }
        };
        for attachment in &attachments {
            if !frankmd_crypto::is_media_encrypted(&std::fs::read(attachment).unwrap_or_default())
                || shared.contains(attachment)
            {
                continue;
            }
            let _ = decrypt_media_file(&state.encryption, attachment);
        }
    }

    let refreshed = service.read(&path).map_err(|e| fs_error(&e))?;
    Ok(Json(json!({
        "path": path,
        "encrypted": refreshed.encrypted,
        "attachments": attachments.len()
    })))
}

fn encrypt_media_file(session: &frankmd_crypto::Session, path: &FsPath) -> Result<bool, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if frankmd_crypto::is_media_encrypted(&bytes) {
        return Ok(false);
    }
    let encrypted = session.encrypt_media(&bytes).map_err(|e| e.to_string())?;
    frankmd_fs::atomic::atomic_replace(path, &encrypted, frankmd_fs::atomic::ModePolicy::Preserve)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

fn decrypt_media_file(session: &frankmd_crypto::Session, path: &FsPath) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if !frankmd_crypto::is_media_encrypted(&bytes) {
        return Ok(());
    }
    let plain = session.decrypt_media(&bytes).map_err(|e| e.to_string())?;
    frankmd_fs::atomic::atomic_replace(path, &plain, frankmd_fs::atomic::ModePolicy::Preserve)
        .map_err(|e| e.to_string())
}

fn rollback_media(session: &frankmd_crypto::Session, paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        if let Err(err) = decrypt_media_file(session, path) {
            tracing::warn!(path = %path.display(), %err, "could not roll back attachment encryption");
        }
    }
}

fn local_attachments(note: &str, source: &str, root: &FsPath) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for href in frankmd_media::usage::references_in(source) {
        let Some(path) = resolve_attachment(note, &href, root) else {
            continue;
        };
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

fn resolve_attachment(note: &str, href: &str, root: &FsPath) -> Option<PathBuf> {
    let href = href.trim();
    if href.is_empty()
        || href.starts_with("//")
        || href
            .split_once(':')
            .is_some_and(|(scheme, _)| !scheme.is_empty())
    {
        return None;
    }
    let path = href.split(['#', '?']).next().unwrap_or_default();
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .replace("&amp;", "&")
        .replace("&quot;", "\"");
    let mut parts: Vec<&str> = if decoded.starts_with('/') {
        Vec::new()
    } else {
        note.rsplit_once('/')
            .map(|(dir, _)| dir.split('/').collect())
            .unwrap_or_default()
    };
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(segment),
        }
    }
    let rel = parts.join("/");
    if rel.is_empty() || rel.ends_with(".md") || rel.ends_with(".fed") {
        return None;
    }
    let full = root.join(&rel);
    if !full.starts_with(root) {
        return None;
    }
    let meta = std::fs::symlink_metadata(&full).ok()?;
    (meta.is_file() && !meta.file_type().is_symlink()).then_some(full)
}

fn encrypted_note_attachments(
    state: &crate::state::ServerState,
    service: &NotesService<'_>,
    current: &str,
) -> Result<std::collections::HashSet<PathBuf>, String> {
    let mut out = std::collections::HashSet::new();
    let tree = state.vault.list_tree().map_err(|e| e.to_string())?;
    fn walk(
        nodes: &[frankmd_core::tree::TreeNode],
        current: &str,
        state: &crate::state::ServerState,
        service: &NotesService<'_>,
        out: &mut std::collections::HashSet<PathBuf>,
    ) -> Result<(), String> {
        for node in nodes {
            match node {
                frankmd_core::tree::TreeNode::Folder { children, .. } => {
                    walk(children, current, state, service, out)?;
                }
                frankmd_core::tree::TreeNode::File { path, .. } => {
                    let path = path.as_str();
                    if path == current {
                        continue;
                    }
                    let Ok((encrypted, _)) = state.vault.note_encryption(path) else {
                        continue;
                    };
                    if !encrypted {
                        continue;
                    }
                    let note = service.read(path).map_err(|e| e.to_string())?;
                    let source = String::from_utf8_lossy(&note.bytes);
                    for attachment in local_attachments(path, &source, state.vault.root()) {
                        out.insert(attachment);
                    }
                }
            }
        }
        Ok(())
    }
    walk(&tree, current, state, service, &mut out)?;
    Ok(out)
}
