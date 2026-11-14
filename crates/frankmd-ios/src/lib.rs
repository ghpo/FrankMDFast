//! C ABI surface used by the SwiftUI iOS client.
//!
//! All strings are UTF-8 and owned by this library. Call
//! `frankmd_ios_free_string` after consuming a returned string. A null return
//! means the operation failed; the Swift wrapper should show a generic error.

#![allow(
    unsafe_code,
    missing_docs,
    clippy::disallowed_methods,
    clippy::not_unsafe_ptr_arg_deref
)]

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::Arc;

use frankmd_core::tree::TreeNode;
use frankmd_crypto::{SealFile, Session};
use frankmd_fs::service::NotesService;
use frankmd_fs::{NoteFile, TrashBackend, Vault};
use serde::Serialize;

pub struct FrankMDHandle {
    vault: Vault,
    session: Arc<Session>,
}

fn string(value: &str) -> *mut c_char {
    CString::new(value.replace('\0', "")).map_or(std::ptr::null_mut(), CString::into_raw)
}

fn input(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

fn open(root: &str, state: &str) -> Result<FrankMDHandle, String> {
    let state_dir = PathBuf::from(state);
    let seal_path = state_dir.join("encryption.json");
    let session = std::fs::read(&seal_path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| SealFile::decode(&text).ok())
        .map_or_else(
            || Arc::new(Session::new()),
            |seal| Arc::new(Session::with_seal(seal)),
        );
    let mut vault = Vault::open(root, &state_dir)
        .map_err(|e| e.to_string())?
        .with_trash(TrashBackend::Directory(state_dir.join("trash")));
    vault.set_encryption(Some(Arc::clone(&session)));
    Ok(FrankMDHandle { vault, session })
}

fn with_handle<R>(
    handle: *mut FrankMDHandle,
    f: impl FnOnce(&FrankMDHandle) -> Result<R, String>,
) -> Result<R, String> {
    if handle.is_null() {
        return Err("null handle".to_owned());
    }
    let handle = unsafe { &*handle };
    f(handle)
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
    folder: String,
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
                title: name.trim_end_matches(".md").to_owned(),
                folder: path
                    .as_str()
                    .rsplit_once('/')
                    .map_or_else(String::new, |(dir, _)| dir.to_owned()),
                encrypted: *encrypted,
                locked: *locked,
            }),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_open(
    root: *const c_char,
    state: *const c_char,
) -> *mut FrankMDHandle {
    let (Some(root), Some(state)) = (input(root), input(state)) else {
        return std::ptr::null_mut();
    };
    open(&root, &state).map_or(std::ptr::null_mut(), |handle| {
        Box::into_raw(Box::new(handle))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_close(handle: *mut FrankMDHandle) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle)) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_free_string(value: *mut c_char) {
    if !value.is_null() {
        unsafe { drop(CString::from_raw(value)) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_list(handle: *mut FrankMDHandle) -> *mut c_char {
    let result = with_handle(handle, |handle| {
        let tree = handle.vault.list_tree().map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        tree_notes(&tree, &mut entries);
        serde_json::to_string(&entries).map_err(|e| e.to_string())
    });
    result.map_or(std::ptr::null_mut(), |json| string(&json))
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_read(handle: *mut FrankMDHandle, path: *const c_char) -> *mut c_char {
    let path = input(path);
    let result = with_handle(handle, |handle| {
        let path = path.ok_or_else(|| "path is required".to_owned())?;
        let note: NoteFile = NotesService::new(&handle.vault)
            .read(&path)
            .map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&note.bytes).into_owned())
    });
    result.map_or(std::ptr::null_mut(), |text| string(&text))
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_write(
    handle: *mut FrankMDHandle,
    path: *const c_char,
    text: *const c_char,
) -> *mut c_char {
    let path = input(path);
    let text = input(text);
    let result = with_handle(handle, |handle| {
        let path = path.ok_or_else(|| "path is required".to_owned())?;
        let text = text.ok_or_else(|| "text is required".to_owned())?;
        NotesService::new(&handle.vault)
            .write(&path, text.as_bytes())
            .map(|revision| revision.0)
            .map_err(|e| e.to_string())
    });
    result.map_or(std::ptr::null_mut(), |revision| string(&revision))
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_create(
    handle: *mut FrankMDHandle,
    title: *const c_char,
) -> *mut c_char {
    let title = input(title).unwrap_or_else(|| "Nova nota".to_owned());
    let result = with_handle(handle, |handle| {
        let safe = title
            .trim()
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-");
        let safe = if safe.is_empty() { "Nova nota" } else { &safe };
        let mut path = format!("{safe}.md");
        let mut n = 2;
        while handle.vault.root().join(&path).exists() {
            path = format!("{safe} {n}.md");
            n += 1;
        }
        NotesService::new(&handle.vault)
            .write(&path, format!("# {safe}\n\n").as_bytes())
            .map(|_| path)
            .map_err(|e| e.to_string())
    });
    result.map_or(std::ptr::null_mut(), |path| string(&path))
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_rename(
    handle: *mut FrankMDHandle,
    old_path: *const c_char,
    new_path: *const c_char,
) -> *mut c_char {
    let old_path = input(old_path);
    let new_path = input(new_path);
    let result = with_handle(handle, |handle| {
        let old_path = old_path.ok_or_else(|| "old path is required".to_owned())?;
        let new_path = new_path.ok_or_else(|| "new path is required".to_owned())?;
        NotesService::new(&handle.vault)
            .rename(&old_path, &new_path)
            .map_err(|e| e.to_string())?;
        Ok(new_path)
    });
    result.map_or(std::ptr::null_mut(), |path| string(&path))
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_delete(handle: *mut FrankMDHandle, path: *const c_char) -> bool {
    let path = input(path);
    with_handle(handle, |handle| {
        let path = path.ok_or_else(|| "path is required".to_owned())?;
        NotesService::new(&handle.vault)
            .trash(&path, None)
            .map_err(|e| e.to_string())
    })
    .is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_status(handle: *mut FrankMDHandle) -> *mut c_char {
    let result = with_handle(handle, |handle| {
        Ok(serde_json::json!({
            "configured": handle.session.seal().is_some(),
            "unlocked": handle.session.is_unlocked()
        })
        .to_string())
    });
    result.map_or(std::ptr::null_mut(), |json| string(&json))
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_unlock(handle: *mut FrankMDHandle, password: *const c_char) -> bool {
    let password = input(password);
    with_handle(handle, |handle| {
        let password = password.ok_or_else(|| "password is required".to_owned())?;
        handle.session.unlock(&password).map_err(|e| e.to_string())
    })
    .is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_lock(handle: *mut FrankMDHandle) {
    if let Ok(()) = with_handle(handle, |handle| {
        handle.session.lock();
        Ok(())
    }) {}
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_create_seal(
    handle: *mut FrankMDHandle,
    password: *const c_char,
) -> bool {
    let password = input(password);
    with_handle(handle, |handle| {
        let password = password.ok_or_else(|| "password is required".to_owned())?;
        if handle.session.seal().is_some() {
            return Err("encryption is already configured".to_owned());
        }
        if password.chars().count() < 8 {
            return Err("password must have at least 8 characters".to_owned());
        }
        let seal = handle
            .session
            .create(&password)
            .map_err(|e| e.to_string())?;
        let dir = handle.vault.state_dir();
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        frankmd_fs::atomic::atomic_replace(
            &dir.join("encryption.json"),
            seal.encode().as_bytes(),
            frankmd_fs::atomic::ModePolicy::Fixed(0o600),
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_import_seal(handle: *mut FrankMDHandle, json: *const c_char) -> bool {
    let json = input(json);
    with_handle(handle, |handle| {
        let json = json.ok_or_else(|| "seal json is required".to_owned())?;
        let seal = SealFile::decode(&json).map_err(|e| e.to_string())?;
        let dir = handle.vault.state_dir();
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        frankmd_fs::atomic::atomic_replace(
            &dir.join("encryption.json"),
            seal.encode().as_bytes(),
            frankmd_fs::atomic::ModePolicy::Fixed(0o600),
        )
        .map_err(|e| e.to_string())
    })
    .is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_set_encrypted(
    handle: *mut FrankMDHandle,
    path: *const c_char,
    encrypted: bool,
) -> bool {
    let path = input(path);
    with_handle(handle, |handle| {
        let path = path.ok_or_else(|| "path is required".to_owned())?;
        let (already, locked) = handle
            .vault
            .note_encryption(&path)
            .map_err(|e| e.to_string())?;
        if encrypted && already {
            return Ok(());
        }
        if !encrypted && already && locked {
            return Err("vault key is locked".to_owned());
        }
        NotesService::new(&handle.vault)
            .set_encrypted(&path, encrypted)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_search(
    handle: *mut FrankMDHandle,
    query: *const c_char,
    limit: usize,
) -> *mut c_char {
    let query = input(query).unwrap_or_default().trim().to_lowercase();
    let limit = limit.max(1);
    let result = with_handle(handle, |handle| {
        let tree = handle.vault.list_tree().map_err(|e| e.to_string())?;
        let mut paths = Vec::new();
        collect_note_paths(&tree, &mut paths);
        let service = NotesService::new(&handle.vault);
        let mut hits = Vec::new();
        for path in paths {
            let title = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .trim_end_matches(".md")
                .to_owned();
            let (encrypted, locked) = handle
                .vault
                .note_encryption(&path)
                .unwrap_or((false, false));
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
                hits.push(SearchHit {
                    path,
                    title,
                    snippet,
                    locked: encrypted && locked,
                });
                if hits.len() >= limit {
                    break;
                }
            }
        }
        serde_json::to_string(&hits).map_err(|e| e.to_string())
    });
    result.map_or(std::ptr::null_mut(), |json| string(&json))
}

#[unsafe(no_mangle)]
pub extern "C" fn frankmd_ios_preview(
    handle: *mut FrankMDHandle,
    path: *const c_char,
) -> *mut c_char {
    let path = input(path);
    let result = with_handle(handle, |handle| {
        let path = path.ok_or_else(|| "path is required".to_owned())?;
        let note = NotesService::new(&handle.vault)
            .read(&path)
            .map_err(|e| e.to_string())?;
        let source = String::from_utf8_lossy(&note.bytes);
        let doc = frankmd_markdown::render(&source, &frankmd_markdown::RenderOptions::default());
        let html = frankmd_markdown::html::to_html(&doc);
        let base = handle
            .vault
            .root()
            .join(&path)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| handle.vault.root().display().to_string());
        Ok(serde_json::json!({ "html": html, "base": base }).to_string())
    });
    result.map_or(std::ptr::null_mut(), |json| string(&json))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn open_write_list_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let state = dir.path().join("state");
        std::fs::create_dir_all(&root).unwrap();
        let mut handle =
            open(root.to_str().expect("root"), state.to_str().expect("state")).expect("open");
        let path = CString::new("note.md").expect("path");
        let text = CString::new("# hello").expect("text");
        let revision = frankmd_ios_write(&mut handle as *mut _, path.as_ptr(), text.as_ptr());
        assert!(!revision.is_null());
        frankmd_ios_free_string(revision);
        let list = frankmd_ios_list(&mut handle as *mut _);
        assert!(!list.is_null());
        let output = unsafe { CStr::from_ptr(list) }
            .to_str()
            .expect("list")
            .to_owned();
        frankmd_ios_free_string(list);
        assert!(output.contains("note.md"));
    }

    #[test]
    fn create_seal_encrypt_and_decrypt_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let state = dir.path().join("state");
        std::fs::create_dir_all(&root).unwrap();
        let mut handle =
            open(root.to_str().expect("root"), state.to_str().expect("state")).expect("open");
        let password = CString::new("senha de teste longa").expect("password");
        assert!(frankmd_ios_create_seal(
            &mut handle as *mut _,
            password.as_ptr()
        ));
        let path = CString::new("secret.md").expect("path");
        let text = CString::new("# Segredo\nconteudo\n").expect("text");
        let revision = frankmd_ios_write(&mut handle as *mut _, path.as_ptr(), text.as_ptr());
        assert!(!revision.is_null());
        frankmd_ios_free_string(revision);
        assert!(frankmd_ios_set_encrypted(
            &mut handle as *mut _,
            path.as_ptr(),
            true
        ));
        let raw = std::fs::read(root.join("secret.md")).expect("raw");
        assert!(frankmd_crypto::is_note_encrypted(&raw));
        let read = frankmd_ios_read(&mut handle as *mut _, path.as_ptr());
        assert!(!read.is_null());
        let plain = unsafe { CStr::from_ptr(read) }
            .to_str()
            .expect("read")
            .to_owned();
        frankmd_ios_free_string(read);
        assert!(plain.contains("conteudo"));
        assert!(frankmd_ios_set_encrypted(
            &mut handle as *mut _,
            path.as_ptr(),
            false
        ));
        let raw = std::fs::read(root.join("secret.md")).expect("raw");
        assert!(!frankmd_crypto::is_note_encrypted(&raw));
    }
}
