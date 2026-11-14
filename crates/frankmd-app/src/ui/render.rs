//! Root layout of the shell (ui-spec §3): sidebar, editor, preview and the
//! status line, the overlay layer on top; the two splitters (§3.1: 1 px line,
//! 6 px hit area, double-click resets, widths persisted per vault). Region
//! views live in sibling modules.

use crate::scale::px;
use frankmd_workspace::layout::{EDITOR_MIN_WIDTH, PREVIEW_MIN_WIDTH, SIDEBAR_WIDTH};
use frankmd_workspace::ui_state::{PREVIEW_RATIO_DEFAULT, SPLITTER_STEP, UiState};

use super::prelude::*;
use gpui_kit::*;

/// A splitter being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Splitter {
    /// Between the sidebar and the editor.
    Sidebar,
    /// Between the editor and the preview.
    Preview,
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.sync_rescan_pending {
            self.sync_rescan_pending = false;
            self.dispatch(
                frankmd_workspace::Command::External(frankmd_watch::WatchEvent::VaultChanged {
                    paths: Vec::new(),
                    removed: Vec::new(),
                }),
                window,
                cx,
            );
        }
        let width = f32::from(window.viewport_size().width);
        if (width - self.width).abs() > f32::EPSILON {
            self.on_width(width, window, cx);
        }
        let theme = cx.theme().clone();
        let arrangement = self.arrangement();
        let handler = Shell::command_handler(cx);
        let docked = arrangement.sidebar == SidebarMode::Docked;
        let split_preview = arrangement.preview == PreviewMode::Split && !arrangement.media_library;
        let sidebar_width = self.sidebar_width();

        let main = h_flex()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .when(docked, |el| {
                el.child(self.render_sidebar(Some(sidebar_width), &theme, cx))
                    .child(self.render_splitter(Splitter::Sidebar, &theme, cx))
            })
            .when(arrangement.media_library, |el| {
                el.child(self.render_media_library(&theme, cx))
            })
            .when(arrangement.editor && !arrangement.media_library, |el| {
                el.child(self.render_editor_pane(&arrangement, &theme, window, cx))
            })
            .when(split_preview && arrangement.editor, |el| {
                el.child(self.render_splitter(Splitter::Preview, &theme, cx))
            })
            .when(
                arrangement.preview != PreviewMode::Hidden && !arrangement.media_library,
                |el| {
                    el.child(self.render_preview(
                        arrangement.preview == PreviewMode::Swapped,
                        &theme,
                        cx,
                    ))
                },
            );

        let this = cx.entity().downgrade();
        let root = div()
            .id("shell")
            .key_context(actions::CX_WORKSPACE)
            .track_focus(&self.root_focus)
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .text_size(px(13.))
            // Ctrl released ends the Ctrl+Tab switcher with the selected note (§5.2).
            .on_modifiers_changed({
                let this = this.clone();
                move |ev: &ModifiersChangedEvent, window, cx| {
                    if !ev.modifiers.control {
                        let _ = this.update(cx, |shell, cx| {
                            if shell.switcher.is_active()
                                && shell
                                    .overlays
                                    .is_open(&OverlayKind::Dialog("recent-switcher"))
                            {
                                shell.recent_switch_commit(window, cx);
                            }
                        });
                    }
                }
            })
            .when(self.splitter.is_some(), |el| {
                let this = this.clone();
                let up = this.clone();
                el.cursor_col_resize()
                    .on_mouse_move(move |ev: &MouseMoveEvent, _, cx| {
                        let _ = this.update(cx, |shell, cx| {
                            shell.drag_splitter(f32::from(ev.position.x), cx)
                        });
                    })
                    .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                        let _ = up.update(cx, |shell, cx| shell.end_splitter_drag(cx));
                    })
            });
        actions::on_commands(root, handler)
            // The window opens with `TitleBar::window_options()` (transparent
            // titlebar, traffic lights at (9,9)); the TitleBar row belongs to
            // that setup: on macOS its 80 px inset clears the traffic lights
            // (#2), elsewhere it draws the window controls and owns dragging
            // under client-side decorations.
            .child(gpui_kit::component::TitleBar::new())
            .child(main)
            .child(self.render_status_line(&arrangement, &theme, cx))
            .when(arrangement.sidebar == SidebarMode::Drawer, |el| {
                el.child(self.render_drawer(&theme, cx))
            })
            .when_some(self.overlays.current().cloned(), |el, kind| {
                el.child(self.render_overlay(kind, &theme, window, cx))
            })
    }
}

