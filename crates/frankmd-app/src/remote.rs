//! Remote-access lifecycle: a detached `frankmd --serve-only` daemon.
//!
//! The server does not live inside the GUI process: Settings and startup spawn
//! the same executable with `--serve-only`, so closing the desktop window
//! (SUPER+W) keeps the remote server running. The daemon's PID/URL are stored
//! in `<vault state dir>/remote.json`, which Settings reads to show the status.
//!
//! Token precedence is `--serve-token` > `FRANKMD_AUTH_TOKEN` > `.fed
//! auth_token` > the global config. A missing or short token is a hard error:
//! the server never starts unauthenticated.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context as _, bail};
use frankmd_config::dirs::AppDirs;
use frankmd_config::{EnvSnapshot, Key, Value};
use frankmd_fs::Vault;
use frankmd_remote::{RemoteConfig, ServerState};
use frankmd_workspace::app_prefs::AppPrefs;
use serde::{Deserialize, Serialize};

use crate::cli::{Cli, Launch};

/// Status file inside the per-vault state directory.
const STATUS_FILE: &str = "remote.json";

/// What Settings › Remote shows about the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteStatus {
    /// The daemon is running.
    pub running: bool,
    /// Advertised URL (`http://<ip>:<port>`).
    pub url: String,
    /// TCP port.
    pub port: u16,
    /// Requested bind spec (`tailscale`, `loopback`, an IP, ...).
    pub bind_spec: String,
    /// Daemon PID (0 = never started / stopped).
    pub pid: u32,
    /// Startup/bind failure, if any.
    pub error: Option<String>,
}

impl Default for RemoteStatus {
    fn default() -> Self {
        Self {
            running: false,
            url: String::new(),
            port: frankmd_remote::DEFAULT_PORT,
            bind_spec: "tailscale".to_owned(),
            pid: 0,
            error: None,
        }
    }
}

static TAILSCALE: OnceLock<Option<IpAddr>> = OnceLock::new();

/// Cached Tailscale IPv4 probe (spawning `tailscale` on every repaint is too
/// expensive; the address changes only when Tailscale restarts).
pub fn tailscale_ipv4_cached() -> Option<IpAddr> {
    *TAILSCALE.get_or_init(frankmd_remote::tailscale_ipv4)
}

fn status_file(state_dir: &Path) -> PathBuf {
    state_dir.join(STATUS_FILE)
}

/// Read the daemon status for this vault. A stale PID is reported as stopped.
pub fn status(state_dir: &Path) -> Option<RemoteStatus> {
    let bytes = std::fs::read(status_file(state_dir)).ok()?;
    let mut status: RemoteStatus = serde_json::from_slice(&bytes).ok()?;
    if status.running && !pid_alive(status.pid) {
        status.running = false;
        status.error = status
            .error
            .or_else(|| Some("the remote server process is no longer running".to_owned()));
    }
    Some(status)
}

/// `true` while the daemon for this vault is running.
pub fn is_running(state_dir: &Path) -> bool {
    status(state_dir).is_some_and(|status| status.running)
}

/// Write a failure so Settings can explain why remote access is off.
pub fn record_error(state_dir: &Path, message: &str) {
    let mut status = status(state_dir).unwrap_or_default();
    status.running = false;
    status.pid = 0;
    status.error = Some(message.to_owned());
    write_status(state_dir, &status);
}

/// Everything needed to build or spawn a server (CLI or Settings).
#[derive(Debug, Clone)]
pub struct RemoteRequest {
    /// Notes root.
    pub vault_root: PathBuf,
    /// Per-vault app state directory.
    pub state_dir: PathBuf,
    /// Environment snapshot (used by the in-process daemon).
    pub env: EnvSnapshot,
    /// `tailscale`, `loopback`, `any` or an IP.
    pub bind_spec: String,
    /// Allow a non-Tailscale/non-loopback bind.
    pub bind_any: bool,
    /// TCP port.
    pub port: u16,
    /// Access token.
    pub token: String,
    /// UI locale override.
    pub locale: Option<String>,
}

