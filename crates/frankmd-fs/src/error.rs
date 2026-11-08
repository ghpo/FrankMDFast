//! Typed filesystem errors (mapped to localized UI messages by the app).
//!
//! Each variant carries the **[COMPAT]** English message of the Rails
//! `NotesService` exception it replaces (`Display` is that message verbatim,
//! e.g. `"Note not found: a.md"`), and [`FsError::compat_class`] names the
//! Rails class so the golden fixtures' `{error, message}` can be compared.

use std::io;

/// **[COMPAT]** the message of the `ArgumentError` `BacklinksService` raised
/// for a target containing `..`.
pub const BACKLINKS_INVALID_PATH: &str = "Invalid path";

/// Errors from vault operations.
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    /// Destination already exists (create / rename target):
    /// `"Destination already exists: <path>"`. A subclass of InvalidPath in Rails.
    #[error("{0}")]
    AlreadyExists(String),
    /// Source does not exist (or vanished mid-operation):
    /// `"Note not found: <path>"`, `"Folder not found: <path>"`, `"Path not found: <path>"`.
    #[error("{0}")]
    NotFound(String),
    /// The file changed since it was loaded (revision mismatch on delete):
    /// `"Note has changed: <path>"`.
    #[error("{0}")]
    RevisionConflict(String),
    /// Folder delete refused because it is not empty (hidden files count):
    /// `"Folder not empty: <path>"` (an InvalidPathError in Rails).
    #[error("{0}")]
    NotEmpty(String),
    /// Path escapes the vault, traverses a symlink outside it, or is reserved:
    /// `"Invalid path: <path>"`, `"Reserved application path"`, `"Invalid path"`
    /// (backlinks' `ArgumentError`).
    #[error("{0}")]
    InvalidPath(String),
    /// The note/media is encrypted and the vault key is locked.
    #[error("{0}")]
    Locked(String),
    /// Encryption/decryption failed.
    #[error("{0}")]
    Crypto(String),
    /// EACCES / EPERM / EROFS.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// Any other I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl FsError {
    /// The Rails exception class this error replaces
    /// (`"NotesService::NotFoundError"`, `"NotesService::AlreadyExistsError"`,
    /// `"NotesService::InvalidPathError"`, `"NotesService::RevisionConflictError"`,
    /// `"Errno::EACCES"`, …). Backlinks' invalid path is `"ArgumentError"` and is
    /// produced as [`FsError::InvalidPath`] with message `"Invalid path"`.
    pub fn compat_class(&self) -> &'static str {
        match self {
            Self::AlreadyExists(_) => "NotesService::AlreadyExistsError",
            Self::NotFound(_) => "NotesService::NotFoundError",
            Self::RevisionConflict(_) => "NotesService::RevisionConflictError",
            Self::NotEmpty(_) => "NotesService::InvalidPathError",
            Self::Locked(_) | Self::Crypto(_) => "FrankMDFast::CryptoError",
            Self::InvalidPath(message) if message == BACKLINKS_INVALID_PATH => "ArgumentError",
            Self::InvalidPath(_) => "NotesService::InvalidPathError",
            Self::PermissionDenied(_) => "Errno::EACCES",
            Self::Io(err) => errno_class(err),
        }
    }

    /// The POSIX `errno` behind this error, when it is an OS error
    /// (Windows codes translated, see [`crate::errno`]).
    pub fn raw_os_error(&self) -> Option<i32> {
        match self {
            Self::Io(err) => crate::errno::of(err),
            _ => None,
        }
    }

    /// EACCES / EPERM (Rails `rescue Errno::EACCES, Errno::EPERM`).
    pub fn is_permission_denied(&self) -> bool {
        matches!(self, Self::PermissionDenied(_))
            || matches!(
                self.raw_os_error(),
                Some(crate::errno::EACCES | crate::errno::EPERM)
            )
    }

    /// ENOENT (Rails `rescue Errno::ENOENT`).
    pub fn is_enoent(&self) -> bool {
        self.raw_os_error() == Some(crate::errno::ENOENT)
    }

    /// `true` for AlreadyExists, NotEmpty and InvalidPath (Rails
    /// `rescue NotesService::InvalidPathError` catches all three → 422).
    pub fn is_invalid_path(&self) -> bool {
        matches!(
            self,
            Self::AlreadyExists(_) | Self::NotEmpty(_) | Self::InvalidPath(_)
        )
    }
}

/// Ruby's `Errno::*` class name for an OS error.
fn errno_class(err: &io::Error) -> &'static str {
    use crate::errno::*;
    match of(err) {
        Some(EPERM) => "Errno::EPERM",
        Some(ENOENT) => "Errno::ENOENT",
        Some(EIO) => "Errno::EIO",
        Some(EACCES) => "Errno::EACCES",
        Some(EBUSY) => "Errno::EBUSY",
        Some(EEXIST) => "Errno::EEXIST",
        Some(EXDEV) => "Errno::EXDEV",
        Some(ENOTDIR) => "Errno::ENOTDIR",
        Some(EISDIR) => "Errno::EISDIR",
        Some(EINVAL) => "Errno::EINVAL",
        Some(ENOSPC) => "Errno::ENOSPC",
        Some(EROFS) => "Errno::EROFS",
        Some(EMLINK) => "Errno::EMLINK",
        Some(ENAMETOOLONG) => "Errno::ENAMETOOLONG",
        Some(ENOSYS) => "Errno::ENOSYS",
        Some(ENOTEMPTY) => "Errno::ENOTEMPTY",
        Some(ELOOP) => "Errno::ELOOP",
        Some(EOPNOTSUPP) => "Errno::EOPNOTSUPP",
        Some(_) => "SystemCallError",
        None if err.kind() == io::ErrorKind::InvalidData => "ArgumentError",
        None => "IOError",
    }
}
