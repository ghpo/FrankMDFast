//! Per-note encryption actions (tree context menu).

use super::prelude::*;
use gpui_kit::*;

impl Shell {
    /// Encrypt/decrypt one note. The vault key must be unlocked.
    pub(crate) fn set_note_encryption(
        &mut self,
        path: &str,
        encrypt: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.encryption.is_unlocked() {
            let message = i18n::t(cx, "settings_sheet.encryption.locked_hint");
            self.show_message(&message, true, cx);
            return;
        }
        let result = self
            .core
            .as_ref()
            .map(|core| frankmd_fs::NotesService::new(&core.vault).set_encrypted(path, encrypt));
        match result {
            Some(Ok(_)) => {
                let key = if encrypt {
                    "settings_sheet.encryption.note_encrypted"
                } else {
                    "settings_sheet.encryption.note_decrypted"
                };
                let message = i18n::t(cx, key);
                self.show_message(&message, false, cx);
            }
            Some(Err(err)) => {
                let message = SharedString::from(err.to_string());
                self.show_message(&message, true, cx);
            }
            None => {
                let message = i18n::t(cx, "settings_sheet.remote.vault_unavailable");
                self.show_message(&message, true, cx);
            }
        }
        if self.active_path().as_deref() == Some(path)
            && let Some(core) = self.core.as_mut()
        {
            core.ws.set_open_encrypted(encrypt);
        }
        let effect = self.core.as_mut().map(|core| core.ws.rescan());
        if let Some(effect) = effect {
            self.run_effect(effect, cx);
        }
        let _ = window;
        cx.notify();
    }
}