/// A server ready to bind (used by the `--serve-only` daemon).
#[derive(Clone)]
pub struct RemoteLaunch {
    /// Server state (vault, config, media, auth).
    pub state: Arc<ServerState>,
    /// Address to listen on.
    pub addr: SocketAddr,
    /// URL to print/log.
    pub url: String,
}

use std::sync::Arc;

/// Resolve flags/preferences into a request.
pub fn build_request(
    cli: &Cli,
    launch: &Launch,
    env: &EnvSnapshot,
    dirs: &AppDirs,
    prefs: &AppPrefs,
) -> anyhow::Result<RemoteRequest> {
    let real = launch
        .vault_root
        .canonicalize()
        .unwrap_or_else(|_| launch.vault_root.clone());
    let state_dir = dirs.vault_state(&real);
    if let Err(err) = crate::app_fs::ensure_private_dir(&state_dir) {
        tracing::warn!(%err, "could not create the remote state directory");
    }
    let token = resolve_token(cli, env, &launch.vault_root, &state_dir, dirs)?;
    let bind_spec = cli
        .serve_bind
        .clone()
        .unwrap_or_else(|| prefs.remote_bind.clone());
    let port = cli
        .serve_port
        .or_else(|| {
            env.present("FRANKMD_REMOTE_PORT")
                .and_then(|p| p.parse().ok())
        })
        .unwrap_or(prefs.remote_port);
    Ok(RemoteRequest {
        vault_root: launch.vault_root.clone(),
        state_dir,
        env: env.clone(),
        bind_spec,
        bind_any: cli.serve_bind_any,
        port,
        token,
        locale: cli.locale.clone(),
    })
}

/// Build the in-process server used by `--serve-only`.
pub fn prepare_request(request: &RemoteRequest) -> anyhow::Result<RemoteLaunch> {
    if let Err(err) = crate::app_fs::ensure_private_dir(&request.state_dir) {
        tracing::warn!(%err, "could not create the remote state directory");
    }
    let spec = frankmd_remote::bind::parse_bind(&request.bind_spec)?;
    let bind = frankmd_remote::resolve_bind(spec, request.bind_any)?;
    let config = RemoteConfig {
        vault_root: request.vault_root.clone(),
        state_dir: request.state_dir.clone(),
        env: request.env.clone(),
        bind,
        port: request.port,
        token: request.token.clone(),
        locale: request.locale.clone(),
        trash_dir: None,
    };
    let state = ServerState::new(&config).context("could not open the vault for remote access")?;
    if let Err(err) = state.start_watchers() {
        tracing::warn!(%err, "remote change notifications unavailable");
    }
    let addr = SocketAddr::new(bind, request.port);
    let url = format!(
        "http://{}",
        frankmd_remote::bind::describe(bind, request.port)
    );
    tracing::info!(%url, vault = %request.vault_root.display(), "remote server ready");
    Ok(RemoteLaunch { state, addr, url })
}

/// Record that the `--serve-only` process is serving (called by the daemon).
pub fn write_running_status(state_dir: &Path, pid: u32, url: &str, port: u16, bind_spec: &str) {
    write_status(
        state_dir,
        &RemoteStatus {
            running: true,
            url: url.to_owned(),
            port,
            bind_spec: bind_spec.to_owned(),
            pid,
            error: None,
        },
    );
}

/// Spawn the detached daemon for the CLI/preferences.
pub fn spawn_daemon(
    cli: &Cli,
    launch: &Launch,
    env: &EnvSnapshot,
    dirs: &AppDirs,
    prefs: &AppPrefs,
) -> anyhow::Result<RemoteStatus> {
    let request = build_request(cli, launch, env, dirs, prefs)?;
    spawn_daemon_request(&request)
}

