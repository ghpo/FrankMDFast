//! Settings, editor config, translations, theme, logs, session and health.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use frankmd_config::{Key, Value as ConfigValue};
use frankmd_i18n::{Locale, resolve_locale, served_translations};
use frankmd_theme::{Mode, UiTokens, builtin, omarchy};

use super::AppState;
use crate::error::ApiError;

/// `GET /up` (unauthenticated, CORS-open like the Rails health check).
pub async fn health() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8".to_owned()),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".to_owned()),
        ],
        "OK",
    )
        .into_response()
}

/// `GET /api/session`
pub async fn session(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "authenticated": true,
        "vault": state.root().display().to_string(),
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.started.elapsed().as_secs()
    }))
}

/// `GET /api/config`
pub async fn get_config(State(state): State<AppState>) -> Json<Value> {
    let resolved = state.resolved();
    let settings = settings_object(&resolved);
    let status = state.media.api.storage_status();
    Json(json!({
        "settings": settings,
        "media_storage_provider": status.media_storage_provider,
        "media_storage_fallback": status.media_storage_fallback,
        "features": {
            "s3_upload": resolved.feature_available(frankmd_config::Feature::S3Upload),
            "youtube_search": resolved.feature_available(frankmd_config::Feature::YoutubeSearch),
            "google_search": resolved.feature_available(frankmd_config::Feature::GoogleSearch),
            "local_images": resolved.feature_available(frankmd_config::Feature::LocalImages),
            "ai": resolved.feature_available(frankmd_config::Feature::Ai),
        },
        "warnings": resolved.warnings,
        "config_file": config_file_path(&state)
    }))
}

fn settings_object(resolved: &frankmd_config::ResolvedConfig) -> Map<String, Value> {
    let mut settings = Map::new();
    for (key, value) in resolved.ui_settings() {
        settings.insert(
            key.name().to_owned(),
            value.map_or(Value::Null, |v| v.to_json()),
        );
    }
    settings
}

fn config_file_path(state: &AppState) -> String {
    state.root().join(".fed").display().to_string()
}

/// `PATCH /api/config`: UI keys only, all-or-nothing.
pub async fn patch_config(
    State(state): State<AppState>,
    Json(body): Json<Map<String, Value>>,
) -> Result<Json<Value>, ApiError> {
    let mut changes: Vec<(Key, Option<ConfigValue>)> = Vec::new();
    for (name, input) in &body {
        let Some(key) = Key::from_name(name) else {
            continue;
        };
        match frankmd_config::value::coerce(key, input) {
            Ok(value) => changes.push((key, value)),
            Err(err) => {
                return Err(ApiError::unprocessable(
                    "invalid_value",
                    format!("{name}: {err}"),
                ));
            }
        }
    }
    if changes.is_empty() {
        return Err(ApiError::unprocessable(
            "no_valid_settings",
            "No valid settings provided",
        ));
    }
    let resolved = state
        .settings
        .with_store(|store| store.update_ui_settings(&changes))
        .map_err(|e| ApiError::unprocessable("failed_to_save", e.to_string()))?;
    let _ = state.events.send(crate::events::RemoteEvent::Config);
    Ok(Json(json!({
        "settings": settings_object(&resolved),
        "message": "Settings saved"
    })))
}

/// `GET /api/config/editor`: the settings the editor needs.
pub async fn editor_config(State(state): State<AppState>) -> Json<Value> {
    let resolved = state.resolved();
    let pick = |key: Key| resolved.value(key).map_or(Value::Null, |v| v.to_json());
    Json(json!({
        "editor_font": pick(Key::EditorFont),
        "editor_font_size": pick(Key::EditorFontSize),
        "preview_zoom": pick(Key::PreviewZoom),
        "view_mode": pick(Key::ViewMode),
        "vim_mode": pick(Key::VimMode),
        "scroll_sync": pick(Key::ScrollSync),
        "editor_indent": pick(Key::EditorIndent),
        "editor_line_numbers": pick(Key::EditorLineNumbers),
        "show_stats": pick(Key::ShowStats),
        "editor_width": pick(Key::EditorWidth),
        "typewriter_mode": pick(Key::TypewriterMode),
        "cursor_recenter_on_click": pick(Key::CursorRecenterOnClick),
        "theme": pick(Key::Theme),
        "locale": pick(Key::Locale),
    }))
}

/// `?locale=xx`
#[derive(Debug, Deserialize)]
pub struct LocaleQuery {
    /// Locale code.
    pub locale: Option<String>,
}

