//! Full-text search (**[COMPAT]** `NotesService#search_content`, backend.md §3).
//!
//! The original, in short:
//!
//! ```ruby
//! return [] if query.blank?
//! regex = Regexp.new(query, IGNORECASE, timeout: 0.5) rescue Regexp.new(Regexp.escape(query), ...)
//! collect_markdown_files_unsorted(base) do |file|      # readdir order, no symlinks, no dot-entries
//!   break if results.size >= max_results
//!   matches = search_file(file, regex, context_lines, max_results - results.size)
//!   ...
//! end
//! results.sort_by { |r| -r[:mtime].to_i }
//! ```
//!
//! Unlike the original, skipped files are reported ([`Results::skipped`]).

use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use onig::{MatchParam, Regex, RegexOptions, SearchOptions, Syntax};

/// Files strictly larger than this are skipped (`MAX_SEARCH_FILE_BYTES`, 5 MiB).
pub const MAX_SEARCH_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// Per-match regex time budget (`SEARCH_REGEX_TIMEOUT = 0.5`).
pub const SEARCH_REGEX_TIMEOUT: Duration = Duration::from_millis(500);
/// `search_content` default `context_lines:`.
pub const DEFAULT_CONTEXT_LINES: i64 = 3;
/// `search_content` default `max_results:`.
pub const DEFAULT_MAX_RESULTS: usize = 50;
/// What the UI search (`GET /notes/search`) passes: `context_lines: 3`.
pub const UI_CONTEXT_LINES: i64 = 3;
/// What the UI search (`GET /notes/search`) passes: `max_results: 20`.
pub const UI_MAX_RESULTS: usize = 20;

/// Backtracking steps allowed per millisecond of [`Query::timeout`]. Oniguruma
/// has no wall-clock limit, so the budget becomes a retry limit (catastrophic
/// patterns hit it quickly); the wall clock is also checked after each line.
const RETRIES_PER_MS: u128 = 10_000;

/// Search request.
#[derive(Debug, Clone)]
pub struct Query {
    /// Pattern text exactly as typed (surrounding spaces are significant).
    /// `None` behaves like Ruby `nil` (blank → no results).
    pub pattern: Option<String>,
    /// Lines of context on each side. Negative values yield an empty
    /// context (Ruby's empty `(start..end)` range), not an error.
    pub context_lines: i64,
    /// Max results (checked before each file and per match).
    pub max_results: usize,
    /// Regex match time budget; a match that exceeds it skips that file.
    pub timeout: Duration,
}

impl Query {
    /// `search_content(pattern)` with the service defaults (context 3, max 50).
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: Some(pattern.into()),
            context_lines: DEFAULT_CONTEXT_LINES,
            max_results: DEFAULT_MAX_RESULTS,
            timeout: SEARCH_REGEX_TIMEOUT,
        }
    }

    /// The parameters the UI search uses (context 3, max 20).
    pub fn ui(pattern: impl Into<String>) -> Self {
        Self {
            context_lines: UI_CONTEXT_LINES,
            max_results: UI_MAX_RESULTS,
            ..Self::new(pattern)
        }
    }
}

/// ActiveSupport `blank?` for strings: empty or only Unicode whitespace
/// (`/\A[[:space:]]*\z/`, which includes NBSP U+00A0).
pub fn is_blank(query: &str) -> bool {
    query.chars().all(char::is_whitespace)
}

/// The match timed out (`Regexp::TimeoutError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("regex match timed out")]
pub struct MatchTimeout;

/// A compiled search pattern with Ruby semantics: `Regexp.new(q, IGNORECASE,
/// timeout: 0.5)`, falling back to `Regexp.escape(q)` (still IGNORECASE)
/// when `q` is not a valid Ruby regex. Backed by `onig` (Ruby syntax,
/// multi-char case folding).
pub struct SearchRegex {
    inner: Regex,
    /// Effective pattern (escaped when `literal_fallback`).
    pattern: String,
    timeout: Duration,
    literal_fallback: bool,
}

impl std::fmt::Debug for SearchRegex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchRegex")
            .field("pattern", &self.pattern)
            .field("literal_fallback", &self.literal_fallback)
            .finish()
    }
}

impl SearchRegex {
    /// Compile `query`. `None` when the query is blank (no search at all).
    pub fn compile(query: &str, timeout: Duration) -> Option<Self> {
        if is_blank(query) {
            return None;
        }
        let build = |pattern: &str| {
            Regex::with_options(
                pattern,
                RegexOptions::REGEX_OPTION_IGNORECASE,
                Syntax::ruby(),
            )
        };
        if let Ok(inner) = build(query) {
            return Some(Self {
                inner,
                pattern: query.to_owned(),
                timeout,
                literal_fallback: false,
            });
        }
        let escaped = ruby_regexp_escape(query);
        let inner = build(&escaped).ok()?;
        Some(Self {
            inner,
            pattern: escaped,
            timeout,
            literal_fallback: true,
        })
    }

