//! Locked-note UX: padlock content, quick unlock dialog and the vault lock
//! button in the sidebar dock.

use crate::scale::px;

use super::prelude::*;
use gpui_kit::*;

impl Shell {
    /// Path shown in the title/tree/status: the locked note or the open one.
    pub(crate) fn current_path(&self) -> Option<String> {
        self.locked_note.clone().or_else(|| self.active_path())
    }

    /// Encrypt/Decrypt from the context menu or palette. Locked vault → ask
    /// for the password and keep the action pending.
    pub(crate) fn request_note_encryption(
        &mut self,
        path: &str,
        encrypt: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.encryption.seal().is_none() {
            self.open_encryption_settings(window, cx);
            return;
        }
        if self.encryption.is_unlocked() {
            self.set_note_encryption(path, encrypt, window, cx);
            return;
        }
        self.pending_encryption = Some((path.to_owned(), encrypt));
        self.open_unlock(window, cx);
    }

    /// Lock the vault key; encrypted open notes become the padlock view.
    pub(crate) fn lock_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let encrypted_open = self.active_path().filter(|path| {
            self.core
                .as_ref()
                .and_then(|core| core.vault.note_encryption(path).ok())
                .is_some_and(|(encrypted, _)| encrypted)
        });
        self.encryption.lock();
        self.preview
            .update(cx, |preview, cx| preview.clear_media_cache(cx));
        if let Some(path) = encrypted_open {
            self.locked_note = Some(path);
            self.dispatch(Command::CloseNote, window, cx);
            self.update_title(window);
        }
        self.unlock_error = None;
        let message = i18n::t(cx, "settings_sheet.encryption.locked");
        self.show_message(&message, false, cx);
        cx.notify();
    }

    /// Open Settings › Encryption (create key / backup / change).
    pub(crate) fn open_encryption_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.category = super::settings::Category::Encryption;
        self.open_overlay(OverlayKind::Settings, window, cx);
    }

    /// Show the quick unlock dialog (or Settings when there is no seal yet).
    pub(crate) fn open_unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.encryption.seal().is_none() {
            self.open_encryption_settings(window, cx);
            return;
        }
        if self.encryption.is_unlocked() {
            return;
        }
        self.unlock_error = None;
        self.open_overlay(OverlayKind::Dialog("unlock"), window, cx);
        self.unlock_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
    }

    /// Verify the unlock password and resume the pending action/note.
    pub(crate) fn submit_unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.unlock_input.read(cx).value().to_string();
        match self.encryption.unlock(&password) {
            Ok(()) => {
                self.unlock_error = None;
                self.unlock_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                let pending = self.pending_encryption.take();
                let pending_attachment = self.pending_attachment.take();
                if self.overlays.is_open(&OverlayKind::Dialog("unlock")) {
                    self.close_overlay(window, cx);
                }
                let locked = self.locked_note.take();
                let effect = self.core.as_mut().map(|core| core.ws.rescan());
                if let Some(effect) = effect {
                    self.run_effect(effect, cx);
                }
                if let Some((path, encrypt)) = pending {
                    self.set_note_encryption(&path, encrypt, window, cx);
                } else if let Some(path) = locked {
                    self.open_path(&path, window, cx);
                }
                self.resume_pending_attachment(pending_attachment, window, cx);
                let message = i18n::t(cx, "settings_sheet.encryption.unlocked");
                self.show_message(&message, false, cx);
            }
            Err(_) => {
                self.unlock_error = Some(i18n::t(cx, "settings_sheet.encryption.wrong_password"));
                self.unlock_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                cx.notify();
            }
        }
    }

    /// Padlock placeholder shared by the editor and preview panes.
    pub(super) fn render_locked_panel(
        &mut self,
        id: &'static str,
        path: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".md")
            .to_owned();
        let has_seal = self.encryption.seal().is_some();
        let unlock_this = cx.entity().downgrade();
        let settings_this = cx.entity().downgrade();
        let action = if has_seal {
            Button::new("locked-note-unlock")
                .primary()
                .label(i18n::t(cx, "settings_sheet.encryption.unlock"))
                .on_click(move |_, window, cx| {
                    let _ = unlock_this.update(cx, |shell, cx| shell.open_unlock(window, cx));
                })
        } else {
            Button::new("locked-note-create")
                .primary()
                .label(i18n::t(cx, "settings_sheet.encryption.create"))
                .on_click(move |_, window, cx| {
                    let _ = settings_this
                        .update(cx, |shell, cx| shell.open_encryption_settings(window, cx));
                })
        };
        v_flex()
            .id(id)
            .test_support()
            .flex_1()
            .h_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_6()
            .child(
                crate::ui::icons::ph("lock")
                    .with_size(px(48.))
                    .text_color(theme.warning),
            )
            .child(div().text_lg().font_semibold().child(name))
            .child(
                div()
                    .max_w(px(460.))
                    .text_center()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(i18n::t(
                        cx,
                        if has_seal {
                            "settings_sheet.encryption.locked_body"
                        } else {
                            "settings_sheet.encryption.locked_no_key"
                        },
                    )),
            )
            .child(
                h_flex().gap_2().child(action).child(
                    Button::new("locked-note-settings")
                        .label(i18n::t(cx, "settings_sheet.encryption.open_settings"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_encryption_settings(window, cx)
                        })),
                ),
            )
            .into_any_element()
    }

    /// Quick unlock modal (`OverlayKind::Dialog("unlock")`).
    pub(super) fn render_unlock_dialog(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = i18n::t(cx, "settings_sheet.encryption.unlock_title");
        let submit = cx.entity().downgrade();
        let cancel = cx.entity().downgrade();
        let input = self.unlock_input.clone();
        v_flex()
            .id("unlock-dialog")
            .test_support()
            .w(px(380.))
            .p_4()
            .gap_2()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .child(div().text_sm().font_semibold().child(title))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(i18n::t(cx, "settings_sheet.encryption.unlock_hint")),
            )
            .child(Input::new(&input).id("unlock-password"))
            .when_some(self.unlock_error.clone(), |el, error| {
                el.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("unlock-cancel")
                            .label(i18n::t(cx, "common.cancel"))
                            .on_click(move |_, window, cx| {
                                let _ =
                                    cancel.update(cx, |shell, cx| shell.close_overlay(window, cx));
                            }),
                    )
                    .child(
                        Button::new("unlock-confirm")
                            .primary()
                            .label(i18n::t(cx, "settings_sheet.encryption.unlock"))
                            .on_click(move |_, window, cx| {
                                let _ =
                                    submit.update(cx, |shell, cx| shell.submit_unlock(window, cx));
                            }),
                    ),
            )
            .into_any_element()
    }
}
