//! Desktop sync client against a wiremock remote server.

use std::collections::BTreeMap;

use frankmd_fs::Vault;
use frankmd_net::NetRuntime;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn pulls_remote_files_and_pushes_local_files() {
    let runtime = NetRuntime::new().expect("runtime");
    let server = runtime.handle().block_on(MockServer::start());
    let token = "a".repeat(32);

    runtime.handle().block_on(async {
        Mock::given(method("GET"))
            .and(path("/api/sync/manifest"))
            .and(header("authorization", format!("Bearer {token}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [{"path": "remote.md", "revision": "remote-revision", "size": 6, "mtime": 0}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/sync/file"))
            .and(query_param("path", "remote.md"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"remote".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("PUT"))
            .and(path("/api/sync/file"))
            .and(query_param("path", "local.md"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "path": "local.md", "revision": "local-revision"
            })))
            .expect(1)
            .mount(&server)
            .await;
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let vault = Vault::open(dir.path().join("notes"), dir.path().join("state")).expect("vault");
    vault
        .sync_write("local.md", b"local", None)
        .expect("seed local");

    let outcome = runtime
        .handle()
        .block_on(crate::sync_run(&vault, &server.uri(), &token))
        .expect("sync");

    assert_eq!(outcome.downloaded, 1);
    assert_eq!(outcome.uploaded, 1);
    assert_eq!(outcome.deleted, 0);
    assert_eq!(outcome.conflicts, 0);
    assert_eq!(
        String::from_utf8(vault.sync_read("remote.md").expect("remote")).expect("utf8"),
        "remote"
    );
    assert_eq!(
        outcome.base.get("remote.md").map(String::as_str),
        Some("remote-revision")
    );
    assert_eq!(
        outcome.base.get("local.md").map(String::as_str),
        Some("local-revision")
    );
}

fn sync_run(
    vault: &Vault,
    server: &str,
    token: &str,
) -> impl std::future::Future<Output = anyhow::Result<frankmd_app::sync::SyncOutcome>> {
    frankmd_app::sync::run(vault.clone(), server, token, BTreeMap::new())
}
#[test]
fn conflict_keeps_a_local_copy_and_takes_the_remote_note() {
    let runtime = NetRuntime::new().expect("runtime");
    let server = runtime.handle().block_on(MockServer::start());
    let token = "b".repeat(32);

    runtime.handle().block_on(async {
        Mock::given(method("GET"))
            .and(path("/api/sync/manifest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [{"path": "note.md", "revision": "remote-revision", "size": 6, "mtime": 0}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/sync/file"))
            .and(query_param("path", "note.md"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"remote".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let vault = Vault::open(dir.path().join("notes"), dir.path().join("state")).expect("vault");
    vault
        .sync_write("note.md", b"local", None)
        .expect("seed local");
    let mut base = BTreeMap::new();
    base.insert("note.md".to_owned(), "base-revision".to_owned());

    let outcome = runtime
        .handle()
        .block_on(frankmd_app::sync::run(
            vault.clone(),
            &server.uri(),
            &token,
            base,
        ))
        .expect("sync");

    assert_eq!(outcome.conflicts, 1);
    assert_eq!(
        String::from_utf8(vault.sync_read("note.md").expect("note")).expect("utf8"),
        "remote"
    );
    let conflict = vault
        .sync_manifest()
        .expect("manifest")
        .into_iter()
        .find(|entry| entry.path.contains("(conflito desktop"))
        .expect("conflict copy");
    assert_eq!(
        String::from_utf8(vault.sync_read(&conflict.path).expect("conflict")).expect("utf8"),
        "local"
    );
}
