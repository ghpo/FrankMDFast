//! App-wide preferences that belong to this machine, not to a notes folder
//! (so they are neither `.fed` keys nor per-vault UI state): the UI scale and
//! the remote-access startup preference.
//! Stored as `<app state dir>/prefs.json`, written atomically.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name inside the app state dir.
pub const APP_PREFS_FILE: &str = "prefs.json";

/// UI scale bounds in percent.
pub const UI_SCALE_MIN: u16 = 80;
/// See [`UI_SCALE_MIN`].
pub const UI_SCALE_MAX: u16 = 200;

/// Default remote port (kept in sync with `frankmd_remote::DEFAULT_PORT`).
pub const DEFAULT_REMOTE_PORT: u16 = 8787;
/// Bind choices accepted by [`AppPrefs::remote_bind`].
pub const REMOTE_BINDS: [&str; 2] = ["tailscale", "loopback"];

/// The persisted values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPrefs {
    /// Interface size in percent (100 = gpui-kit's default sizes).
    pub ui_scale: u16,
    /// Start the remote server when the app launches.
    pub remote_start: bool,
    /// `"tailscale"` (default) or `"loopback"`.
    pub remote_bind: String,
    /// Remote TCP port.
    pub remote_port: u16,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            ui_scale: 100,
            remote_start: false,
            remote_bind: "tailscale".to_owned(),
            remote_port: DEFAULT_REMOTE_PORT,
        }
    }
}

/// Loads and saves [`AppPrefs`].
#[derive(Debug, Default)]
pub struct AppPrefsStore {
    file: Option<PathBuf>,
    prefs: AppPrefs,
}

impl AppPrefsStore {
    /// Never persisted (tests).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load `file`; missing or malformed → defaults, values clamped.
    pub fn open(file: &Path) -> Self {
        let mut prefs = std::fs::read(file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AppPrefs>(&bytes).ok())
            .unwrap_or_default();
        prefs.ui_scale = prefs.ui_scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX);
        if !REMOTE_BINDS.contains(&prefs.remote_bind.as_str()) {
            prefs.remote_bind = "tailscale".to_owned();
        }
        if prefs.remote_port == 0 {
            prefs.remote_port = DEFAULT_REMOTE_PORT;
        }
        Self {
            file: Some(file.to_path_buf()),
            prefs,
        }
    }

    /// Current values.
    pub fn get(&self) -> &AppPrefs {
        &self.prefs
    }

    /// Set the UI scale (clamped) and persist it when it changed.
    pub fn set_ui_scale(&mut self, percent: u16) {
        let percent = percent.clamp(UI_SCALE_MIN, UI_SCALE_MAX);
        if percent != self.prefs.ui_scale {
            self.prefs.ui_scale = percent;
            self.persist();
        }
    }

    /// Start the remote server on launch.
    pub fn set_remote_start(&mut self, enabled: bool) {
        if enabled != self.prefs.remote_start {
            self.prefs.remote_start = enabled;
            self.persist();
        }
    }

    /// Change the remote bind target (`tailscale` or `loopback`).
    pub fn set_remote_bind(&mut self, bind: &str) {
        if REMOTE_BINDS.contains(&bind) && bind != self.prefs.remote_bind {
            self.prefs.remote_bind = bind.to_owned();
            self.persist();
        }
    }

    /// Change the remote port (`0` resets to the default).
    pub fn set_remote_port(&mut self, port: u16) {
        let port = if port == 0 { DEFAULT_REMOTE_PORT } else { port };
        if port != self.prefs.remote_port {
            self.prefs.remote_port = port;
            self.persist();
        }
    }

    /// Best effort: losing it only costs the preference.
    fn persist(&self) {
        let Some(file) = &self.file else {
            return;
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.prefs) {
            let _ = frankmd_fs::atomic::atomic_replace(
                file,
                &bytes,
                frankmd_fs::atomic::ModePolicy::Fixed(0o600),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_clamps() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join(APP_PREFS_FILE);
        let mut store = AppPrefsStore::open(&file);
        assert_eq!(store.get().ui_scale, 100);
        store.set_ui_scale(150);
        assert_eq!(AppPrefsStore::open(&file).get().ui_scale, 150);
        store.set_ui_scale(999);
        assert_eq!(AppPrefsStore::open(&file).get().ui_scale, UI_SCALE_MAX);
    }

    #[test]
    fn remote_preferences_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join(APP_PREFS_FILE);
        let mut store = AppPrefsStore::open(&file);
        assert!(!store.get().remote_start);
        assert_eq!(store.get().remote_bind, "tailscale");
        assert_eq!(store.get().remote_port, DEFAULT_REMOTE_PORT);

        store.set_remote_start(true);
        store.set_remote_bind("loopback");
        store.set_remote_port(9000);
        let reloaded = AppPrefsStore::open(&file);
        assert!(reloaded.get().remote_start);
        assert_eq!(reloaded.get().remote_bind, "loopback");
        assert_eq!(reloaded.get().remote_port, 9000);

        store.set_remote_bind("nonsense");
        assert_eq!(store.get().remote_bind, "loopback");
        store.set_remote_port(0);
        assert_eq!(store.get().remote_port, DEFAULT_REMOTE_PORT);
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // test fixture setup
    fn malformed_file_gives_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join(APP_PREFS_FILE);
        std::fs::write(&file, b"{not json").expect("write");
        assert_eq!(AppPrefsStore::open(&file).get(), &AppPrefs::default());
    }
}
