//! Per-note encryption actions (tree context menu) and the attachment cascade.
//!
//! A note and the local files it references are encrypted/decrypted together.
//! Shared attachments are handled conservatively: on decrypt, an attachment
//! still referenced by another encrypted note stays encrypted.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use frankmd_core::tree::TreeNode;
use frankmd_crypto::Session;
use frankmd_fs::service::NotesService;
use frankmd_fs::{NoteFile, Vault};

use super::prelude::*;
use crate::note_attachments::attachments_for_source;
use gpui_kit::*;

/// What a note encryption/decryption did to its attachments.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AttachmentCascade {
    /// Attachments newly encrypted.
    pub(crate) encrypted: usize,
    /// Attachments decrypted.
    pub(crate) decrypted: usize,
    /// Attachments left encrypted because another encrypted note uses them.
    pub(crate) kept: usize,
    /// Attachments that failed to decrypt (the note itself is already plain).
    pub(crate) failed: usize,
    /// The reference scan could not be completed; all encrypted attachments
    /// were kept encrypted.
    pub(crate) scan_warning: bool,
}

impl Shell {
    /// Encrypt/decrypt one note and its referenced local attachments.
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
        let Some(core) = self.core.as_ref() else {
            let message = i18n::t(cx, "settings_sheet.remote.vault_unavailable");
            self.show_message(&message, true, cx);
            return;
        };
        let service = NotesService::new(&core.vault);
        let session = Arc::clone(&self.encryption);
        let result = if encrypt {
            encrypt_note_with_attachments(&service, &session, path)
        } else {
            decrypt_note_with_attachments(&service, &session, path)
        };
        match result {
            Ok(cascade) => {
                let key = if encrypt {
                    "settings_sheet.encryption.note_encrypted"
                } else {
                    "settings_sheet.encryption.note_decrypted"
                };
                let message = if encrypt && cascade.encrypted > 0 {
                    i18n::t_args(
                        cx,
                        "settings_sheet.encryption.note_encrypted_attachments",
                        &[("count", &cascade.encrypted.to_string())],
                    )
                } else if !encrypt && cascade.decrypted > 0 {
                    i18n::t_args(
                        cx,
                        "settings_sheet.encryption.note_decrypted_attachments",
                        &[("count", &cascade.decrypted.to_string())],
                    )
                } else {
                    i18n::t(cx, key)
                };
                self.show_message(&message, false, cx);
                if cascade.kept > 0 {
                    let message = i18n::t_args(
                        cx,
                        "settings_sheet.encryption.attachments_kept",
                        &[("count", &cascade.kept.to_string())],
                    );
                    self.show_message(&message, false, cx);
                }
                if cascade.scan_warning {
                    let message = i18n::t(cx, "settings_sheet.encryption.attachment_scan_failed");
                    self.show_message(&message, true, cx);
                }
                if cascade.failed > 0 {
                    let message = i18n::t_args(
                        cx,
                        "settings_sheet.encryption.attachments_failed",
                        &[("count", &cascade.failed.to_string())],
                    );
                    self.show_message(&message, true, cx);
                }
            }
            Err(err) => {
                self.show_message(&SharedString::from(err), true, cx);
                return;
            }
        }
        if self.active_path().as_deref() == Some(path)
            && let Some(core) = self.core.as_mut()
        {
            core.ws.set_open_encrypted(encrypt);
        }
        self.preview
            .update(cx, |preview, cx| preview.clear_media_cache(cx));
        let effect = self.core.as_mut().map(|core| core.ws.rescan());
        if let Some(effect) = effect {
            self.run_effect(effect, cx);
        }
        let _ = window;
        cx.notify();
    }

    /// Settings › Encryption: encrypt plaintext attachments of every already
    /// encrypted note. This is the one-time migration for vaults created by
    /// earlier versions.
    pub(crate) fn encrypt_attachments_of_encrypted_notes(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.encryption.is_unlocked() {
            self.settings.encryption_repair =
                Some((i18n::t(cx, "settings_sheet.encryption.locked_hint"), true));
            cx.notify();
            return;
        }
        let Some(core) = self.core.as_ref() else {
            self.settings.encryption_repair =
                Some((i18n::t(cx, "settings_sheet.remote.vault_unavailable"), true));
            cx.notify();
            return;
        };
        let vault = core.vault.clone();
        let session = Arc::clone(&self.encryption);
        self.settings.encryption_repair = Some((
            i18n::t(cx, "settings_sheet.encryption.attachment_repair_running"),
            false,
        ));
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { repair_encrypted_note_attachments(&vault, &session) })
                .await;
            this.update(cx, |shell, cx| {
                shell.settings.encryption_repair = Some(match result {
                    Ok(count) => (
                        i18n::t_args(
                            cx,
                            "settings_sheet.encryption.attachment_repair_done",
                            &[("count", &count.to_string())],
                        ),
                        false,
                    ),
                    Err(error) => (SharedString::from(error), true),
                });
                shell
                    .preview
                    .update(cx, |preview, cx| preview.clear_media_cache(cx));
                cx.notify();
            })
            .ok();
        });
        self.settings.encryption_jobs.push(task);
    }
}

