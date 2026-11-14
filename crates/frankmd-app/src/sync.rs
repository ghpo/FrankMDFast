//! Desktop sync client for the raw-file remote protocol.
//!
//! The desktop can both host a remote server and connect to one. This module
//! owns the client half: a per-vault `sync.json` with the selected server,
//! token and the last synced revision map, plus the pull/push algorithm.
//!
//! Encryption is transparent here: notes and attachments are synced as the
//! bytes on disk (including their encrypted envelopes), exactly like the
//! Android/iOS clients. Conflicts keep the local version as a sibling copy and
//! take the server version for the original path.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use frankmd_fs::Vault;
use frankmd_net::TrustedClient;
use serde::{Deserialize, Serialize};
use url::Url;

/// File inside the per-vault state directory.
pub const SYNC_FILE: &str = "sync.json";

/// Persisted connection and revision state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct SyncState {
    server_url: String,
    token: String,
    auto: bool,
    /// Base revisions per normalized server URL. Keeping one map per server
    /// makes switching servers safe: the new server starts without a base
    /// and the old server can be resumed later.
    bases: BTreeMap<String, BTreeMap<String, String>>,
}

/// Per-vault sync settings.
#[derive(Debug, Default)]
pub struct SyncStore {
    file: Option<PathBuf>,
    state: SyncState,
}

impl SyncStore {
    /// Load `<state_dir>/sync.json`; missing or malformed values fall back to
    /// defaults. The token remains in memory and is only written on save.
    pub fn open(state_dir: &Path) -> Self {
        let file = state_dir.join(SYNC_FILE);
        let state = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<SyncState>(&bytes).ok())
            .unwrap_or_default();
        Self {
            file: Some(file),
            state,
        }
    }

    /// In-memory store for tests.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Currently selected server root (empty when not configured).
    pub fn server_url(&self) -> &str {
        &self.state.server_url
    }

    /// Stored access token (empty when not configured).
    pub fn token(&self) -> &str {
        &self.state.token
    }

    /// Whether a sync pass should run when the vault opens.
    pub fn auto(&self) -> bool {
        self.state.auto
    }

    /// Whether both a server and a token are available.
    pub fn configured(&self) -> bool {
        !self.state.server_url.trim().is_empty() && !self.state.token.trim().is_empty()
    }

    /// Save the selected server/token. An empty `token` keeps the existing one.
    pub fn set_connection(&mut self, server_url: &str, token: Option<&str>) -> Result<()> {
        let server = normalize_server(server_url)?;
        let server_changed = !self.state.server_url.is_empty() && self.state.server_url != server;
        if let Some(token) = token {
            let token = token.trim();
            if !token.is_empty() {
                if token.len() < 32 {
                    bail!("O token deve ter pelo menos 32 caracteres");
                }
                self.state.token = token.to_owned();
            }
        }
        if self.state.token.trim().is_empty() {
            bail!("Configure o token do servidor");
        }
        if server_changed && token.is_none() {
            bail!("Informe o token do novo servidor");
        }
        self.state.server_url = server;
        self.persist();
        Ok(())
    }

    /// Persist the launch-sync preference.
    pub fn set_auto(&mut self, enabled: bool) {
        self.state.auto = enabled;
        self.persist();
    }

    /// Last synced revision map for the active server.
    pub fn base(&self) -> BTreeMap<String, String> {
        self.state
            .bases
            .get(&self.state.server_url)
            .cloned()
            .unwrap_or_default()
    }

    /// Persist the revision map for the active server.
    pub fn save_base(&mut self, base: BTreeMap<String, String>) {
        let server = self.state.server_url.clone();
        self.save_base_for(&server, base);
    }

    /// Save the base map for a specific server (the pass may finish after the
    /// user switched the active server in Settings).
    pub fn save_base_for(&mut self, server: &str, base: BTreeMap<String, String>) {
        if let Ok(server) = normalize_server(server) {
            self.state.bases.insert(server, base);
            self.persist();
        }
    }

    fn persist(&self) {
        let Some(file) = &self.file else {
            return;
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.state) {
            let _ = frankmd_fs::atomic::atomic_replace(
                file,
                &bytes,
                frankmd_fs::atomic::ModePolicy::Fixed(0o600),
            );
        }
    }
}