/// `GET /api/translations?locale=xx`
pub async fn translations(
    State(state): State<AppState>,
    Query(query): Query<LocaleQuery>,
) -> Result<Json<Value>, ApiError> {
    let configured = state
        .resolved()
        .value(Key::Locale)
        .and_then(|v| v.as_str().map(str::to_owned));
    let locale = resolve_locale(query.locale.as_deref(), configured.as_deref());
    let body = served_translations(locale).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(
        json!({ "locale": locale.code(), "translations": body }),
    ))
}

/// `GET /api/theme`: the active theme as tokens + CSS custom properties.
pub async fn theme(State(state): State<AppState>) -> Json<Value> {
    let resolved = state.resolved();
    let configured = resolved
        .value(Key::Theme)
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "dark".to_owned());
    let tokens = resolve_tokens(&state, &resolved, &configured);
    let mode = match tokens.mode {
        Mode::Dark => "dark",
        Mode::Light => "light",
    };
    let mut colors = Map::new();
    let mut css = String::new();
    for (name, color) in &tokens.colors {
        let hex = color.to_hex();
        colors.insert((*name).to_owned(), Value::String(hex.clone()));
        css.push_str(&format!("--{name}: {hex};\n"));
    }
    Json(json!({
        "id": configured,
        "mode": mode,
        "colors": colors,
        "css": css,
        "builtin_themes": frankmd_theme::builtin_ids()
    }))
}

fn resolve_tokens(
    state: &AppState,
    resolved: &frankmd_config::ResolvedConfig,
    configured: &str,
) -> UiTokens {
    if configured == "custom" {
        let theme = resolved.value(Key::CustomTheme);
        if let Some(ConfigValue::CustomTheme(custom)) = theme
            && let Some(base) = builtin(&custom.base)
        {
            return base.with_overrides(&custom);
        }
        return builtin("dark").unwrap_or_else(fallback_tokens);
    }
    if configured == "omarchy" {
        if let Some(home) = &state.home {
            let paths = omarchy::OmarchyPaths::from_home(home);
            if let Ok(Some((_, palette))) = omarchy::load_current(&paths) {
                return omarchy::from_omarchy(&palette);
            }
        }
        return builtin("dark").unwrap_or_else(fallback_tokens);
    }
    builtin(configured)
        .or_else(|| builtin("dark"))
        .unwrap_or_else(fallback_tokens)
}

fn fallback_tokens() -> UiTokens {
    let mut colors = std::collections::BTreeMap::new();
    for key in frankmd_theme::COLOR_KEYS {
        colors.insert(key, frankmd_theme::Rgb(0, 0, 0));
    }
    UiTokens {
        mode: Mode::Dark,
        colors,
    }
}

/// `?lines=N`
#[derive(Debug, Deserialize)]
pub struct TailQuery {
    /// Number of lines (ignored: the desktop app logs to stderr/journalctl).
    pub lines: Option<usize>,
}

/// `GET /api/logs/tail`: the desktop app does not write a log file.
pub async fn logs_tail(Query(query): Query<TailQuery>) -> Json<Value> {
    Json(json!({
        "environment": "desktop",
        "file": Value::Null,
        "lines": [],
        "requested": query.lines.unwrap_or(100).clamp(1, 500),
        "note": "FrankMDFast logs to stderr; use journalctl --user -u frankmdfast or the terminal that launched it."
    }))
}

/// `GET /api/logs/config`: masked configuration introspection.
pub async fn logs_config(State(state): State<AppState>) -> Json<Value> {
    let resolved = state.resolved();
    let entries: Vec<Value> = resolved
        .entries()
        .into_iter()
        .map(|entry| {
            json!({
                "key": entry.key.name(),
                "value": entry.value,
                "source": entry.source.as_str(),
                "env_var": entry.env_var,
                "sensitive": entry.sensitive
            })
        })
        .collect();
    Json(json!({
        "config_file": config_file_path(&state),
        "config_file_exists": state.root().join(".fed").exists(),
        "ai_configured_in_file": resolved.ai_configured_in_file(),
        "environment": "desktop",
        "entries": entries,
        "settings": resolved.all_settings_masked()
    }))
}

/// `GET /api/locales`: the locale list for the picker.
pub async fn locales() -> Json<Value> {
    let all: Vec<Value> = Locale::ALL
        .iter()
        .map(|locale| json!({ "code": locale.code(), "name": locale.code() }))
        .collect();
    Json(json!({ "locales": all }))
}