/// Encrypt a note's content and all plaintext attachments it references.
pub(crate) fn encrypt_note_with_attachments(
    service: &NotesService<'_>,
    session: &Session,
    path: &str,
) -> Result<AttachmentCascade, String> {
    let note = service
        .read(path)
        .map_err(|err| format!("não foi possível ler a nota: {err}"))?;
    let source = String::from_utf8_lossy(&note.bytes);
    let attachments = attachments_for_source(&source, path, service.vault().root());
    let mut changed = Vec::new();
    for attachment in &attachments {
        if crate::encryption::media_is_encrypted(attachment) {
            continue;
        }
        match crate::encryption::encrypt_media_file(session, attachment) {
            Ok(()) => changed.push(attachment.clone()),
            Err(err) => {
                rollback_attachments(session, &changed);
                return Err(format!(
                    "falha ao cifrar o anexo {}: {err:#}",
                    attachment.display()
                ));
            }
        }
    }
    if !note.encrypted
        && let Err(err) = service.set_encrypted(path, true)
    {
        rollback_attachments(session, &changed);
        return Err(format!("falha ao cifrar a nota: {err}"));
    }
    Ok(AttachmentCascade {
        encrypted: changed.len(),
        ..AttachmentCascade::default()
    })
}

/// Decrypt a note's content, then its attachments except those still used by
/// another encrypted note.
pub(crate) fn decrypt_note_with_attachments(
    service: &NotesService<'_>,
    session: &Session,
    path: &str,
) -> Result<AttachmentCascade, String> {
    let note = service
        .read(path)
        .map_err(|err| format!("não foi possível ler a nota: {err}"))?;
    if note.encrypted
        && let Err(err) = service.set_encrypted(path, false)
    {
        return Err(format!("falha ao decifrar a nota: {err}"));
    }
    let source = String::from_utf8_lossy(&note.bytes);
    let attachments = attachments_for_source(&source, path, service.vault().root());
    let (shared, scan_warning) = match encrypted_attachments_in_other_notes(service, session, path)
    {
        Ok(shared) => (shared, false),
        Err(err) => {
            tracing::warn!(%err, "could not scan encrypted notes; keeping attachments encrypted");
            (HashSet::new(), true)
        }
    };
    let mut cascade = AttachmentCascade {
        scan_warning,
        ..AttachmentCascade::default()
    };
    for attachment in attachments {
        if !crate::encryption::media_is_encrypted(&attachment) {
            continue;
        }
        if scan_warning || shared.contains(&attachment) {
            cascade.kept += 1;
            continue;
        }
        match crate::encryption::decrypt_media_file(session, &attachment) {
            Ok(()) => cascade.decrypted += 1,
            Err(err) => {
                tracing::warn!(path = %attachment.display(), %err, "attachment decrypt failed");
                cascade.failed += 1;
            }
        }
    }
    Ok(cascade)
}

/// Encrypt plaintext attachments under every encrypted note.
pub(crate) fn repair_encrypted_note_attachments(
    vault: &Vault,
    session: &Session,
) -> Result<usize, String> {
    let service = NotesService::new(vault);
    let paths = note_paths(vault)?;
    let mut encrypted = 0;
    for path in paths {
        let Ok((is_encrypted, _)) = vault.note_encryption(&path) else {
            continue;
        };
        if !is_encrypted {
            continue;
        }
        let note = service
            .read(&path)
            .map_err(|err| format!("não foi possível ler {path}: {err}"))?;
        let source = String::from_utf8_lossy(&note.bytes);
        for attachment in attachments_for_source(&source, &path, vault.root()) {
            if crate::encryption::media_is_encrypted(&attachment) {
                continue;
            }
            crate::encryption::encrypt_media_file(session, &attachment).map_err(|err| {
                format!("falha ao cifrar o anexo {}: {err:#}", attachment.display())
            })?;
            encrypted += 1;
        }
    }
    Ok(encrypted)
}

fn rollback_attachments(session: &Session, changed: &[PathBuf]) {
    for path in changed.iter().rev() {
        if let Err(err) = crate::encryption::decrypt_media_file(session, path) {
            tracing::error!(path = %path.display(), %err, "could not roll back attachment encryption");
        }
    }
}