/// Result of one successful sync pass.
#[derive(Debug, Default)]
pub struct SyncOutcome {
    /// Normalized server URL the pass ran against.
    pub server: String,
    /// Files pulled from the server.
    pub downloaded: usize,
    /// Files pushed to the server.
    pub uploaded: usize,
    /// Files removed on either side.
    pub deleted: usize,
    /// Both-sided edits resolved by keeping a local conflict copy.
    pub conflicts: usize,
    /// New base revision map for the active server.
    pub base: BTreeMap<String, String>,
    /// Paths touched locally (for the watcher/rescan).
    pub changed_paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteManifest {
    files: Vec<RemoteFile>,
}

#[derive(Debug, Deserialize)]
struct RemoteFile {
    path: String,
    revision: String,
}

#[derive(Debug, Deserialize)]
struct RevisionResponse {
    revision: String,
}

/// Validate and normalize a server root such as `http://100.x:8787/`.
pub fn normalize_server(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('/');
    let parsed = Url::parse(trimmed).context("URL do servidor inválida")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("Use uma URL http:// ou https://");
    }
    if parsed.host_str().is_none() {
        bail!("A URL do servidor não tem host");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("Não inclua usuário ou senha na URL");
    }
    Ok(trimmed.to_owned())
}

/// Run one sync pass. The caller is responsible for saving the returned base
/// map only after this function succeeds.
pub async fn run(
    vault: Vault,
    server: &str,
    token: &str,
    base: BTreeMap<String, String>,
) -> Result<SyncOutcome> {
    let server = normalize_server(server)?;
    if token.trim().is_empty() {
        bail!("Token do servidor não configurado");
    }
    let client = TrustedClient::new().context("Não foi possível iniciar o cliente HTTP")?;

    let remote = fetch_manifest(&client, &server, token)
        .await
        .context("Falha ao ler o manifesto do servidor")?;
    let local_entries = vault
        .sync_manifest()
        .context("Falha ao ler as notas locais")?;
    let local: BTreeMap<String, String> = local_entries
        .into_iter()
        .map(|entry| (entry.path, entry.revision))
        .collect();

    let mut remote: BTreeMap<String, String> = remote
        .into_iter()
        .map(|file| (file.path, file.revision))
        .collect();
    let mut next_base = BTreeMap::new();
    let mut outcome = SyncOutcome {
        server: server.clone(),
        ..SyncOutcome::default()
    };
    let mut paths: BTreeSet<String> = local.keys().cloned().collect();
    paths.extend(remote.keys().cloned());
    paths.extend(base.keys().cloned());

    for path in paths {
        let local_rev = local.get(&path).cloned();
        let remote_rev = remote.remove(&path);
        let base_rev = base.get(&path).cloned();

        match (local_rev, remote_rev) {
            (Some(local_rev), None) => {
                if base_rev.as_deref() == Some(local_rev.as_str()) {
                    // Deleted remotely while unchanged locally.
                    if vault.sync_delete(&path, Some(&local_rev)).is_ok() {
                        outcome.deleted += 1;
                        outcome.changed_paths.push(path);
                    }
                } else {
                    let revision = upload(&client, &server, token, &vault, &path, None).await?;
                    next_base.insert(path.clone(), revision);
                    outcome.uploaded += 1;
                    outcome.changed_paths.push(path);
                }
            }
            (None, Some(remote_rev)) => {
                if base_rev.as_deref() == Some(remote_rev.as_str()) {
                    // Deleted locally while unchanged remotely.
                    delete_remote(&client, &server, token, &path, &remote_rev).await?;
                    outcome.deleted += 1;
                } else {
                    download(&client, &server, token, &vault, &path).await?;
                    next_base.insert(path.clone(), remote_rev);
                    outcome.downloaded += 1;
                    outcome.changed_paths.push(path);
                }
            }
            (Some(local_rev), Some(remote_rev)) => {
                if local_rev == remote_rev {
                    next_base.insert(path, remote_rev);
                } else if base_rev.as_deref() == Some(local_rev.as_str()) {
                    download(&client, &server, token, &vault, &path).await?;
                    next_base.insert(path.clone(), remote_rev);
                    outcome.downloaded += 1;
                    outcome.changed_paths.push(path);
                } else if base_rev.as_deref() == Some(remote_rev.as_str()) {
                    let revision =
                        upload(&client, &server, token, &vault, &path, Some(&remote_rev)).await?;
                    next_base.insert(path.clone(), revision);
                    outcome.uploaded += 1;
                    outcome.changed_paths.push(path);
                } else {
                    // Both sides changed. Preserve the local edit as a new
                    // note, then take the server version for the original.
                    save_conflict(&vault, &path)?;
                    download(&client, &server, token, &vault, &path).await?;
                    next_base.insert(path.clone(), remote_rev);
                    outcome.conflicts += 1;
                    outcome.downloaded += 1;
                    outcome.changed_paths.push(path);
                }
            }
            (None, None) => {}
        }
    }

    outcome.base = next_base;
    Ok(outcome)
}

