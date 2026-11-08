//! Explorer view model: expansion, selection, context menu, creation
//! destination and drag & drop for the file tree. GUI-free; the app renders
//! rows from `frankmd_core::tree_view::tree_rows` and feeds events here.

use frankmd_core::path::is_config_file;
use std::collections::BTreeSet;

/// Whether a tree item is a note/file or a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A file (note or config).
    File,
    /// A folder.
    Folder,
}

/// A vault-relative path with its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Vault-relative path.
    pub path: String,
    /// File or folder.
    pub kind: ItemKind,
    /// File is stored in a FrankMDFast encryption envelope.
    pub encrypted: bool,
}

/// Modifier keys held during a click.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    /// Ctrl (Cmd) held.
    pub ctrl: bool,
    /// Shift held.
    pub shift: bool,
}

/// What the app must do after a click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerEffect {
    /// Open the note at this path.
    OpenNote(String),
    /// Toggle the expansion of this folder (caller calls `toggle_folder`).
    ToggleFolder(String),
}

/// What a context menu was opened on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    /// A tree row.
    Row(Item),
    /// Empty tree space.
    Background,
}

/// An entry of a context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    /// Create a note (in the folder, or at the root for the background).
    NewNote,
    /// Create a folder.
    NewFolder,
    /// Rename the target.
    Rename,
    /// Move the target to another folder.
    MoveTo,
    /// Delete the target.
    Delete,
    /// Select every visible row.
    SelectAll,
    /// Open quick open.
    FindFile,
    /// Collapse every folder.
    CollapseAll,
    /// Encrypt the selected note (per-note opt-in).
    Encrypt,
    /// Decrypt the selected note.
    Decrypt,
}

/// An open context menu and its anchor point.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    /// What it was opened on.
    pub target: MenuTarget,
    /// Window x of the click.
    pub x: f32,
    /// Window y of the click.
    pub y: f32,
}

/// What the pointer hovers while dragging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropHover {
    /// A folder row.
    Folder(String),
    /// A file row (drops into its folder).
    File(String),
    /// Empty tree space (drops at the root).
    Background,
}

/// A resolved drop destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropTarget {
    /// Into this folder.
    Folder(String),
    /// Into the vault root.
    Root,
}

/// The moves a drop performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovePlan {
    /// Paths to move, in order.
    pub items: Vec<String>,
    /// Destination folder (`""` = root).
    pub dest: String,
}

/// The Explorer state for one window.
#[derive(Debug, Default)]
pub struct Explorer {
    expanded: BTreeSet<String>,
    selection: Vec<Item>,
    anchor: Option<String>,
    cursor: Option<String>,
    context_menu: Option<ContextMenu>,
    drag: Vec<Item>,
}

fn parent_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// `path` is strictly inside `folder` (segment-wise).
fn is_inside(path: &str, folder: &str) -> bool {
    path.strip_prefix(folder)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn remap_path(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        Some(to.to_string())
    } else if is_inside(path, from) {
        Some(format!("{to}{}", &path[from.len()..]))
    } else {
        None
    }
}

fn is_same_or_inside(path: &str, folder: &str) -> bool {
    path == folder || is_inside(path, folder)
}

impl Explorer {
    /// Expanded folder paths.
    pub fn expanded(&self) -> &BTreeSet<String> {
        &self.expanded
    }

    /// Replaces the expanded set (restoring persisted state).
    pub fn set_expanded(&mut self, it: impl IntoIterator<Item = String>) {
        self.expanded = it.into_iter().collect();
    }

