//! Encryption integration tests for notes.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use frankmd_core::tree::TreeNode;
use frankmd_crypto::Session;
use frankmd_fs::{FsError, NotesService, Vault};

fn setup() -> (tempfile::TempDir, Vault, Arc<Session>) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let mut vault = Vault::open(dir.path().join("notes"), &state).unwrap();
    let session = Arc::new(Session::new());
    session
        .create("uma senha longa e unica para o teste")
        .unwrap();
    vault.set_encryption(Some(Arc::clone(&session)));
    (dir, vault, session)
}

#[test]
fn encrypt_read_lock_update_decrypt_and_tree_flags() {
    let (dir, vault, session) = setup();
    let service = NotesService::new(&vault);
    service
        .create("secret.md", b"# Conteudo\nsegredo\n")
        .unwrap();
    let plain = std::fs::read(dir.path().join("notes/secret.md")).unwrap();
    assert!(!frankmd_crypto::is_note_encrypted(&plain));

    service.set_encrypted("secret.md", true).unwrap();
    let raw = std::fs::read(dir.path().join("notes/secret.md")).unwrap();
    assert!(frankmd_crypto::is_note_encrypted(&raw));
    let read = service.read("secret.md").unwrap();
    assert!(read.encrypted);
    assert_eq!(read.bytes, b"# Conteudo\nsegredo\n");

    session.lock();
    assert!(matches!(service.read("secret.md"), Err(FsError::Locked(_))));
    assert!(matches!(
        service.update("secret.md", b"nope"),
        Err(FsError::Locked(_))
    ));
    let tree = service.list_tree().unwrap();
    let secret = tree
        .iter()
        .find_map(|node| match node {
            TreeNode::File {
                path,
                encrypted,
                locked,
                ..
            } if path.as_str() == "secret.md" => Some((*encrypted, *locked)),
            _ => None,
        })
        .unwrap();
    assert_eq!(secret, (true, true));

    session
        .unlock("uma senha longa e unica para o teste")
        .unwrap();
    service.update("secret.md", b"alterado\n").unwrap();
    let raw_updated = std::fs::read(dir.path().join("notes/secret.md")).unwrap();
    assert!(frankmd_crypto::is_note_encrypted(&raw_updated));
    assert_eq!(service.read("secret.md").unwrap().bytes, b"alterado\n");

    service.set_encrypted("secret.md", false).unwrap();
    let raw_plain = std::fs::read(dir.path().join("notes/secret.md")).unwrap();
    assert_eq!(raw_plain, b"alterado\n");
    assert_eq!(service.read("secret.md").unwrap().bytes, b"alterado\n");
}

#[test]
fn plain_notes_need_no_key() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().join("notes"), dir.path().join("state")).unwrap();
    let service = NotesService::new(&vault);
    service.create("plain.md", b"oi").unwrap();
    assert!(!service.read("plain.md").unwrap().encrypted);
    service.update("plain.md", b"tchau").unwrap();
    assert_eq!(service.read("plain.md").unwrap().bytes, b"tchau");
}
