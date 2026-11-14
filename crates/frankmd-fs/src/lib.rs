//! # frankmd-fs
//!
//! Every filesystem **mutation** in FrankMD goes through this crate (HP-004,
//! HP-015). Other crates are blocked from `std::fs::write` & co. by
//! `clippy.toml`.
//!
//! Guarantees:
//! - **Atomic write**: same-dir temp `.<basename>.<12hex>.tmp` (**[COMPAT]**
//!   name), write, fsync, preserve mode, `rename(2)`.
//! - **No-clobber create**: `O_EXCL` temp + `link(2)` (fallback
//!   `renameat2(RENAME_NOREPLACE)`), mapping `EEXIST` → [`FsError::AlreadyExists`].
//!   Never check-then-act (HP-005).
//! - **Save never creates** ([`Vault::update_existing`]; HP-006).
//! - **Compat locks**: `flock(2)` on `.frankmd-config.lock` /
//!   `.frankmd-filesystem.lock` in the notes root so a Rails FrankMD sharing the
//!   folder coordinates with us.
//! - **Tolerant reads**: vanished files and non-UTF-8 bytes are reported, not
//!   fatal (HP-028).
#![allow(clippy::disallowed_methods)] // this crate is the sanctioned writer

/// Hit a test fault point (`frankmd_fs::fault`); a no-op `Ok(())` unless the
/// `fault-injection` feature is on. The point expression is only evaluated
/// with the feature.
macro_rules! fault_check {
    ($point:expr) => {{
        #[cfg(feature = "fault-injection")]
        let result = crate::fault::check(&$point);
        #[cfg(not(feature = "fault-injection"))]
        let result: std::io::Result<()> = Ok(());
        result
    }};
}
pub(crate) use fault_check;

pub mod atomic;
pub mod errno;
pub mod error;
#[cfg(feature = "fault-injection")]
pub mod fault;
pub mod lock;
pub mod meta;
pub mod model;
pub mod path_safety;
pub mod recent;
pub mod service;
pub mod vault;

pub use error::FsError;
pub use recent::{RecentFilesService, RecentStore};
pub use service::NotesService;
pub use vault::{ContainedPath, NoteFile, SyncEntry, TrashBackend, Vault};
