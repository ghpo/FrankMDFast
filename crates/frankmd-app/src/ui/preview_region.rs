//! Preview pane header and the media library slot (ui-spec §3.5, §5.15).

use super::prelude::*;
use crate::scale::px;
use gpui_kit::*;

impl Shell {
    // ----- preview & library -----------------------------------------------------------

    pub(super) fn render_preview(
        &mut self,
        swapped: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let zoom_out = this.clone();
        let zoom_reset = this.clone();
        let zoom_in = this.clone();
        let sync = this.clone();
        let attachments = this.clone();
        let zoom = self.preview.read(cx).zoom();
        let attachment_count = self.preview.read(cx).attachments().len();
        let sync_on = self.scroll_sync;
        let preview_width = self.preview_ratio() * self.panes_width();
        v_flex()
            .id("preview-region")
            .when(swapped, |el| el.flex_1())
            .when(!swapped, |el| {
                el.w(gpui_kit::px(preview_width))
                    .flex_shrink_0()
                    .min_w(gpui_kit::px(frankmd_workspace::layout::PREVIEW_MIN_WIDTH))
            })
            .h_full()
            .border_l_1()
            .border_color(theme.sidebar_border)
            .child(
                h_flex()
                    .id("preview-header")
                    .h(px(32.))
                    .px_3()
                    .gap_1()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(i18n::t(cx, "preview.title")),
                    )
                    .child(div().flex_1())
                    .when(attachment_count > 0, |el| {
                        el.child(
                            Button::new("preview-attachments")
                                .ghost()
                                .xsmall()
                                .icon(crate::ui::icons::ph("paperclip"))
                                .tooltip(i18n::t_args(
                                    cx,
                                    "preview.attachments_count",
                                    &[("count", &attachment_count.to_string())],
                                ))
                                .on_click(move |_, window, cx| {
                                    let _ = attachments.update(cx, |shell, cx| {
                                        shell.open_attachments(None, window, cx)
                                    });
                                }),
                        )
                    })
                    .child(
                        Button::new("preview-zoom-out")
                            .ghost()
                            .xsmall()
                            .icon(crate::ui::icons::ph("minus"))
                            .tooltip(i18n::t(cx, "preview.zoom_out"))
                            .on_click(move |_, _, cx| {
                                let _ = zoom_out.update(cx, |shell, cx| {
                                    shell.preview.update(cx, |p, cx| p.zoom_out(cx))
                                });
                            }),
                    )
                    .child(
                        Button::new("preview-zoom-reset")
                            .ghost()
                            .xsmall()
                            .label(format!("{zoom}%"))
                            .tooltip(i18n::t(cx, "shell.reset_zoom"))
                            .on_click(move |_, _, cx| {
                                let _ = zoom_reset.update(cx, |shell, cx| {
                                    shell.preview.update(cx, |p, cx| p.reset_zoom(cx))
                                });
                            }),
                    )
                    .child(
                        Button::new("preview-zoom-in")
                            .ghost()
                            .xsmall()
                            .icon(crate::ui::icons::ph("plus"))
                            .tooltip(i18n::t(cx, "preview.zoom_in"))
                            .on_click(move |_, _, cx| {
                                let _ = zoom_in.update(cx, |shell, cx| {
                                    shell.preview.update(cx, |p, cx| p.zoom_in(cx))
                                });
                            }),
                    )
                    .child(
                        Button::new("preview-scroll-sync")
                            .ghost()
                            .xsmall()
                            .selected(sync_on)
                            .icon(crate::ui::icons::ph("link"))
                            .tooltip(i18n::t(cx, "shell.scroll_sync"))
                            .on_click(move |_, window, cx| {
                                let _ = sync.update(cx, |shell, cx| {
                                    shell.run_command("view.toggle_scroll_sync", window, cx)
                                });
                            }),
                    )
                    .child(
                        Button::new("preview-close")
                            .ghost()
                            .xsmall()
                            .icon(crate::ui::icons::ph("x"))
                            .tooltip(i18n::t(cx, "shell.close_preview"))
                            .on_click(move |_, window, cx| {
                                let _ = this.update(cx, |shell, cx| {
                                    shell.run_command("view.toggle_preview", window, cx)
                                });
                            }),
                    ),
            )
            .child(if let Some(path) = self.locked_note.clone() {
                self.render_locked_panel("locked-preview", &path, theme, cx)
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.preview.clone())
                    .into_any_element()
            })
    }

    pub(super) fn render_media_library(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(view) = self.media_library_view() {
            // The library pane carries the `media-library` id itself.
            return div()
                .flex_1()
                .h_full()
                .min_w_0()
                .track_focus(&self.library_focus)
                .child(view)
                .into_any_element();
        }
        v_flex()
            .id("media-library")
            .test_support()
            .track_focus(&self.library_focus)
            .flex_1()
            .h_full()
            .items_center()
            .justify_center()
            .text_color(theme.muted_foreground)
            .child(i18n::t(cx, "shell.media_library_later"))
            .into_any_element()
    }
}
