//! Status line (ui-spec §3.6).

use super::prelude::*;
use crate::scale::px;
use gpui_kit::*;

impl Shell {
    // ----- status line -------------------------------------------------------------------

    pub(super) fn render_status_line(
        &mut self,
        arrangement: &frankmd_workspace::layout::PaneArrangement,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let save = self.save_state();
        let save_text = save.label_key().map(|key| {
            let glyph = match save {
                SaveState::Unsaved => "●",
                SaveState::Saving => "↻",
                SaveState::Saved => "✓",
                SaveState::Failed => "✕",
                SaveState::DraftNotStored => "⚠",
                SaveState::Clean => "",
            };
            let label = if save == SaveState::DraftNotStored {
                "⚠".into()
            } else {
                i18n::t(cx, key)
            };
            format!("{glyph} {label}")
        });
        let message = self.messages.current(self.clock.instant()).cloned();
        let has_note = self.active_path().is_some();
        let (line, col) = if has_note {
            self.editor.caret()
        } else {
            (0, 0)
        };
        let vim_mode = if self.vim {
            self.editor.vim_mode(cx)
        } else {
            None
        };
        let vim_line = if self.vim {
            self.editor.vim_line(cx)
        } else {
            None
        };
        let compact = arrangement.status_compact;
        // §3.6 context chip: what Ctrl+. does at the caret (clickable).
        let chip = if has_note && !compact {
            self.editor
                .pane
                .read(cx)
                .context_action(cx)
                .map(|a| i18n::t(cx, a.label_key()))
        } else {
            None
        };
        let vault_down = self
            .banners
            .is_raised(frankmd_workspace::banners::BannerKind::VaultUnreachable);
        h_flex()
            .id("status-line")
            .test_support()
            .h(px(24.))
            .px_2()
            .gap_3()
            .flex_shrink_0()
            .text_xs()
            .bg(theme.status_bar)
            .border_t_1()
            .border_color(theme.status_bar_border)
            .when(self.layout.zen, |el| el.opacity(0.5))
            .when_some(vim_mode, |el, mode| {
                // §3.6 mode colours: normal accent, insert green, visual magenta, command yellow.
                let bg = match mode {
                    "INSERT" => theme.success,
                    "VISUAL" | "V-LINE" | "V-BLOCK" => theme.magenta,
                    "COMMAND" => theme.warning,
                    _ => theme.primary,
                };
                let label = mode;
                el.child(
                    div()
                        .id("status-mode")
                        .test_support()
                        .px_1()
                        .bg(bg)
                        .text_color(theme.primary_foreground)
                        .font_semibold()
                        .child(label),
                )
            })
            .child(
                div()
                    .id("status-save")
                    .test_support()
                    .when(save.is_error(), |el| el.text_color(theme.danger))
                    .when(save == SaveState::Saved, |el| el.text_color(theme.success))
                    .children(save_text),
            )
            .child(
                div()
                    .id("status-message")
                    .test_support()
                    .flex_1()
                    .truncate()
                    .when_some(vim_line.clone(), |el, line| {
                        el.font_family(theme.mono_font_family.clone()).child(line)
                    })
                    .when(vim_line.is_none(), |el| {
                        el.when_some(message, |el, m| {
                            el.when(m.error, |el| el.text_color(theme.danger))
                                .child(m.text)
                        })
                    }),
            )
            .when_some(chip, |el, label| {
                let this = cx.entity().downgrade();
                el.child(
                    div()
                        .id("status-context-chip")
                        .test_support()
                        .px_1()
                        .rounded(theme.radius)
                        .bg(theme.secondary)
                        .hover(|s| s.bg(theme.list_hover))
                        .child(format!("Ctrl+. {label}"))
                        .on_click(move |_, window, cx| {
                            let _ = this.update(cx, |shell, cx| {
                                shell.run_command("edit.context_action", window, cx)
                            });
                        }),
                )
            })
            .when(self.locked_note.is_some(), |el| {
                el.child(
                    div()
                        .id("status-encrypted")
                        .test_support()
                        .text_color(theme.warning)
                        .child(i18n::t(cx, "settings_sheet.encryption.locked_tag")),
                )
            })
            .when(has_note, |el| {
                el.child(div().id("status-position").child(format!("{line}:{col}")))
            })
            .when(self.layout.typewriter && !compact, |el| el.child("⌶"))
            .when(!self.scroll_sync && !compact, |el| el.child("⛓"))
            .when(
                !compact && self.layout.width_class == WidthClass::Wide,
                |el| {
                    el.child(
                        div()
                            .id("status-vault")
                            .when(vault_down, |el| el.text_color(theme.danger))
                            .child(self.vault_name.clone()),
                    )
                },
            )
    }
}
