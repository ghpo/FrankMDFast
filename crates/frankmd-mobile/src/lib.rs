//! JNI surface for the Android client.
//!
//! The Kotlin side owns the vault path (app-private storage) and passes it to
//! [`Java_com_ghpo_frankmd_NativeVault_nativeOpen`]. All filesystem mutation
//! stays in `frankmd-fs`; this crate only adapts the API to Java strings/JSON.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use frankmd_core::tree::TreeNode;
use frankmd_crypto::{SealFile, Session};
use frankmd_fs::service::NotesService;
use frankmd_fs::{NoteFile, TrashBackend, Vault};
use jni::EnvUnowned;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jlong, jstring};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
enum BridgeError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Jni(#[from] jni::errors::Error),
    #[error(transparent)]
    Fs(#[from] frankmd_fs::FsError),
}

impl From<String> for BridgeError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

struct OpenedVault {
    vault: Arc<Vault>,
    session: Arc<Session>,
}

static HANDLES: OnceLock<Mutex<HashMap<i64, Arc<OpenedVault>>>> = OnceLock::new();
static NEXT_HANDLE: OnceLock<Mutex<i64>> = OnceLock::new();

fn handles() -> &'static Mutex<HashMap<i64, Arc<OpenedVault>>> {
    HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> i64 {
    let mut next = NEXT_HANDLE
        .get_or_init(|| Mutex::new(1))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = *next;
    *next += 1;
    id
}

fn get_string(value: &JString<'_>) -> Result<String, BridgeError> {
    Ok(value.to_string())
}

fn local_attachments(note: &str, source: &str, root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for href in frankmd_media::usage::references_in(source) {
        let Some(path) = resolve_attachment(note, &href, root) else {
            continue;
        };
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

fn resolve_attachment(note: &str, href: &str, root: &Path) -> Option<PathBuf> {
    let href = href.trim();
    if href.is_empty()
        || href.starts_with("//")
        || href
            .split_once(':')
            .is_some_and(|(scheme, _)| !scheme.is_empty())
    {
        return None;
    }
    let path = href.split(['#', '?']).next().unwrap_or_default();
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .replace("&amp;", "&")
        .replace("&quot;", "\"");
    let mut parts: Vec<&str> = if decoded.starts_with('/') {
        Vec::new()
    } else {
        note.rsplit_once('/')
            .map(|(dir, _)| dir.split('/').collect())
            .unwrap_or_default()
    };
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(segment),
        }
    }
    let rel = parts.join("/");
    if rel.is_empty() || rel.ends_with(".md") || rel.ends_with(".fed") {
        return None;
    }
    let full = root.join(&rel);
    if !full.starts_with(root) {
        return None;
    }
    let meta = std::fs::symlink_metadata(&full).ok()?;
    (meta.is_file() && !meta.file_type().is_symlink()).then_some(full)
}

fn encrypt_media_file(session: &frankmd_crypto::Session, path: &Path) -> Result<bool, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if frankmd_crypto::is_media_encrypted(&bytes) {
        return Ok(false);
    }
    let encrypted = session.encrypt_media(&bytes).map_err(|e| e.to_string())?;
    frankmd_fs::atomic::atomic_replace(path, &encrypted, frankmd_fs::atomic::ModePolicy::Preserve)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

fn decrypt_media_file(session: &frankmd_crypto::Session, path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if !frankmd_crypto::is_media_encrypted(&bytes) {
        return Ok(());
    }
    let plain = session.decrypt_media(&bytes).map_err(|e| e.to_string())?;
    frankmd_fs::atomic::atomic_replace(path, &plain, frankmd_fs::atomic::ModePolicy::Preserve)
        .map_err(|e| e.to_string())
}

fn with_vault<R>(
    handle: jlong,
    f: impl FnOnce(&Vault) -> Result<R, BridgeError>,
) -> Result<R, BridgeError> {
    let opened = handles()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&handle)
        .cloned()
        .ok_or_else(|| BridgeError::Message("vault handle is closed".to_owned()))?;
    f(&opened.vault)
}

#[derive(Serialize)]
struct SearchHit {
    path: String,
    title: String,
    snippet: String,
    locked: bool,
}

