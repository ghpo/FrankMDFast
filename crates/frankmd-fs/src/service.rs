//! **[COMPAT]** facade with the exact semantics of the Rails `NotesService`
//! (plus the IO halves of `Note`, `Folder`, `HugoService`, `BacklinksService`),
//! taking raw string paths as the UI/workspace receives them.
//!
//! Every path goes through [`Vault::safe_path`]; every mutation holds the
//! exclusive filesystem lock; recents are updated through the optional
//! [`RecentStore`] exactly when Rails did (only when `.fed` is a regular file,
//! errors swallowed). The workspace layer adds DocId remapping on top.

use crate::error::BACKLINKS_INVALID_PATH;
use crate::lock::{FILESYSTEM_LOCK, LockKind};
use crate::recent::{RecentFilesService, RecentStore};
use crate::{FsError, NoteFile, Vault};
use frankmd_core::Revision;
use frankmd_core::hugo::{self, GeneratedPost, HugoPathStyle, NewPost};
use frankmd_core::path::NOTE_EXT;
use frankmd_core::tree::TreeNode;
use frankmd_core::wikilink::{self, Backlink, BacklinkQuery};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

/// Rails `NotesService` over a [`Vault`].
pub struct NotesService<'a> {
    vault: &'a Vault,
    recents: Option<&'a dyn RecentStore>,
}

impl<'a> NotesService<'a> {
    /// Service without recents coupling.
    pub fn new(vault: &'a Vault) -> Self {
        Self {
            vault,
            recents: None,
        }
    }

    /// Couple delete/rename/delete_folder to the recents store.
    pub fn with_recents(mut self, store: &'a dyn RecentStore) -> Self {
        self.recents = Some(store);
        self
    }

    /// The vault.
    pub fn vault(&self) -> &Vault {
        self.vault
    }

    /// `list_tree` (shared lock; creates the lock file).
    pub fn list_tree(&self) -> Result<Vec<TreeNode>, FsError> {
        self.vault.list_tree()
    }

    /// `Folder#children`: the tree below `path` (`[]` when missing).
    /// **[COMPAT]** the folder is looked up in the full tree by exact path
    /// string, so only a path spelled as the tree spells it matches.
    pub fn folder_children(&self, path: &str) -> Result<Vec<TreeNode>, FsError> {
        let tree = self.list_tree()?;
        Ok(
            find_in_tree(&tree, path).map_or_else(Vec::new, |node| match node {
                TreeNode::Folder { children, .. } => children.clone(),
                TreeNode::File { .. } => Vec::new(),
            }),
        )
    }

    /// `read(path)`: exact bytes. `NotFound("Note not found: <path>")` unless
    /// it is a regular file (symlinks inside the vault are followed).
    pub fn read(&self, path: &str) -> Result<NoteFile, FsError> {
        let full = self.vault.safe_path(path, true)?;
        self.vault.read_note_as(&full, path)
    }

    /// `write(path, content)` (upsert): `mkdir -p` parent + atomic replace,
    /// keeping the existing mode.
    pub fn write(&self, path: &str, bytes: &[u8]) -> Result<Revision, FsError> {
        let _lock = self.exclusive()?;
        let full = self.vault.safe_path(path, false)?;
        self.vault.write_note_locked(&full, bytes)
    }

    /// `create(path, content)`: no-clobber (a dangling symlink counts as
    /// existing) → `AlreadyExists("Destination already exists: <path>")`.
    pub fn create(&self, path: &str, bytes: &[u8]) -> Result<Revision, FsError> {
        let _lock = self.exclusive()?;
        let full = self.vault.safe_path(path, false)?;
        self.vault.create_note_locked(&full, bytes, path)
    }

    /// Encrypt or decrypt one note in place (atomic; requires an unlocked key
    /// when either side is encrypted).
    pub fn set_encrypted(&self, path: &str, encrypted: bool) -> Result<Revision, FsError> {
        let _lock = self.exclusive()?;
        let full = self.vault.safe_path(path, true)?;
        let note = self.vault.read_note_as(&full, path)?;
        if note.encrypted == encrypted {
            return Ok(note.revision);
        }
        self.vault
            .write_note_with_encryption(&full, &note.bytes, encrypted)
    }

    /// `update(path, content)`: the file must exist (`NotFound`); parents are
    /// never created (HP-006).
    pub fn update(&self, path: &str, bytes: &[u8]) -> Result<Revision, FsError> {
        let _lock = self.exclusive()?;
        let full = self.vault.safe_path(path, true)?;
        self.vault.update_locked(&full, bytes, path)
    }