impl Shell {
    /// Docked sidebar width (persisted, clamped).
    pub fn sidebar_width(&self) -> f32 {
        UiState::clamp_sidebar(self.ui_state.get().sidebar_width)
    }

    /// Preview share of the editor + preview width (persisted).
    pub fn preview_ratio(&self) -> f32 {
        self.ui_state.get().preview_ratio
    }

    /// Width shared by editor and preview.
    pub(crate) fn panes_width(&self) -> f32 {
        let sidebar = if self.arrangement().sidebar == SidebarMode::Docked {
            self.sidebar_width()
        } else {
            0.0
        };
        (self.width - sidebar).max(1.0)
    }

    fn render_splitter(
        &mut self,
        which: Splitter,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let id = match which {
            Splitter::Sidebar => "splitter-sidebar",
            Splitter::Preview => "splitter-preview",
        };
        let active = self.splitter == Some(which);
        let focus = match which {
            Splitter::Sidebar => self.splitter_focus.0.clone(),
            Splitter::Preview => self.splitter_focus.1.clone(),
        };
        div()
            .id(id)
            .test_support()
            .track_focus(&focus)
            .w(px(6.))
            .mx(px(-3.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .justify_center()
            .cursor_col_resize()
            .child(div().w(px(1.)).h_full().bg(if active {
                theme.primary
            } else {
                theme.sidebar_border
            }))
            .on_mouse_down(MouseButton::Left, {
                let this = this.clone();
                let focus = focus.clone();
                move |ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    focus.focus(window, cx);
                    let _ = this.update(cx, |shell, cx| {
                        if ev.click_count >= 2 {
                            shell.reset_splitter(which, cx);
                        } else {
                            shell.splitter = Some(which);
                            cx.notify();
                        }
                    });
                }
            })
            .on_key_down(move |ev: &KeyDownEvent, _, cx| {
                let step = match ev.keystroke.key.as_str() {
                    "left" => -SPLITTER_STEP,
                    "right" => SPLITTER_STEP,
                    _ => return,
                };
                cx.stop_propagation();
                let _ = this.update(cx, |shell, cx| shell.nudge_splitter(which, step, cx));
            })
    }

    /// Pointer at window x while dragging a splitter.
    pub(crate) fn drag_splitter(&mut self, x: f32, cx: &mut Context<Self>) {
        match self.splitter {
            Some(Splitter::Sidebar) => {
                let width = UiState::clamp_sidebar(x);
                self.ui_state.update_transient(|s| s.sidebar_width = width);
            }
            Some(Splitter::Preview) => {
                let total = self.panes_width();
                let ratio = (self.width - x) / total;
                let ratio = UiState::clamp_ratio(ratio, total, EDITOR_MIN_WIDTH, PREVIEW_MIN_WIDTH);
                self.ui_state.update_transient(|s| s.preview_ratio = ratio);
            }
            None => return,
        }
        cx.notify();
    }

    pub(crate) fn end_splitter_drag(&mut self, cx: &mut Context<Self>) {
        self.splitter = None;
        self.ui_state.flush();
        cx.notify();
    }

    /// Double-click: back to the default width / share.
    pub(crate) fn reset_splitter(&mut self, which: Splitter, cx: &mut Context<Self>) {
        self.splitter = None;
        match which {
            Splitter::Sidebar => self.ui_state.update(|s| s.sidebar_width = SIDEBAR_WIDTH),
            Splitter::Preview => self
                .ui_state
                .update(|s| s.preview_ratio = PREVIEW_RATIO_DEFAULT),
        }
        cx.notify();
    }

    /// Left/Right on a focused splitter moves it by 16 px (FE-021 fixed).
    pub(crate) fn nudge_splitter(&mut self, which: Splitter, step: f32, cx: &mut Context<Self>) {
        match which {
            Splitter::Sidebar => {
                let width = UiState::clamp_sidebar(self.sidebar_width() + step);
                self.ui_state.update(|s| s.sidebar_width = width);
            }
            Splitter::Preview => {
                let total = self.panes_width();
                // Moving the splitter right shrinks the preview.
                let ratio = self.preview_ratio() - step / total;
                let ratio = UiState::clamp_ratio(ratio, total, EDITOR_MIN_WIDTH, PREVIEW_MIN_WIDTH);
                self.ui_state.update(|s| s.preview_ratio = ratio);
            }
        }
        cx.notify();
    }
}