async fn fetch_manifest(
    client: &TrustedClient,
    server: &str,
    token: &str,
) -> Result<Vec<RemoteFile>> {
    let url = format!("{server}/api/sync/manifest");
    let response = client.reqwest().get(url).bearer_auth(token).send().await?;
    let status = response.status();
    if !status.is_success() {
        bail!("HTTP {}", status.as_u16());
    }
    let body = response.text().await?;
    let manifest: RemoteManifest = serde_json::from_str(&body)?;
    Ok(manifest.files)
}

async fn upload(
    client: &TrustedClient,
    server: &str,
    token: &str,
    vault: &Vault,
    path: &str,
    expected: Option<&str>,
) -> Result<String> {
    let bytes = vault.sync_read(path)?;
    let url = file_url(server, path, expected)?;
    let response = client
        .reqwest()
        .put(url)
        .bearer_auth(token)
        .header("Content-Type", "application/octet-stream")
        .body(bytes)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        bail!("Upload de {path}: HTTP {}", status.as_u16());
    }
    let body = response.text().await?;
    Ok(serde_json::from_str::<RevisionResponse>(&body)?.revision)
}

async fn download(
    client: &TrustedClient,
    server: &str,
    token: &str,
    vault: &Vault,
    path: &str,
) -> Result<()> {
    let url = file_url(server, path, None)?;
    let response = client.reqwest().get(url).bearer_auth(token).send().await?;
    let status = response.status();
    if !status.is_success() {
        bail!("Download de {path}: HTTP {}", status.as_u16());
    }
    let bytes = response.bytes().await?;
    vault.sync_write(path, &bytes, None)?;
    Ok(())
}

async fn delete_remote(
    client: &TrustedClient,
    server: &str,
    token: &str,
    path: &str,
    revision: &str,
) -> Result<()> {
    let url = file_url(server, path, Some(revision))?;
    let response = client
        .reqwest()
        .delete(url)
        .bearer_auth(token)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        bail!("Exclusão de {path}: HTTP {}", status.as_u16());
    }
    Ok(())
}

fn file_url(server: &str, path: &str, expected: Option<&str>) -> Result<Url> {
    let mut url = Url::parse(&format!("{server}/api/sync/file"))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("path", path);
        if let Some(expected) = expected {
            query.append_pair("expected_revision", expected);
        }
    }
    Ok(url)
}

fn save_conflict(vault: &Vault, path: &str) -> Result<()> {
    let bytes = vault.sync_read(path)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let (parent, name) = path.rsplit_once('/').map_or(("", path), |(p, n)| (p, n));
    let (stem, extension) = name
        .rsplit_once('.')
        .map_or((name, ""), |(stem, extension)| (stem, extension));
    let extension = if extension.is_empty() {
        String::new()
    } else {
        format!(".{extension}")
    };
    for suffix in 0..100 {
        let number = if suffix == 0 {
            String::new()
        } else {
            format!(" {suffix}")
        };
        let candidate_name = format!("{stem} (conflito desktop {stamp}{number}){extension}");
        let candidate = if parent.is_empty() {
            candidate_name
        } else {
            format!("{parent}/{candidate_name}")
        };
        if vault.sync_read(&candidate).is_err() {
            vault.sync_write(&candidate, &bytes, None)?;
            return Ok(());
        }
    }
    bail!("Não foi possível criar uma cópia de conflito para {path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_server_and_keeps_base_per_server() {
        assert_eq!(
            normalize_server(" http://100.64.0.1:8787/ ").expect("url"),
            "http://100.64.0.1:8787"
        );
        assert!(normalize_server("file:///tmp/notes").is_err());
        assert!(normalize_server("http://user:pass@host").is_err());
    }

    #[test]
    fn store_persists_connection_and_separate_bases() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = SyncStore::open(dir.path());
        store
            .set_connection("http://one.test:8787/", Some(&"x".repeat(32)))
            .expect("connection");
        let mut base = BTreeMap::new();
        base.insert("a.md".to_owned(), "rev-a".to_owned());
        store.save_base(base);
        store
            .set_connection("http://two.test:8787", Some(&"y".repeat(32)))
            .expect("second server");
        assert!(store.base().is_empty());
        let mut other = BTreeMap::new();
        other.insert("b.md".to_owned(), "rev-b".to_owned());
        store.save_base(other);
        store
            .set_connection("http://one.test:8787", Some(&"z".repeat(32)))
            .expect("back");
        assert_eq!(store.base().get("a.md").map(String::as_str), Some("rev-a"));
        assert_eq!(store.token().len(), 32);
    }
}
