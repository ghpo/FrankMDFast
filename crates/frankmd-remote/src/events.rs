//! Vault change notifications → SSE.

use frankmd_watch::{EventSink, WatchEvent};
use serde::Serialize;
use tokio::sync::broadcast;

/// One message pushed to `/api/events`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteEvent {
    /// The tree changed (paths are vault-relative).
    Tree {
        /// Affected paths.
        paths: Vec<String>,
        /// Subset that no longer exists.
        removed: Vec<String>,
    },
    /// The root `.fed` (or global config) changed.
    Config,
    /// The Omarchy palette changed.
    Theme,
    /// Language or fonts changed.
    System,
    /// A watcher reported a problem.
    Error {
        /// Short, path-free message.
        message: String,
    },
}

/// Bridges the `frankmd-watch` callback into the tokio broadcast channel.
#[derive(Debug)]
pub struct BroadcastSink {
    tx: broadcast::Sender<RemoteEvent>,
}

impl BroadcastSink {
    /// Wrap a sender.
    pub fn new(tx: broadcast::Sender<RemoteEvent>) -> Self {
        Self { tx }
    }
}

impl EventSink for BroadcastSink {
    fn send(&self, ev: WatchEvent) -> bool {
        let mapped = match ev {
            WatchEvent::VaultChanged { paths, removed } => RemoteEvent::Tree {
                paths: paths.iter().map(|p| p.as_str().to_owned()).collect(),
                removed: removed.iter().map(|p| p.as_str().to_owned()).collect(),
            },
            WatchEvent::ConfigChanged => RemoteEvent::Config,
            WatchEvent::OmarchyThemeChanged => RemoteEvent::Theme,
            WatchEvent::SystemLanguageChanged | WatchEvent::FontsChanged => RemoteEvent::System,
            WatchEvent::Error(message) => RemoteEvent::Error { message },
        };
        // No subscribers is not an error: keep watching.
        let _ = self.tx.send(mapped);
        true
    }
}
