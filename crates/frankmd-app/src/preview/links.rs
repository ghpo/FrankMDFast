//! What a click on a link or an image source is allowed to do.
//!
//! SECURITY (app brief): links open through the system opener only for
//! `http(s)` and `mailto`; `file:` and every other scheme are never followed.
//! Images load only from files inside the vault or from `https` URLs (through
//! frankmd-net's egress client). Everything here is pure so it is unit-tested.

use std::path::{Path, PathBuf};

/// Where a link click goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Open in the default browser / mail client.
    External(String),
    /// Scroll the preview to a heading anchor (without `#`).
    Anchor(String),
    /// Open another note (absolute path, contained in the vault).
    Note(PathBuf),
    /// A local file referenced by the note (audio, video, PDF, archive, …).
    /// Absolute path, contained in the vault.
    Attachment(PathBuf),
    /// Not followed (unsupported scheme, escapes the vault, …).
    Blocked,
}

/// Where an image's bytes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    /// A file inside the vault (absolute path).
    Local(PathBuf),
    /// An `https` URL fetched through the egress client.
    Remote(String),
    /// Not loaded (http, data:, file:, outside the vault…).
    Blocked,
}

/// Paths a link or image may resolve against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// The open note's folder (relative links resolve here).
    pub base_dir: Option<PathBuf>,
    /// The vault root (containment boundary; `/x` resolves here).
    pub vault_root: Option<PathBuf>,
}

impl Context {
    fn root(&self) -> Option<&Path> {
        self.vault_root.as_deref().or(self.base_dir.as_deref())
    }

    /// Resolve a vault path (`/abs` from the root, else relative to the note's
    /// folder) and prove it stays inside the vault (lexical `..` cleaning plus
    /// the symlink-aware realpath check of `PathSafety.contain`).
    fn contained(&self, raw: &str) -> Option<PathBuf> {
        let root = self.root()?;
        let decoded = percent_decode(raw);
        let rel = match decoded.strip_prefix('/') {
            Some(abs) => abs.to_owned(),
            None => {
                let base_rel = self
                    .base_dir
                    .as_deref()
                    .and_then(|b| b.strip_prefix(root).ok())
                    .unwrap_or(Path::new(""));
                let base_rel = base_rel.to_str()?;
                if base_rel.is_empty() {
                    decoded
                } else {
                    format!("{base_rel}/{decoded}")
                }
            }
        };
        frankmd_fs::path_safety::contain(root, Some(&rel))
    }
}

/// Classify a link `href` (already through the IR's URL allowlist).
pub fn classify_link(href: &str, cx: &Context) -> LinkTarget {
    let lower = href.trim().to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        return LinkTarget::External(href.trim().to_owned());
    }
    if let Some(anchor) = href.strip_prefix('#') {
        return LinkTarget::Anchor(percent_decode(anchor));
    }
    if has_scheme(&lower) || lower.starts_with("//") {
        return LinkTarget::Blocked;
    }
    // FrankMD's in-app URLs: `/notes/<path>` and the legacy image preview
    // route `/images/preview/<path>`.
    let path = href.split(['#', '?']).next().unwrap_or_default();
    let path = path
        .strip_prefix("/notes/")
        .map(|p| format!("/{p}"))
        .or_else(|| {
            path.strip_prefix("/images/preview/")
                .map(|p| format!("/images/{p}"))
        })
        .unwrap_or_else(|| path.to_owned());
    if path.is_empty() {
        return LinkTarget::Blocked;
    }
    match cx.contained(&path) {
        Some(abs) if !abs.to_string_lossy().ends_with(".md") => LinkTarget::Attachment(abs),
        Some(abs) => LinkTarget::Note(abs),
        None => LinkTarget::Blocked,
    }
}

/// Classify an image `src` (already through the IR's URL allowlist).
pub fn classify_image(src: &str, cx: &Context) -> ImageSource {
    let lower = src.trim().to_ascii_lowercase();
    if lower.starts_with("https://") {
        return ImageSource::Remote(src.trim().to_owned());
    }
    if has_scheme(&lower) || lower.starts_with("//") {
        // http:, data:, file:, blob:, … are never loaded.
        return ImageSource::Blocked;
    }
    // FrankMD's legacy image route `/images/preview/<path>` is vault-relative.
    let path = src.split(['#', '?']).next().unwrap_or_default();
    let path = path
        .strip_prefix("/images/preview/")
        .map(|p| format!("/images/{p}"))
        .unwrap_or_else(|| path.to_owned());
    match cx.contained(&path) {
        Some(abs) => ImageSource::Local(abs),
        None => ImageSource::Blocked,
    }
}