    /// Toggles a folder's expansion.
    pub fn toggle_folder(&mut self, path: &str) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_string());
        }
    }

    /// Expands a folder.
    pub fn expand(&mut self, path: &str) {
        self.expanded.insert(path.to_string());
    }

    /// Collapses a folder.
    pub fn collapse(&mut self, path: &str) {
        self.expanded.remove(path);
    }

    /// Collapses every folder.
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }

    /// Expands every ancestor folder of `path` (not `path` itself).
    pub fn reveal(&mut self, path: &str) {
        let mut parent = parent_of(path);
        while !parent.is_empty() {
            self.expanded.insert(parent.to_string());
            parent = parent_of(parent);
        }
    }

    /// The keyboard cursor row.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Moves the keyboard cursor.
    pub fn set_cursor(&mut self, path: Option<String>) {
        self.cursor = path;
    }

    /// Handles a row click; see the module docs for modifier semantics.
    pub fn click(
        &mut self,
        item: Item,
        mods: Modifiers,
        visible: &[Item],
    ) -> Option<ExplorerEffect> {
        self.context_menu = None;
        self.cursor = Some(item.path.clone());
        if mods.shift {
            self.select_range(item, visible, mods.ctrl);
            return None;
        }
        if mods.ctrl {
            self.toggle_selected(item);
            return None;
        }
        let effect = match item.kind {
            ItemKind::File => ExplorerEffect::OpenNote(item.path.clone()),
            ItemKind::Folder => ExplorerEffect::ToggleFolder(item.path.clone()),
        };
        self.select_only(item);
        Some(effect)
    }

    fn select_range(&mut self, to: Item, visible: &[Item], additive: bool) {
        let pos = |p: &str| visible.iter().position(|i| i.path == p);
        let range = self
            .anchor
            .as_deref()
            .and_then(pos)
            .zip(pos(&to.path))
            .map(|(a, b)| &visible[a.min(b)..=a.max(b)]);
        let Some(range) = range else {
            self.select_only(to);
            return;
        };
        let mut next = if additive {
            self.selection.clone()
        } else {
            Vec::new()
        };
        for item in range {
            if !next.iter().any(|i| i.path == item.path) {
                next.push(item.clone());
            }
        }
        self.selection = next;
    }

    /// Selects only `item` and makes it the anchor.
    pub fn select_only(&mut self, item: Item) {
        self.anchor = Some(item.path.clone());
        self.selection = vec![item];
    }

    /// Toggles `item` in the selection and makes it the anchor.
    pub fn toggle_selected(&mut self, item: Item) {
        self.anchor = Some(item.path.clone());
        if let Some(i) = self.selection.iter().position(|s| s.path == item.path) {
            self.selection.remove(i);
        } else {
            self.selection.push(item);
        }
    }

    /// Shift+Arrow: selects the range from the anchor to `to`.
    pub fn extend_selection(&mut self, to: Item, visible: &[Item]) {
        self.cursor = Some(to.path.clone());
        self.select_range(to, visible, false);
    }

    /// Selects every visible row.
    pub fn select_all(&mut self, visible: &[Item]) {
        self.selection = visible.to_vec();
        self.anchor = visible.first().map(|i| i.path.clone());
        self.cursor = visible.last().map(|i| i.path.clone());
    }

    /// Clears the selection.
    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.anchor = None;
    }

    /// A click on empty tree space: clears the selection only.
    pub fn click_background(&mut self) {
        self.clear_selection();
    }

    /// Whether `path` is selected.
    pub fn is_selected(&self, path: &str) -> bool {
        self.selection.iter().any(|i| i.path == path)
    }

    /// The selection in insertion order.
    pub fn selection(&self) -> Vec<Item> {
        self.selection.clone()
    }

    /// Right-click. Config rows get no menu; an unselected row is reselected.
    pub fn open_context_menu(&mut self, target: MenuTarget, x: f32, y: f32) {
        if let MenuTarget::Row(item) = &target {
            if is_config_file(&item.path) {
                self.context_menu = None;
                return;
            }
            if !self.is_selected(&item.path) {
                self.select_only(item.clone());
            }
            self.cursor = Some(item.path.clone());
        }
        self.context_menu = Some(ContextMenu { target, x, y });
    }

    /// The open context menu, if any.
    pub fn context_menu(&self) -> Option<&ContextMenu> {
        self.context_menu.as_ref()
    }

    /// Closes the context menu.
    pub fn close_context_menu(&mut self) {
        self.context_menu = None;
    }

    /// The entries of the menu for `target`.
    pub fn menu_items(target: &MenuTarget) -> Vec<MenuItem> {
        use MenuItem::*;
        match target {
            MenuTarget::Row(Item {
                kind: ItemKind::File,
                encrypted,
                ..
            }) => {
                if *encrypted {
                    vec![Decrypt, Rename, MoveTo, Delete]
                } else {
                    vec![Encrypt, Rename, MoveTo, Delete]
                }
            }
            MenuTarget::Row(_) => vec![NewNote, NewFolder, Rename, MoveTo, Delete],
            MenuTarget::Background => vec![NewNote, NewFolder, SelectAll, FindFile, CollapseAll],
        }
    }

    /// Items a menu action applies to: the whole selection when `row` is
    /// selected, else just `row`.
    pub fn context_targets(&self, row: &Item) -> Vec<Item> {
        if self.is_selected(&row.path) {
            Self::delete_targets(&self.selection)
        } else {
            Self::delete_targets(std::slice::from_ref(row))
        }
    }

    /// Destination folder for New note/folder: the single selected folder,
    /// else the root (`""`).
    pub fn creation_folder(&self) -> String {
        match self.selection.as_slice() {
            [
                Item {
                    path,
                    kind: ItemKind::Folder,
                    ..
                },
            ] => path.clone(),
            _ => String::new(),
        }
    }

    /// De-duplicates, drops config files and items inside another listed folder.
    pub fn delete_targets(items: &[Item]) -> Vec<Item> {
        let mut out: Vec<Item> = Vec::new();
        for item in items {
            let covered = items
                .iter()
                .any(|f| f.kind == ItemKind::Folder && is_inside(&item.path, &f.path));
            if is_config_file(&item.path) || covered || out.iter().any(|o| o.path == item.path) {
                continue;
            }
            out.push(item.clone());
        }
        out
    }

    /// Starts dragging `grabbed`; config items cannot be dragged.
    pub fn begin_drag(&mut self, grabbed: Item) {
        self.drag = if is_config_file(&grabbed.path) {
            Vec::new()
        } else if self.is_selected(&grabbed.path) && self.selection.len() > 1 {
            Self::delete_targets(&self.selection)
        } else {
            vec![grabbed]
        };
    }

    /// Whether a drag is in progress.
    pub fn is_dragging(&self) -> bool {
        !self.drag.is_empty()
    }

    /// The dragged items.
    pub fn drag_payload(&self) -> &[Item] {
        &self.drag
    }

    /// Number shown on the drag badge.
    pub fn drag_badge_count(&self) -> usize {
        self.drag.len()
    }

    /// Resolves where a drop on `hover` would go, or `None` when invalid.
    pub fn drop_target(&self, hover: &DropHover) -> Option<DropTarget> {
        if self.drag.is_empty() {
            return None;
        }
        let dest = match hover {
            DropHover::Folder(p) => p.as_str(),
            DropHover::File(p) => parent_of(p),
            DropHover::Background => "",
        };
        let blocked = self
            .drag
            .iter()
            .any(|d| d.kind == ItemKind::Folder && is_same_or_inside(dest, &d.path));
        let all_there = self.drag.iter().all(|d| parent_of(&d.path) == dest);
        if blocked || all_there {
            return None;
        }
        Some(if dest.is_empty() {
            DropTarget::Root
        } else {
            DropTarget::Folder(dest.to_string())
        })
    }

    /// Drops on `hover`, ending the drag. `None` when the drop is invalid.
    pub fn drop(&mut self, hover: &DropHover) -> Option<MovePlan> {
        let target = self.drop_target(hover);
        let payload = std::mem::take(&mut self.drag);
        let dest = match target? {
            DropTarget::Folder(p) => p,
            DropTarget::Root => String::new(),
        };
        let items = payload
            .into_iter()
            .filter(|d| parent_of(&d.path) != dest)
            .map(|d| d.path)
            .collect();
        Some(MovePlan { items, dest })
    }

    /// Cancels the drag.
    pub fn end_drag(&mut self) {
        self.drag.clear();
    }

    /// Follows a rename/move of `from` to `to` (segment-wise prefix remap).
    pub fn remap(&mut self, from: &str, to: &str) {
        let map = |p: &str| remap_path(p, from, to);
        self.expanded = self
            .expanded
            .iter()
            .map(|p| map(p).unwrap_or_else(|| p.clone()))
            .collect();
        for item in &mut self.selection {
            if let Some(n) = map(&item.path) {
                item.path = n;
            }
        }
        for slot in [&mut self.anchor, &mut self.cursor] {
            if let Some(n) = slot.as_deref().and_then(map) {
                *slot = Some(n);
            }
        }
    }

    /// Forgets `path` and everything below it.
    pub fn remove(&mut self, path: &str) {
        let gone = |p: &str| is_same_or_inside(p, path);
        self.expanded.retain(|p| !gone(p));
        self.selection.retain(|i| !gone(&i.path));
        self.anchor.take_if(|p| gone(p));
        self.cursor.take_if(|p| gone(p));
    }

    /// After a rescan, drops selection/cursor/anchor entries that vanished.
    pub fn prune(&mut self, exists: impl Fn(&str) -> bool) {
        self.selection.retain(|i| exists(&i.path));
        self.anchor.take_if(|p| !exists(p));
        self.cursor.take_if(|p| !exists(p));
    }
}

/// Type-ahead: the next visible item after `from` (wrapping) whose display
/// name (last segment without `.md`) starts with `prefix`, case-insensitively.
pub fn type_ahead(visible: &[Item], from: Option<&str>, prefix: &str) -> Option<String> {
    let prefix = prefix.to_lowercase();
    let start = from
        .and_then(|f| visible.iter().position(|i| i.path == f))
        .map_or(0, |i| i + 1);
    let n = visible.len();
    (0..n)
        .map(|k| &visible[(start + k) % n])
        .find(|item| {
            let name = item.path.rsplit('/').next().unwrap_or(&item.path);
            let name = name.strip_suffix(".md").unwrap_or(name);
            name.to_lowercase().starts_with(&prefix)
        })
        .map(|item| item.path.clone())
}
