//! End-to-end tests of the remote router against a temporary vault.

#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use frankmd_config::EnvSnapshot;
use frankmd_remote::{RemoteConfig, ServerState, build_router};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

struct TestApp {
    dir: tempfile::TempDir,
    state: Arc<ServerState>,
    router: Router,
}

fn app() -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("notes");
    let state_dir = dir.path().join("state");
    let trash = tempfile::tempdir_in(dir.path()).unwrap();
    let config = RemoteConfig {
        vault_root: vault,
        state_dir,
        env: EnvSnapshot::capture(),
        bind: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: TOKEN.to_owned(),
        locale: None,
        trash_dir: Some(trash.path().to_path_buf()),
    };
    let state = ServerState::new(&config).unwrap();
    state
        .notes()
        .create("hello.md", b"# Hello\n\nworld\n")
        .unwrap();
    state.notes().create("other.md", b"second note\n").unwrap();
    state.notes().create_folder("blog").unwrap();
    let router = build_router(state.clone())
        .layer(MockConnectInfo(SocketAddr::from(([100, 64, 0, 1], 43210))));
    TestApp { dir, state, router }
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    auth: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = auth {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, headers, value)
}

async fn call_auth(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, _, value) = call(app, method, uri, body, Some(TOKEN)).await;
    (status, value)
}

