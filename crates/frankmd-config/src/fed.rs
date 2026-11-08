//! Line-preserving `.fed` text operations.
//!
//! Hand-written because the writer must *uncomment* `# key = default` lines
//! in place and leave every other byte alone (backend.md §1.5–1.8). This is
//! **not** TOML: no inline comments, first `=` splits, last duplicate wins.
//!
//! Everything here is pure `&str → String`; [`crate::store::ConfigStore`]
//! adds locking and atomic writes on top.

use crate::value::{cast, present_str, ruby_strip};
use crate::{Key, Value};
use std::sync::OnceLock;

/// One parsed `.fed` entry, before casting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    /// Schema key (line key lowercased, then [`Key::from_name`]).
    pub key: Key,
    /// Value after the first `=`, stripped, quotes removed (still uncast).
    pub raw: String,
}

/// Result of [`parse`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    /// Present keys in first-insertion order, last duplicate wins (**[COMPAT]**
    /// Ruby hash semantics). `None` = present but nil (blocks ENV).
    pub values: Vec<(Key, Option<Value>)>,
    /// One message per skipped line (bad JSON / invalid custom theme); each
    /// names the key (`Ignoring invalid setting custom_theme`).
    pub warnings: Vec<String>,
}

/// **[COMPAT]** `parse_config` (backend.md §1.5): LF or CRLF; comments,
/// blanks, malformed lines and unknown keys ignored.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for entry in raw_entries(text) {
        match cast(&entry.raw, entry.key.spec().ty) {
            Ok(value) => match parsed.values.iter_mut().find(|(k, _)| *k == entry.key) {
                Some(slot) => slot.1 = value,
                None => parsed.values.push((entry.key, value)),
            },
            Err(_) => parsed
                .warnings
                .push(format!("Ignoring invalid setting {}", entry.key.name())),
        }
    }
    parsed
}

/// Every recognised entry, uncast, in file order (duplicates kept).
pub fn raw_entries(text: &str) -> Vec<RawEntry> {
    ruby_lines(text)
        .filter_map(|line| {
            let line = ruby_strip(line);
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (name, rest) = split_key_line(line)?;
            let key = Key::from_name(&name)?;
            Some(RawEntry {
                key,
                raw: unquote(ruby_strip(rest)).to_owned(),
            })
        })
        .collect()
}

/// Ruby `String#each_line`: pieces ending in `\n` (kept), plus a final
/// unterminated piece. A lone `\r` does not end a line.
fn ruby_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive('\n')
}

/// Ruby `String#chomp`: drop one trailing `\n`, `\r\n` or `\r`.
fn chomp(line: &str) -> &str {
    match line.strip_suffix('\n') {
        Some(rest) => rest.strip_suffix('\r').unwrap_or(rest),
        None => line.strip_suffix('\r').unwrap_or(line),
    }
}

/// Ruby regex `\s` (`[ \t\n\v\f\r]`).
fn is_regex_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r')
}

/// `^([a-z0-9_]+)\s*=`/i at the start of `line`: the lowercased key and the
/// text after the first `=` (leading `\s*` not yet removed).
fn split_key_line(line: &str) -> Option<(String, &str)> {
    let end = line
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(line.len());
    if end == 0 {
        return None;
    }
    let rest = line[end..]
        .trim_start_matches(is_regex_space)
        .strip_prefix('=')?;
    Some((line[..end].to_ascii_lowercase(), rest))
}

/// The key of a `#`-commented assignment (`^#\s*([a-z0-9_]+)\s*=`/i).
fn commented_key(stripped: &str) -> Option<String> {
    let rest = stripped
        .strip_prefix('#')?
        .trim_start_matches(is_regex_space);
    split_key_line(rest).map(|(k, _)| k)
}

/// **[COMPAT]** quote removal: one pass for `"…"`, then one for `'…'`; a lone
/// quote character becomes empty.
fn unquote(value: &str) -> &str {
    let strip = |v: &'_ str, q: char| -> Option<usize> {
        (v.starts_with(q) && v.ends_with(q)).then_some(v.len())
    };
    let mut v = value;
    for q in ['"', '\''] {
        if let Some(len) = strip(v, q) {
            v = if len >= 2 { &v[1..len - 1] } else { "" };
        }
    }
    v
}

