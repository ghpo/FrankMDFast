//! Explorer tree (ui-spec §3.3, §5.14): rows from `frankmd_core::tree_view`,
//! selection / context menu / drag & drop state from
//! `frankmd_workspace::explorer`, inline rename from
//! `frankmd_workspace::dialogs::InlineRename`. This module only renders and
//! routes input.

use crate::scale::px;
use std::time::{Duration, Instant};

use frankmd_core::tree_view::{RowKind, TreeRow, TreeView, tree_rows};
use frankmd_workspace::explorer::{
    DropHover, DropTarget, Explorer, ExplorerEffect, Item, ItemKind, MenuItem, MenuTarget,
    Modifiers,
};

use super::prelude::*;
use gpui_kit::*;

/// Type-ahead buffer resets after this pause (ui-spec §5.14).
const TYPE_AHEAD_RESET: Duration = Duration::from_millis(900);

/// Hovering a collapsed folder this long while dragging expands it (§5.14).
const DRAG_EXPAND_DELAY: Duration = Duration::from_millis(600);

/// Explorer input state that is pure UI (not a view model).
#[derive(Default)]
pub(crate) struct TreeUi {
    type_buffer: String,
    last_typed: Option<Instant>,
    /// Folder under the pointer while dragging, and since when.
    drag_hover: Option<(String, Instant)>,
}

/// What a tree drag carries (the payload itself lives in the explorer VM).
#[derive(Clone)]
pub(crate) struct ExplorerDrag {
    count: usize,
    label: SharedString,
}

/// The floating badge under the pointer while dragging (count for multi-drag).
struct DragBadge {
    label: SharedString,
    count: usize,
}

impl Render for DragBadge {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.primary)
            .text_xs()
            .child(self.label.clone())
            .when(self.count > 1, |el| {
                el.child(
                    div()
                        .px_1()
                        .rounded(theme.radius)
                        .bg(theme.primary)
                        .text_color(theme.primary_foreground)
                        .child(self.count.to_string()),
                )
            })
    }
}

fn item_of(row: &TreeRow) -> Item {
    Item {
        path: row.path.clone(),
        kind: if row.kind == RowKind::Folder {
            ItemKind::Folder
        } else {
            ItemKind::File
        },
        encrypted: row.encrypted,
    }
}

/// Image file names get the "drop on the editor" hint (§5.14).
fn is_image_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".svg"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

fn modifiers_of(m: &gpui_kit::Modifiers) -> Modifiers {
    Modifiers {
        ctrl: m.control || m.platform,
        shift: m.shift,
    }
}

impl Shell {
    /// Visible rows in display order.
    pub(crate) fn visible_rows(&self) -> Vec<TreeRow> {
        let tree = self
            .core
            .as_ref()
            .map(|c| c.ws.snapshot().tree)
            .unwrap_or_default();
        let active = self.current_path().unwrap_or_default();
        match tree_rows(&tree, self.explorer.expanded(), &active) {
            TreeView::Rows(rows) => rows.into_iter().filter(|r| r.visible).collect(),
            TreeView::Empty => Vec::new(),
        }
    }

    fn visible_items(&self) -> Vec<Item> {
        self.visible_rows().iter().map(item_of).collect()
    }

    /// Kind of the tree entry at `path` (folders are rows with children or empty folders).
    pub(crate) fn kind_of(&self, path: &str) -> ItemKind {
        let folder = self.core.as_ref().is_some_and(|c| {
            frankmd_fs::service::NotesService::new(&c.vault)
                .is_dir(path)
                .unwrap_or(false)
        });
        if folder {
            ItemKind::Folder
        } else {
            ItemKind::File
        }
    }

    /// The explorer view model (tests inspect selection and expansion).
    pub fn explorer(&self) -> &Explorer {
        &self.explorer
    }