/// Does `s` start with a URL scheme (`[a-z][a-z0-9+.-]*:`)?
fn has_scheme(s: &str) -> bool {
    let Some(colon) = s.find(':') else {
        return false;
    };
    let scheme = &s[..colon];
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        && !scheme.is_empty()
        // A Windows drive letter is not a scheme, but we never accept those either.
        && !s[..colon].contains('/')
}

/// Decode `%XX` escapes (FrankMD percent-encodes inserted media paths).
/// Invalid escapes and non-UTF-8 results are kept verbatim.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(b) = hex {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(dir: &Path) -> Context {
        Context {
            base_dir: Some(dir.join("notes")),
            vault_root: Some(dir.to_path_buf()),
        }
    }

    #[test]
    fn only_web_and_mail_links_go_external() {
        let tmp = std::env::temp_dir();
        let c = cx(&tmp);
        assert_eq!(
            classify_link("https://x.org/a", &c),
            LinkTarget::External("https://x.org/a".into())
        );
        assert_eq!(
            classify_link("mailto:a@b.c", &c),
            LinkTarget::External("mailto:a@b.c".into())
        );
        assert_eq!(classify_link("file:///etc/passwd", &c), LinkTarget::Blocked);
        assert_eq!(classify_link("ftp://x.org", &c), LinkTarget::Blocked);
        assert_eq!(classify_link("tel:123", &c), LinkTarget::Blocked);
        assert_eq!(classify_link("//evil.org/x", &c), LinkTarget::Blocked);
        assert_eq!(
            classify_link("#my-section", &c),
            LinkTarget::Anchor("my-section".into())
        );
    }

    #[test]
    fn note_links_stay_in_the_vault() {
        let dir = tempfile_dir();
        let c = cx(&dir);
        assert_eq!(
            classify_link("other.md", &c),
            LinkTarget::Note(dir.join("notes/other.md"))
        );
        assert_eq!(
            classify_link("/notes/target.md", &c),
            LinkTarget::Note(dir.join("target.md"))
        );
        assert_eq!(
            classify_link("../x%20y.md#h", &c),
            LinkTarget::Note(dir.join("x y.md"))
        );
        assert_eq!(classify_link("../../outside.md", &c), LinkTarget::Blocked);
        assert_eq!(
            classify_link("../images/talk.m4a", &c),
            LinkTarget::Attachment(dir.join("images/talk.m4a"))
        );
        assert_eq!(
            classify_link("/images/preview/talk%20one.mp3", &c),
            LinkTarget::Attachment(dir.join("images/talk one.mp3"))
        );
    }

    #[test]
    fn images_load_only_from_vault_or_https() {
        let dir = tempfile_dir();
        let c = cx(&dir);
        assert_eq!(
            classify_image("pic.png", &c),
            ImageSource::Local(dir.join("notes/pic.png"))
        );
        assert_eq!(
            classify_image("../images/a%20b.png", &c),
            ImageSource::Local(dir.join("images/a b.png"))
        );
        assert_eq!(
            classify_image("https://x.org/a.png", &c),
            ImageSource::Remote("https://x.org/a.png".into())
        );
        assert_eq!(
            classify_image("http://x.org/a.png", &c),
            ImageSource::Blocked
        );
        assert_eq!(
            classify_image("file:///etc/passwd", &c),
            ImageSource::Blocked
        );
        assert_eq!(
            classify_image("data:image/png;base64,AAAA", &c),
            ImageSource::Blocked
        );
        assert_eq!(
            classify_image("../../../etc/passwd", &c),
            ImageSource::Blocked
        );
        assert_eq!(
            classify_image("/images/preview/x.png", &c),
            ImageSource::Local(dir.join("images/x.png"))
        );
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_decode("bad%zz%"), "bad%zz%");
        assert_eq!(percent_decode("%E2%9C%93"), "✓");
    }

    #[allow(clippy::disallowed_methods)] // test fixture setup
    fn tempfile_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("frankmd-preview-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("notes")).expect("mkdir");
        dir.canonicalize().expect("canonical")
    }
}