/// **[COMPAT]** `save_keys_without_lock` line rewrite (backend.md §1.8):
/// first matching `# key =` comment is uncommented, every active `key =` line
/// replaced, whitespace-only lines emptied, missing keys appended in `changes`
/// order, output `lines.join("\n") + "\n"` (CRLF → LF).
/// `changes` are already-formatted lines (`key = value`).
pub fn apply_changes(text: &str, changes: &[(Key, String)]) -> String {
    // Ruby hash semantics: a repeated key keeps its first position, last value.
    let mut ordered: Vec<(Key, &str)> = Vec::new();
    for (key, line) in changes {
        match ordered.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = line,
            None => ordered.push((*key, line)),
        }
    }
    let change_for = |name: &str| {
        ordered
            .iter()
            .find(|(k, _)| k.name() == name)
            .map(|(k, l)| (*k, *l))
    };

    let mut lines: Vec<&str> = Vec::new();
    let mut written: Vec<Key> = Vec::new();
    for line in ruby_lines(text) {
        let stripped = ruby_strip(line);
        if stripped.is_empty() {
            lines.push("");
        } else if stripped.starts_with('#') {
            match commented_key(stripped).and_then(|name| change_for(&name)) {
                Some((key, new)) if !written.contains(&key) => {
                    lines.push(new);
                    written.push(key);
                }
                _ => lines.push(chomp(line)),
            }
        } else {
            match split_key_line(stripped).and_then(|(name, _)| change_for(&name)) {
                Some((key, new)) => {
                    lines.push(new);
                    if !written.contains(&key) {
                        written.push(key);
                    }
                }
                None => lines.push(chomp(line)),
            }
        }
    }
    for (key, new) in &ordered {
        if !written.contains(key) {
            lines.push(new);
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// **[COMPAT]** The upgrade condition of `upgrade_config_file` (§1.6): no line
/// contains `# AI/LLM` and no line matches `/^#?\s*KEY\s*=/i` for any of
/// [`Key::AI_UPGRADE_MARKERS`].
pub fn needs_ai_section(text: &str) -> bool {
    !ruby_lines(text).map(chomp).any(|line| {
        line.contains("# AI/LLM")
            || Key::AI_UPGRADE_MARKERS
                .iter()
                .any(|k| mentions_key(line, k.name()))
    })
}

/// `/^#?\s*KEY\s*=/i` (not anchored to a word boundary after `#?\s*`).
fn mentions_key(line: &str, key: &str) -> bool {
    let rest = line
        .strip_prefix('#')
        .unwrap_or(line)
        .trim_start_matches(is_regex_space);
    rest.get(..key.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(key))
        && rest[key.len()..]
            .trim_start_matches(is_regex_space)
            .starts_with('=')
}

/// **[COMPAT]** `upgrade_config_file`: `Some(new text)` when
/// [`needs_ai_section`]; the original lines (split on `\n`, chomped) get one
/// `""` appended if the last is non-blank, then [`ai_section_lines`], joined
/// with `\n` plus a trailing `\n`. `None` when no upgrade applies.
pub fn upgrade_ai_section(text: &str) -> Option<String> {
    if !needs_ai_section(text) {
        return None;
    }
    let mut lines: Vec<&str> = ruby_lines(text).map(chomp).collect();
    if lines
        .last()
        .is_some_and(|last| present_str(ruby_strip(last)))
    {
        lines.push("");
    }
    lines.extend_from_slice(ai_section_lines());
    let mut out = lines.join("\n");
    out.push('\n');
    Some(out)
}

/// A `TEMPLATE_SECTIONS` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateSection {
    /// The section's marker comment (`# UI Settings`, `# AI/LLM (for grammar checking)`, ...).
    pub marker: &'static str,
    /// Its lines (leading `""` included where the original has it).
    pub lines: &'static [&'static str],
}

/// **[COMPAT]** `Config::TEMPLATE_SECTIONS`, in order.
pub fn template_sections() -> &'static [TemplateSection] {
    &TEMPLATE_SECTIONS
}

/// The last template section (the AI/LLM block appended by the upgrade; it
/// starts with `""`).
pub fn ai_section_lines() -> &'static [&'static str] {
    TEMPLATE_SECTIONS[TEMPLATE_SECTIONS.len() - 1].lines
}

/// The verbatim auto-generated template (3032 bytes, SHA-256
/// `796513f7…fb154`). Golden: `tests/fixtures/golden/fed_template.fed`.
pub fn template() -> &'static str {
    static TEMPLATE: OnceLock<String> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let lines: Vec<&str> = TEMPLATE_SECTIONS
            .iter()
            .flat_map(|s| s.lines.iter().copied())
            .collect();
        let mut out = lines.join("\n");
        out.push('\n');
        out
    })
}

