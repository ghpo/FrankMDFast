//! At-rest encryption for FrankMDFast.
//!
//! - **KDF**: Argon2id (m=64 MiB, t=2, p=1) over a 16-byte salt.
//! - **AEAD**: XChaCha20-Poly1305, random 24-byte nonce per file.
//! - **Seal** (`encryption.json`): salt + parameters + an encrypted verifier.
//!   It never contains the password or the key; losing it plus the password
//!   makes the ciphertext unrecoverable.
//! - **Notes** keep the `.md` name and gain a text envelope (per-note opt-in).
//! - **Media** keep their name and gain a binary envelope.
//!
//! The key only lives in [`Session`] memory and is zeroized on lock/drop.

use std::sync::{Mutex, PoisonError};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Note envelope start line.
pub const NOTE_BEGIN: &str = "-----BEGIN FRANKMD ENCRYPTED-----";
/// Note envelope end line.
pub const NOTE_END: &str = "-----END FRANKMD ENCRYPTED-----";
/// Media envelope magic.
pub const MEDIA_MAGIC: &[u8] = b"FRANKMDENC1";
/// Refuse media larger than this (decrypt/encrypt happens in memory).
pub const MAX_MEDIA_BYTES: usize = 512 * 1024 * 1024;

const NOTE_AAD: &[u8] = b"frankmd-note-v1";
const MEDIA_AAD: &[u8] = b"frankmd-media-v1";
const SEAL_AAD: &[u8] = b"frankmd-seal-v1";
const SEAL_VERIFY: &[u8] = b"frankmd-seal-verify-v1";
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const M_COST: u32 = 65_536;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

/// Crypto failures.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// No key in the session (locked).
    #[error("the vault key is locked")]
    Locked,
    /// No seal was created/imported for this vault.
    #[error("no encryption key is configured for this vault")]
    NoSeal,
    /// The password does not match the seal.
    #[error("wrong password")]
    WrongPassword,
    /// The file is not in a recognized encrypted envelope.
    #[error("not an encrypted FrankMDFast file")]
    NotEncrypted,
    /// Malformed envelope/seal.
    #[error("invalid encrypted format")]
    InvalidFormat,
    /// AEAD failure (tampering or wrong key).
    #[error("could not decrypt (wrong key or corrupted data)")]
    Decrypt,
    /// Encryption/write failure.
    #[error("could not encrypt")]
    Encrypt,
    /// No OS randomness.
    #[error("could not obtain secure randomness")]
    Random,
    /// KDF failure.
    #[error("key derivation failed")]
    Kdf,
    /// Media too large for the in-memory format.
    #[error("media file is too large to encrypt in memory")]
    TooLarge,
}

/// A password-derived key (zeroized on drop).
pub type SecretKey = Zeroizing<[u8; KEY_LEN]>;

/// Serialized seal (`encryption.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealFile {
    /// Format version.
    pub version: u32,
    /// KDF id.
    pub kdf: String,
    /// Argon2id memory cost (KiB).
    pub m_cost: u32,
    /// Argon2id iterations.
    pub t_cost: u32,
    /// Argon2id parallelism.
    pub p_cost: u32,
    /// Base64 salt.
    pub salt: String,
    /// Base64 verifier nonce.
    pub nonce: String,
    /// Base64 encrypted verifier.
    pub verifier: String,
}

