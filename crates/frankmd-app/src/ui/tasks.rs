//! Background work of a window: tree scans, autosave ticks, file watchers,
//! Hyprland radius and editor font lookups. Results come back as commands.

use frankmd_fs::Vault;

use super::appearance;
use super::prelude::*;
use gpui_kit::*;

impl Shell {
    pub(crate) fn start_watchers(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = async_channel::unbounded::<frankmd_watch::WatchEvent>();
        struct Sink(async_channel::Sender<frankmd_watch::WatchEvent>);
        impl frankmd_watch::EventSink for Sink {
            fn send(&self, ev: frankmd_watch::WatchEvent) -> bool {
                self.0.send_blocking(ev).is_ok()
            }
        }
        let config = frankmd_watch::WatchConfig::with_system_defaults(self.vault_root.clone());
        match frankmd_watch::start_with(config, Sink(tx)) {
            Ok(watchers) => {
                if let Some(core) = self.core.as_mut() {
                    core.ws.set_self_writes(watchers.self_writes());
                }
                self.watchers = Some(watchers);
            }
            Err(err) => {
                tracing::warn!(%err, "file watchers unavailable");
                return;
            }
        }
        self.spawn_watch_loop(rx, cx);
    }

    pub(crate) fn load_hyprland_radius(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let radius = cx
                .background_executor()
                .spawn(async { appearance::hyprland_rounding() })
                .await;
            if let Some(radius) = radius {
                let _ = this.update(cx, |this, cx| {
                    this.radius = radius;
                    let tokens = this.theme.tokens.clone();
                    this.apply_tokens(&tokens, cx);
                });
            }
        })
        .detach();
    }

    pub(crate) fn load_editor_font(&mut self, cx: &mut Context<Self>) {
        let id = self
            .core
            .as_ref()
            .and_then(|c| c.ws.snapshot().config.value(Key::EditorFont))
            .and_then(|v| v.as_str().map(str::to_owned));
        self.editor_font_id = Some(id.clone());
        let window = self.window;
        cx.spawn(async move |this, cx| {
            let font = cx
                .background_executor()
                .spawn(async move { super::fonts::resolve_editor_font(id.as_deref()) })
                .await;
            let _ = window.update(cx, |_, window, cx| {
                let _ = this.update(cx, |shell, cx| {
                    shell.editor_font = font.map(SharedString::from);
                    shell.apply_settings(window, cx);
                });
            });
        })
        .detach();
    }

    pub(crate) fn spawn_scan(
        &self,
        vault: Vault,
        generation: frankmd_core::Generation,
        cx: &mut Context<Self>,
    ) {
        let window = self.window;
        cx.spawn(async move |this, cx| {
            let tree = cx
                .background_executor()
                .spawn(async move { vault.list_tree().unwrap_or_default() })
                .await;
            let _ = window.update(cx, |_, window, cx| {
                let _ = this.update(cx, |shell, cx| {
                    shell.dispatch(Command::TreeScanned { generation, tree }, window, cx)
                });
            });
        })
        .detach();
    }

    pub(crate) fn spawn_tick(
        &self,
        delay: std::time::Duration,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let window = self.window;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = window.update(cx, |_, window, cx| {
                let _ = this.update(cx, |shell, cx| shell.dispatch(Command::Tick, window, cx));
            });
        })
    }

    pub(crate) fn spawn_watch_loop(
        &self,
        rx: async_channel::Receiver<frankmd_watch::WatchEvent>,
        cx: &mut Context<Self>,
    ) {
        let window = self.window;
        cx.spawn(async move |this, cx| {
            while let Ok(ev) = rx.recv().await {
                tracing::debug!(?ev, "watch event");
                let stop = window
                    .update(cx, |_, window, cx| {
                        this.update(cx, |shell, cx| match ev {
                            frankmd_watch::WatchEvent::OmarchyThemeChanged
                            | frankmd_watch::WatchEvent::SystemLanguageChanged
                            | frankmd_watch::WatchEvent::FontsChanged
                            | frankmd_watch::WatchEvent::Error(_) => {
                                shell.apply_system(ev, window, cx)
                            }
                            other => shell.dispatch(Command::External(other), window, cx),
                        })
                        .is_err()
                    })
                    .unwrap_or(true);
                if stop {
                    break;
                }
            }
        })
        .detach();
    }

    /// Polling fallback for the open note. The inotify watcher can miss events
    /// (overflow, network filesystems, another process replacing the file) and
    /// the web client can save while the desktop is idle. A single `stat` per
    /// second is cheap; the note is only re-read when `mtime` or size changes.
    pub(crate) fn spawn_open_doc_poll(&self, cx: &mut Context<Self>) {
        let window = self.window;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let stop = window
                    .update(cx, |_, window, cx| {
                        this.update(cx, |shell, cx| shell.poll_open_doc(window, cx))
                            .is_err()
                    })
                    .unwrap_or(true);
                if stop {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll_open_doc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .core
            .as_ref()
            .and_then(|c| c.ws.open_doc())
            .map(|d| d.path.as_str().to_owned())
        else {
            self.open_doc_poll = None;
            return;
        };
        let abs = self.vault_root.join(&path);
        let Ok(meta) = std::fs::metadata(&abs) else {
            self.open_doc_poll = None;
            return;
        };
        let key = (path.clone(), meta.modified().ok(), meta.len());
        if self.open_doc_poll.as_ref() == Some(&key) {
            return;
        }
        self.open_doc_poll = Some(key);
        let Ok(bytes) = std::fs::read(&abs) else {
            return;
        };
        // Our own autosave just replaced the file: the editor already holds
        // these bytes, so dispatching would only reset the caret and undo.
        if bytes.as_slice() == self.editor.text(cx).as_bytes() {
            return;
        }
        let Ok(rel) = frankmd_core::RelPath::parse(&path) else {
            return;
        };
        self.dispatch(
            Command::External(frankmd_watch::WatchEvent::VaultChanged {
                paths: vec![rel],
                removed: Vec::new(),
            }),
            window,
            cx,
        );
    }
}
