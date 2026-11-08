//! The axum router, authentication middleware and login/logout endpoints.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Form, Path, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{delete, get, patch, post};
use serde::Deserialize;

use crate::auth::{AuthVia, COOKIE_NAME};
use crate::handlers::{self, AppState};
use crate::state::ServerState;
use crate::web;

/// 16 MiB for JSON bodies (a note is text; this is already generous).
const JSON_BODY_LIMIT: usize = 16 * 1024 * 1024;
/// Upload bodies: the media pipeline cap plus multipart overhead.
const UPLOAD_BODY_LIMIT: usize =
    (frankmd_media::upload_storage::MAX_UPLOAD_BYTES as usize) + 64 * 1024 * 1024;

/// Build the complete router (useful for tests).
pub fn build_router(state: Arc<ServerState>) -> Router {
    let public = Router::new()
        .route("/up", get(handlers::config::health))
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", post(logout).delete(logout))
        .route("/assets/{*file}", get(asset))
        .with_state(state.clone());

    let protected = Router::new()
        .route("/", get(index))
        .route("/api/session", get(handlers::config::session))
        .route("/api/encryption", get(handlers::encryption::status))
        .route("/api/encryption/unlock", post(handlers::encryption::unlock))
        .route("/api/encryption/lock", post(handlers::encryption::lock))
        .route("/api/tree", get(handlers::notes::tree))
        .route(
            "/api/note",
            get(handlers::notes::get_note)
                .post(handlers::notes::create_note)
                .patch(handlers::notes::update_note)
                .delete(handlers::notes::delete_note),
        )
        .route("/api/note/rename", post(handlers::notes::rename_note))
        .route(
            "/api/folder",
            post(handlers::notes::create_folder).delete(handlers::notes::delete_folder),
        )
        .route("/api/folder/rename", post(handlers::notes::rename_folder))
        .route("/api/hugo", post(handlers::notes::create_hugo))
        .route("/api/preview", post(handlers::notes::preview))
        .route("/api/backlinks", get(handlers::notes::backlinks))
        .route("/api/asset/{*path}", get(handlers::notes::get_asset))
        .route("/api/search", get(handlers::search::search))
        .route("/api/finder", get(handlers::search::finder_route))
        .route(
            "/api/recent",
            get(handlers::search::recent)
                .post(handlers::search::record_recent)
                .delete(handlers::search::clear_recent),
        )
        .route(
            "/api/config",
            get(handlers::config::get_config).patch(handlers::config::patch_config),
        )
        .route("/api/config/editor", get(handlers::config::editor_config))
        .route("/api/translations", get(handlers::config::translations))
        .route("/api/locales", get(handlers::config::locales))
        .route("/api/theme", get(handlers::config::theme))
        .route("/api/logs/tail", get(handlers::config::logs_tail))
        .route("/api/logs/config", get(handlers::config::logs_config))
        .route("/api/events", get(handlers::events::sse))
        .route("/api/images/config", get(handlers::media::images_config))
        .route("/api/images", get(handlers::media::images))
        .route(
            "/api/images/preview/{*path}",
            get(handlers::media::image_preview),
        )
        .route(
            "/api/images/file/{*path}",
            delete(handlers::media::image_delete),
        )
        .route(
            "/api/images/upload",
            post(handlers::media::image_upload).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        .route(
            "/api/media/upload",
            post(handlers::media::media_upload).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        .route(
            "/api/images/upload_base64",
            post(handlers::media::image_upload_base64)
                .layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        .route("/api/images/import", post(handlers::media::image_import))
        .route(
            "/api/images/upload_to_s3",
            post(handlers::media::image_to_s3),
        )
        .route(
            "/api/images/upload_external_to_s3",
            post(handlers::media::image_external_to_s3),
        )
        .route("/api/images/search_web", get(handlers::media::search_web))
        .route(
            "/api/images/search_pinterest",
            get(handlers::media::search_pinterest),
        )
        .route(
            "/api/images/search_google",
            get(handlers::media::search_google),
        )
        .route("/api/library", get(handlers::media::library))
        .route("/api/library/usage", get(handlers::media::library_usage))
        .route(
            "/api/library/file/{*path}",
            delete(handlers::media::library_delete),
        )
        .route(
            "/api/library/file/{*path}",
            patch(handlers::media::library_rename),
        )
        .route("/api/youtube/config", get(handlers::media::youtube_config))
        .route("/api/youtube/search", get(handlers::media::youtube_search))
        .route("/api/ai/config", get(handlers::ai::ai_config))
        .route("/api/ai/image_config", get(handlers::ai::image_config))
        .route("/api/ai/fix_grammar", post(handlers::ai::fix_grammar))
        .route("/api/ai/generate_image", post(handlers::ai::generate_image))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .with_state(state.clone());

    public
        .merge(protected)
        .fallback(fallback)
        .layer(middleware::from_fn(security_headers))
        .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT))
        .with_state(state)
}

/// Bind the socket and serve until the process exits.
pub async fn serve(state: Arc<ServerState>, addr: SocketAddr) -> std::io::Result<()> {
    serve_until(state, addr, std::future::pending()).await
}

/// Bind the socket and serve until `shutdown` resolves (Settings toggle).
pub async fn serve_until<F>(
    state: Arc<ServerState>,
    addr: SocketAddr,
    shutdown: F,
) -> std::io::Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(address = %addr, "FrankMDFast remote server listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

async fn index() -> Html<&'static str> {
    Html(web::INDEX_HTML)
}

async fn asset(Path(file): Path<String>) -> Response {
    match file.as_str() {
        "app.js" => (
            [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
            web::APP_JS,
        )
            .into_response(),
        "style.css" => (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            web::STYLE_CSS,
        )
            .into_response(),
        "login.js" => (
            [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
            web::LOGIN_JS,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn fallback(request: Request) -> Response {
    if request.uri().path().starts_with("/api/") {
        return crate::error::ApiError::not_found("not_found", "unknown endpoint").into_response();
    }
    Html(web::INDEX_HTML).into_response()
}

/// Reject unauthenticated requests; enforce the custom-header CSRF guard for
/// cookie-authenticated mutations.
async fn require_auth(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Some(via) = state.auth.authenticate(request.headers()) else {
        return if request.uri().path().starts_with("/api/") {
            crate::error::ApiError::unauthorized().into_response()
        } else {
            Redirect::to("/login").into_response()
        };
    };
    if via == AuthVia::Cookie
        && is_mutation(request.method())
        && request.headers().get("x-frankmd-csrf").is_none()
    {
        return crate::error::ApiError::forbidden("csrf_required", "missing X-FrankMD-CSRF header")
            .into_response();
    }
    next.run(request).await
}

fn is_mutation(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PATCH | Method::PUT | Method::DELETE
    )
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data: https: http:; \
             media-src 'self' https: http:; style-src 'self' 'unsafe-inline'; \
             script-src 'self'; connect-src 'self'; frame-ancestors 'none'; \
             base-uri 'none'; form-action 'self'",
        ),
    );
    response
}

async fn login_page() -> Html<&'static str> {
    Html(web::LOGIN_HTML)
}

#[derive(Debug, Deserialize)]
struct LoginForm {
    password: String,
}

async fn login_submit(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Form(form): Form<LoginForm>,
) -> Response {
    let ip = remote.ip();
    if !state.auth.may_attempt(ip) {
        return crate::error::ApiError::too_many_requests().into_response();
    }
    if !state.auth.verify_password(&form.password) {
        state.auth.record_failure(ip);
        tracing::warn!(%ip, "remote login failed");
        return Redirect::to("/login?error=invalid").into_response();
    }
    state.auth.reset_failures(ip);
    let cookie = format!(
        "{COOKIE_NAME}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000",
        state.auth.cookie_value()
    );
    ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response()
}

async fn logout() -> Response {
    let cookie = format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    ([(header::SET_COOKIE, cookie)], Redirect::to("/login")).into_response()
}