impl SealFile {
    /// Create a new seal and key from a password.
    pub fn create(password: &str) -> Result<(Self, SecretKey), CryptoError> {
        let mut salt = [0u8; SALT_LEN];
        fill(&mut salt)?;
        let key = derive_key(password, &salt, M_COST, T_COST, P_COST)?;
        let mut nonce = [0u8; NONCE_LEN];
        fill(&mut nonce)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&*key));
        let verifier = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: SEAL_VERIFY,
                    aad: SEAL_AAD,
                },
            )
            .map_err(|_| CryptoError::Decrypt)?;
        Ok((
            Self {
                version: 1,
                kdf: "argon2id".to_owned(),
                m_cost: M_COST,
                t_cost: T_COST,
                p_cost: P_COST,
                salt: B64.encode(salt),
                nonce: B64.encode(nonce),
                verifier: B64.encode(verifier),
            },
            key,
        ))
    }

    /// Verify a password and return the derived key.
    pub fn unlock(&self, password: &str) -> Result<SecretKey, CryptoError> {
        if self.version != 1 || self.kdf != "argon2id" {
            return Err(CryptoError::InvalidFormat);
        }
        let salt = B64
            .decode(&self.salt)
            .map_err(|_| CryptoError::InvalidFormat)?;
        let nonce = B64
            .decode(&self.nonce)
            .map_err(|_| CryptoError::InvalidFormat)?;
        let verifier = B64
            .decode(&self.verifier)
            .map_err(|_| CryptoError::InvalidFormat)?;
        if salt.len() != SALT_LEN || nonce.len() != NONCE_LEN {
            return Err(CryptoError::InvalidFormat);
        }
        let key = derive_key(password, &salt, self.m_cost, self.t_cost, self.p_cost)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&*key));
        let plain = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &verifier,
                    aad: SEAL_AAD,
                },
            )
            .map_err(|_| CryptoError::WrongPassword)?;
        if plain != SEAL_VERIFY {
            return Err(CryptoError::WrongPassword);
        }
        Ok(key)
    }

    /// Serialize to pretty JSON (for the backup/clipboard).
    pub fn encode(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Parse JSON.
    pub fn decode(text: &str) -> Result<Self, CryptoError> {
        serde_json::from_str(text).map_err(|_| CryptoError::InvalidFormat)
    }
}

/// One vault's in-memory key session.
#[derive(Debug)]
pub struct Session {
    key: Mutex<Option<SecretKey>>,
    seal: Mutex<Option<SealFile>>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            key: Mutex::new(None),
            seal: Mutex::new(None),
        }
    }
}

impl Session {
    /// Session with no seal/key.
    pub fn new() -> Self {
        Self::default()
    }

    /// Session with a seal loaded but locked.
    pub fn with_seal(seal: SealFile) -> Self {
        Self {
            key: Mutex::new(None),
            seal: Mutex::new(Some(seal)),
        }
    }

