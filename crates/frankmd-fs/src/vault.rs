//! The notes folder ("vault") and the note operations on it.
//!
//! Every operation that mutates the vault, or reads the tree, holds the
//! **[COMPAT]** `.frankmd-filesystem.lock` (exclusive for mutations, shared
//! for tree reads), exactly where Rails `NotesService` held it. The
//! `*_as` variants take the label used in error messages: the raw request
//! path for [`crate::NotesService`], the vault-relative path otherwise.

use crate::atomic::{self, ModePolicy};
use crate::lock::{self, CONFIG_LOCK, FILESYSTEM_LOCK, LockGuard, LockKind};
use crate::{FsError, path_safety};
use frankmd_core::tree::{self, DirEntryInfo, EntryKind, FileType, TreeNode};
use frankmd_core::{RelPath, Revision};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Where deleted items go.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TrashBackend {
    /// The freedesktop / OS trash via the `trash` crate (default).
    #[default]
    System,
    /// Move items into this directory (tests, and a fallback); the name is
    /// made unique (`name`, `name.1`, `name.2`, ...) with a no-clobber rename.
    /// Crossing filesystems (EXDEV) falls back to copy + delete like
    /// [`Vault::rename`]-style moves do; symlinks are not specially handled
    /// there, so keep the directory on the vault's filesystem.
    Directory(PathBuf),
}

/// An opened notes root. Cheap to clone; holds no file handles.
#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
    real_root: PathBuf,
    state_dir: PathBuf,
    /// Locks always live under `state_dir` (folder that is not a notes root).
    private_locks: bool,
    trash: TrashBackend,
    encryption: Option<std::sync::Arc<frankmd_crypto::Session>>,
}

/// A path proven to be inside the vault (component-wise, symlink-aware).
/// Only obtainable from [`Vault::contain`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainedPath {
    rel: RelPath,
    abs: PathBuf,
}

impl ContainedPath {
    /// Vault-relative form.
    pub fn rel(&self) -> &RelPath {
        &self.rel
    }
    /// Absolute (lexical, not canonical) form.
    pub fn abs(&self) -> &Path {
        &self.abs
    }
}

/// A note as read from disk.
#[derive(Debug, Clone)]
pub struct NoteFile {
    /// Exact bytes (no BOM stripping, CRLF preserved, **[COMPAT]**).
    pub bytes: Vec<u8>,
    /// SHA-256 revision of `bytes`.
    pub revision: Revision,
    /// mtime, whole seconds.
    pub mtime: i64,
    /// `false` if the bytes were not valid UTF-8 (UI shows a warning and
    /// opens read-only rather than failing, HP-028).
    pub valid_utf8: bool,
    /// The file is stored in a FrankMDFast encryption envelope.
    pub encrypted: bool,
}

impl NoteFile {
    fn from_bytes(bytes: Vec<u8>, mtime: i64) -> Self {
        let revision = Revision::of(&bytes);
        let valid_utf8 = std::str::from_utf8(&bytes).is_ok();
        Self {
            bytes,
            revision,
            mtime,
            valid_utf8,
            encrypted: false,
        }
    }
}