#[tokio::test]
async fn health_is_public_and_api_is_not() {
    let app = app();
    let (status, headers, body) = call(&app.router, Method::GET, "/up", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "*"
    );
    assert_eq!(body, Value::String("OK".to_owned()));

    let (status, _, _) = call(&app.router, Method::GET, "/api/tree", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) = call(&app.router, Method::GET, "/api/tree", None, Some("wrong")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_sets_the_digest_cookie_and_csrf_is_enforced() {
    let app = app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("password={TOKEN}")))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.contains("frankmd_auth_token_digest="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    let value = cookie
        .split(';')
        .next()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_owned();

    // GET with the cookie works.
    let request = Request::builder()
        .uri("/api/tree")
        .header(header::COOKIE, format!("frankmd_auth_token_digest={value}"))
        .body(Body::empty())
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // A mutation without the custom header is refused.
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/note")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("frankmd_auth_token_digest={value}"))
        .body(Body::from(
            json!({"path": "csrf.md", "content": "x"}).to_string(),
        ))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // With it, the same request succeeds.
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/note")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("frankmd_auth_token_digest={value}"))
        .header("x-frankmd-csrf", "1")
        .body(Body::from(
            json!({"path": "csrf.md", "content": "x"}).to_string(),
        ))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn wrong_login_is_rejected_and_repeated_failures_are_throttled() {
    let app = app();
    for expected in [
        StatusCode::SEE_OTHER,
        StatusCode::SEE_OTHER,
        StatusCode::SEE_OTHER,
        StatusCode::SEE_OTHER,
        StatusCode::SEE_OTHER,
        StatusCode::SEE_OTHER,
        StatusCode::SEE_OTHER,
        StatusCode::SEE_OTHER,
        StatusCode::SEE_OTHER,
        StatusCode::SEE_OTHER,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/login")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("password=nope"))
            .unwrap();
        let response = app.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn tree_and_note_crud() {
    let app = app();
    let (status, tree) = call_auth(&app.router, Method::GET, "/api/tree", None).await;
    assert_eq!(status, StatusCode::OK);
    let text = tree.to_string();
    assert!(text.contains("hello.md"), "{text}");
    assert!(text.contains("blog"), "{text}");

    let (status, note) = call_auth(&app.router, Method::GET, "/api/note?path=hello.md", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(note["content"], "# Hello\n\nworld\n");
    let revision = note["revision"].as_str().unwrap().to_owned();

    // Stale revision is a conflict.
    let (status, _) = call_auth(
        &app.router,
        Method::PATCH,
        "/api/note",
        Some(json!({"path": "hello.md", "content": "nope", "expected_revision": "deadbeef"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, updated) = call_auth(
        &app.router,
        Method::PATCH,
        "/api/note",
        Some(json!({"path": "hello.md", "content": "# Changed\n", "expected_revision": revision})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(updated["revision"].is_string());

    let (status, created) = call_auth(
        &app.router,
        Method::POST,
        "/api/note",
        Some(json!({"path": "blog/post.md", "content": "post"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["path"], "blog/post.md");

    let (status, _) = call_auth(
        &app.router,
        Method::POST,
        "/api/note",
        Some(json!({"path": "blog/post.md", "content": "again"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "no-clobber");

    let (status, _) = call_auth(
        &app.router,
        Method::POST,
        "/api/note/rename",
        Some(json!({"path": "blog/post.md", "new_path": "blog/renamed.md"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = call_auth(
        &app.router,
        Method::DELETE,
        "/api/note?path=blog/renamed.md",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = call_auth(
        &app.router,
        Method::GET,
        "/api/note?path=blog/renamed.md",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn folders_and_hugo_posts() {
    let app = app();
    let (status, _) = call_auth(
        &app.router,
        Method::POST,
        "/api/folder",
        Some(json!({"path": "blog/2026"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = call_auth(
        &app.router,
        Method::POST,
        "/api/folder/rename",
        Some(json!({"path": "blog/2026", "new_path": "blog/archive"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, post) = call_auth(
        &app.router,
        Method::POST,
        "/api/hugo",
        Some(json!({"parent": "blog", "title": "My Post", "slug": "my-post"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let path = post["path"].as_str().unwrap();
    assert!(
        path.starts_with("blog/") && path.ends_with("/my-post/index.md"),
        "{path}"
    );
}

#[tokio::test]
async fn paths_cannot_escape_or_touch_fed() {
    let app = app();
    let (status, _) = call_auth(
        &app.router,
        Method::GET,
        "/api/note?path=../../etc/passwd",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call_auth(&app.router, Method::GET, "/api/note?path=.fed", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn search_finder_preview_and_backlinks() {
    let app = app();
    let (status, search) = call_auth(&app.router, Method::GET, "/api/search?q=world", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(search["count"].as_u64().unwrap() >= 1, "{search}");

    let (status, finder) = call_auth(&app.router, Method::GET, "/api/finder?q=hello", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        finder["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "hello.md")
    );

    let (status, preview) = call_auth(
        &app.router,
        Method::POST,
        "/api/preview",
        Some(json!({"content": "# Hi\n\n**bold**"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = preview["html"].as_str().unwrap();
    assert!(html.contains("<h1"), "{html}");
    assert!(html.contains("<strong>bold</strong>"), "{html}");

    state_note(
        &app.state,
        "links.md",
        "see [[hello]] and [[hello|the note]]",
    );
    let (status, links) = call_auth(
        &app.router,
        Method::GET,
        "/api/backlinks?path=hello.md",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(links["count"].as_u64().unwrap() >= 1, "{links}");
}

fn state_note(state: &ServerState, path: &str, content: &str) {
    state.notes().create(path, content.as_bytes()).unwrap();
}

#[tokio::test]
async fn config_theme_and_translations() {
    let app = app();
    let (status, config) = call_auth(&app.router, Method::GET, "/api/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(config["settings"].is_object());
    assert!(config["features"].is_object());

    let (status, patched) = call_auth(
        &app.router,
        Method::PATCH,
        "/api/config",
        Some(json!({"editor_font_size": 18, "vim_mode": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(patched["settings"]["editor_font_size"], 18);
    assert_eq!(patched["settings"]["vim_mode"], true);

    let (status, _) = call_auth(&app.router, Method::PATCH, "/api/config", Some(json!({}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, theme) = call_auth(&app.router, Method::GET, "/api/theme", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(theme["colors"]["bg-primary"].is_string());
    assert!(theme["css"].as_str().unwrap().contains("--bg-primary"));

    let (status, translations) =
        call_auth(&app.router, Method::GET, "/api/translations", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(translations["locale"], "en");
    assert!(translations["translations"].is_object());
}

#[tokio::test]
async fn recents_are_shared_through_fed() {
    let app = app();
    let (status, _) = call_auth(
        &app.router,
        Method::POST,
        "/api/recent",
        Some(json!({"path": "hello.md"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, recent) = call_auth(&app.router, Method::GET, "/api/recent", None).await;
    assert_eq!(recent["recent"][0]["path"], "hello.md");
    let (status, _) = call_auth(&app.router, Method::DELETE, "/api/recent", None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, recent) = call_auth(&app.router, Method::GET, "/api/recent", None).await;
    assert_eq!(recent["recent"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn media_base64_upload_and_library() {
    let app = app();
    // 1x1 transparent PNG.
    let data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let (status, uploaded) = call_auth(
        &app.router,
        Method::POST,
        "/api/images/upload_base64",
        Some(json!({"data": data, "mime_type": "image/png", "filename": "px.png"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
    let url = uploaded["url"].as_str().unwrap();
    assert!(url.starts_with("images/"), "{url}");
    assert!(app.dir.path().join("notes").join(url).exists());

    let (status, library) = call_auth(&app.router, Method::GET, "/api/library", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        library["images"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["path"] == url),
        "{library}"
    );

    let (status, config) = call_auth(&app.router, Method::GET, "/api/images/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(config["image_extensions"].as_array().unwrap().len() >= 4);
}

#[tokio::test]
async fn ai_and_youtube_report_unconfigured() {
    let app = app();
    let (status, ai) = call_auth(&app.router, Method::GET, "/api/ai/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ai["enabled"], false);

    let (status, youtube) = call_auth(&app.router, Method::GET, "/api/youtube/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(youtube["enabled"], false);
}

#[tokio::test]
async fn events_endpoint_streams_sse() {
    let app = app();
    let request = Request::builder()
        .uri("/api/events")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "text/event-stream"
    );
}

#[tokio::test]
async fn short_tokens_are_refused_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let config = RemoteConfig {
        vault_root: dir.path().join("notes"),
        state_dir: dir.path().join("state"),
        env: EnvSnapshot::capture(),
        bind: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: "short".to_owned(),
        locale: None,
        trash_dir: None,
    };
    assert!(ServerState::new(&config).is_err());
}

#[tokio::test]
async fn unknown_api_is_json_404_and_spa_falls_back_to_index() {
    let app = app();
    let (status, _, value) = call(
        &app.router,
        Method::GET,
        "/api/does-not-exist",
        None,
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(value["code"], "not_found");

    let (status, headers, _) = call(
        &app.router,
        Method::GET,
        "/some/spa/route",
        None,
        Some(TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
}
