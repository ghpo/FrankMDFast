//! Wiring of the dialogs that live outside `src/ui` (docs/testing/
//! wiring-media.md, wiring-insert.md): the media surfaces (insert image,
//! insert video, media library, storage provider), the grammar check, and
//! the editor's requests (links, wikilinks, `/image`, `/video`). Those
//! views render themselves; the shell creates them, opens them in the
//! overlay layer or the main area, and answers their events.

use frankmd_core::text::pending::PendingAction;
use frankmd_media::vm::storage::Provider;

use crate::dialogs::ai::{GrammarDialog, GrammarEvent};
use crate::dialogs::media::{MediaContext, MediaEvent, MediaHub};
use crate::editor::EditorRequest;

use super::prelude::*;
use gpui_kit::*;

/// Overlay ids of the hosted dialogs (`OverlayKind::Dialog`).
pub(crate) const INSERT_IMAGE: &str = "insert-image";
/// Insert video.
pub(crate) const INSERT_VIDEO: &str = "insert-video";
/// Grammar check result.
pub(crate) const GRAMMAR: &str = "grammar";

/// The hosted surfaces of one window (absent when the vault couldn't open).
pub(crate) struct MediaHost {
    pub(crate) hub: Option<MediaHub>,
    pub(crate) grammar: Option<Entity<GrammarDialog>>,
    /// "Checking grammar…" banner text while a check runs.
    pub(crate) grammar_banner: Option<SharedString>,
}

impl MediaHost {
    /// Create the views and subscribe the shell to them.
    pub(crate) fn new(
        ctx: Option<MediaContext>,
        window: &mut Window,
        cx: &mut Context<Shell>,
    ) -> (Self, Vec<Subscription>) {
        let mut subs = Vec::new();
        let hub = ctx.and_then(|ctx| match MediaHub::new(&ctx, window, cx) {
            Ok(hub) => Some(hub),
            Err(err) => {
                tracing::warn!(%err, "media dialogs unavailable");
                None
            }
        });
        let grammar = hub.as_ref().map(|hub| {
            let services = hub.services.clone();
            cx.new(|cx| GrammarDialog::new(services, window, cx))
        });
        if let Some(hub) = &hub {
            subs.push(cx.subscribe_in(
                &hub.picker,
                window,
                |shell, _, ev: &MediaEvent, window, cx| shell.on_media_event(ev, window, cx),
            ));
            subs.push(cx.subscribe_in(
                &hub.video,
                window,
                |shell, _, ev: &MediaEvent, window, cx| shell.on_media_event(ev, window, cx),
            ));
            subs.push(cx.subscribe_in(
                &hub.library,
                window,
                |shell, _, ev: &MediaEvent, window, cx| shell.on_media_event(ev, window, cx),
            ));
            subs.push(cx.subscribe_in(
                &hub.storage,
                window,
                |shell, _, ev: &crate::dialogs::media::storage::StorageEvent, _, cx| {
                    if *ev == crate::dialogs::media::storage::StorageEvent::Fallback {
                        shell.raise_banner(BannerKind::StorageFallback, cx);
                    }
                    cx.notify();
                },
            ));
        }
        if let Some(grammar) = &grammar {
            subs.push(cx.subscribe_in(
                grammar,
                window,
                |shell, _, ev: &GrammarEvent, window, cx| shell.on_grammar_event(ev, window, cx),
            ));
        }
        (
            Self {
                hub,
                grammar,
                grammar_banner: None,
            },
            subs,
        )
    }
}

impl Shell {
    /// Subscribe to the editor's requests (second event type of `EditorPane`).
    pub(crate) fn subscribe_editor_requests(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        let pane = self.editor.pane.clone();
        cx.subscribe_in(
            &pane,
            window,
            |shell, _, req: &EditorRequest, window, cx| shell.on_editor_request(req, window, cx),
        )
    }

    fn on_editor_request(
        &mut self,
        req: &EditorRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match req {
            EditorRequest::OpenLink(url) => self.open_link(url, window, cx),
            EditorRequest::OpenWikilink(target) => self.open_wikilink(target, window, cx),
            EditorRequest::ViewImage(_) => {
                // The viewer lives in the preview (§5.20): show it there.
                if self.arrangement().preview == PreviewMode::Hidden {
                    self.layout.toggle_preview();
                }
                self.focus_region(FocusRegion::Preview, window, cx);
            }
            EditorRequest::OpenMediaDialog(PendingAction::Image) => {
                self.open_insert_image(window, cx)
            }
            EditorRequest::OpenMediaDialog(PendingAction::Video) => {
                self.open_insert_video(window, cx)
            }
            EditorRequest::OpenMediaDialog(_) => {}
        }
    }

