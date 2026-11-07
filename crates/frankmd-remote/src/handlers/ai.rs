//! AI: grammar fix and image generation (OpenRouter / provider model).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use frankmd_ai::{
    AiRequestError, AiService, NoteSource, ReferenceImage, ReferenceImages, fix_grammar_note,
    generate_image_request,
};
use frankmd_fs::{NotesService, Vault};

use super::AppState;
use crate::error::ApiError;

struct VaultNotes<'a> {
    vault: &'a Vault,
}

impl NoteSource for VaultNotes<'_> {
    fn read_note(&self, path: &str) -> Option<String> {
        NotesService::new(self.vault)
            .read(path)
            .ok()
            .map(|file| String::from_utf8_lossy(&file.bytes).into_owned())
    }
}

struct MediaReferences {
    config: frankmd_media::MediaConfig,
}

impl ReferenceImages for MediaReferences {
    fn find(&self, rel_path: &str) -> Option<ReferenceImage> {
        let path = frankmd_media::server_folder::find_image(&self.config, rel_path)?;
        let bytes = std::fs::read(&path).ok()?;
        let mime_type = mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string();
        Some(ReferenceImage { bytes, mime_type })
    }
}

fn ai_request_error(err: &AiRequestError) -> ApiError {
    match err {
        AiRequestError::BadRequest(key) => ApiError::bad_request(key, *key),
        AiRequestError::NotFound(key) => ApiError::not_found(key, *key),
        AiRequestError::Forbidden => ApiError::forbidden("forbidden", "forbidden"),
        AiRequestError::Service(service) => {
            let status = match service {
                frankmd_ai::AiError::NotConfigured
                | frankmd_ai::AiError::ImageNotConfigured
                | frankmd_ai::AiError::NoProvider => StatusCode::SERVICE_UNAVAILABLE,
                _ => StatusCode::UNPROCESSABLE_ENTITY,
            };
            ApiError::new(status, "ai_error", service.to_string())
        }
    }
}

/// `GET /api/ai/config`
pub async fn ai_config(State(state): State<AppState>) -> Json<Value> {
    let service = AiService::new(state.resolved(), state.media.http.clone());
    let info = service.provider_info();
    Json(json!({
        "enabled": info.enabled,
        "provider": info.provider.map(|p| p.as_str()),
        "model": info.model,
        "available_providers": info.available_providers.iter().map(|p| p.as_str()).collect::<Vec<_>>()
    }))
}

/// `GET /api/ai/image_config`
pub async fn image_config(State(state): State<AppState>) -> Json<Value> {
    let service = AiService::new(state.resolved(), state.media.http.clone());
    let info = service.image_generation_info();
    Json(json!({ "enabled": info.enabled, "model": info.model }))
}

/// `POST /api/ai/fix_grammar` (`{path}`)
#[derive(Debug, Deserialize)]
pub struct GrammarBody {
    /// Note path.
    pub path: String,
}

/// `POST /api/ai/fix_grammar` (`{path}`).
pub async fn fix_grammar(
    State(state): State<AppState>,
    Json(body): Json<GrammarBody>,
) -> Result<Json<Value>, ApiError> {
    let service = AiService::new(state.resolved(), state.media.http.clone());
    let notes = VaultNotes {
        vault: &state.vault,
    };
    let response = fix_grammar_note(&service, &notes, &body.path)
        .await
        .map_err(|e| ai_request_error(&e))?;
    Ok(Json(json!({
        "original": response.original,
        "corrected": response.fix.corrected,
        "provider": response.fix.provider.as_str(),
        "model": response.fix.model
    })))
}

/// `POST /api/ai/generate_image` (`{prompt, reference_image_path?}`)
#[derive(Debug, Deserialize)]
pub struct ImageBody {
    /// Prompt.
    pub prompt: String,
    /// Optional reference image (images_path-relative).
    #[serde(default)]
    pub reference_image_path: Option<String>,
}

/// `POST /api/ai/generate_image` (`{prompt, reference_image_path?}`).
pub async fn generate_image(
    State(state): State<AppState>,
    Json(body): Json<ImageBody>,
) -> Result<Json<Value>, ApiError> {
    let service = AiService::new(state.resolved(), state.media.http.clone());
    let references = MediaReferences {
        config: state.media_config(),
    };
    let image = generate_image_request(
        &service,
        &body.prompt,
        body.reference_image_path.as_deref(),
        &references,
    )
    .await
    .map_err(|e| ai_request_error(&e))?;
    Ok(Json(json!({
        "data": image.data,
        "mime_type": image.mime_type,
        "model": image.model
    })))
}