    /// Current seal, if configured.
    pub fn seal(&self) -> Option<SealFile> {
        self.seal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replace the seal (import). The session stays locked until unlock.
    pub fn set_seal(&self, seal: Option<SealFile>) {
        *self.seal.lock().unwrap_or_else(PoisonError::into_inner) = seal;
        self.lock();
    }

    /// `true` when a key is in memory.
    pub fn is_unlocked(&self) -> bool {
        self.key
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Drop/zeroize the key.
    pub fn lock(&self) {
        *self.key.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Create a seal and keep the key in memory.
    pub fn create(&self, password: &str) -> Result<SealFile, CryptoError> {
        let (seal, key) = SealFile::create(password)?;
        *self.seal.lock().unwrap_or_else(PoisonError::into_inner) = Some(seal.clone());
        *self.key.lock().unwrap_or_else(PoisonError::into_inner) = Some(key);
        Ok(seal)
    }

    /// Unlock with a password.
    pub fn unlock(&self, password: &str) -> Result<(), CryptoError> {
        let seal = self.seal().ok_or(CryptoError::NoSeal)?;
        let key = seal.unlock(password)?;
        *self.key.lock().unwrap_or_else(PoisonError::into_inner) = Some(key);
        Ok(())
    }

    /// Run `f` with a copy of the key (zeroized afterwards).
    fn with_key<T>(
        &self,
        f: impl FnOnce(&[u8; KEY_LEN]) -> Result<T, CryptoError>,
    ) -> Result<T, CryptoError> {
        let guard = self.key.lock().unwrap_or_else(PoisonError::into_inner);
        let key = guard.as_ref().ok_or(CryptoError::Locked)?;
        f(key)
    }

    /// Encrypt a note to its text envelope.
    pub fn encrypt_note(&self, plain: &[u8]) -> Result<Vec<u8>, CryptoError> {
        self.with_key(|key| encrypt_note_with(key, plain))
    }

    /// Decrypt a note envelope.
    pub fn decrypt_note(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        self.with_key(|key| decrypt_note_with(key, data))
    }

    /// Encrypt media.
    pub fn encrypt_media(&self, plain: &[u8]) -> Result<Vec<u8>, CryptoError> {
        self.with_key(|key| encrypt_media_with(key, plain))
    }

    /// Decrypt media.
    pub fn decrypt_media(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        self.with_key(|key| decrypt_media_with(key, data))
    }

    /// Re-encrypt a whole vault with a new password. `transform` receives the
    /// old and new keys and must rewrite every encrypted file atomically.
    /// On success the session keeps the new key.
    pub fn rekey(
        &self,
        new_password: &str,
        mut transform: impl FnMut(&[u8; KEY_LEN], &[u8; KEY_LEN]) -> Result<(), CryptoError>,
    ) -> Result<SealFile, CryptoError> {
        let old = {
            let guard = self.key.lock().unwrap_or_else(PoisonError::into_inner);
            guard.as_ref().ok_or(CryptoError::Locked)?.clone()
        };
        let (seal, new_key) = SealFile::create(new_password)?;
        transform(&old, &new_key)?;
        *self.seal.lock().unwrap_or_else(PoisonError::into_inner) = Some(seal.clone());
        *self.key.lock().unwrap_or_else(PoisonError::into_inner) = Some(new_key);
        Ok(seal)
    }
}

/// `true` for a note text envelope.
pub fn is_note_encrypted(data: &[u8]) -> bool {
    data.starts_with(NOTE_BEGIN.as_bytes())
}

/// `true` for a media envelope.
pub fn is_media_encrypted(data: &[u8]) -> bool {
    data.starts_with(MEDIA_MAGIC)
}

/// `true` for either envelope.
pub fn is_encrypted(data: &[u8]) -> bool {
    is_note_encrypted(data) || is_media_encrypted(data)
}

/// Encrypt a note with an explicit key.
pub fn encrypt_note_with(key: &[u8; KEY_LEN], plain: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let mut nonce = [0u8; NONCE_LEN];
    fill(&mut nonce)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: NOTE_AAD,
            },
        )
        .map_err(|_| CryptoError::Decrypt)?;
    let header = serde_json::json!({
        "v": 1,
        "alg": "xchacha20poly1305",
        "nonce": B64.encode(nonce),
    });
    let out = format!("{NOTE_BEGIN}\n{}\n{}\n{NOTE_END}\n", header, B64.encode(ct));
    Ok(out.into_bytes())
}

/// Decrypt a note with an explicit key.
pub fn decrypt_note_with(key: &[u8; KEY_LEN], data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let text = std::str::from_utf8(data).map_err(|_| CryptoError::InvalidFormat)?;
    let mut lines = text.lines();
    if lines.next() != Some(NOTE_BEGIN) {
        return Err(CryptoError::NotEncrypted);
    }
    let header: serde_json::Value =
        serde_json::from_str(lines.next().ok_or(CryptoError::InvalidFormat)?)
            .map_err(|_| CryptoError::InvalidFormat)?;
    let nonce = header
        .get("nonce")
        .and_then(|v| v.as_str())
        .ok_or(CryptoError::InvalidFormat)
        .and_then(|s| B64.decode(s).map_err(|_| CryptoError::InvalidFormat))?;
    if nonce.len() != NONCE_LEN {
        return Err(CryptoError::InvalidFormat);
    }
    let body = lines.next().ok_or(CryptoError::InvalidFormat)?;
    let ct = B64.decode(body).map_err(|_| CryptoError::InvalidFormat)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ct,
                aad: NOTE_AAD,
            },
        )
        .map_err(|_| CryptoError::Decrypt)
}

/// Encrypt media with an explicit key.
pub fn encrypt_media_with(key: &[u8; KEY_LEN], plain: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if plain.len() > MAX_MEDIA_BYTES {
        return Err(CryptoError::TooLarge);
    }
    let mut nonce = [0u8; NONCE_LEN];
    fill(&mut nonce)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: MEDIA_AAD,
            },
        )
        .map_err(|_| CryptoError::Decrypt)?;
    let mut out = Vec::with_capacity(MEDIA_MAGIC.len() + 1 + NONCE_LEN + ct.len());
    out.extend_from_slice(MEDIA_MAGIC);
    out.push(1);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt media with an explicit key.
