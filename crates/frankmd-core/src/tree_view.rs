//! The data part of the Explorer rendering (`app/helpers/tree_helper.rb`,
//! `FoldersController#load_tree_for_turbo_stream`).
//!
//! Rails rendered HTML; the GPUI Explorer renders rows. What must stay the same
//! is *which* rows exist, in which order, at which depth, and their flags
//! (expanded, selected, draggable, context menu, config vs note). Golden:
//! `tree_helper.json` (the HTML is parsed back into rows by the test).

use crate::tree::{FileType, TreeNode};
use std::collections::BTreeSet;

/// Kind of an Explorer row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A folder header (focusable, toggles, drop target, draggable).
    Folder,
    /// A markdown note (draggable, has a context menu).
    Note,
    /// The `.fed` config file: selectable, renamable/deletable by keyboard,
    /// **not** draggable and **no** context menu (**[COMPAT]**).
    Config,
}

/// One rendered Explorer row, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// Vault-relative path (`data-path`).
    pub path: String,
    /// Display name.
    pub name: String,
    /// Nesting depth (0 = root level).
    pub depth: usize,
    /// Row kind.
    pub kind: RowKind,
    /// Folder rows: path is in the expanded set (chevron `expanded`).
    pub expanded: bool,
    /// File rows: path equals the selected file (`selected` class). A folder
    /// whose path equals the selection is **not** marked.
    pub selected: bool,
    /// `draggable="true"` (folders and notes, not config).
    pub draggable: bool,
    /// Has `contextmenu->app#showContextMenu` (folders and notes, not config).
    pub context_menu: bool,
    /// `false` when an ancestor folder is collapsed (row is in a `hidden`
    /// `tree-children` container).
    pub visible: bool,
    /// File rows: `data-mtime`, when known.
    pub mtime: Option<i64>,
    /// File rows: stored in a FrankMDFast encryption envelope.
    pub encrypted: bool,
}

/// Result of rendering a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeView {
    /// Top-level list is empty: show `sidebar.no_notes_yet`.
    Empty,
    /// Rows in document order (collapsed descendants included, `visible: false`).
    Rows(Vec<TreeRow>),
}

/// **[COMPAT]** `render_tree_items(items, expanded_folders:, selected_file:)`
/// as data. Empty nested folders render nothing (no "no notes" message).
pub fn tree_rows(items: &[TreeNode], expanded: &BTreeSet<String>, selected_file: &str) -> TreeView {
    if items.is_empty() {
        return TreeView::Empty;
    }
    let mut rows = Vec::new();
    push_rows(&mut rows, items, expanded, selected_file, 0, true);
    TreeView::Rows(rows)
}

fn push_rows(
    rows: &mut Vec<TreeRow>,
    items: &[TreeNode],
    expanded: &BTreeSet<String>,
    selected_file: &str,
    depth: usize,
    visible: bool,
) {
    for item in items {
        match item {
            TreeNode::Folder {
                name,
                path,
                children,
            } => {
                let is_expanded = expanded.contains(path.as_str());
                rows.push(TreeRow {
                    path: path.as_str().to_owned(),
                    name: name.clone(),
                    depth,
                    kind: RowKind::Folder,
                    expanded: is_expanded,
                    selected: false,
                    draggable: true,
                    context_menu: true,
                    visible,
                    mtime: None,
                    encrypted: false,
                });
                push_rows(
                    rows,
                    children,
                    expanded,
                    selected_file,
                    depth + 1,
                    visible && is_expanded,
                );
            }
            TreeNode::File {
                name,
                path,
                file_type,
                mtime,
                encrypted,
                ..
            } => {
                let kind = file_row_kind(*file_type);
                let movable = kind != RowKind::Config;
                rows.push(TreeRow {
                    path: path.as_str().to_owned(),
                    name: name.clone(),
                    depth,
                    kind,
                    expanded: false,
                    selected: path.as_str() == selected_file,
                    draggable: movable,
                    context_menu: movable,
                    visible,
                    mtime: Some(*mtime),
                    encrypted: *encrypted,
                });
            }
        }
    }
}

/// Row kind of a file leaf.
pub fn file_row_kind(file_type: FileType) -> RowKind {
    match file_type {
        FileType::Markdown => RowKind::Note,
        FileType::Config => RowKind::Config,
    }
}

/// **[COMPAT]** expanded-folder remap after a folder rename
/// (`FoldersController#load_tree_for_turbo_stream`): every entry equal to
/// `from` or under `from + "/"` is re-rooted onto `to`; others are kept.
/// Input order is kept and duplicates collapse (Ruby `Set`). Parsing of the
/// comma list (`""` → empty) is [`parse_expanded`].
pub fn remap_expanded(expanded: &[String], from: &str, to: &str) -> Vec<String> {
    let remapped = expanded.iter().map(|path| {
        let under = path
            .strip_prefix(from)
            .filter(|rest| rest.is_empty() || rest.starts_with('/'));
        match under {
            Some(rest) => format!("{to}{rest}"),
            None => path.clone(),
        }
    });
    dedupe(remapped)
}

/// Keep the first occurrence of each entry (Ruby `Set` insertion order).
fn dedupe(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

/// **[COMPAT]** `params[:expanded].to_s.split(",").to_set`, in insertion order.
pub fn parse_expanded(param: &str) -> Vec<String> {
    // Ruby `split(",")` drops trailing empty fields (and yields `[]` for "").
    let mut parts: Vec<&str> = param.split(',').collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    dedupe(parts.into_iter().map(str::to_owned))
}
