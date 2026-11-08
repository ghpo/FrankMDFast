//! Opening notes (ui-spec §5.2, §6.1): open by path, wikilinks, `:n`/`:prev`,
//! back/forward history and the Ctrl+Tab recent switcher.

use super::prelude::*;
use gpui_kit::*;

impl Shell {
    pub(crate) fn open_initial(
        &mut self,
        arg: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(core) = self.core.as_ref() else {
            self.focus_region(FocusRegion::Editor, window, cx);
            return;
        };
        let root = core.vault.root().to_path_buf();
        let real = core.vault.real_root().to_path_buf();
        let vault = core.vault.clone();
        let target = frankmd_workspace::startup::resolve_initial_note(
            &[root.as_path(), real.as_path()],
            arg,
            |p| vault.note_exists(p.as_str()),
        );
        match target {
            frankmd_workspace::startup::StartupTarget::Note(path) => {
                self.open_note(path, window, cx)
            }
            frankmd_workspace::startup::StartupTarget::Missing(path) => {
                let name = path.as_str().trim_end_matches(".md").to_owned();
                self.show_message(
                    i18n::t_args(cx, "status.note_not_found", &[("name", &name)]).as_ref(),
                    false,
                    cx,
                );
                self.focus_region(FocusRegion::Editor, window, cx);
                self.quick_open
                    .input
                    .update(cx, |input, cx| input.set_value(name, window, cx));
            }
            frankmd_workspace::startup::StartupTarget::None => {
                self.focus_region(FocusRegion::Editor, window, cx)
            }
        }
    }

    /// Open `path` and give the editor focus.
    pub(crate) fn open_note(&mut self, path: RelPath, window: &mut Window, cx: &mut Context<Self>) {
        if frankmd_core::path::is_config_file(path.as_str()) {
            let message = i18n::t(cx, "errors.failed_to_load");
            self.show_message(&message, true, cx);
            return;
        }
        if !path.as_str().ends_with(frankmd_core::path::NOTE_EXT) {
            let abs = self.vault_root.join(path.as_str());
            if abs.is_file() {
                self.open_attachments(Some(abs), window, cx);
            } else {
                let message = i18n::t(cx, "errors.not_a_note");
                self.show_message(&message, true, cx);
            }
            return;
        }
        let locked = self
            .core
            .as_ref()
            .and_then(|core| core.vault.note_encryption(path.as_str()).ok())
            .is_some_and(|(_, locked)| locked);
        if locked {
            if self.active_path().is_some() {
                self.dispatch(Command::CloseNote, window, cx);
            }
            self.locked_note = Some(path.as_str().to_owned());
            if self.overlays.current().is_some() {
                self.overlays.close();
            }
            self.focus_region(FocusRegion::Editor, window, cx);
            self.update_title(window);
            cx.notify();
            return;
        }
        self.locked_note = None;
        self.dispatch(Command::OpenNote { path }, window, cx);
        if self.overlays.current().is_some() {
            self.overlays.close();
        }
        self.focus_region(FocusRegion::Editor, window, cx);
    }

    /// Open the note at vault-relative `path` (string form).
    pub(crate) fn open_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        match RelPath::parse(path) {
            Ok(p) => self.open_note(p, window, cx),
            Err(_) => tracing::warn!(path, "invalid note path"),
        }
    }

    /// `[[target]]`: open the matching note, or the note-not-found flow.
    pub(crate) fn open_wikilink(
        &mut self,
        target: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let link = frankmd_core::wikilink::extract(&format!("[[{target}]]"))
            .into_iter()
            .next();
        let found = link.and_then(|link| {
            self.quick_open_candidates().into_iter().find(|p| {
                RelPath::parse(p).is_ok_and(|rel| frankmd_core::wikilink::resolves_to(&link, &rel))
            })
        });
        match found {
            Some(path) => self.open_path(&path, window, cx),
            None => {
                let text = i18n::t_args(cx, "status.note_not_found", &[("name", target)]);
                self.show_message(&text, false, cx);
                self.open_overlay(OverlayKind::QuickOpen, window, cx);
                let target = target.to_owned();
                self.quick_open
                    .input
                    .update(cx, |input, cx| input.set_value(target.clone(), window, cx));
                self.quick_open.model.set_query(&target);
            }
        }
    }

    /// `:n` / `:prev`: next or previous note in tree order (wrapping).
    pub(crate) fn step_note(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let notes = self.quick_open_candidates();
        if notes.is_empty() {
            return;
        }
        let at = self
            .active_path()
            .and_then(|p| notes.iter().position(|n| *n == p));
        let len = notes.len();
        let next = match at {
            Some(i) if forward => (i + 1) % len,
            Some(i) => (i + len - 1) % len,
            None => 0,
        };
        let path = notes[next].clone();
        self.open_path(&path, window, cx);
    }
}