/// **[COMPAT]** `Config::TEMPLATE_SECTIONS` verbatim (config.rb:134-280).
static TEMPLATE_SECTIONS: [TemplateSection; 9] = [
    TemplateSection {
        marker: "# FrankMD Configuration",
        lines: &[
            "# FrankMD Configuration",
            "# Uncomment and modify values as needed.",
            "# Environment variables are used as defaults if not specified here.",
        ],
    },
    TemplateSection {
        marker: "# UI Settings",
        lines: &[
            "",
            "# UI Settings",
            "",
            "# Theme: light, dark, gruvbox, tokyo-night, solarized-dark,",
            "#        solarized-light, nord, cappuccino, osaka, hackerman",
            "# theme = dark",
            "",
            "# Editor font: cascadia-code, jetbrains-mono, fira-code,",
            "#              source-code-pro, ubuntu-mono, roboto-mono, hack",
            "# editor_font = cascadia-code",
            "",
            "# editor_font_size = 14",
            "# preview_zoom = 100",
            "# sidebar_visible = true",
            "# typewriter_mode = false",
            "# cursor_recenter_on_click = false",
            "# show_stats = false",
            "",
            "# Vim mode: hjkl navigation, modal editing, and : ex-commands (default: false)",
            "# vim_mode = false",
            "",
            "# Editor indent: 0 = tab, 1-6 = spaces (default: 2)",
            "# editor_indent = 2",
            "",
            "# Editor width in characters (default: 72, minimum: 72)",
            "# Increase for wider text area, e.g., 100, 120, or 150",
            "# editor_width = 72",
        ],
    },
    TemplateSection {
        marker: "# Local Images",
        lines: &["", "# Local Images", "", "# images_path = /path/to/images"],
    },
    TemplateSection {
        marker: "# Media Uploads",
        lines: &[
            "",
            "# Media Uploads (drag-and-drop)",
            "",
            "# Destination for new media: local or s3 (requires AWS S3 configuration).",
            "# media_storage_provider = local",
            "",
            "# Comma-separated file extensions accepted by the image/video drop tabs.",
            "# image_upload_extensions = .jpg,.jpeg,.png,.gif,.webp,.bmp",
            "# video_upload_extensions = .mp4,.webm,.mkv,.mov,.avi,.m4v,.ogv",
        ],
    },
    TemplateSection {
        marker: "# AWS S3",
        lines: &[
            "",
            "# AWS S3 (for image uploads)",
            "",
            "# aws_access_key_id = your-access-key",
            "# aws_secret_access_key = your-secret-key",
            "# aws_s3_bucket = your-bucket-name",
            "# aws_region = us-east-1",
            "",
            "# S3-compatible services (MinIO, SeaweedFS, ...): set a custom endpoint",
            "# and force path-style URLs (bucket in the path).",
            "# aws_s3_endpoint = http://localhost:9000",
            "# aws_s3_force_path_style = true",
        ],
    },
    TemplateSection {
        marker: "# YouTube API",
        lines: &[
            "",
            "# YouTube API (for video search)",
            "",
            "# youtube_api_key = your-youtube-api-key",
        ],
    },
    TemplateSection {
        marker: "# Google Custom Search",
        lines: &[
            "",
            "# Google Custom Search (for image search)",
            "",
            "# google_api_key = your-google-api-key",
            "# google_cse_id = your-custom-search-engine-id",
        ],
    },
    TemplateSection {
        marker: "# Hugo",
        lines: &[
            "",
            "# Hugo Blog Posts",
            "",
            "# Path style: dated = YYYY/MM/DD/slug/index.md (default)",
            "#             flat  = slug.md (in current folder)",
            "# hugo_path_style = dated",
            "",
            "# Custom frontmatter: create a .hugo_template.md file at the notes root",
            "# to override the generated frontmatter. Placeholders {{title}}, {{slug}}",
            "# and {{date}} are substituted. Leave unset to use the built-in template.",
        ],
    },
    TemplateSection {
        marker: "# AI/LLM",
        lines: &[
            "",
            "# AI/LLM (for grammar checking)",
            "# Provider priority when ai_provider = auto: openai > deepseek > anthropic > openrouter > ollama > gemini",
            "# Gemini is lowest priority for text (its key is mainly for image generation)",
            "",
            "# ai_provider = auto",
            "# ai_model = (uses provider-specific default if not set)",
            "",
            "# OpenAI (recommended)",
            "# openai_api_key = sk-...",
            "# openai_model = gpt-4o-mini",
            "",
            "# DeepSeek (OpenAI-compatible)",
            "# deepseek_api_key = sk-...",
            "# deepseek_model = deepseek-chat",
            "",
            "# Anthropic (Claude models)",
            "# anthropic_api_key = sk-ant-...",
            "# anthropic_model = claude-sonnet-4-20250514",
            "",
            "# Google Gemini",
            "# gemini_api_key = ...",
            "# gemini_model = gemini-2.0-flash",
            "",
            "# OpenRouter (multiple providers, pay-per-use)",
            "# openrouter_api_key = sk-or-...",
            "# openrouter_model = openai/gpt-4o-mini",
            "",
            "# Ollama (local, free) - for privacy, requires local setup",
            "# ollama_api_base = http://localhost:11434",
            "# ollama_model = llama3.2:latest",
        ],
    },
];
