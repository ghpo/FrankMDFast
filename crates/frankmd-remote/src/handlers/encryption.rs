//! Encryption status/unlock/lock for the web client.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::error::ApiError;
use crate::state::ServerState;

/// Pick up a seal created/imported/changed by the desktop app while this
/// daemon was already running. A changed seal locks the current key.
fn reload_seal(state: &ServerState) {
    let path = state.state_dir.join("encryption.json");
    let seal = std::fs::read(&path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| frankmd_crypto::SealFile::decode(&text).ok());
    match (seal, state.encryption.seal()) {
        (Some(new), Some(old)) if new == old => {}
        (Some(new), _) => state.encryption.set_seal(Some(new)),
        (None, Some(_)) => state.encryption.set_seal(None),
        (None, None) => {}
    }
}

/// `GET /api/encryption`
pub async fn status(State(state): State<AppState>) -> Json<Value> {
    reload_seal(&state);
    Json(json!({
        "configured": state.encryption.seal().is_some(),
        "unlocked": state.encryption.is_unlocked()
    }))
}

/// Unlock body.
#[derive(Debug, Deserialize)]
pub struct UnlockBody {
    /// Vault encryption password.
    pub password: String,
}

/// `POST /api/encryption/unlock` (authenticated; IP-throttled).
pub async fn unlock(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<UnlockBody>,
) -> Result<Json<Value>, ApiError> {
    let ip = remote.ip();
    reload_seal(&state);
    if !state.auth.may_attempt(ip) {
        return Err(ApiError::too_many_requests());
    }
    match state.encryption.unlock(&body.password) {
        Ok(()) => {
            state.auth.reset_failures(ip);
            Ok(Json(json!({"unlocked": true})))
        }
        Err(_) => {
            state.auth.record_failure(ip);
            Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "wrong_password",
                "Wrong encryption password",
            ))
        }
    }
}

/// `POST /api/encryption/lock`
pub async fn lock(State(state): State<AppState>) -> Json<Value> {
    state.encryption.lock();
    Json(json!({"unlocked": false}))
}