    /// `delete(path, expected_revision:)`: `RevisionConflict` when the given
    /// revision is not the SHA-256 hex of the current bytes; removes recents.
    pub fn delete(&self, path: &str, expected_revision: Option<&str>) -> Result<(), FsError> {
        {
            let _lock = self.exclusive()?;
            let full = self.vault.safe_path(path, true)?;
            self.vault
                .delete_note_locked(&full, expected_revision, path)?;
        }
        self.update_recents(|recents| recents.remove(path));
        Ok(())
    }

    /// The app's **Delete**: move a note or folder to the Trash (FE-267 fixed:
    /// non-empty folders can be deleted honestly, and nothing is lost for
    /// good). `RevisionConflict` when `expected_revision` no longer matches a
    /// file; a symlink is trashed itself, never its target; removes recents
    /// under `path`.
    pub fn trash(&self, path: &str, expected_revision: Option<&str>) -> Result<(), FsError> {
        {
            let _lock = self.exclusive()?;
            let full = self.vault.trash_path(path)?;
            self.vault.trash_locked(&full, expected_revision, path)?;
        }
        self.update_recents(|recents| recents.remove(path));
        Ok(())
    }

    /// `rename(old, new)` for files **and** folders: source must exist
    /// (`NotFound("Note not found: <old>")`), destination must not
    /// (`AlreadyExists`), `mkdir -p` dest parent, no-clobber rename (EXDEV →
    /// copy + delete), remap recents. Never rewrites links in other notes.
    pub fn rename(&self, old: &str, new: &str) -> Result<(), FsError> {
        {
            let _lock = self.exclusive()?;
            let from = self.vault.safe_path(old, true)?;
            let to = self.vault.safe_path(new, false)?;
            self.vault.rename_locked(&from, &to, old, new)?;
        }
        self.update_recents(|recents| recents.remap(old, new));
        Ok(())
    }

    /// `Folder#rename`: [`NotesService::rename`], then rewrite the Hugo slug
    /// of `<new>/index.md` to `slugify(basename(new))` when its front matter
    /// has one (errors from the slug update are ignored).
    pub fn rename_folder(&self, old: &str, new: &str) -> Result<(), FsError> {
        self.rename(old, new)?;
        // Rails logged and swallowed every failure of the slug update.
        let _ = self.update_hugo_index_slug(new);
        Ok(())
    }