impl Vault {
    /// Open (and `mkdir -p`) a notes root. `state_dir` is the app-owned
    /// per-vault state directory outside the vault (HP-012).
    pub fn open(root: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Result<Self, FsError> {
        let root = root.into();
        if !root.exists() {
            fs::create_dir_all(&root)?;
        }
        let real_root = root.canonicalize()?;
        Ok(Self {
            root,
            real_root,
            state_dir: state_dir.into(),
            private_locks: false,
            trash: TrashBackend::default(),
            encryption: None,
        })
    }

    /// Use another trash backend (default [`TrashBackend::System`]).
    pub fn with_trash(mut self, backend: TrashBackend) -> Self {
        self.trash = backend;
        self
    }

    /// The configured trash backend.
    pub fn trash_backend(&self) -> &TrashBackend {
        &self.trash
    }

    /// Open a folder that is **not** a notes root (e.g. the user's
    /// `images_path` picture folder): same containment and no-clobber
    /// semantics, but locks are taken only under `state_dir`, so no app file
    /// is ever created inside the user's folder (no Rails instance shares it).
    pub fn open_foreign(
        root: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
    ) -> Result<Self, FsError> {
        Ok(Self {
            private_locks: true,
            ..Self::open(root, state_dir)?
        })
    }

    /// Notes root as given.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Canonical (realpath) notes root; `ContainedPath::abs` is built on it.
    pub fn real_root(&self) -> &Path {
        &self.real_root
    }

    /// The app-owned state directory of this vault.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// **[COMPAT]** `<realpath root>/.frankmd-filesystem.lock`: the one lock
    /// file every instance (Rails or Rust) on this folder shares.
    pub fn filesystem_lock_path(&self) -> PathBuf {
        self.real_root.join(FILESYSTEM_LOCK)
    }

    /// Take one of the shared compat locks (`FILESYSTEM_LOCK`, `CONFIG_LOCK`)
    /// of this vault, falling back to a private lock under the state dir
    /// when the notes root cannot hold the lock file.
    pub fn lock(&self, name: &str, kind: LockKind) -> Result<LockGuard, FsError> {
        let id = Revision::of(self.real_root.as_os_str().as_encoded_bytes()).0;
        let fallback = self.state_dir.join("locks").join(id);
        if self.private_locks {
            return lock::acquire_private(name, kind, &fallback);
        }
        lock::acquire(&self.real_root, name, kind, &fallback)
    }

    /// Prove `rel` is inside the vault (backend.md §2.4 semantics, plus
    /// per-component `lstat` for symlinks). Rejects reserved app files.
    pub fn contain(&self, rel: &RelPath) -> Result<ContainedPath, FsError> {
        self.safe_path(rel.as_str(), false)
    }

    /// **[COMPAT]** `NotesService#safe_path(path, must_exist:)` on a raw path:
    /// `.frankmd-config.lock` → `InvalidPath("Reserved application path")`;
    /// rejected by `PathSafety.contain` → `InvalidPath("Invalid path: <path>")`;
    /// `must_exist` and missing (symlinks followed) →
    /// `NotFound("Path not found: <path>")`.
    pub fn safe_path(&self, raw: &str, must_exist: bool) -> Result<ContainedPath, FsError> {
        let abs = path_safety::contain(&self.real_root, Some(raw));
        if abs.as_deref() == Some(self.real_root.join(CONFIG_LOCK).as_path()) {
            return Err(FsError::InvalidPath("Reserved application path".to_owned()));
        }
        let abs = abs.ok_or_else(|| FsError::InvalidPath(format!("Invalid path: {raw}")))?;
        let rel = self
            .rel_of(&abs)
            .ok_or_else(|| FsError::InvalidPath(format!("Invalid path: {raw}")))?;
        if must_exist && !abs.exists() {
            return Err(FsError::NotFound(format!("Path not found: {raw}")));
        }
        Ok(ContainedPath { rel, abs })
    }

    /// Vault-relative path of an absolute path under the real root.
    fn rel_of(&self, abs: &Path) -> Option<RelPath> {
        let rest = abs.strip_prefix(&self.real_root).ok()?;
        RelPath::parse(rest.to_str()?).ok()
    }

    /// Build the Explorer tree under a shared filesystem lock.
    /// Install the vault's encryption session (notes/media cryptography).
    pub fn set_encryption(&mut self, session: Option<std::sync::Arc<frankmd_crypto::Session>>) {
        self.encryption = session;
    }

    /// The installed encryption session, if any.
    pub fn encryption(&self) -> Option<&std::sync::Arc<frankmd_crypto::Session>> {
        self.encryption.as_ref()
    }

    /// `(encrypted, locked)` for a note path (no full read).
    pub fn note_encryption(&self, path: &str) -> Result<(bool, bool), FsError> {
        let full = self.safe_path(path, true)?;
        if !self.file_is_encrypted(&full.abs) {
            return Ok((false, false));
        }
        let locked = self
            .encryption
            .as_ref()
            .is_none_or(|session| !session.is_unlocked());
        Ok((true, locked))
    }

    /// `true` when `path` resolves to an existing file inside the vault.
    pub fn note_exists(&self, path: &str) -> bool {
        self.safe_path(path, true)
            .is_ok_and(|full| full.abs.is_file())
    }

    fn encrypt_note_bytes(&self, bytes: &[u8], label: &str) -> Result<Vec<u8>, FsError> {
        let session = self.encryption.as_ref().ok_or_else(|| {
            FsError::Locked(format!(
                "Note is encrypted and no vault key is loaded: {label}"
            ))
        })?;
        if !session.is_unlocked() {
            return Err(FsError::Locked(format!(
                "Note is encrypted and the vault is locked: {label}"
            )));
        }
        session
            .encrypt_note(bytes)
            .map_err(|err| FsError::Crypto(err.to_string()))
    }

    fn file_is_encrypted(&self, abs: &Path) -> bool {
        let Ok(mut file) = fs::File::open(abs) else {
            return false;
        };
        use std::io::Read as _;
        let mut prefix = [0u8; 40];
        let n = file.read(&mut prefix).unwrap_or(0);
        frankmd_crypto::is_note_encrypted(&prefix[..n])
    }

    fn file_encryption(&self, abs: &Path) -> (bool, bool) {
        if !self.file_is_encrypted(abs) {
            return (false, false);
        }
        let locked = self
            .encryption
            .as_ref()
            .is_none_or(|session| !session.is_unlocked());
        (true, locked)
    }

    pub(crate) fn write_note_with_encryption(
        &self,
        path: &ContainedPath,
        plaintext: &[u8],
        encrypt: bool,
    ) -> Result<Revision, FsError> {
        let bytes = if encrypt {
            self.encrypt_note_bytes(plaintext, path.rel.as_str())?
        } else {
            plaintext.to_vec()
        };
        atomic::atomic_replace(&path.abs, &bytes, ModePolicy::Preserve)?;
        Ok(Revision::of(&bytes))
    }

    /// The tree (folders first, then notes).
    pub fn list_tree(&self) -> Result<Vec<TreeNode>, FsError> {
        let _lock = self.lock(FILESYSTEM_LOCK, LockKind::Shared)?;
        self.build_tree(&self.real_root)
    }

    /// The tree below one folder (`Folder#children`; `[]` when missing).
    pub fn folder_children(&self, path: &ContainedPath) -> Result<Vec<TreeNode>, FsError> {
        let _lock = self.lock(FILESYSTEM_LOCK, LockKind::Shared)?;
        match fs::symlink_metadata(&path.abs) {
            Ok(meta) if meta.is_dir() => self.build_tree(&path.abs),
            _ => Ok(Vec::new()),
        }
    }

    /// **[COMPAT]** `NotesService#build_tree`: dot entries skipped before any
    /// `lstat`, symlinks skipped, folders first (by lowercase name), then
    /// `.md` files newest first; entries vanishing mid-walk are tolerated.
    fn build_tree(&self, dir: &Path) -> Result<Vec<TreeNode>, FsError> {
        let reading = match fs::read_dir(dir) {
            Ok(reading) => reading,
            Err(e) if is_gone(&e) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut entries = Vec::new();
        for entry in reading {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) if is_gone(&e) => continue,
                Err(e) => return Err(e.into()),
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let meta = crate::fault_check!(crate::fault::FaultPoint::Lstat(name.clone()))
                .and_then(|()| fs::symlink_metadata(entry.path()));
            let meta = match meta {
                Ok(meta) => meta,
                Err(e) if is_gone(&e) => continue,
                Err(e) => return Err(e.into()),
            };
            let kind = if meta.file_type().is_symlink() {
                EntryKind::Symlink
            } else if meta.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            entries.push(DirEntryInfo {
                name,
                kind,
                mtime: crate::meta::mtime_secs(&meta),
            });
        }
        tree::sort_entries(&mut entries);

        let at_root = dir == self.real_root;
        let mut nodes = Vec::new();
        for entry in entries {
            let abs = dir.join(&entry.name);
            let has_markdown =
                |p: &Path| entry.kind == EntryKind::Dir && at_root && has_markdown_below(p);
            if !tree::is_visible(&entry, at_root, has_markdown(&abs)) {
                continue;
            }
            let Some(path) = self.rel_of(&abs) else {
                continue;
            };
            match entry.kind {
                EntryKind::Dir => {
                    let children = self.build_tree(&abs)?;
                    nodes.push(TreeNode::Folder {
                        name: entry.name,
                        path,
                        children,
                    });
                }
                EntryKind::File => {
                    let name = entry
                        .name
                        .strip_suffix(frankmd_core::path::NOTE_EXT)
                        .unwrap_or(&entry.name)
                        .to_owned();
                    let (encrypted, locked) = self.file_encryption(&abs);
                    nodes.push(TreeNode::File {
                        name,
                        path,
                        file_type: FileType::Markdown,
                        mtime: entry.mtime,
                        encrypted,
                        locked,
                    });
                }
                EntryKind::Symlink => {}
            }
        }
        Ok(nodes)
    }

    /// Read a note. Tolerates non-UTF-8.
    pub fn read_note(&self, path: &ContainedPath) -> Result<NoteFile, FsError> {
        self.read_note_as(path, path.rel.as_str())
    }

    /// [`Vault::read_note`] with `label` in the error message.
    pub(crate) fn read_note_as(
        &self,
        path: &ContainedPath,
        label: &str,
    ) -> Result<NoteFile, FsError> {
        let not_found = || FsError::NotFound(format!("Note not found: {label}"));
        if !path.abs.is_file() {
            return Err(not_found());
        }
        let raw =
            fs::read(&path.abs).map_err(|e| if is_gone(&e) { not_found() } else { e.into() })?;
        let mtime = fs::metadata(&path.abs).map_or(0, |m| crate::meta::mtime_secs(&m));
        if frankmd_crypto::is_note_encrypted(&raw) {
            let session = self
                .encryption
                .as_ref()
                .ok_or_else(|| FsError::Locked(format!("Note is encrypted and locked: {label}")))?;
            if !session.is_unlocked() {
                return Err(FsError::Locked(format!(
                    "Note is encrypted and locked: {label}"
                )));
            }
            let plain = session
                .decrypt_note(&raw)
                .map_err(|err| FsError::Crypto(err.to_string()))?;
            let valid_utf8 = std::str::from_utf8(&plain).is_ok();
            return Ok(NoteFile {
                bytes: plain,
                revision: Revision::of(&raw),
                mtime,
                valid_utf8,
                encrypted: true,
            });
        }
        Ok(NoteFile::from_bytes(raw, mtime))
    }

    /// Create a new note; `AlreadyExists` if anything is there. Creates parents.
    pub fn create_note(&self, path: &ContainedPath, bytes: &[u8]) -> Result<Revision, FsError> {
        let _lock = self.lock(FILESYSTEM_LOCK, LockKind::Exclusive)?;
        self.create_note_locked(path, bytes, path.rel.as_str())
    }

    /// `create` body; the caller holds the exclusive lock.
    pub(crate) fn create_note_locked(
        &self,
        path: &ContainedPath,
        bytes: &[u8],
        label: &str,
    ) -> Result<Revision, FsError> {
        mkdir_parent(&path.abs)?;
        if path_exists(&path.abs) {
            return Err(FsError::AlreadyExists(format!(
                "Destination already exists: {label}"
            )));
        }
        atomic::create_new(&path.abs, bytes).map_err(|e| match e {
            FsError::AlreadyExists(_) => {
                FsError::AlreadyExists(format!("Destination already exists: {}", path.rel))
            }
            other => other,
        })?;
        Ok(Revision::of(bytes))
    }

    /// Upsert (`NotesService#write`): `mkdir -p` the parent, atomic replace
    /// keeping the existing mode (new files get `0666 & ~umask`).
    pub fn write_note(&self, path: &ContainedPath, bytes: &[u8]) -> Result<Revision, FsError> {
        let _lock = self.lock(FILESYSTEM_LOCK, LockKind::Exclusive)?;
        self.write_note_locked(path, bytes)
    }

    /// `write` body; the caller holds the exclusive lock.
    pub(crate) fn write_note_locked(
        &self,
        path: &ContainedPath,
        bytes: &[u8],
    ) -> Result<Revision, FsError> {
        mkdir_parent(&path.abs)?;
        let stored = if self.file_is_encrypted(&path.abs) {
            self.encrypt_note_bytes(bytes, path.rel.as_str())?
        } else {
            bytes.to_vec()
        };
        atomic::atomic_replace(&path.abs, &stored, ModePolicy::Preserve)?;
        Ok(Revision::of(&stored))
    }

    /// Save over an **existing** note; `NotFound` if it is gone (never
    /// resurrects a renamed/deleted note, HP-006).
    pub fn update_existing(&self, path: &ContainedPath, bytes: &[u8]) -> Result<Revision, FsError> {
        let _lock = self.lock(FILESYSTEM_LOCK, LockKind::Exclusive)?;
        self.update_locked(path, bytes, path.rel.as_str())
    }

    /// `update` body; the caller holds the exclusive lock.
    pub(crate) fn update_locked(
        &self,
        path: &ContainedPath,
        bytes: &[u8],
        label: &str,
    ) -> Result<Revision, FsError> {
        if !path.abs.is_file() {
            return Err(FsError::NotFound(format!("Note not found: {label}")));
        }
        let stored = if self.file_is_encrypted(&path.abs) {
            self.encrypt_note_bytes(bytes, label)?
        } else {
            bytes.to_vec()
        };
        atomic::atomic_replace(&path.abs, &stored, ModePolicy::Preserve)?;
        Ok(Revision::of(&stored))
    }

    /// Rename/move a file or folder without clobbering.
    pub fn rename(&self, from: &ContainedPath, to: &ContainedPath) -> Result<(), FsError> {
        let _lock = self.lock(FILESYSTEM_LOCK, LockKind::Exclusive)?;
        self.rename_locked(from, to, from.rel.as_str(), to.rel.as_str())
    }

    /// `rename` body; the caller holds the exclusive lock.
    pub(crate) fn rename_locked(
        &self,
        from: &ContainedPath,
        to: &ContainedPath,
        from_label: &str,
        to_label: &str,
    ) -> Result<(), FsError> {
        let already = || FsError::AlreadyExists(format!("Destination already exists: {to_label}"));
        if !from.abs.exists() {
            return Err(FsError::NotFound(format!("Note not found: {from_label}")));
        }
        if path_exists(&to.abs) {
            return Err(already());
        }
        mkdir_parent(&to.abs)?;
        atomic::rename_noclobber(&from.abs, &to.abs).map_err(|e| match e {
            FsError::AlreadyExists(_) => already(),
            other => other,
        })
    }

    /// Delete a note, optionally only if its revision still matches.
    pub fn delete_note(
        &self,
        path: &ContainedPath,
        expected: Option<&Revision>,
    ) -> Result<(), FsError> {
        let _lock = self.lock(FILESYSTEM_LOCK, LockKind::Exclusive)?;
        self.delete_note_locked(path, expected.map(|r| r.0.as_str()), path.rel.as_str())
    }

    /// `delete` body; the caller holds the exclusive lock, so the revision
    /// check and the removal cannot be interleaved by another FrankMD writer.
    pub(crate) fn delete_note_locked(
        &self,
        path: &ContainedPath,
        expected: Option<&str>,
        label: &str,
    ) -> Result<(), FsError> {
        if !path.abs.is_file() {
            return Err(FsError::NotFound(format!("Note not found: {label}")));
        }
        if let Some(expected) = expected {
            let current = Revision::of(&fs::read(&path.abs)?);
            if current.0 != expected {
                return Err(FsError::RevisionConflict(format!(
                    "Note has changed: {label}"
                )));
            }
        }
        fs::remove_file(&path.abs)?;
        Ok(())
    }

    /// Resolve `raw` for trashing: like `safe_path(raw, true)`, except that a
    /// symlink as the final component is accepted even when it points
    /// outside the vault (or dangles), because only the link itself moves.
    /// Its parent must still be contained.
    pub(crate) fn trash_path(&self, raw: &str) -> Result<ContainedPath, FsError> {
        let root = self.real_root.to_str().unwrap_or_default();
        let invalid = || FsError::InvalidPath(format!("Invalid path: {raw}"));
        if let Some(cand) = frankmd_core::path::lexical_contain(root, Some(raw)).map(PathBuf::from)
            && cand != self.real_root
            && fs::symlink_metadata(&cand).is_ok_and(|m| m.file_type().is_symlink())
        {
            let parent = cand.parent().ok_or_else(invalid)?;
            if parent != self.real_root {
                let rel_parent = parent
                    .strip_prefix(&self.real_root)
                    .ok()
                    .and_then(Path::to_str)
                    .ok_or_else(invalid)?;
                path_safety::contain(&self.real_root, Some(rel_parent)).ok_or_else(invalid)?;
            }
            let rel = self.rel_of(&cand).ok_or_else(invalid)?;
            return Ok(ContainedPath { rel, abs: cand });
        }
        self.safe_path(raw, true)
    }

    /// Move a file, symlink or folder (empty or not) to the trash. The
    /// caller holds the exclusive lock. A symlink is moved itself, never its
    /// target. `expected` is the revision check for regular files.
    pub(crate) fn trash_locked(
        &self,
        path: &ContainedPath,
        expected: Option<&str>,
        label: &str,
    ) -> Result<(), FsError> {
        if path.abs == self.real_root || path.rel.as_str().is_empty() {
            return Err(FsError::InvalidPath(format!("Invalid path: {label}")));
        }
        if path.abs == self.real_root.join(CONFIG_LOCK) {
            return Err(FsError::InvalidPath("Reserved application path".to_owned()));
        }
        if frankmd_core::path::is_config_file(path.rel.as_str()) {
            return Err(FsError::PermissionDenied(
                "the configuration file cannot be deleted".to_owned(),
            ));
        }
        let meta = fs::symlink_metadata(&path.abs)
            .map_err(|_| FsError::NotFound(format!("Note not found: {label}")))?;
        if meta.is_file()
            && let Some(expected) = expected
        {
            let current = Revision::of(&fs::read(&path.abs)?);
            if current.0 != expected {
                return Err(FsError::RevisionConflict(format!(
                    "Note has changed: {label}"
                )));
            }
        }
        match &self.trash {
            TrashBackend::System => trash::delete(&path.abs).map_err(|e| {
                FsError::Io(io::Error::other(format!("could not move to trash: {e}")))
            }),
            TrashBackend::Directory(dir) => {
                fs::create_dir_all(dir)?;
                let name = path
                    .abs
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| FsError::InvalidPath(format!("Invalid path: {label}")))?;
                let mut n = 0u32;
                loop {
                    let target = if n == 0 {
                        dir.join(name)
                    } else {
                        dir.join(format!("{name}.{n}"))
                    };
                    match atomic::rename_noclobber(&path.abs, &target) {
                        Err(FsError::AlreadyExists(_)) => n += 1,
                        other => return other,
                    }
                }
            }
        }
    }