#[derive(Serialize)]
struct NoteEntry {
    path: String,
    title: String,
    encrypted: bool,
    locked: bool,
}

fn collect_note_paths(nodes: &[TreeNode], out: &mut Vec<String>) {
    for node in nodes {
        match node {
            TreeNode::Folder { children, .. } => collect_note_paths(children, out),
            TreeNode::File { path, .. } => out.push(path.as_str().to_owned()),
        }
    }
}

fn tree_notes(nodes: &[TreeNode], out: &mut Vec<NoteEntry>) {
    for node in nodes {
        match node {
            TreeNode::Folder { children, .. } => tree_notes(children, out),
            TreeNode::File {
                path,
                name,
                encrypted,
                locked,
                ..
            } => out.push(NoteEntry {
                path: path.as_str().to_owned(),
                title: name.clone(),
                encrypted: *encrypted,
                locked: *locked,
            }),
        }
    }
}

/// Open a vault and return its JNI handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeOpen(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    root: JString<'_>,
    state_dir: JString<'_>,
) -> jlong {
    env.with_env(|_env| -> Result<jlong, BridgeError> {
        let root = get_string(&root)?;
        let state_dir = get_string(&state_dir)?;
        let trash = PathBuf::from(&state_dir).join("trash");
        let seal_path = PathBuf::from(&state_dir).join("encryption.json");
        let session = match std::fs::read(&seal_path) {
            Ok(bytes) => std::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| SealFile::decode(text).ok())
                .map_or_else(
                    || Arc::new(Session::new()),
                    |seal| Arc::new(Session::with_seal(seal)),
                ),
            Err(_) => Arc::new(Session::new()),
        };
        let mut vault = Vault::open(root, &state_dir)?.with_trash(TrashBackend::Directory(trash));
        vault.set_encryption(Some(Arc::clone(&session)));
        let id = next_handle();
        handles()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id,
                Arc::new(OpenedVault {
                    vault: Arc::new(vault),
                    session,
                }),
            );
        Ok(id)
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Close a vault handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeClose(
    _env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    handles()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&handle);
}