    pub(super) fn render_explorer(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tree = self
            .core
            .as_ref()
            .map(|c| c.ws.snapshot().tree)
            .unwrap_or_default();
        let active = self.current_path().unwrap_or_default();
        let view = tree_rows(&tree, self.explorer.expanded(), &active);
        let this = cx.entity().downgrade();
        let cursor = self.explorer.cursor().map(str::to_owned);
        let focused = self.sidebar_focus.clone();
        let dragging = self.explorer.is_dragging();
        let root_target =
            dragging && self.explorer.drop_target(&DropHover::Background) == Some(DropTarget::Root);
        let rename_row = self
            .files
            .rename
            .as_ref()
            .filter(|r| r.target == frankmd_workspace::dialogs::RenameTarget::Row)
            .map(|r| r.path.clone());
        let mut list = v_flex()
            .id("explorer-tree")
            .test_support()
            .track_focus(&self.sidebar_focus)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py_1()
            .when(root_target, |el| {
                el.border_1()
                    .border_dashed()
                    .border_color(theme.primary)
                    .bg(theme.primary.opacity(0.06))
            })
            .on_key_down({
                let this = this.clone();
                move |ev: &KeyDownEvent, window, cx| {
                    let _ = this.update(cx, |shell, cx| {
                        if shell.explorer_key(&ev.keystroke, window, cx) {
                            cx.stop_propagation();
                        }
                    });
                }
            })
            // Left click on empty space only clears the selection (§5.14).
            .on_click({
                let this = this.clone();
                move |_, window, cx| {
                    let _ = this.update(cx, |shell, cx| {
                        shell.explorer.click_background();
                        shell.explorer.close_context_menu();
                        shell.sidebar_focus.focus(window, cx);
                        cx.notify();
                    });
                }
            })
            .on_mouse_down(MouseButton::Right, {
                let this = this.clone();
                move |ev: &MouseDownEvent, _, cx| {
                    let _ = this.update(cx, |shell, cx| {
                        shell.explorer.open_context_menu(
                            MenuTarget::Background,
                            f32::from(ev.position.x),
                            f32::from(ev.position.y),
                        );
                        cx.notify();
                    });
                }
            })
            .on_drop({
                let this = this.clone();
                move |_: &ExplorerDrag, window, cx| {
                    let _ = this.update(cx, |shell, cx| {
                        shell.drop_on(&DropHover::Background, window, cx)
                    });
                }
            })
            .on_drop({
                let this = this.clone();
                move |paths: &ExternalPaths, _, cx| {
                    let _ =
                        this.update(cx, |shell, cx| shell.import_external(paths.paths(), "", cx));
                }
            });
        match view {
            TreeView::Empty => {
                list = list.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(theme.muted_foreground)
                        .child(i18n::t(cx, "sidebar.no_notes_yet")),
                );
            }
            TreeView::Rows(rows) => {
                for row in rows.into_iter().filter(|r| r.visible) {
                    let editing = rename_row.as_deref() == Some(row.path.as_str());
                    let el = self.render_row(&row, cursor.as_deref(), editing, theme, cx);
                    list = list.child(el);
                }
            }
        }
        let _ = focused;
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .relative()
            .child(list)
            .when_some(self.explorer.context_menu().cloned(), |el, menu| {
                el.child(self.render_context_menu(menu, theme, cx))
            })
    }

    fn render_row(
        &mut self,
        row: &TreeRow,
        cursor: Option<&str>,
        editing: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let this = cx.entity().downgrade();
        let path = row.path.clone();
        let item = item_of(row);
        let is_cursor = cursor == Some(path.as_str());
        let selected = self.explorer.is_selected(&path);
        #[allow(clippy::cast_precision_loss)]
        let indent = 8.0 + row.depth as f32 * 12.0;
        let glyph = match row.kind {
            RowKind::Folder if row.expanded => "▾",
            RowKind::Folder => "▸",
            _ => " ",
        };
        let drop_hover = if row.kind == RowKind::Folder {
            DropHover::Folder(path.clone())
        } else {
            DropHover::File(path.clone())
        };
        let valid_drop = self.explorer.is_dragging()
            && row.kind == RowKind::Folder
            && self.explorer.drop_target(&drop_hover).is_some();
        let label: AnyElement = if editing {
            div()
                .flex_1()
                .child(
                    Input::new(&self.files.rename_input)
                        .id("explorer-rename-input")
                        .xsmall(),
                )
                .into_any_element()
        } else {
            div()
                .flex_1()
                .truncate()
                .child(row.name.clone())
                .into_any_element()
        };
        let mut el = h_flex()
            .id(SharedString::from(format!("explorer-row-{}", row.path)))
            .test_support()
            .h(px(24.))
            .pl(px(indent))
            .pr_2()
            .gap_1()
            .when(selected, |el| el.bg(theme.selection))
            .when(row.selected, |el| {
                el.border_l_2().border_color(theme.primary)
            })
            .when(is_cursor, |el| {
                el.border_1().border_color(theme.primary.opacity(0.6))
            })
            .when(valid_drop, |el| {
                el.border_1()
                    .border_color(theme.primary)
                    .bg(theme.primary.opacity(0.12))
            })
            .hover(|s| s.bg(theme.list_hover))
            .tooltip({
                let path: SharedString = row.path.clone().into();
                move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(path.clone()).build(window, cx)
                }
            })
            .child(
                div()
                    .w(px(12.))
                    .text_color(theme.muted_foreground)
                    .child(glyph),
            )
            .when(row.encrypted, |el| {
                let (icon, color) = if self.encryption.is_unlocked() {
                    ("lock-open", theme.success)
                } else {
                    ("lock", theme.warning)
                };
                el.child(
                    div()
                        .id(SharedString::from(format!("explorer-lock-{}", row.path)))
                        .test_support()
                        .child(crate::ui::icons::ph(icon).xsmall().text_color(color)),
                )
            })
            .child(label);
        if editing {
            return el.into_any_element();
        }
        el = el
            .on_click({
                let this = this.clone();
                let item = item.clone();
                move |ev: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    let mods = modifiers_of(&ev.modifiers());
                    let _ = this.update(cx, |shell, cx| {
                        shell.click_row(item.clone(), mods, window, cx)
                    });
                }
            })
            .on_mouse_down(MouseButton::Right, {
                let this = this.clone();
                let item = item.clone();
                move |ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let _ = this.update(cx, |shell, cx| {
                        shell.explorer.open_context_menu(
                            MenuTarget::Row(item.clone()),
                            f32::from(ev.position.x),
                            f32::from(ev.position.y),
                        );
                        shell.sidebar_focus.focus(window, cx);
                        cx.notify();
                    });
                }
            });
        if row.draggable {
            let drag = ExplorerDrag {
                count: 1,
                label: row.name.clone().into(),
            };
            el = el
                .on_drag(drag, {
                    let this = this.clone();
                    let item = item.clone();
                    move |drag: &ExplorerDrag, _, _, cx| {
                        let count = this
                            .update(cx, |shell, cx| {
                                shell.explorer.close_context_menu();
                                shell.explorer.begin_drag(item.clone());
                                cx.notify();
                                shell.explorer.drag_badge_count()
                            })
                            .unwrap_or(drag.count);
                        let label = drag.label.clone();
                        cx.new(|_| DragBadge { label, count })
                    }
                })
                .on_drop({
                    let this = this.clone();
                    let hover = drop_hover.clone();
                    move |_: &ExplorerDrag, window, cx| {
                        cx.stop_propagation();
                        let _ = this.update(cx, |shell, cx| shell.drop_on(&hover, window, cx));
                    }
                })
                .on_drag_move({
                    let this = this.clone();
                    let path = path.clone();
                    let folder = row.kind == RowKind::Folder && !row.expanded;
                    move |ev: &DragMoveEvent<ExplorerDrag>, _, cx| {
                        if !folder || !ev.bounds.contains(&ev.event.position) {
                            return;
                        }
                        let _ = this.update(cx, |shell, cx| shell.drag_hover_folder(&path, cx));
                    }
                });
        }
        if row.kind == RowKind::Folder {
            let target = path.clone();
            el = el.on_drop({
                let this = this.clone();
                move |paths: &ExternalPaths, _, cx| {
                    cx.stop_propagation();
                    let _ = this.update(cx, |shell, cx| {
                        shell.import_external(paths.paths(), &target, cx)
                    });
                }
            });
        }
        el.into_any_element()
    }

    /// Mouse click on a row (F.4 / §5.14): plain opens notes and toggles
    /// folders; Ctrl toggles, Shift ranges, neither opens.
    pub(crate) fn click_row(
        &mut self,
        item: Item,
        mods: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible = self.visible_items();
        match self.explorer.click(item, mods, &visible) {
            Some(ExplorerEffect::OpenNote(path)) => {
                self.open_path(&path, window, cx);
                // An explorer click keeps the sidebar focused (notes_test.rb:33).
                self.sidebar_focus.focus(window, cx);
                if self.layout.width_class == WidthClass::Narrow {
                    self.layout.close_drawer();
                    self.focus_region(FocusRegion::Editor, window, cx);
                }
            }
            Some(ExplorerEffect::ToggleFolder(path)) => {
                self.explorer.toggle_folder(&path);
                self.persist_expanded();
                self.sidebar_focus.focus(window, cx);
            }
            None => self.sidebar_focus.focus(window, cx),
        }
        cx.notify();
    }

    /// Expand a collapsed folder after hovering it for 600 ms during a drag.
    fn drag_hover_folder(&mut self, path: &str, cx: &mut Context<Self>) {
        let now = Instant::now();
        match &self.tree_ui.drag_hover {
            Some((p, since)) if p == path => {
                if now.duration_since(*since) >= DRAG_EXPAND_DELAY {
                    self.explorer.expand(path);
                    self.persist_expanded();
                    self.tree_ui.drag_hover = None;
                    cx.notify();
                }
            }
            _ => self.tree_ui.drag_hover = Some((path.to_owned(), now)),
        }
    }

    /// A tree drag was dropped on `hover`.
    pub(crate) fn drop_on(
        &mut self,
        hover: &DropHover,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tree_ui.drag_hover = None;
        let Some(plan) = self.explorer.drop(hover) else {
            cx.notify();
            return;
        };
        self.run_move_plan(plan, window, cx);
    }

    /// OS files dropped on the tree (§5.14): `.md` files are copied into the
    /// folder; images get a hint to drop them on the editor.
    pub(crate) fn import_external(
        &mut self,
        paths: &[std::path::PathBuf],
        folder: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(core) = self.core.as_ref() else {
            return;
        };
        let service = frankmd_fs::service::NotesService::new(&core.vault);
        let mut copied = 0usize;
        let mut images = false;
        for path in paths {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let is_md = std::path::Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"));
            if !is_md {
                images |= is_image_name(name);
                continue;
            }
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let dest = if folder.is_empty() {
                name.to_owned()
            } else {
                format!("{folder}/{name}")
            };
            if service.create(&dest, &bytes).is_ok() {
                copied += 1;
            }
        }
        if images {
            let text = i18n::t(cx, "status.drop_images_on_editor");
            self.show_message(&text, false, cx);
        }
        if copied > 0 {
            if !folder.is_empty() {
                self.explorer.expand(folder);
                self.persist_expanded();
            }
            if let Some(core) = self.core.as_mut() {
                let effect = core.ws.rescan();
                self.run_effect(effect, cx);
            }
        }
    }

    /// Explorer keyboard (§5.14). `true` when the key was handled.
    fn explorer_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.files.rename.is_some() {
            return false;
        }
        let rows = self.visible_rows();
        if rows.is_empty() {
            return false;
        }
        let items: Vec<Item> = rows.iter().map(item_of).collect();
        let at = self
            .explorer
            .cursor()
            .and_then(|c| rows.iter().position(|r| r.path == c));
        let current = at.and_then(|i| rows.get(i)).cloned();
        let m = &keystroke.modifiers;
        let plain = !m.control && !m.alt && !m.platform;
        let key = keystroke.key.as_str();
        match key {
            "down" | "up" if plain => {
                let next = match (key, at) {
                    ("down", Some(i)) => (i + 1).min(rows.len() - 1),
                    ("up", Some(i)) => i.saturating_sub(1),
                    _ => 0,
                };
                let item = items[next].clone();
                if m.shift {
                    self.explorer.extend_selection(item, &items);
                } else {
                    self.explorer.set_cursor(Some(item.path));
                }
            }
            "right" if plain => {
                if let Some(row) = current.filter(|r| r.kind == RowKind::Folder) {
                    if row.expanded {
                        if let Some(child) = rows
                            .get(at.unwrap_or(0) + 1)
                            .filter(|c| c.depth > row.depth)
                        {
                            self.explorer.set_cursor(Some(child.path.clone()));
                        }
                    } else {
                        self.explorer.expand(&row.path);
                        self.persist_expanded();
                    }
                }
            }
            "left" if plain => {
                if let Some(row) = current {
                    if row.kind == RowKind::Folder && row.expanded {
                        self.explorer.collapse(&row.path);
                        self.persist_expanded();
                    } else if let Some((parent, _)) = row.path.rsplit_once('/') {
                        self.explorer.set_cursor(Some(parent.to_owned()));
                    }
                }
            }
            "enter" if plain && !m.shift => {
                let Some(row) = current else {
                    return false;
                };
                if row.kind == RowKind::Folder {
                    self.explorer.toggle_folder(&row.path);
                    self.persist_expanded();
                } else {
                    self.explorer.select_only(item_of(&row));
                    self.open_path(&row.path, window, cx);
                    // Enter behaves like a click: focus stays on the tree (OQ-3).
                    if self.layout.width_class != WidthClass::Narrow {
                        self.sidebar_focus.focus(window, cx);
                    }
                }
            }
            "space" if plain && !m.shift => {
                let Some(row) = current else {
                    return false;
                };
                self.explorer.toggle_selected(item_of(&row));
            }
            "a" if m.control && !m.shift && !m.alt => self.explorer.select_all(&items),
            "f2" if plain && !m.shift => {
                let target = self.single_target(current.as_ref());
                let Some(item) = target else {
                    return false;
                };
                self.start_inline_rename(
                    frankmd_workspace::dialogs::RenameTarget::Row,
                    &item.path,
                    item.kind,
                    window,
                    cx,
                );
            }
            "delete" if plain && !m.shift => {
                let mut targets = self.explorer.selection();
                if targets.is_empty() {
                    targets.extend(current.as_ref().map(item_of));
                }
                self.open_delete(targets, window, cx);
            }
            "*" | "shift-8" => {
                if let Some(row) = current {
                    for sibling in rows.iter().filter(|r| {
                        r.kind == RowKind::Folder
                            && r.depth == row.depth
                            && r.path.rsplit_once('/').map(|(p, _)| p)
                                == row.path.rsplit_once('/').map(|(p, _)| p)
                    }) {
                        self.explorer.expand(&sibling.path);
                    }
                    self.persist_expanded();
                }
            }
            _ => {
                let typed = keystroke.key_char.as_deref().filter(|c| {
                    plain
                        && c.chars().count() == 1
                        && c.chars().all(|ch| !ch.is_control() && ch != ' ')
                });
                let Some(ch) = typed else {
                    return false;
                };
                let now = Instant::now();
                if self
                    .tree_ui
                    .last_typed
                    .is_none_or(|t| now.duration_since(t) > TYPE_AHEAD_RESET)
                {
                    self.tree_ui.type_buffer.clear();
                }
                self.tree_ui.type_buffer.push_str(ch);
                self.tree_ui.last_typed = Some(now);
                let from = if self.tree_ui.type_buffer.chars().count() == 1 {
                    self.explorer.cursor().map(str::to_owned)
                } else {
                    // Extending the prefix keeps matching from the row before the cursor.
                    at.and_then(|i| i.checked_sub(1))
                        .and_then(|i| rows.get(i))
                        .map(|r| r.path.clone())
                };
                let prefix = self.tree_ui.type_buffer.clone();
                if let Some(path) =
                    frankmd_workspace::explorer::type_ahead(&items, from.as_deref(), &prefix)
                {
                    self.explorer.set_cursor(Some(path));
                }
            }
        }
        cx.notify();
        true
    }

    /// The one item an action like F2 applies to: the selection when it is a
    /// single item, else the cursor row.
    fn single_target(&self, current: Option<&TreeRow>) -> Option<Item> {
        let selection = self.explorer.selection();
        match selection.as_slice() {
            [one] => Some(one.clone()),
            [] => current.map(item_of),
            _ => None,
        }
    }

    // ----- context menu (F.5, F.6, §5.14) ---------------------------------------------

    fn render_context_menu(
        &mut self,
        menu: frankmd_workspace::explorer::ContextMenu,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let items = Explorer::menu_items(&menu.target);
        let rows = items.into_iter().map(|item| {
            let (id, key) = match item {
                MenuItem::NewNote => ("new-note", "context_menu.new_note"),
                MenuItem::NewFolder => ("new-folder", "context_menu.new_folder"),
                MenuItem::Rename => ("rename", "context_menu.rename"),
                MenuItem::MoveTo => ("move", "context_menu.move_to"),
                MenuItem::Delete => ("delete", "context_menu.delete"),
                MenuItem::SelectAll => ("select-all", "context_menu.select_all"),
                MenuItem::FindFile => ("find-file", "context_menu.find_file"),
                MenuItem::CollapseAll => ("collapse-all", "context_menu.collapse_all"),
                MenuItem::Encrypt => ("encrypt", "context_menu.encrypt"),
                MenuItem::Decrypt => ("decrypt", "context_menu.decrypt"),
            };
            let this = this.clone();
            let target = menu.target.clone();
            div()
                .id(SharedString::from(format!("explorer-menu-{id}")))
                .test_support()
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .when(item == MenuItem::Delete, |el| el.text_color(theme.danger))
                .hover(|s| s.bg(theme.list_hover))
                .child(i18n::t(cx, key))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    let _ = this.update(cx, |shell, cx| {
                        shell.explorer.close_context_menu();
                        shell.run_menu_item(item, &target, window, cx);
                    });
                })
        });
        let rows: Vec<_> = rows.collect();
        // Position inside the sidebar, clamped to stay visible (menu_bounds).
        let sidebar_w = self.ui_state.get().sidebar_width;
        let x = menu.x.clamp(4.0, (sidebar_w - 168.0).max(4.0));
        let y = (menu.y - 32.0).max(4.0);
        div()
            .id("explorer-context-menu")
            .test_support()
            .absolute()
            .left(gpui_kit::px(x))
            .top(gpui_kit::px(y))
            .min_w(px(160.))
            .py_1()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .on_mouse_down_out({
                let this = this.clone();
                move |_, _, cx| {
                    let _ = this.update(cx, |shell, cx| {
                        shell.explorer.close_context_menu();
                        cx.notify();
                    });
                }
            })
            .children(rows)
    }

    /// Run a context-menu item.
    pub(crate) fn run_menu_item(
        &mut self,
        item: MenuItem,
        target: &MenuTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use frankmd_workspace::dialogs::{CreationOrigin, RenameTarget};
        let row = match target {
            MenuTarget::Row(row) => Some(row.clone()),
            MenuTarget::Background => None,
        };
        match (item, row) {
            (MenuItem::NewNote, Some(folder)) => {
                self.open_new_note(CreationOrigin::FolderContext(folder.path), None, window, cx)
            }
            (MenuItem::NewNote, None) => self.open_new_note(
                CreationOrigin::Global {
                    selected_folder: String::new(),
                },
                None,
                window,
                cx,
            ),
            (MenuItem::NewFolder, Some(folder)) => self.open_new_folder(&folder.path, window, cx),
            (MenuItem::NewFolder, None) => self.open_new_folder("", window, cx),
            (MenuItem::Rename, Some(row)) => {
                self.start_inline_rename(RenameTarget::Row, &row.path, row.kind, window, cx)
            }
            (MenuItem::MoveTo, Some(row)) => self.open_move(&row.path, row.kind, window, cx),
            (MenuItem::Encrypt, Some(row)) if row.kind == ItemKind::File => {
                self.request_note_encryption(&row.path, true, window, cx)
            }
            (MenuItem::Decrypt, Some(row)) if row.kind == ItemKind::File => {
                self.request_note_encryption(&row.path, false, window, cx)
            }
            (MenuItem::Delete, Some(row)) => {
                let targets = self.explorer.context_targets(&row);
                self.open_delete(targets, window, cx);
            }
            (MenuItem::SelectAll, _) => {
                let items = self.visible_items();
                self.explorer.select_all(&items);
                self.sidebar_focus.focus(window, cx);
            }
            (MenuItem::FindFile, _) => self.open_overlay(OverlayKind::QuickOpen, window, cx),
            (MenuItem::CollapseAll, _) => {
                self.explorer.collapse_all();
                self.persist_expanded();
            }
            _ => {}
        }
        cx.notify();
    }
}
