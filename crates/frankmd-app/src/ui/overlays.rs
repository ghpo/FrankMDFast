//! Overlay stack wiring (ui-spec §2, §5): open/close with focus restore and
//! the Esc chain.

use super::prelude::*;
use gpui_kit::*;

impl Shell {
    /// Open `kind` (replacing any open overlay) and focus it.
    pub(crate) fn open_overlay(
        &mut self,
        kind: OverlayKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = self.focused_region(window, cx).unwrap_or(self.last_region);
        self.overlays.open(kind.clone(), from);
        match kind {
            OverlayKind::Palette(mode) => self.palette.open(mode, window, cx),
            OverlayKind::QuickOpen => {
                let input = self.quick_open.input.clone();
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                    input.focus(window, cx);
                });
                self.quick_open.model.set_query("");
            }
            OverlayKind::Settings => self.settings.open(window, cx),
            OverlayKind::Dialog("unlock") => {
                self.unlock_input
                    .update(cx, |input, cx| input.focus(window, cx));
            }
            OverlayKind::Help | OverlayKind::Dialog(_) | OverlayKind::SearchInFiles => {
                self.overlay_focus.focus(window, cx)
            }
        }
        cx.notify();
    }

    /// Close the overlay and give focus back.
    pub(crate) fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.overlays.current() {
            Some(OverlayKind::Palette(_)) => self.palette_cancel(cx),
            Some(OverlayKind::Dialog("recent-switcher")) => self.switcher.cancel(),
            Some(OverlayKind::Dialog(super::file_dialogs::NEW_NOTE)) => self.files.new_note = None,
            Some(OverlayKind::Dialog(super::file_dialogs::NEW_FOLDER)) => {
                self.files.new_folder = None
            }
            Some(OverlayKind::Dialog(super::file_dialogs::MOVE)) => self.files.move_form = None,
            Some(OverlayKind::Dialog(super::file_dialogs::DELETE)) => self.files.delete = None,
            Some(OverlayKind::Dialog("unlock")) => {
                self.unlock_error = None;
                self.pending_attachment = None;
                self.pending_encryption = None;
            }
            Some(OverlayKind::Dialog("attachments")) => self.attachments = None,
            _ => {}
        }
        if let Some(region) = self.overlays.close() {
            self.focus_region(region.fallback(&self.arrangement()), window, cx);
        }
        cx.notify();
    }

    /// Esc: inline layers first (§6.1 chain); never leaves zen.
    pub(crate) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Inline rename → popover/menu → overlay → drawer → library → selection.
        if self.files.rename.is_some() {
            self.cancel_inline_rename(window, cx);
            return;
        }
        if self.explorer.context_menu().is_some() {
            self.explorer.close_context_menu();
            cx.notify();
            return;
        }
        if self
            .overlays
            .is_open(&OverlayKind::Dialog("recent-switcher"))
        {
            self.switcher.cancel();
        }
        if let Some(kind) = self.overlays.current() {
            if kind.closes_on_escape() {
                self.close_overlay(window, cx);
            }
            return;
        }
        if self.media.grammar_banner.is_some() {
            self.cancel_grammar_check(window, cx);
            return;
        }
        if self.layout.close_drawer() {
            self.focus_region(FocusRegion::Editor, window, cx);
            return;
        }
        if self.layout.media_library {
            self.layout.close_media_library();
            self.focus_region(FocusRegion::Editor, window, cx);
            return;
        }
        if self.explorer.selection().len() > 1 {
            self.explorer.clear_selection();
        }
        cx.notify();
    }
}
