//! Shared server state: the vault, the live configuration, the media/AI
//! services and the change broadcaster.
//!
//! Configuration is re-read from `.fed` on every request (like Rails
//! `Config.new`), so edits made by the desktop app, another client or the user
//! take effect immediately.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use frankmd_config::store::ConfigStore;
use frankmd_config::{EnvSnapshot, Key, ResolvedConfig};
use frankmd_fs::{NotesService, Vault};
use frankmd_media::api::MediaApi;
use frankmd_media::{MediaConfig, MediaCtx, MediaEnv, SettingsStore};
use tokio::sync::broadcast;

use crate::auth::AuthConfig;
use crate::events::{BroadcastSink, RemoteEvent};

/// Everything the server needs to start.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    /// Notes root.
    pub vault_root: PathBuf,
    /// Per-vault app state directory (draft storage, private locks).
    pub state_dir: PathBuf,
    /// Process environment snapshot (config resolution).
    pub env: EnvSnapshot,
    /// Already validated, already resolved address.
    pub bind: IpAddr,
    /// TCP port.
    pub port: u16,
    /// Token (>= 32 bytes, validated by [`AuthConfig`]).
    pub token: String,
    /// UI locale override.
    pub locale: Option<String>,
    /// Trash backend override (tests/headless); `None` = the OS trash.
    pub trash_dir: Option<PathBuf>,
}

/// `.fed` as the media layer's [`SettingsStore`] (fresh read per call).
#[derive(Debug)]
pub struct FedSettings {
    store: Mutex<ConfigStore>,
    notes_root: PathBuf,
}

impl FedSettings {
    fn new(vault: Vault, env: EnvSnapshot) -> Self {
        let notes_root = vault.root().to_path_buf();
        Self {
            store: Mutex::new(ConfigStore::open(vault, env)),
            notes_root,
        }
    }

    /// Fresh resolved configuration.
    pub fn resolved(&self) -> Arc<ResolvedConfig> {
        let mut store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        store.reload().unwrap_or_else(|_| store.current())
    }

    /// Re-read `.fed` and return the resolved config (`ConfigStore::reload`).
    pub fn reload(&self) -> Arc<ResolvedConfig> {
        self.resolved()
    }

    /// Run `f` with the configuration store locked (updates, recents).
    pub fn with_store<T>(&self, f: impl FnOnce(&mut ConfigStore) -> T) -> T {
        let mut store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut store)
    }
}

impl SettingsStore for FedSettings {
    fn current(&self) -> MediaConfig {
        MediaConfig::from_resolved(&self.resolved(), &self.notes_root)
    }

    fn set_provider_local(&self) -> bool {
        self.with_store(|store| {
            store
                .update(&[(
                    Key::MediaStorageProvider,
                    Some(frankmd_config::Value::Str("local".to_owned())),
                )])
                .is_ok()
        })
    }
}

/// Media, search and AI services of one remote server.
pub struct MediaStack {
    /// Upload pipeline and per-endpoint facade.
    pub api: MediaApi,
    /// Web image search (DuckDuckGo / Google / Pinterest).
    pub search: frankmd_media::search::ImageSearch,
    /// YouTube Data API.
    pub youtube: frankmd_media::youtube::YoutubeSearch,
    /// Trusted HTTP client (AI providers).
    pub http: frankmd_net::TrustedClient,
    /// Which notes reference which media.
    pub usage: frankmd_media::usage::MediaUsage,
}

impl std::fmt::Debug for MediaStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaStack").finish_non_exhaustive()
    }
}

impl MediaStack {
    fn new(vault: &Vault, settings: Arc<FedSettings>) -> anyhow::Result<Self> {
        let http = frankmd_net::TrustedClient::new()?;
        let egress = frankmd_net::EgressClient::new(frankmd_net::Limits::EXTERNAL_IMAGE)?;
        let env = MediaEnv::new(settings, MediaCtx::system(), http.clone(), egress);
        Ok(Self {
            api: MediaApi::new(env),
            search: frankmd_media::search::ImageSearch::new(http.clone(), Default::default()),
            youtube: frankmd_media::youtube::YoutubeSearch::new(http.clone()),
            http,
            usage: frankmd_media::usage::MediaUsage::new(vault.root()),
        })
    }
}

/// The state cloned into every axum handler.
pub struct ServerState {
    /// The notes root.
    pub vault: Vault,
    /// `.fed` + global config.
    pub settings: Arc<FedSettings>,
    /// Media / search / AI.
    pub media: MediaStack,
    /// Token verifier and login throttle.
    pub auth: AuthConfig,
    /// Live change notifications.
    pub events: broadcast::Sender<RemoteEvent>,
    /// UI locale override.
    pub locale: Option<String>,
    /// The user's home directory (Omarchy theme lookup).
    pub home: Option<PathBuf>,
    /// Startup instant (health/status).
    pub started: std::time::Instant,
    /// Keeps the vault watcher alive for the server's lifetime.
    watchers: Mutex<Option<frankmd_watch::Watchers>>,
}

impl std::fmt::Debug for ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerState")
            .field("vault", &self.vault.root())
            .finish_non_exhaustive()
    }
}

impl ServerState {
    /// Open the vault and wire every service. Does not bind a socket yet.
    pub fn new(config: &RemoteConfig) -> anyhow::Result<Arc<Self>> {
        let mut vault = Vault::open(&config.vault_root, &config.state_dir)?;
        if let Some(dir) = &config.trash_dir {
            vault = vault.with_trash(frankmd_fs::TrashBackend::Directory(dir.clone()));
        }
        let settings = Arc::new(FedSettings::new(vault.clone(), config.env.clone()));
        let media = MediaStack::new(&vault, Arc::clone(&settings))?;
        let auth = AuthConfig::new(&config.token)?;
        let (events, _) = broadcast::channel(256);
        Ok(Arc::new(Self {
            vault,
            settings,
            media,
            auth,
            events,
            locale: config.locale.clone(),
            home: config.env.raw("HOME").map(PathBuf::from),
            started: std::time::Instant::now(),
            watchers: Mutex::new(None),
        }))
    }

    /// Start the vault watcher and push changes to SSE subscribers.
    pub fn start_watchers(&self) -> Result<(), String> {
        let watchers = frankmd_watch::start_with(
            frankmd_watch::WatchConfig::with_system_defaults(self.vault.root().to_path_buf()),
            BroadcastSink::new(self.events.clone()),
        )?;
        *self.watchers.lock().unwrap_or_else(PoisonError::into_inner) = Some(watchers);
        Ok(())
    }

    /// Fresh resolved configuration.
    pub fn resolved(&self) -> Arc<ResolvedConfig> {
        self.settings.resolved()
    }

    /// Fresh media configuration.
    pub fn media_config(&self) -> MediaConfig {
        self.settings.current()
    }

    /// A stateless notes service over this vault.
    pub fn notes(&self) -> NotesService<'_> {
        NotesService::new(&self.vault)
    }

    /// Vault root.
    pub fn root(&self) -> &Path {
        self.vault.root()
    }
}
