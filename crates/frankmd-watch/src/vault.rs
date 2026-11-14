//! Recursive, debounced watcher on the notes root.

use crate::classify::{PathClass, classify};
use crate::emit::{Emitter, describe};
use crate::{SharedSelfWrites, WatchEvent};
use frankmd_core::path::CONFIG_FILE;
use frankmd_core::{RelPath, Revision};
use notify_debouncer_full::notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, DebouncedEvent, new_debouncer};
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// How often worker threads check the stop flag.
pub(crate) const POLL: Duration = Duration::from_millis(50);

struct VaultCtx {
    root: PathBuf,
    canonical_root: PathBuf,
    self_writes: SharedSelfWrites,
    emitter: Arc<Emitter>,
}

/// Arm the vault watcher synchronously and run it on a worker thread.
pub(crate) fn spawn(
    root: PathBuf,
    debounce: Duration,
    self_writes: SharedSelfWrites,
    emitter: Arc<Emitter>,
) -> Result<JoinHandle<()>, String> {
    if !root.is_dir() {
        return Err("vault watcher: notes root is not a directory".to_owned());
    }
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(debounce, None, tx).map_err(|e| describe("vault", &e))?;
    debouncer
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| describe("vault", &e))?;
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
    let ctx = VaultCtx {
        root,
        canonical_root,
        self_writes,
        emitter,
    };
    std::thread::Builder::new()
        .name("frankmd-watch-vault".to_owned())
        .spawn(move || {
            let _keep_alive = debouncer;
            while !ctx.emitter.is_stopped() {
                match rx.recv_timeout(POLL) {
                    Ok(Ok(events)) => ctx.handle_batch(&events),
                    Ok(Err(errors)) => {
                        for err in errors {
                            ctx.emitter.emit(WatchEvent::Error(describe("vault", &err)));
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .map_err(|e| format!("vault watcher: {}", e.kind()))
}

impl VaultCtx {
    fn handle_batch(&self, events: &[DebouncedEvent]) {
        let roots = [self.root.as_path(), self.canonical_root.as_path()];
        let mut touched = BTreeSet::new();
        let mut config_touched = false;
        let mut rescan = false;
        for event in events {
            rescan |= event.need_rescan();
            for path in &event.paths {
                match classify(&roots, path) {
                    PathClass::Ignore => {}
                    PathClass::Config => config_touched = true,
                    PathClass::Vault(rel) => {
                        touched.insert(rel);
                    }
                }
            }
        }

        if config_touched && !self.is_own(Path::new(CONFIG_FILE)) {
            self.emitter.emit(WatchEvent::ConfigChanged);
        }

        let mut paths = Vec::new();
        let mut removed = Vec::new();
        if rescan {
            // A rescan covers every path; reporting the individual batch as
            // well only bloats the SSE payload and makes the web client
            // reload the tree for nothing. Consumers treat the root as
            // "rescan everything".
            paths.push(RelPath::root());
        } else {
            for rel in touched {
                if self.is_own(Path::new(rel.as_str())) {
                    continue;
                }
                if !self.exists(&rel) {
                    removed.push(rel.clone());
                }
                paths.push(rel);
            }
        }
        if !paths.is_empty() {
            self.emitter
                .emit(WatchEvent::VaultChanged { paths, removed });
        }
    }

    fn abs(&self, rel: &Path) -> PathBuf {
        self.root.join(rel)
    }

    fn exists(&self, rel: &RelPath) -> bool {
        match self.abs(Path::new(rel.as_str())).symlink_metadata() {
            Ok(_) => true,
            Err(e) => e.kind() != ErrorKind::NotFound,
        }
    }

    /// `true` when the current state of `rel` matches a [`crate::SelfWrites`]
    /// token (which is then consumed).
    fn is_own(&self, rel: &Path) -> bool {
        let lock = || {
            self.self_writes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        };
        if !lock().has_pending(rel) {
            return false;
        }
        // Read outside the lock so the workspace is never blocked on our I/O.
        match std::fs::read(self.abs(rel)) {
            Ok(bytes) => lock().is_own(rel, &Revision::of(&bytes)),
            Err(e) if e.kind() == ErrorKind::NotFound => lock().is_own_removal(rel),
            Err(_) => false,
        }
    }
}
