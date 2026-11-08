//! Encryption glue for the desktop app: seal persistence, per-file media
//! helpers and the whole-vault rekey pass used by Settings › Encryption.

use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use frankmd_crypto::{
    CryptoError, MEDIA_MAGIC, NOTE_BEGIN, SealFile, Session, decrypt_media_with, decrypt_note_with,
    encrypt_media_with, encrypt_note_with, is_media_encrypted, is_note_encrypted,
};

/// Seal file inside the per-vault state directory.
pub const SEAL_FILE: &str = "encryption.json";

/// Load the vault's seal (locked) or an empty session.
pub fn load_session(state_dir: &Path) -> Arc<Session> {
    let path = state_dir.join(SEAL_FILE);
    let seal = std::fs::read(&path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| SealFile::decode(&text).ok());
    Arc::new(match seal {
        Some(seal) => Session::with_seal(seal),
        None => Session::new(),
    })
}

/// Persist the seal (0600) in the per-vault state directory.
pub fn save_seal(state_dir: &Path, seal: &SealFile) -> anyhow::Result<()> {
    crate::app_fs::ensure_private_dir(state_dir)?;
    frankmd_fs::atomic::atomic_replace(
        &state_dir.join(SEAL_FILE),
        seal.encode().as_bytes(),
        frankmd_fs::atomic::ModePolicy::Fixed(0o600),
    )
    .map_err(|err| anyhow!("could not write encryption seal: {err}"))
}

/// `true` when the media file starts with the encrypted-media magic.
/// Only the header is read, so a large audio/video file is never loaded just
/// to decide whether it is encrypted.
pub fn media_is_encrypted(path: &Path) -> bool {
    use std::io::Read as _;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut prefix = [0u8; frankmd_crypto::MEDIA_MAGIC.len()];
    let read = file.read(&mut prefix).unwrap_or(0);
    prefix[..read].starts_with(frankmd_crypto::MEDIA_MAGIC)
}

/// Read media plaintext (decrypting when needed). Media is capped by
/// [`frankmd_crypto::MAX_MEDIA_BYTES`] because the format is in-memory.
pub fn read_media(session: &Session, path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    if is_media_encrypted(&bytes) {
        session.decrypt_media(&bytes).map_err(|err| err.to_string())
    } else {
        Ok(bytes)
    }
}

/// Copy an attachment to a user-chosen destination, decrypting a media
/// envelope when needed. The destination is replaced atomically; an existing
/// file keeps its permissions, a new file gets the normal umask.
pub fn save_attachment(session: &Session, source: &Path, destination: &Path) -> anyhow::Result<()> {
    let bytes = read_media(session, source).map_err(|err| anyhow!(err))?;
    frankmd_fs::atomic::atomic_replace(
        destination,
        &bytes,
        frankmd_fs::atomic::ModePolicy::Preserve,
    )
    .map_err(|err| anyhow!("could not write attachment: {err}"))
}

/// Encrypt one media file in place (atomic).
pub fn encrypt_media_file(session: &Session, path: &Path) -> anyhow::Result<()> {
    let bytes = std::fs::read(path)?;
    if is_media_encrypted(&bytes) {
        return Ok(());
    }
    let encrypted = session
        .encrypt_media(&bytes)
        .map_err(|err| anyhow!(err.to_string()))?;
    frankmd_fs::atomic::atomic_replace(path, &encrypted, frankmd_fs::atomic::ModePolicy::Preserve)
        .map_err(|err| anyhow!("could not write encrypted media: {err}"))
}

/// Decrypt one media file in place (atomic).
pub fn decrypt_media_file(session: &Session, path: &Path) -> anyhow::Result<()> {
    let bytes = std::fs::read(path)?;
    if !is_media_encrypted(&bytes) {
        return Ok(());
    }
    let plain = session
        .decrypt_media(&bytes)
        .map_err(|err| anyhow!(err.to_string()))?;
    frankmd_fs::atomic::atomic_replace(path, &plain, frankmd_fs::atomic::ModePolicy::Preserve)
        .map_err(|err| anyhow!("could not write decrypted media: {err}"))
}

