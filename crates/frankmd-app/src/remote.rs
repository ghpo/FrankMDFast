//! Remote-access startup: turn the CLI flags into a `frankmd-remote` server.
//!
//! The token precedence is `--serve-token` > `FRANKMD_AUTH_TOKEN` > `.fed
//! auth_token` > `auth_token` from the global config layer. A missing or short
//! token is a hard error: the server never starts unauthenticated.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, bail};
use frankmd_config::dirs::AppDirs;
use frankmd_config::{EnvSnapshot, Key, Value};
use frankmd_fs::Vault;
use frankmd_remote::{RemoteConfig, ServerState};

use crate::cli::{Cli, Launch};

/// A server ready to bind.
#[derive(Clone)]
pub struct RemoteLaunch {
    /// Server state (vault, config, media, auth).
    pub state: Arc<ServerState>,
    /// Address to listen on.
    pub addr: SocketAddr,
    /// URL to print/log.
    pub url: String,
}

/// Resolve flags, token and bind address, then open the vault and services.
pub fn prepare(
    cli: &Cli,
    launch: &Launch,
    env: &EnvSnapshot,
    dirs: &AppDirs,
) -> anyhow::Result<RemoteLaunch> {
    let real = launch
        .vault_root
        .canonicalize()
        .unwrap_or_else(|_| launch.vault_root.clone());
    let state_dir = dirs.vault_state(&real);
    if let Err(err) = crate::app_fs::ensure_private_dir(&state_dir) {
        tracing::warn!(%err, "could not create the remote state directory");
    }
    let token = resolve_token(cli, env, &launch.vault_root, &state_dir, dirs)?;
    let spec = frankmd_remote::bind::parse_bind(&cli.serve_bind)?;
    let bind = frankmd_remote::resolve_bind(spec, cli.serve_bind_any)?;
    let port = cli
        .serve_port
        .or_else(|| {
            env.present("FRANKMD_REMOTE_PORT")
                .and_then(|p| p.parse().ok())
        })
        .unwrap_or(frankmd_remote::DEFAULT_PORT);
    let config = RemoteConfig {
        vault_root: launch.vault_root.clone(),
        state_dir,
        env: env.clone(),
        bind,
        port,
        token,
        locale: cli.locale.clone(),
        trash_dir: None,
    };
    let state = ServerState::new(&config).context("could not open the vault for remote access")?;
    if let Err(err) = state.start_watchers() {
        tracing::warn!(%err, "remote change notifications unavailable");
    }
    let addr = SocketAddr::new(bind, port);
    let url = format!("http://{}", frankmd_remote::bind::describe(bind, port));
    tracing::info!(%url, vault = %launch.vault_root.display(), "remote server ready");
    Ok(RemoteLaunch { state, addr, url })
}

/// Start serving on the app's tokio runtime (the handle is detached).
pub fn start_background(runtime: &frankmd_net::NetRuntime, launch: RemoteLaunch) {
    let url = launch.url.clone();
    runtime.spawn(async move {
        if let Err(err) = frankmd_remote::serve(launch.state, launch.addr).await {
            tracing::error!(%err, %url, "remote server stopped");
        }
    });
}

fn resolve_token(
    cli: &Cli,
    env: &EnvSnapshot,
    vault_root: &Path,
    state_dir: &Path,
    dirs: &AppDirs,
) -> anyhow::Result<String> {
    if let Some(token) = cli.serve_token.as_deref().filter(|t| !t.trim().is_empty()) {
        return Ok(token.to_owned());
    }
    if let Some(token) = env
        .present("FRANKMD_AUTH_TOKEN")
        .filter(|t| !t.trim().is_empty())
    {
        return Ok(token.to_owned());
    }
    let vault = Vault::open(vault_root, state_dir)?;
    let global = frankmd_config::global::global_config_path(dirs);
    let mut store = frankmd_config::ConfigStore::open(vault, env.clone()).with_global(global);
    let resolved = store.reload().unwrap_or_else(|_| store.current());
    if let Some(Value::Str(token)) = resolved.value(Key::AuthToken)
        && !token.trim().is_empty()
    {
        return Ok(token);
    }
    bail!(
        "remote access needs a token of at least 32 bytes: pass --serve-token, \
         export FRANKMD_AUTH_TOKEN, or set `auth_token = ...` in .fed"
    );
}