    /// True when `query` failed to compile and was escaped to a literal.
    pub fn is_literal_fallback(&self) -> bool {
        self.literal_fallback
    }

    /// Ruby `line.match?(regex)`, bounded by the timeout.
    pub fn is_match(&self, line: &str) -> Result<bool, MatchTimeout> {
        let started = Instant::now();
        let mut param = MatchParam::default();
        let retries = self
            .timeout
            .as_millis()
            .max(1)
            .saturating_mul(RETRIES_PER_MS);
        param.set_retry_limit_in_match(u32::try_from(retries).unwrap_or(u32::MAX));
        let found = self
            .inner
            .search_with_param(
                line,
                0,
                line.len(),
                SearchOptions::SEARCH_OPTION_NONE,
                None,
                param,
            )
            .map_err(|_| MatchTimeout)?;
        if started.elapsed() > self.timeout {
            return Err(MatchTimeout);
        }
        Ok(found.is_some())
    }
}

/// Ruby `Regexp.escape` (public entry point for callers that build literal patterns).
pub fn regexp_escape(s: &str) -> String {
    ruby_regexp_escape(s)
}

/// Ruby `Regexp.escape`: backslash-escape regex metacharacters and spell
/// out whitespace controls.
fn ruby_regexp_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '[' | ']' | '{' | '}' | '(' | ')' | '|' | '-' | '*' | '.' | '\\' | '?' | '+' | '^'
            | '$' | '#' | ' ' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{0C}' => out.push_str("\\f"),
            '\u{0B}' => out.push_str("\\v"),
            _ => out.push(c),
        }
    }
    out
}

/// One line of a context window (shared with backlinks).
pub use frankmd_core::wikilink::ContextLine;

/// A matching line within one file (`search_file` result item).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    /// 1-based line.
    pub line_number: u32,
    /// The whole matching line (not just the matched span).
    pub match_text: String,
    /// `±context_lines` window clipped to the file; windows of nearby
    /// matches overlap independently (never merged).
    pub context: Vec<ContextLine>,
}

/// One result, the shape of `GET /notes/search` JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Vault-relative path (`folder/nested.md`).
    pub path: String,
    /// Basename without `.md`.
    pub name: String,
    /// File mtime (stat'ed only for files that matched).
    pub mtime: SystemTime,
    /// 1-based line.
    pub line_number: u32,
    /// The whole matching line.
    pub match_text: String,
    /// Context window.
    pub context: Vec<ContextLine>,
}

/// Why a file was skipped (reported, never fatal — HP-028).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Note is encrypted; only name search applies.
    Encrypted,
    /// Larger than [`MAX_SEARCH_FILE_BYTES`].
    TooLarge,
    /// Read/stat failed (EACCES, vanished between listing and read, ...).
    Io(String),
    /// A line match exceeded the regex timeout.
    Timeout,
}

/// A skipped file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Vault-relative path.
    pub path: String,
    /// Reason.
    pub reason: SkipReason,
}

/// Results plus skipped files.
#[derive(Debug, Clone, Default)]
pub struct Results {
    /// Hits sorted by file mtime, newest first, by whole seconds
    /// (Ruby `sort_by { -mtime.to_i }`); line order within a file.
    pub hits: Vec<Hit>,
    /// Files that were not searched.
    pub skipped: Vec<Skipped>,
    /// `max_results` was reached (the walk stopped early).
    pub truncated: bool,
}

/// Ruby `readlines(chomp: true).map(&:scrub)`: split after `\n`, strip a
/// trailing `\n` or `\r\n` (a lone `\r` is NOT a separator), no trailing
/// empty element for a final newline, invalid UTF-8 → U+FFFD. A BOM stays
/// part of the first line.
pub fn split_lines(bytes: &[u8]) -> Vec<String> {
    let body = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if bytes.is_empty() {
        return Vec::new();
    }
    body.split(|b| *b == b'\n')
        .map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            String::from_utf8_lossy(line).into_owned()
        })
        .collect()
}

/// `search_file` over already-split lines: every matching line up to
/// `max_matches`, each with its context window.
pub fn search_lines(
    lines: &[String],
    regex: &SearchRegex,
    context_lines: i64,
    max_matches: usize,
) -> Result<Vec<LineMatch>, MatchTimeout> {
    search_lines_capped(lines, regex, context_lines, max_matches).map(|(matches, _)| matches)
}

