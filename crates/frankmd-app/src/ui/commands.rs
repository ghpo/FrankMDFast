//! Registry commands (HP-018): every shortcut, palette row, button and vim
//! ex command ends in [`Shell::run_command`].

use frankmd_workspace::commands::CommandContext;

use super::prelude::*;
use gpui_kit::*;

impl Shell {
    /// Run registry command `id` (from a shortcut, the palette or a button).
    pub fn run_command(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        // FE-029 changed: inside Settings only Esc and Ctrl+, work.
        if self.overlays.is_open(&OverlayKind::Settings)
            && !matches!(id, "app.settings" | "app.escape" | "list.escape")
        {
            return;
        }
        if !matches!(
            id,
            "app.escape" | "list.escape" | "list.confirm" | "list.select_next" | "list.select_prev"
        ) {
            self.recent_commands.retain(|c| *c != id);
            self.recent_commands.insert(0, id);
            self.recent_commands.truncate(20);
        }
        let doc = self
            .core
            .as_ref()
            .and_then(|c| c.ws.open_doc())
            .map(|d| d.id);
        match id {
            "app.command_palette" => {
                self.toggle_overlay(OverlayKind::Palette(PaletteMode::Commands), window, cx)
            }
            "file.quick_open" => self.toggle_overlay(OverlayKind::QuickOpen, window, cx),
            "theme.select" => {
                self.open_overlay(OverlayKind::Palette(PaletteMode::Theme), window, cx)
            }
            "language.select" => {
                self.open_overlay(OverlayKind::Palette(PaletteMode::Language), window, cx)
            }
            "app.settings" => self.toggle_overlay(OverlayKind::Settings, window, cx),
            "app.diagnostics" => {
                self.settings.category = super::settings::Category::Diagnostics;
                self.open_overlay(OverlayKind::Settings, window, cx);
            }
            "app.settings_remote" => {
                self.settings.category = super::settings::Category::Remote;
                self.open_overlay(OverlayKind::Settings, window, cx);
            }
            "app.settings_encryption" => {
                self.settings.category = super::settings::Category::Encryption;
                self.open_overlay(OverlayKind::Settings, window, cx);
            }
            "file.encrypt" => {
                if let Some(path) = self.active_path() {
                    self.request_note_encryption(&path, true, window, cx);
                }
            }
            "file.decrypt" => {
                if let Some(path) = self.active_path() {
                    self.request_note_encryption(&path, false, window, cx);
                }
            }
            "app.encryption_toggle" => {
                if self.encryption.seal().is_none() {
                    self.open_encryption_settings(window, cx);
                } else if self.encryption.is_unlocked() {
                    self.lock_vault(window, cx);
                } else {
                    self.open_unlock(window, cx);
                }
            }
            "help.toggle" => {
                self.help_tab = HelpTab::Markdown;
                self.toggle_overlay(OverlayKind::Help, window, cx);
            }
            "help.keyboard_shortcuts" => {
                self.help_tab = HelpTab::Keyboard;
                self.open_overlay(OverlayKind::Help, window, cx);
            }
            "app.escape" | "list.escape" => self.escape(window, cx),
            "list.confirm" => self.list_confirm(window, cx),
            "list.select_next" => self.list_step(true, window, cx),
            "list.select_prev" => self.list_step(false, window, cx),
            "app.quit" => self.quit(window, cx),
            "file.save" => {
                if let Some(doc) = doc {
                    self.dispatch(Command::SaveNow { doc }, window, cx);
                }
            }
            "file.close" => {
                self.dispatch(Command::CloseNote, window, cx);
                self.focus_region(FocusRegion::Editor, window, cx);
            }
            "view.toggle_sidebar" => {
                self.layout.toggle_sidebar();
                if self.layout.width_class == WidthClass::Narrow {
                    if self.layout.drawer_open {
                        self.sidebar_focus.focus(window, cx);
                    } else {
                        self.focus_region(FocusRegion::Editor, window, cx);
                    }
                } else {
                    let visible = self.layout.sidebar_visible;
                    self.update_settings(
                        vec![(Key::SidebarVisible, Some(Value::Bool(visible)))],
                        window,
                        cx,
                    );
                }
                self.ensure_focus_visible(window, cx);
            }
            "view.toggle_preview" => {
                let was_swapped = self.arrangement().preview == PreviewMode::Swapped;
                self.layout.toggle_preview();
                if was_swapped {
                    self.focus_region(FocusRegion::Editor, window, cx);
                }
                self.ensure_focus_visible(window, cx);
            }
            "view.toggle_zen" => {
                self.layout.toggle_zen();
                self.ensure_focus_visible(window, cx);
            }
            "view.media_library" => {
                self.layout.toggle_media_library();
                self.media_library_changed(window, cx);
                self.focus_region(FocusRegion::Editor, window, cx);
            }
            "insert.image" if self.media.hub.is_some() => self.open_insert_image(window, cx),
            "insert.video" if self.media.hub.is_some() => self.open_insert_video(window, cx),
            "ai.grammar_check" if self.media.grammar.is_some() => {
                self.start_grammar_check(window, cx)
            }
            "view.toggle_typewriter" => {
                let on = !self.layout.typewriter;
                self.update_settings(
                    vec![(Key::TypewriterMode, Some(Value::Bool(on)))],
                    window,
                    cx,
                );
            }
            "view.toggle_scroll_sync" => {
                let on = !self.scroll_sync;
                self.update_settings(vec![(Key::ScrollSync, Some(Value::Bool(on)))], window, cx);
            }
            "editor.toggle_vim" => {
                let on = !self.vim;
                self.update_settings(vec![(Key::VimMode, Some(Value::Bool(on)))], window, cx);
            }
            "focus.sidebar" => self.focus_region(FocusRegion::Sidebar, window, cx),
            "focus.editor" => self.focus_region(FocusRegion::Editor, window, cx),
            "focus.preview" => self.focus_region(FocusRegion::Preview, window, cx),
            "focus.next_region" | "focus.prev_region" => {
                let arrangement = self.arrangement();
                let from = self.focused_region(window, cx).unwrap_or(self.last_region);
                let to = from.step(&arrangement, false, id == "focus.next_region");
                self.focus_region(to, window, cx);
            }
            "file.copy_path" => {
                if let Some(path) = self.active_path() {
                    cx.write_to_clipboard(ClipboardItem::new_string(path));
                    let text = i18n::t(cx, "status.copied_to_clipboard");
                    self.show_message(&text, false, cx);
                }
            }
            "file.copy_contents" => {
                if doc.is_some() {
                    cx.write_to_clipboard(ClipboardItem::new_string(self.editor.text(cx)));
                    let text = i18n::t(cx, "status.copied_to_clipboard");
                    self.show_message(&text, false, cx);
                }
            }
            "file.reveal" => {
                if let Some(path) = self.active_path() {
                    self.reveal_in_tree(&path);
                    self.explorer.set_cursor(Some(path));
                    self.focus_region(FocusRegion::Sidebar, window, cx);
                }
            }
            "file.new_note" => {
                // Quick open's "Ctrl+N creates <query>" (§5.2): pre-fill the name.
                let prefill = match self.overlays.current() {
                    Some(OverlayKind::QuickOpen) => self.quick_open.model.create_suggestion(),
                    None if self.active_path().is_none() => {
                        self.quick_open.model.create_suggestion()
                    }
                    _ => None,
                };
                let prefill = prefill.map(|n| n.trim_end_matches(".md").to_owned());
                self.new_note_global(prefill.as_deref(), window, cx);
            }
            "file.new_folder" => self.new_folder_global(window, cx),
            "file.rename" => self.rename_active(window, cx),
            "file.move" => {
                if let Some(path) = self.active_path() {
                    self.open_move(
                        &path,
                        frankmd_workspace::explorer::ItemKind::File,
                        window,
                        cx,
                    );
                }
            }
            "file.delete" => self.delete_active(window, cx),
            "search.in_files" => {
                if self.overlays.is_open(&OverlayKind::SearchInFiles) {
                    self.close_overlay(window, cx);
                } else {
                    self.open_search(window, cx);
                }
            }
            "nav.back" => self.navigate_history(false, window, cx),
            "nav.forward" => self.navigate_history(true, window, cx),
            "file.recent_next" => self.recent_switch(true, window, cx),
            "file.recent_prev" => self.recent_switch(false, window, cx),
            "edit.go_to_line" => {
                self.open_overlay(OverlayKind::Palette(PaletteMode::GoToLine), window, cx)
            }
            "view.zoom_in" | "view.zoom_out" | "view.zoom_reset"
                if self.preview_focus.contains_focused(window, cx) =>
            {
                self.preview.update(cx, |p, cx| match id {
                    "view.zoom_in" => p.zoom_in(cx),
                    "view.zoom_out" => p.zoom_out(cx),
                    _ => p.reset_zoom(cx),
                });
            }
            _ => {
                if !self.editor.run_command(id, window, cx) {
                    self.not_available(id, cx);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn not_available(&mut self, id: &str, cx: &mut Context<Self>) {
        let title = self
            .registry
            .get(id)
            .map_or_else(|| id.to_owned(), |c| c.title());
        let text = i18n::t_args(cx, "palette.not_available", &[("command", &title)]);
        self.show_message(&text, false, cx);
    }

    pub(crate) fn toggle_overlay(
        &mut self,
        kind: OverlayKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.is_open(&kind) {
            self.close_overlay(window, cx);
        } else {
            self.open_overlay(kind, window, cx);
        }
    }

    /// Ctrl+Q: drafts make quitting safe; flush them and quit (§5.23).
    pub(crate) fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(Command::FlushDrafts, window, cx);
        cx.quit();
    }

    /// ↑/↓ in the open list.
    pub(crate) fn list_step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        match self.overlays.current() {
            Some(OverlayKind::Palette(_)) => self.palette_step(forward, window, cx),
            Some(OverlayKind::QuickOpen) | None => {
                if forward {
                    self.quick_open.model.select_next();
                } else {
                    self.quick_open.model.select_prev();
                }
            }
            Some(OverlayKind::SearchInFiles) => {
                if forward {
                    self.search.model.select_next();
                } else {
                    self.search.model.select_prev();
                }
            }
            Some(OverlayKind::Dialog("recent-switcher")) => self.switcher.step(forward),
            _ => {}
        }
        cx.notify();
    }

    /// Enter in the open list.
    pub(crate) fn list_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.overlays.current() {
            Some(OverlayKind::Palette(_)) => self.palette_confirm(window, cx),
            Some(OverlayKind::QuickOpen) | None => {
                if let Some(path) = self.quick_open.model.selected_path().map(str::to_owned) {
                    self.overlays.close();
                    self.open_path(&path, window, cx);
                }
            }
            Some(OverlayKind::SearchInFiles) => self.search_activate(false, window, cx),
            Some(OverlayKind::Dialog("recent-switcher")) => self.recent_switch_commit(window, cx),
            _ => {}
        }
    }

    /// A banner button was pressed (`key` is the button's i18n key).
    pub(crate) fn banner_action(
        &mut self,
        kind: BannerKind,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match key {
            "banners.content_loss.undo" => {
                self.banners.clear(BannerKind::ContentLoss);
                self.dispatch(
                    Command::ContentLossUndo {
                        undid_editor_change: false,
                    },
                    window,
                    cx,
                );
            }
            "banners.content_loss.save_anyway" => {
                self.banners.clear(BannerKind::ContentLoss);
                self.dispatch(Command::ContentLossSaveAnyway, window, cx);
            }
            "banners.changed_on_disk.compare" => self.open_compare(window, cx),
            "banners.changed_on_disk.reload" => {
                self.banners.clear(kind);
                if let Some(path) = self.active_path() {
                    self.dispatch(Command::CloseNote, window, cx);
                    self.open_path(&path, window, cx);
                }
            }
            "banners.deleted_on_disk.close" => {
                self.banners.clear(kind);
                self.dispatch(Command::CloseNote, window, cx);
            }
            "banners.deleted_on_disk.save_again" => {
                self.banners.clear(kind);
                if let Some(doc) = self
                    .core
                    .as_ref()
                    .and_then(|c| c.ws.open_doc())
                    .map(|d| d.id)
                {
                    self.dispatch(Command::SaveNow { doc }, window, cx);
                }
            }
            "banners.offline.retry" => {
                if self.vault_root.is_dir() {
                    self.banners.clear(kind);
                }
            }
            "banners.storage_fallback.dismiss" => self.banners.dismiss(kind),
            "banners.storage_fallback.settings" => {
                self.open_overlay(OverlayKind::Settings, window, cx)
            }
            "banners.draft_storage.details" => {
                self.settings.category = super::settings::Category::Diagnostics;
                self.open_overlay(OverlayKind::Settings, window, cx);
            }
            _ => {}
        }
        if self.overlays.current().is_none() {
            self.focus_region(FocusRegion::Editor, window, cx);
        }
        cx.notify();
    }

    /// Commands listener for the root element.
    pub(crate) fn command_handler(cx: &mut Context<Self>) -> super::actions::CommandHandler {
        let this = cx.entity().downgrade();
        Rc::new(move |id, window, cx| {
            let _ = this.update(cx, |shell, cx| shell.run_command(id, window, cx));
        })
    }

    /// Context for palette filtering.
    pub(crate) fn command_context(&self) -> CommandContext {
        let open = self.active_path();
        CommandContext {
            markdown_open: open.as_deref().is_some_and(|p| p.ends_with(".md")),
            caret_in_table: false,
            vim: self.vim,
            note_open: open.is_some(),
            zen: self.layout.zen,
            preview_visible: self.layout.preview_visible,
            sidebar_visible: self.layout.sidebar_visible,
            typewriter: self.layout.typewriter,
            scroll_sync: self.scroll_sync,
        }
    }
}