    /// Ctrl+. on a link: http(s)/mailto go to the system opener (app brief
    /// SECURITY allowlist); a relative link opens the note it names.
    fn open_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let lower = url.trim().to_ascii_lowercase();
        if lower.starts_with("https://")
            || lower.starts_with("http://")
            || lower.starts_with("mailto:")
        {
            cx.open_url(url.trim());
            return;
        }
        if lower.contains(':') {
            return; // file:, javascript:, … are never followed.
        }
        let target = url.split(['#', '?']).next().unwrap_or("");
        let base = self
            .active_path()
            .and_then(|p| p.rsplit_once('/').map(|(dir, _)| dir.to_owned()))
            .unwrap_or_default();
        let joined = if base.is_empty() || target.starts_with('/') {
            target.trim_start_matches('/').to_owned()
        } else {
            format!("{base}/{target}")
        };
        let cleaned = frankmd_core::path::clean_path(&joined);
        if self.quick_open_candidates().contains(&cleaned) {
            self.open_path(&cleaned, window, cx);
        }
    }

    /// Note context for the editor's completions and slash menu.
    pub(crate) fn update_editor_context(&mut self, cx: &mut Context<Self>) {
        let path = self.active_path();
        let markdown = path.as_deref().is_some_and(|p| p.ends_with(".md"));
        let notes = self.quick_open_candidates();
        let locale = i18n::current(cx).locale().code().to_owned();
        self.editor.pane.update(cx, |pane, cx| {
            pane.set_note_context(path, markdown, cx);
            pane.set_vault_notes(notes);
            pane.set_ui_locale(Some(locale));
        });
        if let Some(hub) = &self.media.hub {
            let note = self.active_path();
            hub.library.update(cx, |l, _| l.set_note(note));
        }
    }

    // ----- media dialogs ---------------------------------------------------------------

    /// Ctrl+Shift+I, `/image`, the header button.
    pub(crate) fn open_insert_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.media.hub.as_ref().map(|h| h.picker.clone()) else {
            return;
        };
        let note = self.active_path();
        self.open_overlay(OverlayKind::Dialog(INSERT_IMAGE), window, cx);
        picker.update(cx, |p, cx| p.open(note, window, cx));
    }

    /// `/video`, palette "Insert: Video…".
    pub(crate) fn open_insert_video(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(video) = self.media.hub.as_ref().map(|h| h.video.clone()) else {
            return;
        };
        self.open_overlay(OverlayKind::Dialog(INSERT_VIDEO), window, cx);
        video.update(cx, |v, cx| v.open(window, cx));
    }

    /// Pasted or dropped files go to Insert image › Upload (FE-358…361).
    pub(crate) fn open_insert_image_with_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.media.hub.as_ref().map(|h| h.picker.clone()) else {
            return;
        };
        let note = self.active_path();
        self.open_overlay(OverlayKind::Dialog(INSERT_IMAGE), window, cx);
        picker.update(cx, |p, cx| p.open_with_paths(note, paths, window, cx));
    }

    /// Paste: an image on the clipboard opens Insert image › Upload; text
    /// pastes normally. `true` when the paste was taken.
    pub(crate) fn paste_image(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        use crate::dialogs::media::files;
        let Some(picker) = self.media.hub.as_ref().map(|h| h.picker.clone()) else {
            return false;
        };
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let Some(file) = files::clipboard_image(&item, files::now_ms()) else {
            return false;
        };
        let note = self.active_path();
        self.open_overlay(OverlayKind::Dialog(INSERT_IMAGE), window, cx);
        picker.update(cx, |p, cx| p.open_with_files(note, vec![file], window, cx));
        true
    }

    /// The media library opened or closed (`view.media_library`).
    pub(crate) fn media_library_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layout.media_library {
            return;
        }
        if let Some(library) = self.media.hub.as_ref().map(|h| h.library.clone()) {
            let note = self.active_path();
            library.update(cx, |l, cx| l.open(note, window, cx));
        }
    }

    /// The library view, when the media layer is available.
    pub(crate) fn media_library_view(&self) -> Option<AnyView> {
        self.media.hub.as_ref().map(|h| h.library.clone().into())
    }

    /// The overlay content of a hosted dialog.
    pub(crate) fn hosted_dialog(&self, kind: &str) -> Option<AnyElement> {
        let hub = self.media.hub.as_ref()?;
        match kind {
            INSERT_IMAGE => Some(hub.picker.clone().into_any_element()),
            INSERT_VIDEO => Some(hub.video.clone().into_any_element()),
            GRAMMAR => self
                .media
                .grammar
                .clone()
                .map(IntoElement::into_any_element),
            _ => None,
        }
    }

    /// Backdrop clicks close a hosted dialog unless it holds edits (§5).
    pub(crate) fn hosted_dismissible(&self, kind: &str, cx: &App) -> bool {
        match kind {
            GRAMMAR => !self
                .media
                .grammar
                .as_ref()
                .is_some_and(|g| g.read(cx).state().has_edits()),
            _ => true,
        }
    }

    fn on_media_event(&mut self, ev: &MediaEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            MediaEvent::Insert(insert) => {
                let slash = self
                    .editor
                    .pane
                    .read(cx)
                    .pending_insertion()
                    .map(|p| p.range.clone());
                let pane = self.editor.pane.clone();
                pane.update(cx, |p, _| p.clear_pending_insertion());
                if self.layout.media_library {
                    self.layout.close_media_library();
                }
                self.overlays.close();
                crate::dialogs::media::apply_insert(&pane, insert, slash, window, cx);
                self.focus_region(FocusRegion::Editor, window, cx);
            }
            MediaEvent::Closed => {
                self.editor
                    .pane
                    .update(cx, |p, _| p.clear_pending_insertion());
                if self.overlays.current().is_some() {
                    self.close_overlay(window, cx);
                }
            }
            MediaEvent::Status { text, error } => self.show_message(text, *error, cx),
            MediaEvent::StorageFallback => self.raise_banner(BannerKind::StorageFallback, cx),
            MediaEvent::OpenSettings => self.open_settings_at_integrations(window, cx),
            MediaEvent::OpenNote(path) => {
                if self.layout.media_library {
                    self.layout.close_media_library();
                }
                self.overlays.close();
                self.open_path(path, window, cx);
                self.reveal_in_tree(path);
            }
            MediaEvent::CloseLibrary => {
                self.layout.close_media_library();
                self.focus_region(FocusRegion::Editor, window, cx);
            }
        }
        cx.notify();
    }

    fn open_settings_at_integrations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlays.close();
        self.settings.category = super::settings::Category::Integrations;
        self.open_overlay(OverlayKind::Settings, window, cx);
    }

    // ----- grammar check (§5.10) ----------------------------------------------------------

    /// Ctrl+Shift+G.
    pub(crate) fn start_grammar_check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(grammar) = self.media.grammar.clone() else {
            return;
        };
        let note = self.active_path();
        let text = self.editor.text(cx);
        grammar.update(cx, |g, cx| g.start(note, text, window, cx));
    }

    fn on_grammar_event(&mut self, ev: &GrammarEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            GrammarEvent::FlushAutosave => {
                if let Some(doc) = self
                    .core
                    .as_ref()
                    .and_then(|c| c.ws.open_doc())
                    .map(|d| d.id)
                {
                    self.dispatch(Command::SaveNow { doc }, window, cx);
                }
            }
            GrammarEvent::SetReadOnly(on) => {
                let on = *on;
                self.editor.pane.update(cx, |p, cx| p.set_read_only(on, cx));
            }
            GrammarEvent::Banner(text) => self.media.grammar_banner = text.clone(),
            GrammarEvent::Show => self.open_overlay(OverlayKind::Dialog(GRAMMAR), window, cx),
            GrammarEvent::Apply(text) => {
                let old = self.editor.text(cx);
                let edit = crate::editor::Edit {
                    range: 0..old.len(),
                    text: text.clone(),
                    selection: 0..0,
                }
                .minimized(&old);
                self.overlays.close();
                self.editor
                    .pane
                    .update(cx, |p, cx| p.apply_edit(edit, window, cx));
                self.focus_region(FocusRegion::Editor, window, cx);
            }
            GrammarEvent::Closed => {
                if self.overlays.is_open(&OverlayKind::Dialog(GRAMMAR)) {
                    self.close_overlay(window, cx);
                }
            }
            GrammarEvent::Status { text, error } => self.show_message(text, *error, cx),
            GrammarEvent::OpenSettings => self.open_settings_at_integrations(window, cx),
            GrammarEvent::Retry => self.start_grammar_check(window, cx),
        }
        cx.notify();
    }

    /// Cancel a running grammar check (banner button / Esc).
    pub(crate) fn cancel_grammar_check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(grammar) = self.media.grammar.clone() {
            grammar.update(cx, |g, cx| g.cancel_request(window, cx));
        }
    }

    /// `.fed` changed: the media and AI layers re-read it.
    pub(crate) fn media_config_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(hub) = &self.media.hub {
            hub.storage.update(cx, |s, cx| s.refresh(cx));
        }
        if let Some(grammar) = &self.media.grammar {
            grammar.update(cx, |g, cx| g.reload_config(cx));
        }
    }

    // ----- Settings › Media › Storage for new media ---------------------------------------

    /// `(provider, controls disabled, S3 unavailable reason)` of the storage model.
    pub(crate) fn storage_row(&self, cx: &App) -> Option<(Provider, bool, Option<SharedString>)> {
        let hub = self.media.hub.as_ref()?;
        let state = hub.storage.read(cx).state();
        let reason = state
            .s3_disabled_reason()
            .map(|t| crate::dialogs::media::widgets::tr(cx, &t));
        Some((state.provider(), state.controls_disabled(), reason))
    }

    /// Change the provider through the storage model (saves immediately).
    pub(crate) fn request_storage(&mut self, provider: Provider, cx: &mut Context<Self>) {
        if let Some(hub) = &self.media.hub {
            hub.storage.update(cx, |s, cx| s.request(provider, cx));
        }
    }
}