impl Shell {
    /// A note became active: explorer selection, history, MRU, backlinks.
    pub(crate) fn on_note_opened(&mut self, path: &str, cx: &mut Context<Self>) {
        use frankmd_workspace::explorer::{Item, ItemKind};
        self.locked_note = None;
        self.reveal_in_tree(path);
        self.explorer.select_only(Item {
            path: path.to_owned(),
            kind: ItemKind::File,
            encrypted: false,
        });
        self.explorer.set_cursor(Some(path.to_owned()));
        if !self.navigating {
            self.history.visit(Some(path));
        }
        self.switcher.touch(path);
        self.files.rename = None;
        self.refresh_quick_open(cx);
        self.refresh_backlinks(cx);
        self.update_editor_context(cx);
    }

    /// The active note closed (Ctrl+W, deleted).
    pub(crate) fn on_note_closed(&mut self, cx: &mut Context<Self>) {
        if !self.navigating {
            self.history.visit(None);
        }
        self.files.rename = None;
        self.backlinks.clear();
        cx.notify();
    }

    /// Paths moved under live state (rename, move, folder rename).
    pub(crate) fn on_remapped(
        &mut self,
        remaps: &[frankmd_core::registry::Remap],
        cx: &mut Context<Self>,
    ) {
        for remap in remaps {
            let (from, to) = (remap.from.as_str(), remap.to.as_str());
            self.history.remap(from, to);
            self.switcher.remap(from, to);
        }
        self.refresh_backlinks(cx);
    }

    /// Expand the ancestors of `path` and persist the expansion.
    pub(crate) fn reveal_in_tree(&mut self, path: &str) {
        self.explorer.reveal(path);
        self.persist_expanded();
    }

    /// Store the explorer's expanded folders (ui-spec §5.14: persisted per vault).
    pub(crate) fn persist_expanded(&mut self) {
        let expanded: Vec<String> = self.explorer.expanded().iter().cloned().collect();
        self.ui_state.update(|s| s.expanded = expanded);
    }

    /// The tree was rescanned: quick open, explorer pruning, backlinks.
    pub(crate) fn on_tree_changed(&mut self, cx: &mut Context<Self>) {
        let paths: std::collections::HashSet<String> = self.tree_paths().into_iter().collect();
        self.explorer.prune(|p| paths.contains(p));
        self.refresh_quick_open(cx);
        self.refresh_backlinks(cx);
        self.update_editor_context(cx);
    }

    /// Every file and folder path of the tree.
    pub(crate) fn tree_paths(&self) -> Vec<String> {
        fn walk(nodes: &[frankmd_core::tree::TreeNode], out: &mut Vec<String>) {
            for node in nodes {
                match node {
                    frankmd_core::tree::TreeNode::Folder { path, children, .. } => {
                        out.push(path.as_str().to_owned());
                        walk(children, out);
                    }
                    frankmd_core::tree::TreeNode::File { path, .. } => {
                        out.push(path.as_str().to_owned())
                    }
                }
            }
        }
        let mut out = Vec::new();
        if let Some(core) = self.core.as_ref() {
            walk(&core.ws.snapshot().tree, &mut out);
        }
        out
    }

    /// Ctrl+Alt+← / → (ui-spec §6.1): walk the note history; past the first
    /// entry is the empty state.
    pub(crate) fn navigate_history(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = if forward {
            self.history.forward()
        } else {
            self.history.back()
        };
        let Some(target) = target else {
            return;
        };
        self.navigating = true;
        match target {
            Some(path) => self.open_path(&path, window, cx),
            None => {
                self.dispatch(Command::CloseNote, window, cx);
                self.focus_region(FocusRegion::Editor, window, cx);
            }
        }
        self.navigating = false;
    }

    /// Open `path` with the caret at the start of 1-based `line`, centred
    /// (search results, backlinks; FE-295).
    pub(crate) fn open_at_line(
        &mut self,
        path: &str,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_path(path, window, cx);
        if self.active_path().as_deref() != Some(path) {
            return;
        }
        let text = self.editor.text(cx);
        let offset: usize = text
            .split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::len)
            .sum();
        let offset = offset.min(text.len());
        self.editor.pane.update(cx, |pane, cx| {
            pane.set_selection(offset..offset, cx);
            pane.scroll_to_line(line.saturating_sub(1), cx);
        });
        self.focus_region(FocusRegion::Editor, window, cx);
    }

    // ----- Ctrl+Tab recent switcher (ui-spec §5.2) --------------------------------

    /// Ctrl+Tab / Ctrl+Shift+Tab: open the switcher or move its selection.
    pub(crate) fn recent_switch(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.switcher.mru().is_empty() {
            return;
        }
        if self
            .overlays
            .is_open(&OverlayKind::Dialog("recent-switcher"))
        {
            self.switcher.step(forward);
        } else {
            self.switcher.start(forward);
            self.open_overlay(OverlayKind::Dialog("recent-switcher"), window, cx);
        }
        cx.notify();
    }

    /// Ctrl released or Enter: open the selected note.
    pub(crate) fn recent_switch_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.switcher.commit();
        self.overlays.close();
        match target {
            Some(path) => self.open_path(&path, window, cx),
            None => self.focus_region(self.last_region, window, cx),
        }
        cx.notify();
    }
}