    /// Create a folder (`AlreadyExists` if present).
    pub fn create_folder(&self, path: &ContainedPath) -> Result<(), FsError> {
        let _lock = self.lock(FILESYSTEM_LOCK, LockKind::Exclusive)?;
        self.create_folder_locked(path, path.rel.as_str())
    }

    /// `create_folder` body; the caller holds the exclusive lock.
    pub(crate) fn create_folder_locked(
        &self,
        path: &ContainedPath,
        label: &str,
    ) -> Result<(), FsError> {
        let already = || FsError::AlreadyExists(format!("Destination already exists: {label}"));
        if path_exists(&path.abs) {
            return Err(already());
        }
        mkdir_parent(&path.abs)?;
        match fs::create_dir(&path.abs) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(already()),
            other => Ok(other?),
        }
    }

    /// Delete an empty folder (hidden files count, **[COMPAT]**).
    pub fn delete_empty_folder(&self, path: &ContainedPath) -> Result<(), FsError> {
        let _lock = self.lock(FILESYSTEM_LOCK, LockKind::Exclusive)?;
        self.delete_folder_locked(path, path.rel.as_str())
    }

    /// `delete_folder` body; the caller holds the exclusive lock.
    pub(crate) fn delete_folder_locked(
        &self,
        path: &ContainedPath,
        label: &str,
    ) -> Result<(), FsError> {
        if !path.abs.is_dir() {
            return Err(FsError::NotFound(format!("Folder not found: {label}")));
        }
        if fs::read_dir(&path.abs)?.next().is_some() {
            return Err(FsError::NotEmpty(format!("Folder not empty: {label}")));
        }
        fs::remove_dir(&path.abs)?;
        Ok(())
    }
}

/// **[COMPAT]** `path_exists?`: anything at `path`, including a dangling symlink.
pub(crate) fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// `FileUtils.mkdir_p(path.dirname)`.
fn mkdir_parent(path: &Path) -> Result<(), FsError> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(fs::create_dir_all(parent)?),
        _ => Ok(()),
    }
}

/// ENOENT / ENOTDIR: the entry vanished (or its parent stopped being a dir).
fn is_gone(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::NotFound || err.kind() == io::ErrorKind::NotADirectory
}

/// **[COMPAT]** `entry.glob("**/*.md").any?`: a non-dot name ending in `.md`
/// anywhere below `dir`, without descending into dot or symlinked folders.
fn has_markdown_below(dir: &Path) -> bool {
    let Ok(reading) = fs::read_dir(dir) else {
        return false;
    };
    for entry in reading.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if name.ends_with(frankmd_core::path::NOTE_EXT) {
            return true;
        }
        if entry.file_type().is_ok_and(|t| t.is_dir()) && has_markdown_below(&entry.path()) {
            return true;
        }
    }
    false
}