    /// `Folder#update_hugo_index_slug`.
    fn update_hugo_index_slug(&self, folder: &str) -> Result<(), FsError> {
        let index = format!("{folder}/index.md");
        if !self.is_file(&index)? {
            return Ok(());
        }
        let note = self.read(&index)?;
        let content = String::from_utf8(note.bytes).map_err(|_| {
            FsError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid byte sequence in UTF-8",
            ))
        })?;
        let new_slug = frankmd_core::slug::slugify(frankmd_core::note::ruby_basename(folder, None));
        if let Some(updated) = hugo::update_frontmatter_slug(&content, &new_slug) {
            self.write(&index, updated.as_bytes())?;
        }
        Ok(())
    }

    /// `create_folder(path)` (`mkdir -p` parents; `AlreadyExists` if present).
    pub fn create_folder(&self, path: &str) -> Result<(), FsError> {
        let _lock = self.exclusive()?;
        let full = self.vault.safe_path(path, false)?;
        self.vault.create_folder_locked(&full, path)
    }

    /// `delete_folder(path)`: must be a directory (`NotFound("Folder not
    /// found: <path>")`) and empty incl. hidden files (`NotEmpty`); removes
    /// recents under it.
    pub fn delete_folder(&self, path: &str) -> Result<(), FsError> {
        {
            let _lock = self.exclusive()?;
            let full = self.vault.safe_path(path, true)?;
            self.vault.delete_folder_locked(&full, path)?;
        }
        self.update_recents(|recents| recents.remove(path));
        Ok(())
    }

    /// `exists?(path)` (follows symlinks; `false` for escaping paths is an
    /// error, as in Rails: `InvalidPath`).
    pub fn exists(&self, path: &str) -> Result<bool, FsError> {
        Ok(self.vault.safe_path(path, false)?.abs().exists())
    }

    /// `file?(path)`.
    pub fn is_file(&self, path: &str) -> Result<bool, FsError> {
        Ok(self.vault.safe_path(path, false)?.abs().is_file())
    }

    /// `directory?(path)`.
    pub fn is_dir(&self, path: &str) -> Result<bool, FsError> {
        Ok(self.vault.safe_path(path, false)?.abs().is_dir())
    }

    /// `HugoService.template_source` IO half: the bytes of
    /// `<root>/.hugo_template.md` when it is a regular file (symlinks
    /// followed), `None` otherwise or on a read error (→ default template).
    ///
    /// Hardening over Rails: a template symlink must resolve inside the vault.
    pub fn hugo_template(&self) -> Option<Vec<u8>> {
        let full = self.vault.safe_path(hugo::TEMPLATE_FILE, false).ok()?;
        if !full.abs().is_file() {
            return None;
        }
        fs::read(full.abs()).ok()
    }

    /// `HugoService.generate_blog_post` + `create`: build the post at `now`
    /// with the vault's template, then create it (no-clobber, parents made).
    pub fn create_hugo_post(
        &self,
        title: &str,
        parent: Option<&str>,
        style: HugoPathStyle,
        now: jiff::Zoned,
    ) -> Result<GeneratedPost, FsError> {
        let template = self
            .hugo_template()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let post = hugo::generate_post(&NewPost {
            title,
            parent,
            now,
            style,
            template: template.as_deref(),
        });
        self.create(&post.path, post.content.as_bytes())?;
        Ok(post)
    }

    /// `BacklinksService#find_backlinks(target, context_lines:)`: recursive
    /// walk (dot-entries skipped, **symlinks followed**, `.md` only), target
    /// skipped. `InvalidPath("Invalid path")` for `..` in the target. Order is
    /// unspecified (filesystem order); callers sort by `(path, line_number)`.
    ///
    /// Security divergence from Rails: a symlink (file or folder) whose target
    /// resolves outside the vault is skipped — Rails followed it and leaked
    /// lines of outside files. In-vault symlinks are followed, and a symlink
    /// cycle is walked once instead of recursing until the OS gives up.
    pub fn backlinks(&self, target: &str, context_lines: usize) -> Result<Vec<Backlink>, FsError> {
        let query = BacklinkQuery::new(target)
            .map_err(|_| FsError::InvalidPath(BACKLINKS_INVALID_PATH.to_owned()))?;
        let root = self.vault.real_root();
        let mut files = Vec::new();
        let mut visited = HashSet::new();
        collect_markdown_files(root, root, &mut visited, &mut files)?;
        let mut hits = Vec::new();
        for file in files {
            let Some(rel) = file.strip_prefix(root).ok().and_then(Path::to_str) else {
                continue;
            };
            if query.is_target(rel) {
                continue;
            }
            let bytes = fs::read(&file)?;
            if frankmd_crypto::is_note_encrypted(&bytes) {
                continue;
            }
            let content = String::from_utf8(bytes).map_err(|_| {
                FsError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid byte sequence in UTF-8",
                ))
            })?;
            hits.extend(wikilink::scan_note(&query, rel, &content, context_lines));
        }
        Ok(hits)
    }

    /// The exclusive filesystem lock every mutation holds.
    fn exclusive(&self) -> Result<crate::lock::LockGuard, FsError> {
        self.vault.lock(FILESYSTEM_LOCK, LockKind::Exclusive)
    }

    /// **[COMPAT]** `update_recent_files`: only when `.fed` is a regular file;
    /// failures are swallowed (Rails logged them).
    fn update_recents(&self, f: impl FnOnce(&RecentFilesService<'_>) -> Result<bool, FsError>) {
        if let Some(store) = self.recents.filter(|s| s.is_present()) {
            let _ = f(&RecentFilesService::new(self.vault, store));
        }
    }
}

/// `Folder#find_in_tree`: depth-first search by exact path string.
fn find_in_tree<'t>(nodes: &'t [TreeNode], path: &str) -> Option<&'t TreeNode> {
    for node in nodes {
        if node.path().as_str() == path {
            return Some(node);
        }
        if let TreeNode::Folder { children, .. } = node
            && let Some(found) = find_in_tree(children, path)
        {
            return Some(found);
        }
    }
    None
}

/// `BacklinksService#collect_markdown_files`: dot entries skipped, folders
/// (and in-vault symlinks to folders) descended into once each, `*.md`
/// collected. Symlinks resolving outside `root` are skipped.
fn collect_markdown_files(
    root: &Path,
    dir: &Path,
    visited: &mut HashSet<crate::meta::FileKey>,
    out: &mut Vec<std::path::PathBuf>,
) -> Result<(), FsError> {
    let Ok(meta) = fs::metadata(dir) else {
        return Ok(());
    };
    if !meta.is_dir() || !visited.insert(crate::meta::file_key(dir, &meta)) {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_symlink()) && !resolves_inside(root, &path) {
            continue;
        }
        if path.is_dir() {
            collect_markdown_files(root, &path, visited, out)?;
        } else if name.ends_with(NOTE_EXT) {
            out.push(path);
        }
    }
    Ok(())
}

/// `path`'s realpath lies within `root` (already canonical); dangling or
/// unreadable links count as outside.
fn resolves_inside(root: &Path, path: &Path) -> bool {
    path.canonicalize().is_ok_and(|real| real.starts_with(root))
}