pub fn decrypt_media_with(key: &[u8; KEY_LEN], data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let header = MEDIA_MAGIC.len() + 1 + NONCE_LEN;
    if data.len() < header || !data.starts_with(MEDIA_MAGIC) || data[MEDIA_MAGIC.len()] != 1 {
        return Err(CryptoError::NotEncrypted);
    }
    let nonce = &data[MEDIA_MAGIC.len() + 1..header];
    let ct = &data[header..];
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: MEDIA_AAD,
            },
        )
        .map_err(|_| CryptoError::Decrypt)
}

fn derive_key(
    password: &str,
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<SecretKey, CryptoError> {
    let params =
        Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN)).map_err(|_| CryptoError::Kdf)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(password.as_bytes(), salt, &mut *key)
        .map_err(|_| CryptoError::Kdf)?;
    Ok(key)
}

fn fill(buf: &mut [u8]) -> Result<(), CryptoError> {
    getrandom::fill(buf).map_err(|_| CryptoError::Random)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const PW: &str = "uma frase longa e unica para a vault";

    #[test]
    fn seal_round_trip_and_wrong_password() {
        let (seal, _key) = SealFile::create(PW).unwrap();
        let text = seal.encode();
        let parsed = SealFile::decode(&text).unwrap();
        assert!(parsed.unlock(PW).is_ok());
        assert!(matches!(
            parsed.unlock("wrong password"),
            Err(CryptoError::WrongPassword)
        ));
    }

    #[test]
    fn note_round_trip_and_tamper() {
        let (_, key) = SealFile::create(PW).unwrap();
        let plain = b"# Titulo\n\nconteudo com acentos: \xc3\xa7\xc3\xa3o\n";
        let enc = encrypt_note_with(&key, plain).unwrap();
        assert!(is_note_encrypted(&enc));
        assert_eq!(decrypt_note_with(&key, &enc).unwrap(), plain);
        let text = String::from_utf8(enc).unwrap();
        let body = text.lines().nth(2).unwrap();
        let mut chars: Vec<char> = body.chars().collect();
        chars[0] = if chars[0] == 'A' { 'B' } else { 'A' };
        let tampered = text.replace(body, &chars.iter().collect::<String>());
        assert!(matches!(
            decrypt_note_with(&key, tampered.as_bytes()),
            Err(CryptoError::Decrypt)
        ));
    }

    #[test]
    fn media_round_trip() {
        let (_, key) = SealFile::create(PW).unwrap();
        let plain = b"\x89PNG\r\n\x1a\nfake image bytes";
        let enc = encrypt_media_with(&key, plain).unwrap();
        assert!(is_media_encrypted(&enc));
        assert_eq!(decrypt_media_with(&key, &enc).unwrap(), plain);
    }

    #[test]
    fn session_locks_and_clears_the_key() {
        let session = Session::new();
        assert!(!session.is_unlocked());
        session.create(PW).unwrap();
        assert!(session.is_unlocked());
        let enc = session.encrypt_note(b"hello").unwrap();
        session.lock();
        assert!(!session.is_unlocked());
        assert!(matches!(
            session.decrypt_note(&enc),
            Err(CryptoError::Locked)
        ));
        session.unlock(PW).unwrap();
        assert_eq!(session.decrypt_note(&enc).unwrap(), b"hello");
    }

    #[test]
    fn rekey_changes_the_key() {
        let session = Session::new();
        session.create(PW).unwrap();
        let enc = session.encrypt_note(b"segredo").unwrap();
        let new_pw = "outra frase longa e unica";
        let new_enc = std::sync::Mutex::new(Vec::new());
        session
            .rekey(new_pw, |old, new| {
                let plain = decrypt_note_with(old, &enc)?;
                *new_enc.lock().unwrap() = encrypt_note_with(new, &plain)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            session.decrypt_note(&new_enc.lock().unwrap()).unwrap(),
            b"segredo"
        );
        session.lock();
        assert!(matches!(
            session.unlock(PW),
            Err(CryptoError::WrongPassword)
        ));
        session.unlock(new_pw).unwrap();
    }
}
