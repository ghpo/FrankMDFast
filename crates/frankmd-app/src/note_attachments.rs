//! Local files referenced by a note, independent of the preview UI.
//!
//! Used by the note-encryption cascade, the migration action and tests. The
//! scanner masks code/comments and covers Markdown links/images, reference
//! definitions and HTML media `src` attributes. Only regular files inside the
//! vault are returned; notes (`.md`), `.fed`, symlinks and external URLs are
//! ignored.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::preview::{LinkContext, LinkTarget, classify_link};

/// Paths (absolute) of the local attachments referenced by `source`.
pub(crate) fn attachments_for_source(
    source: &str,
    note_rel: &str,
    vault_root: &Path,
) -> Vec<PathBuf> {
    let note_abs = vault_root.join(note_rel);
    let context = LinkContext {
        base_dir: note_abs.parent().map(Path::to_path_buf),
        vault_root: Some(vault_root.to_path_buf()),
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for href in frankmd_media::usage::references_in(source) {
        let LinkTarget::Attachment(path) = classify_link(&href, &context) else {
            continue;
        };
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() || meta.file_type().is_symlink() {
            continue;
        }
        if path.file_name().is_some_and(|name| name == ".fed") {
            continue;
        }
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn collects_local_media_and_skips_notes_external_and_code() {
        let dir =
            std::env::temp_dir().join(format!("frankmd-note-attachments-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("notes")).unwrap();
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::create_dir_all(dir.join("videos")).unwrap();
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("images/pic.png"), b"png").unwrap();
        std::fs::write(dir.join("images/talk.m4a"), b"m4a").unwrap();
        std::fs::write(dir.join("videos/clip.mp4"), b"mp4").unwrap();
        std::fs::write(dir.join("docs/a.pdf"), b"pdf").unwrap();
        std::fs::write(dir.join("notes/other.md"), b"# other").unwrap();
        let source = concat!(
            "![pic](../images/pic.png)\n",
            "[audio](../images/talk.m4a)\n",
            "<video><source src=\"../videos/clip.mp4\"></video>\n",
            "[pdf](../docs/a.pdf)\n",
            "[again](../images/talk.m4a)\n",
            "[note](other.md)\n",
            "[site](https://example.com/a.pdf)\n",
            "`[code](../images/pic.png)`\n",
        );
        let got = attachments_for_source(source, "notes/note.md", &dir);
        let mut names: Vec<String> = got
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "docs/a.pdf",
                "images/pic.png",
                "images/talk.m4a",
                "videos/clip.mp4"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