/// JSON array of notes (`[{path,title}]`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeList(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jstring {
    env.with_env(|env| -> Result<jstring, BridgeError> {
        let json = with_vault(handle, |vault| {
            let tree = vault
                .list_tree()
                .map_err(|e| BridgeError::Message(e.to_string()))?;
            let mut notes = Vec::new();
            tree_notes(&tree, &mut notes);
            serde_json::to_string(&notes).map_err(|e| BridgeError::Message(e.to_string()))
        })?;
        Ok(JString::from_str(env, json)?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Read one note as UTF-8 (lossy for invalid bytes).
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeRead(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
    path: JString<'_>,
) -> jstring {
    let path_string = get_string(&path);
    env.with_env(|env| -> Result<jstring, BridgeError> {
        let path = path_string?;
        let text = with_vault(handle, |vault| {
            let note: NoteFile = NotesService::new(vault)
                .read(&path)
                .map_err(|e| BridgeError::Message(e.to_string()))?;
            Ok(String::from_utf8_lossy(&note.bytes).into_owned())
        })?;
        Ok(JString::from_str(env, text)?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Import a desktop-exported seal. The caller must reopen the vault afterwards.
#[allow(clippy::disallowed_methods)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeImportSeal(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
    json: JString<'_>,
) -> jboolean {
    let json = get_string(&json);
    env.with_env(|_env| -> Result<jboolean, BridgeError> {
        let json = json?;
        let seal = SealFile::decode(&json)
            .map_err(|e| BridgeError::Message(format!("invalid seal: {e}")))?;
        let opened = handles()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&handle)
            .cloned()
            .ok_or_else(|| BridgeError::Message("vault handle is closed".to_owned()))?;
        let dir = opened.vault.state_dir();
        std::fs::create_dir_all(dir).map_err(|e| BridgeError::Message(e.to_string()))?;
        frankmd_fs::atomic::atomic_replace(
            &dir.join("encryption.json"),
            seal.encode().as_bytes(),
            frankmd_fs::atomic::ModePolicy::Fixed(0o600),
        )
        .map_err(|e| BridgeError::Message(e.to_string()))?;
        Ok(true)
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Encrypt or decrypt one note.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeSetEncrypted(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
    path: JString<'_>,
    encrypted: jboolean,
) -> jstring {
    let path = get_string(&path);
    env.with_env(|env| -> Result<jstring, BridgeError> {
        let path = path?;
        let revision = with_vault(handle, |vault| {
            let (already_encrypted, already_locked) = vault
                .note_encryption(&path)
                .map_err(|e| BridgeError::Message(e.to_string()))?;
            if encrypted && already_encrypted {
                // Already encrypted: the operation is idempotent. Do not read
                // the sealed note while the key is locked just to re-encrypt it.
                return Ok(String::new());
            }
            if !encrypted && already_encrypted && already_locked {
                return Err(BridgeError::Message(
                    "vault key is locked; unlock before decrypting this note".to_owned(),
                ));
            }
            let service = NotesService::new(vault);
            let note = service
                .read(&path)
                .map_err(|e| BridgeError::Message(e.to_string()))?;
            let source = String::from_utf8_lossy(&note.bytes);
            let attachments = local_attachments(&path, &source, vault.root());
            let session = vault
                .encryption()
                .ok_or_else(|| BridgeError::Message("no encryption session".to_owned()))?;
            if encrypted {
                let mut changed = Vec::new();
                for attachment in &attachments {
                    match encrypt_media_file(session, attachment) {
                        Ok(true) => changed.push(attachment.clone()),
                        Ok(false) => {}
                        Err(err) => {
                            for path in changed.iter().rev() {
                                let _ = decrypt_media_file(session, path);
                            }
                            return Err(BridgeError::Message(err));
                        }
                    }
                }
                if !note.encrypted
                    && let Err(err) = service.set_encrypted(&path, true)
                {
                    for path in changed.iter().rev() {
                        let _ = decrypt_media_file(session, path);
                    }
                    return Err(BridgeError::Message(err.to_string()));
                }
            } else {
                if note.encrypted {
                    service
                        .set_encrypted(&path, false)
                        .map_err(|e| BridgeError::Message(e.to_string()))?;
                }
                for attachment in &attachments {
                    let _ = decrypt_media_file(session, attachment);
                }
            }
            let refreshed = service
                .read(&path)
                .map_err(|e| BridgeError::Message(e.to_string()))?;
            Ok(refreshed.revision.0)
        })?;
        Ok(JString::from_str(env, revision)?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Unlock the vault key with the password.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeUnlock(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
    password: JString<'_>,
) -> jboolean {
    let password = get_string(&password);
    env.with_env(|_env| -> Result<jboolean, BridgeError> {
        let password = password?;
        let opened = handles()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&handle)
            .cloned()
            .ok_or_else(|| BridgeError::Message("vault handle is closed".to_owned()))?;
        opened
            .session
            .unlock(&password)
            .map(|()| true)
            .map_err(|e| BridgeError::Message(e.to_string()))
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Lock the in-memory vault key.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeLock(
    _env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if let Some(opened) = handles()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&handle)
    {
        opened.session.lock();
    }
}

/// JSON `{seal, unlocked}`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeStatus(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jstring {
    env.with_env(|env| -> Result<jstring, BridgeError> {
        let opened = handles()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&handle)
            .cloned()
            .ok_or_else(|| BridgeError::Message("vault handle is closed".to_owned()))?;
        let value = serde_json::json!({
            "seal": opened.session.seal().is_some(),
            "unlocked": opened.session.is_unlocked()
        });
        Ok(JString::from_str(env, value.to_string())?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Search note paths/titles/content. Locked encrypted notes match by path/title only.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeSearch(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
    query: JString<'_>,
    limit: jint,
) -> jstring {
    let query = get_string(&query);
    env.with_env(|env| -> Result<jstring, BridgeError> {
        let query = query?.trim().to_lowercase();
        let limit = limit.max(1) as usize;
        let hits = with_vault(handle, |vault| {
            let mut notes = Vec::new();
            collect_note_paths(
                &vault
                    .list_tree()
                    .map_err(|e| BridgeError::Message(e.to_string()))?,
                &mut notes,
            );
            let service = NotesService::new(vault);
            let mut out = Vec::new();
            for path in notes {
                let title = path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&path)
                    .trim_end_matches(".md")
                    .to_owned();
                let (encrypted, locked) = vault.note_encryption(&path).unwrap_or((false, false));
                let content = if locked {
                    String::new()
                } else {
                    service
                        .read(&path)
                        .map(|note| String::from_utf8_lossy(&note.bytes).into_owned())
                        .unwrap_or_default()
                };
                let haystack = format!("{path}\n{title}\n{content}").to_lowercase();
                if !query.is_empty() && haystack.contains(&query) {
                    let snippet = content
                        .lines()
                        .find(|line| line.to_lowercase().contains(&query))
                        .unwrap_or(&title)
                        .chars()
                        .take(140)
                        .collect::<String>();
                    out.push(SearchHit {
                        path,
                        title,
                        snippet,
                        locked: encrypted && locked,
                    });
                    if out.len() >= limit {
                        break;
                    }
                }
            }
            Ok(out)
        })?;
        Ok(JString::from_str(
            env,
            serde_json::to_string(&hits).map_err(|e| BridgeError::Message(e.to_string()))?,
        )?
        .into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Render a note to HTML plus its base URL for the Android WebView.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativePreview(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
    path: JString<'_>,
) -> jstring {
    let path = get_string(&path);
    env.with_env(|env| -> Result<jstring, BridgeError> {
        let path = path?;
        let value = with_vault(handle, |vault| {
            let note = NotesService::new(vault)
                .read(&path)
                .map_err(|e| BridgeError::Message(e.to_string()))?;
            let source = String::from_utf8_lossy(&note.bytes);
            let doc =
                frankmd_markdown::render(&source, &frankmd_markdown::RenderOptions::default());
            let html = frankmd_markdown::html::to_html(&doc);
            let base = vault
                .root()
                .join(&path)
                .parent()
                .map(|dir| dir.display().to_string())
                .unwrap_or_else(|| vault.root().display().to_string());
            Ok(serde_json::json!({
                "html": html,
                "base": base,
                "locked": false,
            }))
        })?;
        Ok(JString::from_str(env, value.to_string())?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Move a note to the Android app trash directory.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeDelete(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
    path: JString<'_>,
) -> jstring {
    let path_string = get_string(&path);
    env.with_env(|env| -> Result<jstring, BridgeError> {
        let path = path_string?;
        with_vault(handle, |vault| {
            NotesService::new(vault)
                .trash(&path, None)
                .map_err(|e| BridgeError::Message(e.to_string()))
        })?;
        Ok(JString::from_str(env, "")?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Rename a note (the destination must not exist).
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeRename(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
    old_path: JString<'_>,
    new_path: JString<'_>,
) -> jstring {
    let old_path = get_string(&old_path);
    let new_path = get_string(&new_path);
    env.with_env(|env| -> Result<jstring, BridgeError> {
        let old_path = old_path?;
        let new_path = new_path?;
        with_vault(handle, |vault| {
            NotesService::new(vault)
                .rename(&old_path, &new_path)
                .map_err(|e| BridgeError::Message(e.to_string()))
        })?;
        Ok(JString::from_str(env, new_path)?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Upsert a note and return its SHA-256 revision.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ghpo_frankmd_NativeVault_nativeWrite(
    mut env: EnvUnowned<'_>,
    _class: JClass<'_>,
    handle: jlong,
    path: JString<'_>,
    text: JString<'_>,
) -> jstring {
    let path_string = get_string(&path);
    let text_string = get_string(&text);
    env.with_env(|env| -> Result<jstring, BridgeError> {
        let path = path_string?;
        let text = text_string?;
        let revision = with_vault(handle, |vault| {
            NotesService::new(vault)
                .write(&path, text.as_bytes())
                .map(|revision| revision.0)
                .map_err(|e| BridgeError::Message(e.to_string()))
        })?;
        Ok(JString::from_str(env, revision)?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}