fn encrypted_attachments_in_other_notes(
    service: &NotesService<'_>,
    _session: &Session,
    current: &str,
) -> Result<HashSet<PathBuf>, String> {
    let paths = note_paths(service.vault())?;
    let mut shared = HashSet::new();
    for path in paths {
        if path == current {
            continue;
        }
        let (encrypted, _) = service
            .vault()
            .note_encryption(&path)
            .map_err(|err| err.to_string())?;
        if !encrypted {
            continue;
        }
        let note: NoteFile = service.read(&path).map_err(|err| err.to_string())?;
        let source = String::from_utf8_lossy(&note.bytes);
        shared.extend(attachments_for_source(
            &source,
            &path,
            service.vault().root(),
        ));
    }
    Ok(shared)
}

fn note_paths(vault: &Vault) -> Result<Vec<String>, String> {
    let tree = vault.list_tree().map_err(|err| err.to_string())?;
    let mut out = Vec::new();
    fn walk(nodes: &[TreeNode], out: &mut Vec<String>) {
        for node in nodes {
            match node {
                TreeNode::Folder { children, .. } => walk(children, out),
                TreeNode::File { path, .. } => out.push(path.as_str().to_owned()),
            }
        }
    }
    walk(&tree, &mut out);
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, clippy::unwrap_used)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use frankmd_crypto::Session;
    use frankmd_fs::Vault;
    use frankmd_fs::service::NotesService;

    use super::{
        decrypt_note_with_attachments, encrypt_note_with_attachments,
        repair_encrypted_note_attachments,
    };

    fn service(dir: &Path) -> (Vault, Arc<Session>) {
        let mut vault = Vault::open(dir, dir.join("state")).unwrap();
        let session = Arc::new(Session::new());
        session.create("test password 1234").unwrap();
        vault.set_encryption(Some(Arc::clone(&session)));
        (vault, session)
    }

    fn write_note(vault: &Vault, path: &str, content: &str) {
        NotesService::new(vault)
            .write(path, content.as_bytes())
            .unwrap();
    }

    fn write_media(dir: &Path, path: &str, bytes: &[u8]) {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, bytes).unwrap();
    }

    #[test]
    fn note_encryption_cascades_to_attachments_and_back() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, session) = service(dir.path());
        write_media(dir.path(), "images/pic.png", b"png");
        write_media(dir.path(), "images/talk.m4a", b"m4a");
        write_note(
            &vault,
            "notes/note.md",
            "![pic](../images/pic.png)\n\n[audio](../images/talk.m4a)\n",
        );
        let cascade =
            encrypt_note_with_attachments(&NotesService::new(&vault), &session, "notes/note.md")
                .unwrap();
        assert_eq!(cascade.encrypted, 2);
        assert!(crate::encryption::media_is_encrypted(
            &dir.path().join("images/pic.png")
        ));
        assert!(crate::encryption::media_is_encrypted(
            &dir.path().join("images/talk.m4a")
        ));
        let cascade =
            decrypt_note_with_attachments(&NotesService::new(&vault), &session, "notes/note.md")
                .unwrap();
        assert_eq!(cascade.decrypted, 2);
        assert!(!crate::encryption::media_is_encrypted(
            &dir.path().join("images/pic.png")
        ));
    }

    #[test]
    fn repair_encrypts_plaintext_attachments_of_an_encrypted_note() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, session) = service(dir.path());
        write_media(dir.path(), "images/old.png", b"png");
        write_note(&vault, "old.md", "![x](images/old.png)\n");
        NotesService::new(&vault)
            .set_encrypted("old.md", true)
            .unwrap();
        assert!(!crate::encryption::media_is_encrypted(
            &dir.path().join("images/old.png")
        ));
        let count = repair_encrypted_note_attachments(&vault, &session).unwrap();
        assert_eq!(count, 1);
        assert!(crate::encryption::media_is_encrypted(
            &dir.path().join("images/old.png")
        ));
    }

    #[test]
    fn shared_attachment_stays_encrypted_until_last_encrypted_note_is_decrypted() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, session) = service(dir.path());
        write_media(dir.path(), "images/shared.png", b"png");
        write_note(&vault, "a.md", "![x](images/shared.png)\n");
        write_note(&vault, "b.md", "![x](images/shared.png)\n");
        for note in ["a.md", "b.md"] {
            encrypt_note_with_attachments(&NotesService::new(&vault), &session, note).unwrap();
        }
        let cascade =
            decrypt_note_with_attachments(&NotesService::new(&vault), &session, "a.md").unwrap();
        assert_eq!(cascade.kept, 1);
        assert!(crate::encryption::media_is_encrypted(
            &dir.path().join("images/shared.png")
        ));
        let cascade =
            decrypt_note_with_attachments(&NotesService::new(&vault), &session, "b.md").unwrap();
        assert_eq!(cascade.decrypted, 1);
        assert!(!crate::encryption::media_is_encrypted(
            &dir.path().join("images/shared.png")
        ));
    }
}