/// Re-encrypt every encrypted note/media under `vault_root` with the new key.
/// Used by the password-change flow inside [`Session::rekey`].
pub fn rekey_files(
    vault_root: &Path,
    old: &[u8; 32],
    new: &[u8; 32],
) -> Result<usize, CryptoError> {
    let mut count = 0;
    walk(vault_root, &mut |path| {
        if path.extension().is_some_and(|ext| ext == "fed") {
            return Ok(());
        }
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(()),
        };
        let rewritten = if is_note_encrypted(&bytes) {
            let plain = decrypt_note_with(old, &bytes)?;
            encrypt_note_with(new, &plain)?
        } else if is_media_encrypted(&bytes) {
            let plain = decrypt_media_with(old, &bytes)?;
            encrypt_media_with(new, &plain)?
        } else {
            return Ok(());
        };
        frankmd_fs::atomic::atomic_replace(
            path,
            &rewritten,
            frankmd_fs::atomic::ModePolicy::Preserve,
        )
        .map_err(|_| CryptoError::Encrypt)?;
        count += 1;
        Ok(())
    })?;
    Ok(count)
}

fn walk(
    dir: &Path,
    f: &mut impl FnMut(&Path) -> Result<(), CryptoError>,
) -> Result<(), CryptoError> {
    let reading = std::fs::read_dir(dir).map_err(|_| CryptoError::Encrypt)?;
    for entry in reading.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            walk(&path, f)?;
        } else if kind.is_file() {
            f(&path)?;
        }
    }
    Ok(())
}

/// Seal backup text (the seal contains no secret, but it is required to
/// decrypt; losing it makes the data unrecoverable).
pub fn seal_backup(session: &Session) -> Option<String> {
    session.seal().map(|seal| seal.encode())
}

/// Export the seal as a JSON file (0600) at a user-chosen path.
pub fn export_seal_to(path: &Path, seal: &SealFile) -> anyhow::Result<()> {
    frankmd_fs::atomic::atomic_replace(
        path,
        seal.encode().as_bytes(),
        frankmd_fs::atomic::ModePolicy::Fixed(0o600),
    )
    .map_err(|err| anyhow!("could not write the seal file: {err}"))
}

/// Read and validate a seal JSON file.
pub fn read_seal_file(path: &Path) -> anyhow::Result<SealFile> {
    let bytes =
        std::fs::read(path).map_err(|err| anyhow!("could not read the seal file: {err}"))?;
    if bytes.len() > 64 * 1024 {
        return Err(anyhow!("the seal file is too large"));
    }
    let text = String::from_utf8(bytes).map_err(|_| anyhow!("the seal file is not UTF-8"))?;
    SealFile::decode(&text).map_err(|err| anyhow!("invalid seal file: {err}"))
}

/// Import a seal from its JSON text (the session stays locked).
pub fn import_seal(session: &Session, text: &str) -> anyhow::Result<SealFile> {
    let seal = SealFile::decode(text.trim()).map_err(|err| anyhow!(err.to_string()))?;
    session.set_seal(Some(seal.clone()));
    Ok(seal)
}

/// Validate a new password before creating/changing the key.
pub fn validate_password(password: &str) -> Result<(), &'static str> {
    if password.chars().count() < 12 {
        Err("use at least 12 characters")
    } else {
        Ok(())
    }
}

/// Marker used by the preview/media code to detect an envelope quickly.
pub fn looks_encrypted(bytes: &[u8]) -> bool {
    bytes.starts_with(NOTE_BEGIN.as_bytes()) || bytes.starts_with(MEDIA_MAGIC)
}

#[allow(clippy::disallowed_methods)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_attachment_decrypts_media_and_honors_locking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("talk.m4a");
        let session = Session::new();
        session
            .create("a long enough test password")
            .expect("create key");
        let plain = b"audio bytes\x00\x01";
        let envelope = session.encrypt_media(plain).expect("encrypt");
        std::fs::write(&source, &envelope).expect("write source");
        assert!(media_is_encrypted(&source));

        let destination = dir.path().join("saved.m4a");
        save_attachment(&session, &source, &destination).expect("save");
        assert_eq!(std::fs::read(&destination).expect("read"), plain);

        session.lock();
        assert!(save_attachment(&session, &source, &dir.path().join("locked.m4a")).is_err());
    }
}
