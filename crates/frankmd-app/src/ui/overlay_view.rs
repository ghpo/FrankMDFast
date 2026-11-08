//! The overlay layer: scrim and the one open overlay (ui-spec §5).

use super::prelude::*;
use crate::scale::px;
use gpui_kit::*;

impl Shell {
    pub(super) fn render_overlay(
        &mut self,
        kind: OverlayKind,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let dismissible = match kind {
            OverlayKind::Dialog("recovery") => false,
            OverlayKind::Dialog("unlock") => true,
            OverlayKind::Dialog(hosted) => self.hosted_dismissible(hosted, cx),
            _ => true,
        };
        let top = if self.layout.width_class == WidthClass::Narrow {
            px(16.)
        } else {
            window.viewport_size().height * 0.12
        };
        let content: AnyElement = match kind {
            OverlayKind::Palette(_) => self.render_palette(window, cx).into_any_element(),
            OverlayKind::QuickOpen => div()
                .w(px(680.))
                .max_w(relative(1.))
                .child(self.render_quick_open_list(false, theme, cx))
                .into_any_element(),
            OverlayKind::Settings => self.render_settings(window, cx).into_any_element(),
            OverlayKind::Help => self.render_help(theme, cx).into_any_element(),
            OverlayKind::Dialog("recovery") => {
                self.render_recovery(theme, window, cx).into_any_element()
            }
            OverlayKind::Dialog("recent-switcher") => {
                self.render_switcher(theme, cx).into_any_element()
            }
            OverlayKind::Dialog("unlock") => {
                self.render_unlock_dialog(theme, cx).into_any_element()
            }
            OverlayKind::Dialog("attachments") => {
                self.render_attachments_dialog(theme, cx).into_any_element()
            }
            OverlayKind::Dialog(kind) => match self.hosted_dialog(kind) {
                Some(view) => view,
                None => self.render_file_dialog(kind, window, cx),
            },
            OverlayKind::SearchInFiles => self.render_search(cx).into_any_element(),
        };
        div()
            .id("overlay-scrim")
            .test_support()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .pt(top)
            .px_4()
            .bg(theme.background.opacity(0.6))
            .on_click(move |_, window, cx| {
                if dismissible {
                    let _ = this.update(cx, |shell, cx| {
                        shell
                            .editor
                            .pane
                            .update(cx, |p, _| p.clear_pending_insertion());
                        shell.close_overlay(window, cx)
                    });
                }
            })
            .child(
                div()
                    .max_w(relative(1.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(content),
            )
    }
}

impl Shell {
    /// Ctrl+Tab switcher (ui-spec §5.2): MRU notes of this session, 400 px.
    fn render_switcher(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let selected = self.switcher.selected().map(str::to_owned);
        let this = cx.entity().downgrade();
        let rows: Vec<_> = self
            .switcher
            .mru()
            .iter()
            .map(|path| {
                let (folder, name) = match path.rsplit_once('/') {
                    Some((f, n)) => (f.to_owned(), n.trim_end_matches(".md").to_owned()),
                    None => (String::new(), path.trim_end_matches(".md").to_owned()),
                };
                let is_selected = selected.as_deref() == Some(path.as_str());
                let this = this.clone();
                let target = path.clone();
                h_flex()
                    .id(SharedString::from(format!("recent-switcher-row-{path}")))
                    .test_support()
                    .h(px(28.))
                    .px_3()
                    .gap_2()
                    .when(is_selected, |el| {
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
                    .on_click(move |_, window, cx| {
                        let target = target.clone();
                        let _ = this.update(cx, |shell, cx| {
                            shell.switcher.cancel();
                            shell.overlays.close();
                            shell.open_path(&target, window, cx);
                        });
                    })
            })
            .collect();
        v_flex()
            .id("recent-switcher")
            .test_support()
            .key_context(actions::CX_OVERLAY)
            .track_focus(&self.overlay_focus)
            .w(px(400.))
            .max_w(relative(1.))
            .py_1()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .children(rows)
    }
}