/// [`search_lines`] that also reports whether `max_matches` cut the scan short.
fn search_lines_capped(
    lines: &[String],
    regex: &SearchRegex,
    context_lines: i64,
    max_matches: usize,
) -> Result<(Vec<LineMatch>, bool), MatchTimeout> {
    let mut matches = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !regex.is_match(line)? {
            continue;
        }
        if matches.len() >= max_matches {
            return Ok((matches, true));
        }
        matches.push(LineMatch {
            line_number: line_number(index),
            match_text: line.clone(),
            context: context_window(lines, index, context_lines),
        });
    }
    Ok((matches, false))
}

fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

/// `(max(0, i - c)..=min(n - 1, i + c))`, empty when the range is inverted.
fn context_window(lines: &[String], index: usize, context_lines: i64) -> Vec<ContextLine> {
    let index_i = i64::try_from(index).unwrap_or(i64::MAX);
    let last = i64::try_from(lines.len()).unwrap_or(i64::MAX) - 1;
    let start = index_i.saturating_sub(context_lines).max(0);
    let end = index_i.saturating_add(context_lines).min(last);
    (start..=end)
        .filter_map(|i| usize::try_from(i).ok())
        .map(|i| ContextLine {
            line_number: i + 1,
            content: lines[i].clone(),
            is_match: i == index,
        })
        .collect()
}

/// Run a search over the notes under `root`. Walks in directory (readdir)
/// order without sorting; skips symlinks (files and dirs), dot-entries and
/// non-`.md` files; root `images/`/`videos/` ARE searched (unlike the tree).
///
/// Security: no symlink is ever followed (the original's behaviour, pinned by
/// the `symlinked dir and file skipped` golden), so nothing outside the vault
/// is read, dangling links are ignored and link cycles cannot occur.
pub fn search(root: &Path, query: &Query) -> Results {
    let mut results = Results::default();
    let Some(regex) = query
        .pattern
        .as_deref()
        .and_then(|p| SearchRegex::compile(p, query.timeout))
    else {
        return results;
    };
    let mut walk = Walk {
        root,
        query,
        regex: &regex,
        results: &mut results,
    };
    // Returns `false` when max_results stopped the walk (already in `truncated`).
    walk.dir(root);
    results
        .hits
        .sort_by_key(|hit| std::cmp::Reverse(whole_seconds(hit.mtime)));
    results
}

/// Ruby `Time#to_i` (floor to whole seconds).
fn whole_seconds(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => {
            let d = e.duration();
            let secs = i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
            if d.subsec_nanos() == 0 {
                -secs
            } else {
                -secs - 1
            }
        }
    }
}

/// `collect_markdown_files_unsorted` + the per-file body of `search_content`.
struct Walk<'a> {
    root: &'a Path,
    query: &'a Query,
    regex: &'a SearchRegex,
    results: &'a mut Results,
}

impl Walk<'_> {
    fn rel(&self, path: &Path) -> String {
        path.strip_prefix(self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    fn skip(&mut self, path: &Path, reason: SkipReason) {
        let path = self.rel(path);
        self.results.skipped.push(Skipped { path, reason });
    }

    /// Returns `false` once `max_results` stops the walk.
    fn dir(&mut self, dir: &Path) -> bool {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                if dir != self.root {
                    self.skip(dir, SkipReason::Io(e.to_string()));
                }
                return true;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    self.skip(dir, SkipReason::Io(e.to_string()));
                    continue;
                }
            };
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if !self.dir(&path) {
                    return false;
                }
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "md") {
                if self.results.hits.len() >= self.query.max_results {
                    self.results.truncated = true;
                    return false;
                }
                self.file(&path);
            }
        }
        true
    }

    fn file(&mut self, path: &Path) {
        let size = match std::fs::metadata(path) {
            Ok(meta) => meta.len(),
            Err(e) => return self.skip(path, SkipReason::Io(e.to_string())),
        };
        if size > MAX_SEARCH_FILE_BYTES {
            return self.skip(path, SkipReason::TooLarge);
        }
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => return self.skip(path, SkipReason::Io(e.to_string())),
        };
        if frankmd_crypto::is_note_encrypted(&bytes) {
            return self.skip(path, SkipReason::Encrypted);
        }
        let lines = split_lines(&bytes);
        let remaining = self.query.max_results - self.results.hits.len();
        let (matches, capped) =
            match search_lines_capped(&lines, self.regex, self.query.context_lines, remaining) {
                Ok(found) => found,
                Err(MatchTimeout) => return self.skip(path, SkipReason::Timeout),
            };
        if matches.is_empty() {
            return;
        }
        let mtime = match std::fs::metadata(path).and_then(|m| m.modified()) {
            Ok(mtime) => mtime,
            Err(e) => return self.skip(path, SkipReason::Io(e.to_string())),
        };
        self.results.truncated |= capped;
        let rel = self.rel(path);
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.results.hits.extend(matches.into_iter().map(|m| Hit {
            path: rel.clone(),
            name: name.clone(),
            mtime,
            line_number: m.line_number,
            match_text: m.match_text,
            context: m.context,
        }));
    }
}
