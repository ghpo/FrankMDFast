//! Sidebar (ui-spec §3.3): vault header, explorer slot, "Linked from"
//! backlinks, dock footer, and the narrow-width drawer.

use crate::scale::px;
use frankmd_core::wikilink::Backlink;

use super::prelude::*;
use gpui_kit::*;

/// Lines of context shown under each backlink row (§3.3: "one line of context").
const BACKLINK_CONTEXT_LINES: usize = 0;

/// The "Linked from" section of the open note (BE-044).
#[derive(Default)]
pub(crate) struct Backlinks {
    /// Note the list was computed for.
    pub(crate) target: Option<String>,
    /// Linking notes, sorted by `(path, line)`.
    pub(crate) items: Vec<Backlink>,
    task: Option<Task<()>>,
}

impl Backlinks {
    /// Forget the list (no note open).
    pub(crate) fn clear(&mut self) {
        self.target = None;
        self.items.clear();
        self.task = None;
    }
}

impl Shell {
    /// Recompute the backlinks of the open note off the UI thread.
    pub(crate) fn refresh_backlinks(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.active_path().filter(|p| p.ends_with(".md")) else {
            self.backlinks.clear();
            return;
        };
        let Some(vault) = self.core.as_ref().map(|c| c.vault.clone()) else {
            return;
        };
        let query = target.clone();
        self.backlinks.task = Some(cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move {
                    frankmd_fs::service::NotesService::new(&vault)
                        .backlinks(&query, BACKLINK_CONTEXT_LINES)
                        .unwrap_or_default()
                })
                .await;
            let _ = this.update(cx, |shell, cx| {
                if shell.active_path().as_deref() == Some(target.as_str()) {
                    let mut items = found;
                    items.sort_by(|a, b| {
                        (a.path.as_str(), a.line_number).cmp(&(b.path.as_str(), b.line_number))
                    });
                    shell.backlinks.target = Some(target);
                    shell.backlinks.items = items;
                    cx.notify();
                }
            });
        }));
    }

    /// Linking notes of the open note (empty while computing or none).
    pub fn backlink_paths(&self) -> Vec<String> {
        self.backlinks
            .items
            .iter()
            .map(|b| b.path.clone())
            .collect()
    }

    /// `width`: docked width; `None` fills the parent (the narrow drawer).
    pub(super) fn render_sidebar(
        &mut self,
        width: Option<f32>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        v_flex()
            .id("sidebar")
            .test_support()
            .map(|el| match width {
                Some(w) => el.w(gpui_kit::px(w)).flex_shrink_0(),
                None => el.size_full(),
            })
            .h_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                h_flex()
                    .h(px(32.))
                    .px_3()
                    .gap_1()
                    .flex_shrink_0()
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .font_semibold()
                            .child(self.vault_name.clone()),
                    )
                    .child(self.dock_button(
                        "sidebar-new-note",
                        "file-plus",
                        "shell.new_note",
                        "file.new_note",
                        cx,
                    ))
                    .child(self.dock_button(
                        "sidebar-new-folder",
                        "folder-plus",
                        "shell.new_folder",
                        "file.new_folder",
                        cx,
                    )),
            )
            .child(self.render_explorer(theme, cx))
            .child(self.render_backlinks(theme, cx))
            .child(
                h_flex()
                    .h(px(28.))
                    .px_2()
                    .gap_1()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme.sidebar_border)
                    .child(self.dock_button(
                        "dock-media-library",
                        "images",
                        "shell.media_library",
                        "view.media_library",
                        cx,
                    ))
                    .child(self.dock_button(
                        "dock-settings",
                        "gear",
                        "shell.settings",
                        "app.settings",
                        cx,
                    ))
                    .child(self.dock_button(
                        "dock-help",
                        "question",
                        "shell.help",
                        "help.toggle",
                        cx,
                    ))
                    .child({
                        let (icon, tooltip) = if self.encryption.seal().is_none() {
                            ("lock", "shell.encryption_create")
                        } else if self.encryption.is_unlocked() {
                            ("lock-open", "shell.encryption_lock")
                        } else {
                            ("lock", "shell.encryption_unlock")
                        };
                        self.dock_button(
                            "dock-encryption",
                            icon,
                            tooltip,
                            "app.encryption_toggle",
                            cx,
                        )
                    }),
            )
    }

    /// "Linked from (N)": only when the open note has backlinks (§3.3).
    fn render_backlinks(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.backlinks.items.is_empty() || self.backlinks.target != self.active_path() {
            return div().into_any_element();
        }
        let collapsed = self.ui_state.get().backlinks_collapsed;
        let this = cx.entity().downgrade();
        let toggle = this.clone();
        let header = h_flex()
            .id("backlinks-header")
            .test_support()
            .h(px(28.))
            .px_3()
            .gap_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .hover(|s| s.bg(theme.list_hover))
            .child(if collapsed { "▸" } else { "▾" })
            .child(i18n::t_args(
                cx,
                "shell.linked_from",
                &[("count", &self.backlinks.items.len().to_string())],
            ))
            .on_click(move |_, _, cx| {
                let _ = toggle.update(cx, |shell, cx| {
                    shell
                        .ui_state
                        .update(|s| s.backlinks_collapsed = !s.backlinks_collapsed);
                    cx.notify();
                });
            });
        let rows: Vec<AnyElement> = if collapsed {
            Vec::new()
        } else {
            self.backlinks
                .items
                .iter()
                .map(|b| {
                    let this = this.clone();
                    let path = b.path.clone();
                    let line = b.line_number;
                    v_flex()
                        .id(SharedString::from(format!(
                            "backlink-{}-{}",
                            b.path, b.line_number
                        )))
                        .test_support()
                        .px_3()
                        .py_1()
                        .hover(|s| s.bg(theme.list_hover))
                        .child(div().truncate().child(b.name.clone()))
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(b.match_text.trim().to_owned()),
                        )
                        .on_click(move |_, window, cx| {
                            let _ = this.update(cx, |shell, cx| {
                                shell.open_at_line(&path, line, window, cx)
                            });
                        })
                        .into_any_element()
                })
                .collect()
        };
        v_flex()
            .id("backlinks")
            .test_support()
            .flex_shrink_0()
            .max_h(px(220.))
            .overflow_y_scroll()
            .border_t_1()
            .border_color(theme.sidebar_border)
            .child(header)
            .children(rows)
            .into_any_element()
    }

    pub(super) fn dock_button(
        &self,
        id: &'static str,
        icon: &'static str,
        tooltip: &str,
        command: &'static str,
        cx: &mut Context<Self>,
    ) -> Button {
        let this = cx.entity().downgrade();
        Button::new(id)
            .ghost()
            .xsmall()
            .icon(crate::ui::icons::ph(icon))
            .tooltip(i18n::t(cx, tooltip))
            .on_click(move |_, window, cx| {
                let _ = this.update(cx, |shell, cx| shell.run_command(command, window, cx));
            })
    }

    pub(super) fn render_drawer(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let drawer_width = (self.width * 0.85).min(320.);
        div()
            .absolute()
            .inset_0()
            .flex()
            .child(
                div()
                    .w(gpui_kit::px(drawer_width))
                    .h_full()
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(self.render_sidebar(None, theme, cx)),
            )
            .child(
                div()
                    .id("drawer-scrim")
                    .test_support()
                    .flex_1()
                    .h_full()
                    .bg(theme.background.opacity(0.4))
                    .on_click(move |_, window, cx| {
                        let _ = this.update(cx, |shell, cx| {
                            shell.layout.close_drawer();
                            shell.focus_region(FocusRegion::Editor, window, cx);
                        });
                    }),
            )
    }
}
