//! The media services of one vault: the `.fed`-backed settings store, the
//! endpoint facade ([`MediaApi`]), the search clients and the AI service.
//!
//! Every method here does IO and must run off the GPUI main thread (the
//! dialogs call them from `background_executor` or the tokio bridge).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use frankmd_config::store::ConfigStore;
use frankmd_config::{EnvSnapshot, Key, ResolvedConfig, Value};
use frankmd_media::api::MediaApi;
use frankmd_media::vm::picker::PickerConfig;
use frankmd_media::vm::storage::Provider;
use frankmd_media::{MediaConfig, MediaCtx, MediaEnv, SettingsStore};

/// Where the dialogs work: the vault and its state directory.
#[derive(Debug, Clone)]
pub struct MediaContext {
    /// Vault root (`NOTES_PATH`).
    pub vault_root: PathBuf,
    /// Per-vault state directory.
    pub state_dir: PathBuf,
    /// The process environment snapshot (config resolution).
    pub env: EnvSnapshot,
    /// Note/media encryption session.
    pub encryption: Arc<frankmd_crypto::Session>,
}

/// `.fed` as the media layer's [`SettingsStore`]: every `current()` re-reads
/// the file (FrankMD re-read `Config.new` per request, so recovery's switch to
/// `local` and edits made elsewhere are seen at once).
#[derive(Debug)]
pub struct FedSettings {
    store: Mutex<ConfigStore>,
    notes_root: PathBuf,
}

impl FedSettings {
    fn new(vault: frankmd_fs::Vault, env: EnvSnapshot) -> Self {
        let notes_root = vault.root().to_path_buf();
        Self {
            store: Mutex::new(ConfigStore::open(vault, env)),
            notes_root,
        }
    }

    /// A fresh resolved config.
    pub fn resolved(&self) -> Arc<ResolvedConfig> {
        let mut store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        store.reload().unwrap_or_else(|_| store.current())
    }

    /// Persist `media_storage_provider`.
    pub fn set_provider(&self, provider: Provider) -> Result<Arc<ResolvedConfig>, String> {
        let mut store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        let change = [(
            Key::MediaStorageProvider,
            Some(Value::Str(provider.as_setting().to_owned())),
        )];
        store.update(&change).map_err(|e| e.to_string())
    }
}

impl SettingsStore for FedSettings {
    fn current(&self) -> MediaConfig {
        MediaConfig::from_resolved(&self.resolved(), &self.notes_root)
    }

    fn set_provider_local(&self) -> bool {
        self.set_provider(Provider::Local).is_ok()
    }
}

/// Everything the media and AI dialogs need for one vault.
pub struct MediaServices {
    /// The vault.
    pub vault: frankmd_fs::Vault,
    /// `.fed`.
    pub settings: Arc<FedSettings>,
    /// The endpoint facade (uploads, imports, recovery's one-shot warning).
    pub api: MediaApi,
    /// Image search (DuckDuckGo / Google / Pinterest).
    pub search: frankmd_media::search::ImageSearch,
    /// YouTube search.
    pub youtube: frankmd_media::youtube::YoutubeSearch,
    /// Trusted client (AI providers).
    pub http: frankmd_net::TrustedClient,
    /// Usage index (cached by fingerprint).
    pub usage: frankmd_media::usage::MediaUsage,
    /// Note/media encryption session.
    pub encryption: Arc<frankmd_crypto::Session>,
}

impl std::fmt::Debug for MediaServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaServices")
            .field("vault", &self.vault.root())
            .finish_non_exhaustive()
    }
}

impl MediaServices {
    /// Wire the services of `ctx` (opens `.fed` once; no network).
    pub fn new(ctx: &MediaContext) -> anyhow::Result<Arc<Self>> {
        let vault = frankmd_fs::Vault::open(&ctx.vault_root, &ctx.state_dir)?;
        let settings = Arc::new(FedSettings::new(vault.clone(), ctx.env.clone()));
        let http = frankmd_net::TrustedClient::new()?;
        let egress = frankmd_net::EgressClient::new(frankmd_net::Limits::EXTERNAL_IMAGE)?;
        let env = MediaEnv::new(settings.clone(), MediaCtx::system(), http.clone(), egress);
        let search = frankmd_media::search::ImageSearch::new(http.clone(), Default::default());
        let youtube = frankmd_media::youtube::YoutubeSearch::new(http.clone());
        let usage = frankmd_media::usage::MediaUsage::new(vault.root());
        Ok(Arc::new(Self {
            api: MediaApi::new(env),
            vault,
            settings,
            search,
            youtube,
            http,
            usage,
            encryption: Arc::clone(&ctx.encryption),
        }))
    }

    /// The notes root.
    pub fn notes_root(&self) -> &Path {
        self.vault.root()
    }

    /// Fresh media config.
    pub fn media_config(&self) -> MediaConfig {
        self.settings.current()
    }

    /// The Insert image configuration (`GET /images/config` + `GET /ai/image_config`).
    /// Also runs storage recovery (the warning stays pending).
    pub fn picker_config(&self) -> (PickerConfig, StorageSnapshot) {
        let info = self.api.images_config();
        let resolved = self.settings.resolved();
        let ai = frankmd_ai::AiService::new(resolved, self.http.clone()).image_generation_info();
        let config = self.media_config();
        let picker = PickerConfig {
            notes_root: self.notes_root().to_path_buf(),
            images_path: config.images_path.clone(),
            server_images: info.enabled,
            google_enabled: info.google_enabled,
            ai_enabled: ai.enabled,
            ai_model: ai.model,
            image_extensions: info.image_extensions,
        };
        let storage = StorageSnapshot {
            provider: Provider::from_setting(info.media_storage_provider.as_deref()),
            s3_available: info.s3_enabled,
            fallback: self.storage_status().1,
        };
        (picker, storage)
    }

    /// `GET /config`: the provider and whether recovery switched it (consumed).
    pub fn storage_status(&self) -> (Provider, bool) {
        let status = self.api.storage_status();
        (
            Provider::from_setting(status.media_storage_provider.as_deref()),
            status.media_storage_fallback,
        )
    }

    /// Provider + S3 availability + consumed fallback.
    pub fn storage_snapshot(&self) -> StorageSnapshot {
        let (provider, fallback) = self.storage_status();
        let s3_available = frankmd_media::images::s3_enabled(&self.media_config());
        StorageSnapshot {
            provider,
            s3_available,
            fallback,
        }
    }

    /// The AI service over a fresh config.
    pub fn ai(&self) -> frankmd_ai::AiService {
        frankmd_ai::AiService::new(self.settings.resolved(), self.http.clone())
    }
}

/// A read of the storage settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageSnapshot {
    /// Persisted provider.
    pub provider: Provider,
    /// S3 configured.
    pub s3_available: bool,
    /// Recovery switched S3 → Local (announce once).
    pub fallback: bool,
}
