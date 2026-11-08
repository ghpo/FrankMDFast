//! Editor pane (ui-spec §3.4, §5.2, §5.12): header, banners, text or the
//! docked quick open of the empty state.

use crate::scale::px;
use gpui_kit::component::input::{MoveDown, MoveUp, Paste, ToggleCodeActions};

use super::prelude::*;
use gpui_kit::*;

impl Shell {
    // ----- editor pane ----------------------------------------------------------------

    pub(super) fn render_editor_pane(
        &mut self,
        arrangement: &frankmd_workspace::layout::PaneArrangement,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let has_note = self.current_path().is_some();
        let focused = self.editor.focus_handle(cx).contains_focused(window, cx);
        v_flex()
            .id("editor-pane")
            .test_support()
            // gpui-kit's editor binds Ctrl+. (code actions) deeper than our
            // Editor-context binding; take it in the capture phase so Ctrl+.
            // is always the context action (§6.2).
            .capture_action(cx.listener(|this, _: &ToggleCodeActions, window, cx| {
                cx.stop_propagation();
                this.run_command("edit.context_action", window, cx);
            }))
            // An image on the clipboard opens Insert image › Upload (FE-358);
            // text pastes normally.
            .capture_action(cx.listener(|this, _: &Paste, window, cx| {
                if this.active_path().is_some_and(|p| p.ends_with(".md"))
                    && this.paste_image(window, cx)
                {
                    cx.stop_propagation();
                }
            }))
            // Debug builds of gpui-component bind Ctrl+Shift+I globally to its
            // inspector; in the editor it is Insert image (§6.2).
            .map(|el| {
                #[cfg(debug_assertions)]
                let el = el.capture_action(cx.listener(
                    |this, _: &gpui_kit::component::ToggleInspector, window, cx| {
                        cx.stop_propagation();
                        this.run_command("insert.image", window, cx);
                    },
                ));
                el
            })
            // Files dropped on the editor go to Insert image › Upload (FE-361).
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if this.active_path().is_some_and(|p| p.ends_with(".md")) {
                    this.open_insert_image_with_paths(paths.paths().to_vec(), window, cx);
                }
            }))
            .flex_1()
            .min_w(px(
                frankmd_workspace::layout::EDITOR_MIN_WIDTH.min(self.width)
            ))
            .h_full()
            .overflow_hidden()
            .when(arrangement.pane_header && has_note, |el| {
                el.child(self.render_pane_header(focused, theme, cx))
            })
            .child(self.render_banners(theme, cx))
            .children(self.render_grammar_banner(theme, cx))
            .children(self.render_rename_error(theme, cx))
            .child(if let Some(path) = self.locked_note.clone() {
                self.render_locked_panel("locked-note", &path, theme, cx)
            } else if has_note {
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.editor.render())
                    .into_any_element()
            } else {
                self.render_empty_state(theme, cx).into_any_element()
            })
    }

    pub(super) fn render_pane_header(
        &mut self,
        focused: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let path = self.current_path().unwrap_or_default();
        let mut parts: Vec<&str> = path.split('/').collect();
        let title = parts
            .pop()
            .unwrap_or_default()
            .trim_end_matches(".md")
            .to_owned();
        let narrow = self.layout.width_class == WidthClass::Narrow;
        let crumbs = if narrow && !parts.is_empty() {
            "…/".to_owned()
        } else {
            parts.iter().map(|p| format!("{p} / ")).collect::<String>()
        };
        let dirty = self.core.as_ref().is_some_and(|c| c.ws.snapshot().dirty);
        let renaming = self
            .files
            .rename
            .as_ref()
            .is_some_and(|r| r.target == frankmd_workspace::dialogs::RenameTarget::Title);
        let sidebar_hidden = self.arrangement().sidebar == SidebarMode::Hidden;
        let preview_on = self.arrangement().preview != PreviewMode::Hidden;
        h_flex()
            .id("pane-header")
            .test_support()
            .h(px(32.))
            .px_3()
            .gap_1()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.sidebar_border)
            .when(focused, |el| el.border_t_2().border_color(theme.primary))
            .when(sidebar_hidden, |el| {
                el.child(self.header_button(
                    "pane-open-sidebar",
                    "◧",
                    "shell.open_sidebar",
                    "view.toggle_sidebar",
                    false,
                    cx,
                ))
            })
            .child(div().text_color(theme.muted_foreground).child(crumbs))
            .child(if renaming {
                div()
                    .min_w(px(160.))
                    .child(
                        Input::new(&self.files.rename_input)
                            .id("pane-title-input")
                            .xsmall(),
                    )
                    .into_any_element()
            } else {
                let this = cx.entity().downgrade();
                div()
                    .id("pane-title")
                    .test_support()
                    .font_medium()
                    .truncate()
                    .cursor_text()
                    .child(title)
                    .on_click(move |_, window, cx| {
                        let _ = this.update(cx, |shell, cx| shell.rename_active(window, cx));
                    })
                    .into_any_element()
            })
            .when(dirty, |el| {
                el.child(div().text_color(theme.muted_foreground).child("●"))
            })
            .child(div().flex_1())
            .when(!narrow, |el| {
                el.child(self.header_button(
                    "pane-insert-image",
                    "image",
                    "shell.insert_image",
                    "insert.image",
                    false,
                    cx,
                ))
                .child(self.header_button(
                    "pane-find",
                    "magnifying-glass",
                    "shell.find",
                    "find.find",
                    false,
                    cx,
                ))
            })
            .child(self.header_button(
                "pane-toggle-preview",
                "sidebar",
                "shell.toggle_preview",
                "view.toggle_preview",
                preview_on,
                cx,
            ))
            .child(self.header_button(
                "pane-menu",
                "dots-three",
                "shell.document_menu",
                "app.command_palette",
                false,
                cx,
            ))
    }

    pub(super) fn header_button(
        &self,
        id: &'static str,
        icon: &'static str,
        tooltip: &str,
        command: &'static str,
        pressed: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let this = cx.entity().downgrade();
        Button::new(id)
            .ghost()
            .xsmall()
            .selected(pressed)
            .icon(crate::ui::icons::ph(icon))
            .tooltip(i18n::t(cx, tooltip))
            .on_click(move |_, window, cx| {
                let _ = this.update(cx, |shell, cx| shell.run_command(command, window, cx));
            })
    }

    pub(super) fn render_banners(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let vault = self.vault_root.display().to_string();
        v_flex()
            .flex_shrink_0()
            .children(self.banners.visible().into_iter().map(|kind| {
                let role = match kind.role() {
                    BannerRole::Danger => theme.danger,
                    BannerRole::Warning => theme.warning,
                    BannerRole::Info => theme.info,
                };
                let message = match kind {
                    BannerKind::VaultUnreachable => {
                        i18n::t_args(cx, kind.message_key(), &[("vault", &vault)])
                    }
                    _ => i18n::t(cx, kind.message_key()),
                };
                let buttons = BannerStack::buttons(kind).into_iter().map(|key| {
                    let this = this.clone();
                    Button::new(SharedString::from(kind.button_id(key)))
                        .xsmall()
                        .outline()
                        .label(i18n::t(cx, key))
                        .on_click(move |_, window, cx| {
                            let _ = this
                                .update(cx, |shell, cx| shell.banner_action(kind, key, window, cx));
                        })
                });
                let buttons: Vec<_> = buttons.collect();
                h_flex()
                    .id(SharedString::from(kind.element_id()))
                    .test_support()
                    .h(px(32.))
                    .px_3()
                    .gap_2()
                    .bg(role.opacity(0.14))
                    .border_l_2()
                    .border_color(role)
                    .child(div().flex_1().truncate().child(message))
                    .children(buttons)
            }))
    }

    pub(super) fn render_empty_state(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let count = i64::try_from(self.note_count()).unwrap_or(i64::MAX);
        let docked = self.overlays.current() != Some(&OverlayKind::QuickOpen);
        let error = self.startup_error.clone();
        let vault = self.vault_root.display().to_string();
        v_flex()
            .id("empty-state")
            .test_support()
            .flex_1()
            .items_center()
            .pt(px(64.))
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        svg()
                            .path("brand/icon.svg")
                            .size(px(40.))
                            .text_color(theme.primary),
                    )
                    .child(
                        v_flex()
                            .child(div().font_semibold().child(self.vault_name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(i18n::t_count(cx, "empty_state.notes", count)),
                            ),
                    ),
            )
            .when_some(error, |el, reason| {
                el.child(div().text_color(theme.danger).child(i18n::t_args(
                    cx,
                    "empty_state.vault_unavailable",
                    &[("vault", &vault), ("reason", &reason)],
                )))
            })
            .when(docked && self.core.is_some(), |el| {
                el.child(
                    div()
                        .w(px(560.))
                        .max_w(relative(1.))
                        .child(self.render_quick_open_list(true, theme, cx)),
                )
            })
            .child(
                h_flex()
                    .gap_3()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("Ctrl+N {}", i18n::t(cx, "empty_state.new_note")))
                    .child(format!(
                        "Ctrl+Shift+F {}",
                        i18n::t(cx, "empty_state.search")
                    ))
                    .child(format!(
                        "Ctrl+Shift+P {}",
                        i18n::t(cx, "empty_state.commands")
                    )),
            )
    }

    /// The quick-open list: docked in the empty state or as an overlay.
    pub(crate) fn render_quick_open_list(
        &mut self,
        docked: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let selected = self.quick_open.model.selected();
        let details = self.quick_open.model.row_details().to_vec();
        let suggestion = self.quick_open.model.create_suggestion();
        let up = this.clone();
        let down = this.clone();
        let rows = details.into_iter().enumerate().map(|(ix, row)| {
            let this = this.clone();
            let path = row.path.clone();
            let (folder, name) = match row.path.rsplit_once('/') {
                Some((folder, name)) => {
                    (folder.to_owned(), name.trim_end_matches(".md").to_owned())
                }
                None => (String::new(), row.path.trim_end_matches(".md").to_owned()),
            };
            h_flex()
                .id(SharedString::from(format!("quick-open-row-{}", row.path)))
                .test_support()
                .h(px(28.))
                .px_3()
                .gap_2()
                .when(selected == Some(ix), |el| {
                    el.bg(theme.selection)
                        .border_l_2()
                        .border_color(theme.primary)
                })
                .child(div().truncate().child(name))
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(folder),
                )
                .when(row.recent, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(i18n::t(cx, "quick_open.recent")),
                    )
                })
                .on_click(move |_, window, cx| {
                    let _ = this.update(cx, |shell, cx| {
                        shell.overlays.close();
                        shell.open_path(&path, window, cx);
                    });
                })
        });
        let rows: Vec<_> = rows.collect();
        let empty = rows.is_empty();
        v_flex()
            .id(if docked {
                "quick-open-docked"
            } else {
                "quick-open"
            })
            .test_support()
            .key_context(actions::CX_OVERLAY)
            .w_full()
            .max_h(px(440.))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .overflow_hidden()
            .when(!docked, |el| el.shadow_lg())
            .capture_action(move |_: &MoveDown, window, cx| {
                let _ = down.update(cx, |shell, cx| shell.list_step(true, window, cx));
                cx.stop_propagation();
            })
            .capture_action(move |_: &MoveUp, window, cx| {
                let _ = up.update(cx, |shell, cx| shell.list_step(false, window, cx));
                cx.stop_propagation();
            })
            .child(
                h_flex()
                    .h(px(36.))
                    .px_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div().flex_1().child(
                            Input::new(&self.quick_open.input)
                                .id("quick-open-input")
                                .appearance(false),
                        ),
                    ),
            )
            .child(
                div()
                    .id("quick-open-rows")
                    .flex_1()
                    .overflow_y_scroll()
                    .py_1()
                    .children(rows)
                    .when(empty, |el| {
                        el.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_color(theme.muted_foreground)
                                .child(i18n::t(cx, "quick_open.no_matches")),
                        )
                        .when_some(suggestion, |el, name| {
                            el.child(div().px_3().pb_2().text_xs().child(i18n::t_args(
                                cx,
                                "quick_open.create_hint",
                                &[("name", &name)],
                            )))
                        })
                    }),
            )
    }

    /// Inline rename error as a banner row under the header (§3.4, FE-279).
    fn render_rename_error(&self, theme: &Theme, cx: &App) -> Option<AnyElement> {
        let err = self.rename_error()?;
        let args: Vec<(&str, &str)> = err.args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let text = i18n::t_args(cx, err.key, &args);
        Some(
            h_flex()
                .id("rename-error")
                .test_support()
                .h(px(28.))
                .px_3()
                .bg(theme.danger.opacity(0.14))
                .border_l_2()
                .border_color(theme.danger)
                .text_color(theme.foreground)
                .child(text)
                .into_any_element(),
        )
    }

    /// "Checking grammar with …" while a check runs (§5.12 AI running), with Cancel.
    fn render_grammar_banner(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = self.media.grammar_banner.clone()?;
        let this = cx.entity().downgrade();
        Some(
            h_flex()
                .id("banner-grammar")
                .test_support()
                .h(px(32.))
                .px_3()
                .gap_2()
                .bg(theme.info.opacity(0.14))
                .border_l_2()
                .border_color(theme.info)
                .child(div().flex_1().truncate().child(text))
                .child(
                    Button::new("banner-grammar-cancel")
                        .xsmall()
                        .outline()
                        .label(format!("{}  Esc", i18n::t(cx, "common.cancel")))
                        .on_click(move |_, window, cx| {
                            let _ =
                                this.update(cx, |shell, cx| shell.cancel_grammar_check(window, cx));
                        }),
                )
                .into_any_element(),
        )
    }
}