/// Spawn `current_exe --serve-only` for `request`, unless the same daemon is
/// already running.
pub fn spawn_daemon_request(request: &RemoteRequest) -> anyhow::Result<RemoteStatus> {
    if let Some(existing) = status(&request.state_dir)
        && existing.running
    {
        if existing.bind_spec == request.bind_spec && existing.port == request.port {
            return Ok(existing);
        }
        stop_daemon(&request.state_dir);
    }

    let spec = frankmd_remote::bind::parse_bind(&request.bind_spec)?;
    let bind = frankmd_remote::resolve_bind(spec, request.bind_any)?;
    let url = format!(
        "http://{}",
        frankmd_remote::bind::describe(bind, request.port)
    );
    let exe = std::env::current_exe().context("could not locate the FrankMDFast executable")?;

    let mut command = std::process::Command::new(exe);
    command
        .arg("--serve-only")
        .arg("--serve-bind")
        .arg(&request.bind_spec)
        .arg("--serve-port")
        .arg(request.port.to_string())
        .arg(&request.vault_root)
        .env("FRANKMD_AUTH_TOKEN", &request.token)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if request.bind_any {
        command.arg("--serve-bind-any");
    }
    if let Some(locale) = &request.locale {
        command.arg("--locale").arg(locale);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Keep the daemon alive when the GUI's process group/terminal goes
        // away; it stays a child and is reparented when the GUI exits.
        command.process_group(0);
    }

    let mut child = command
        .spawn()
        .context("could not start the remote server process")?;
    let pid = child.id();

    // Give the daemon a moment to bind; a bad port/token exits here.
    std::thread::sleep(Duration::from_millis(250));
    if let Ok(Some(exit)) = child.try_wait() {
        let message = format!("the remote server process exited during startup ({exit})");
        let failed = RemoteStatus {
            running: false,
            url,
            port: request.port,
            bind_spec: request.bind_spec.clone(),
            pid,
            error: Some(message.clone()),
        };
        write_status(&request.state_dir, &failed);
        bail!(message);
    }

    let status = RemoteStatus {
        running: true,
        url,
        port: request.port,
        bind_spec: request.bind_spec.clone(),
        pid,
        error: None,
    };
    write_status(&request.state_dir, &status);
    Ok(status)
}

/// Stop the daemon for this vault (SIGTERM) and mark it stopped.
pub fn stop_daemon(state_dir: &Path) {
    let mut status = status(state_dir).unwrap_or_default();
    if status.running && pid_alive(status.pid) {
        let _ = std::process::Command::new("kill")
            .arg(status.pid.to_string())
            .status();
        for _ in 0..20 {
            if !pid_alive(status.pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    status.running = false;
    status.pid = 0;
    status.error = None;
    write_status(state_dir, &status);
}

fn write_status(state_dir: &Path, status: &RemoteStatus) {
    if let Err(err) = crate::app_fs::ensure_private_dir(state_dir) {
        tracing::warn!(%err, "could not create the remote state directory");
        return;
    }
    let Ok(bytes) = serde_json::to_vec_pretty(status) else {
        return;
    };
    if let Err(err) = frankmd_fs::atomic::atomic_replace(
        &status_file(state_dir),
        &bytes,
        frankmd_fs::atomic::ModePolicy::Fixed(0o600),
    ) {
        tracing::warn!(%err, "could not write the remote status file");
    }
}

/// Is `pid` this fork's `--serve-only` daemon? Guards against PID reuse.
#[cfg(target_os = "linux")]
fn pid_alive(pid: u32) -> bool {
    pid != 0
        && std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|cmdline| {
            cmdline
                .split(|byte| *byte == 0)
                .any(|arg| arg == b"--serve-only")
        })
}

#[cfg(not(target_os = "linux"))]
fn pid_alive(pid: u32) -> bool {
    pid != 0
        && std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .is_ok_and(|status| status.success())
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
        "remote access needs a token of at least 32 bytes: Settings › Remote → Access token, \
         export FRANKMD_AUTH_TOKEN, or set `auth_token = ...` in .fed"
    );
}
