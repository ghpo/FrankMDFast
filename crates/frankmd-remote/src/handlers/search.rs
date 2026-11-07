//! Content search and the quick-open finder.

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use frankmd_core::tree::flatten_files;
use frankmd_search::content::{self, Query as ContentQuery};
use frankmd_search::finder;

use super::AppState;
use crate::error::ApiError;

/// `?q=...`
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Pattern (Ruby regex, literal fallback).
    #[serde(default)]
    pub q: String,
    /// Max results (`1..=100`, default 20).
    pub limit: Option<usize>,
}

/// `GET /api/search?q=...`: full-text search across the vault.
pub async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let mut content_query = ContentQuery::ui(&query.q);
    if let Some(limit) = query.limit {
        content_query.max_results = limit.clamp(1, 100);
    }
    let results = content::search(state.root(), &content_query);
    let hits: Vec<Value> = results
        .hits
        .iter()
        .map(|hit| {
            json!({
                "path": hit.path,
                "name": hit.name,
                "mtime": hit.mtime.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                "line_number": hit.line_number,
                "match_text": hit.match_text,
                "context": hit.context.iter().map(|line| json!({
                    "line_number": line.line_number,
                    "content": line.content,
                    "is_match": line.is_match
                })).collect::<Vec<_>>()
            })
        })
        .collect();
    let skipped: Vec<Value> = results
        .skipped
        .iter()
        .map(|skipped| json!({ "path": skipped.path, "reason": format!("{:?}", skipped.reason) }))
        .collect();
    Ok(Json(json!({
        "query": query.q,
        "hits": hits,
        "count": hits.len(),
        "truncated": results.truncated,
        "skipped": skipped
    })))
}

/// `GET /api/finder?q=...`: fuzzy file finder with recents first.
pub async fn finder_route(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let tree = state
        .notes()
        .list_tree()
        .map_err(|e| crate::error::fs_error(&e))?;
    let candidates: Vec<String> = flatten_files(&tree)
        .into_iter()
        .map(|node| node.path().as_str().to_owned())
        .collect();
    let recent = state.settings.with_store(|store| {
        frankmd_fs::recent::RecentFilesService::new(&state.vault, &*store)
            .entries()
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>()
    });
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let ranked = finder::rank(&query.q, &candidates, &recent, limit);
    let paths: Vec<Value> = ranked
        .into_iter()
        .map(|item| json!({ "path": item.path, "score": item.score, "indices": item.indices }))
        .collect();
    Ok(Json(json!({ "files": paths })))
}

/// `GET /api/recent`: shared recent notes, newest first.
pub async fn recent(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let entries = state.settings.with_store(|store| {
        frankmd_fs::recent::RecentFilesService::new(&state.vault, &*store).entries()
    });
    Ok(Json(json!({ "recent": entries })))
}

/// Body of `POST /api/recent`.
#[derive(Debug, Deserialize)]
pub struct RecentBody {
    /// Vault-relative note path.
    pub path: String,
}

/// `POST /api/recent`: record an open.
pub async fn record_recent(
    State(state): State<AppState>,
    Json(body): Json<RecentBody>,
) -> Result<Json<Value>, ApiError> {
    let path = super::note_path(&body.path)?;
    let now = jiff::Timestamp::now();
    state
        .settings
        .with_store(|store| {
            frankmd_fs::recent::RecentFilesService::new(&state.vault, &*store)
                .record(Some(&path), now)
        })
        .map_err(|e| crate::error::fs_error(&e))?;
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /api/recent`: clear the history.
pub async fn clear_recent(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    state
        .settings
        .with_store(|store| {
            frankmd_fs::recent::RecentFilesService::new(&state.vault, &*store).clear()
        })
        .map_err(|e| crate::error::fs_error(&e))?;
    Ok(Json(json!({ "ok": true })))
}
