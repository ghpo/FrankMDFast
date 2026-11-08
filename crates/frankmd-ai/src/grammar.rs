//! GUI-free state of the grammar check (ui-spec §5.10; FE-439…451).
//!
//! Flow: [`GrammarState::start`] validates (open markdown note, non-blank
//! text, AI configured, never `.fed`), then asks the app to flush the pending
//! autosave, lock the note read-only, show the "Checking grammar…" banner and
//! send the saved note. [`GrammarState::finish`] (or [`GrammarState::cancel`])
//! undoes the lock and banner; a success opens the side-by-side word diff.
//!
//! The diff is FrankMD's word LCS (`frankmd_core::text::diff`). Each run of
//! deletions/insertions between equal tokens is one [`Hunk`], accepted by
//! default (so "Apply" without touching anything is FrankMD's "Accept
//! Changes"); rejected hunks keep the original words. The Edit toggle swaps
//! in a free-text editor; once edited, that text is what Apply writes.

use frankmd_core::text::diff::{DiffKind, compute_word_diff};

use crate::{ProviderInfo, is_ai_note_path};

fn availability_from_config(info: &ProviderInfo) -> Availability {
    match (&info.provider, &info.model) {
        (Some(provider), Some(model)) if info.enabled => Availability::Available {
            provider: provider.as_str().to_owned(),
            model: model.clone(),
        },
        _ => Availability::NotConfigured,
    }
}

/// Whether grammar check can run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Availability {
    /// Config not read yet.
    #[default]
    Unknown,
    /// No provider configured: the dialog shows the setup notice.
    NotConfigured,
    /// Ready.
    Available {
        /// Provider id (`anthropic`, …).
        provider: String,
        /// Model.
        model: String,
    },
}

/// Why a check did not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    /// `errors.no_file_open`.
    NoFileOpen,
    /// `errors.no_text_to_check`.
    NoText,
    /// Only markdown notes are sent (security: never `.fed`).
    NotANote,
    /// AI not configured: show the setup notice.
    NotConfigured,
    /// A check is already running.
    Busy,
}

impl StartError {
    /// The locale key shown (status line), if any.
    pub fn key(self) -> Option<&'static str> {
        match self {
            Self::NoFileOpen => Some("errors.no_file_open"),
            Self::NoText => Some("errors.no_text_to_check"),
            Self::NotANote => Some("palette.unavailable.open_markdown_note"),
            Self::NotConfigured | Self::Busy => None,
        }
    }
}

/// Work for the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrammarEffect {
    /// Save the pending autosave now (the provider reads the file on disk).
    FlushAutosave,
    /// Make the note read-only (`true`) or editable again.
    SetReadOnly(bool),
    /// Show the "Checking grammar with <provider> · <model>" banner.
    ShowBanner {
        /// Provider.
        provider: String,
        /// Model.
        model: String,
    },
    /// Remove the banner.
    HideBanner,
    /// Run `fix_grammar_note(path)` → [`GrammarState::finish`].
    Request {
        /// Request id.
        id: u64,
        /// The note's vault path.
        path: String,
    },
    /// Abort the request.
    Abort,
    /// Open the result dialog.
    ShowDialog,
}

/// One change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// Original text (red strikethrough).
    pub deleted: String,
    /// Corrected text (green).
    pub inserted: String,
    /// Accepted (default) or rejected.
    pub accepted: bool,
}

/// A piece of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Unchanged text.
    Equal(String),
    /// Hunk by index.
    Change(usize),
}

/// Diff or free-text edit of the right side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// Side-by-side diff.
    #[default]
    Diff,
    /// Editable corrected text.
    Edit,
}

/// The finished result.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrammarResult {
    /// The note text that was checked.
    pub original: String,
    /// The provider's corrected text.
    pub corrected: String,
    /// Provider used.
    pub provider: String,
    /// Model used.
    pub model: String,
}

/// The grammar check state of one window.
#[derive(Debug, Clone, Default)]
pub struct GrammarState {
    availability: Availability,
    request: Option<u64>,
    next_id: u64,
    result: Option<GrammarResult>,
    segments: Vec<Segment>,
    hunks: Vec<Hunk>,
    focused: usize,
    view: View,
    edited: Option<String>,
    error: Option<String>,
    path: Option<String>,
}

impl GrammarState {
    /// Availability from the AI config (`GET /ai/config`, read once).
    pub fn from_config(info: &ProviderInfo) -> Self {
        Self {
            availability: availability_from_config(info),
            ..Self::default()
        }
    }

    /// Update availability without clearing an open diff/error.
    pub fn set_availability(&mut self, info: &ProviderInfo) {
        self.availability = availability_from_config(info);
    }

    /// Availability.
    pub fn availability(&self) -> &Availability {
        &self.availability
    }

    /// A request is in flight.
    pub fn in_flight(&self) -> bool {
        self.request.is_some()
    }

    /// Start a check of `path` whose editor text is `text`.
    pub fn start(
        &mut self,
        path: Option<&str>,
        text: &str,
    ) -> Result<Vec<GrammarEffect>, StartError> {
        if self.request.is_some() {
            return Err(StartError::Busy);
        }
        let path = path.ok_or(StartError::NoFileOpen)?;
        if text.trim().is_empty() {
            return Err(StartError::NoText);
        }
        if !is_ai_note_path(&frankmd_core::path::normalize_note_path(path)) {
            return Err(StartError::NotANote);
        }
        let Availability::Available { provider, model } = self.availability.clone() else {
            self.error = None;
            self.result = None;
            return Err(StartError::NotConfigured);
        };
        self.next_id += 1;
        self.request = Some(self.next_id);
        self.path = Some(path.to_owned());
        self.error = None;
        Ok(vec![
            GrammarEffect::FlushAutosave,
            GrammarEffect::SetReadOnly(true),
            GrammarEffect::ShowBanner { provider, model },
            GrammarEffect::Request {
                id: self.next_id,
                path: path.to_owned(),
            },
        ])
    }

    /// Retry the last check after an error.
    pub fn retry(&mut self, text: &str) -> Result<Vec<GrammarEffect>, StartError> {
        let path = self.path.clone();
        self.start(path.as_deref(), text)
    }

    /// Esc / Cancel on the banner: abort silently (no-op when idle).
    pub fn cancel(&mut self) -> Vec<GrammarEffect> {
        if self.request.take().is_none() {
            return Vec::new();
        }
        vec![
            GrammarEffect::Abort,
            GrammarEffect::SetReadOnly(false),
            GrammarEffect::HideBanner,
        ]
    }

    /// The request finished; a stale `id` is ignored. Errors show inline in
    /// the dialog with Retry (ui-spec §5.10).
    pub fn finish(&mut self, id: u64, result: Result<GrammarResult, String>) -> Vec<GrammarEffect> {
        if self.request != Some(id) {
            return Vec::new();
        }
        self.request = None;
        let mut effects = vec![GrammarEffect::SetReadOnly(false), GrammarEffect::HideBanner];
        match result {
            Ok(result) => {
                self.load(result);
                effects.push(GrammarEffect::ShowDialog);
            }
            Err(error) => {
                self.result = None;
                self.error = Some(error);
                effects.push(GrammarEffect::ShowDialog);
            }
        }
        effects
    }

    fn load(&mut self, result: GrammarResult) {
        let mut segments = Vec::new();
        let mut hunks: Vec<Hunk> = Vec::new();
        let mut open = false;
        for op in compute_word_diff(&result.original, &result.corrected) {
            match op.kind {
                DiffKind::Equal => {
                    open = false;
                    segments.push(Segment::Equal(op.value));
                }
                DiffKind::Delete | DiffKind::Insert => {
                    if !open {
                        segments.push(Segment::Change(hunks.len()));
                        hunks.push(Hunk {
                            deleted: String::new(),
                            inserted: String::new(),
                            accepted: true,
                        });
                        open = true;
                    }
                    if let Some(hunk) = hunks.last_mut() {
                        if op.kind == DiffKind::Delete {
                            hunk.deleted.push_str(&op.value);
                        } else {
                            hunk.inserted.push_str(&op.value);
                        }
                    }
                }
            }
        }
        self.segments = segments;
        self.hunks = hunks;
        self.focused = 0;
        self.view = View::Diff;
        self.edited = None;
        self.error = None;
        self.result = Some(result);
    }

    /// The result, when the dialog shows one.
    pub fn result(&self) -> Option<&GrammarResult> {
        self.result.as_ref()
    }

    /// The error, when the dialog shows one.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Diff segments.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Hunks.
    pub fn hunks(&self) -> &[Hunk] {
        &self.hunks
    }

    /// `(focused 1-based, total)` for "Change 3 of 17".
    pub fn counter(&self) -> (usize, usize) {
        (
            if self.hunks.is_empty() {
                0
            } else {
                self.focused + 1
            },
            self.hunks.len(),
        )
    }

    /// The focused hunk.
    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Alt+↓ / Alt+↑ (wraps).
    pub fn focus_step(&mut self, forward: bool) {
        let n = self.hunks.len();
        if n == 0 {
            return;
        }
        self.focused = if forward {
            (self.focused + 1) % n
        } else {
            (self.focused + n - 1) % n
        };
    }

    /// Focus a hunk.
    pub fn focus(&mut self, index: usize) {
        if index < self.hunks.len() {
            self.focused = index;
        }
    }

    /// ✓ / ✕ (Y / N on the focused hunk). Ignored once the text was edited.
    pub fn set_accepted(&mut self, index: usize, accepted: bool) {
        if self.edited.is_some() {
            return;
        }
        if let Some(hunk) = self.hunks.get_mut(index) {
            hunk.accepted = accepted;
        }
    }

    /// The corrected text with rejected hunks reverted.
    pub fn merged_text(&self) -> String {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Equal(text) => out.push_str(text),
                Segment::Change(i) => {
                    if let Some(h) = self.hunks.get(*i) {
                        out.push_str(if h.accepted { &h.inserted } else { &h.deleted });
                    }
                }
            }
        }
        out
    }

    /// Current view.
    pub fn view(&self) -> View {
        self.view
    }

    /// Alt+E / Edit ↔ Preview. Entering Edit prefills the merged text; going
    /// back keeps the edits (the diff is not recomputed, FE-450).
    pub fn toggle_edit(&mut self) {
        self.view = match self.view {
            View::Diff => {
                if self.edited.is_none() {
                    self.edited = Some(self.merged_text());
                }
                View::Edit
            }
            View::Edit => View::Diff,
        };
    }

    /// Label key of the toggle button (`common.edit` / `preview.title`).
    pub fn toggle_label_key(&self) -> &'static str {
        match self.view {
            View::Diff => "common.edit",
            View::Edit => "preview.title",
        }
    }

    /// The edit area's text.
    pub fn edited(&self) -> Option<&str> {
        self.edited.as_deref()
    }

    /// The user typed in the edit area.
    pub fn set_edited(&mut self, text: &str) {
        self.edited = Some(text.to_owned());
    }

    /// The text a cancel would protect (for backdrop clicks: edits exist).
    pub fn has_edits(&self) -> bool {
        self.edited.is_some() || self.hunks.iter().any(|h| !h.accepted)
    }

    /// Apply (Ctrl+Enter): the new document text (one undoable edit); closes.
    pub fn apply(&mut self) -> Option<String> {
        self.result.as_ref()?;
        let text = self.edited.clone().unwrap_or_else(|| self.merged_text());
        self.close();
        Some(text)
    }

    /// Cancel / Esc on the dialog: discard.
    pub fn close(&mut self) {
        self.result = None;
        self.error = None;
        self.segments.clear();
        self.hunks.clear();
        self.edited = None;
        self.view = View::Diff;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AiProviderId;

    fn info(provider: AiProviderId, model: &str) -> ProviderInfo {
        ProviderInfo {
            enabled: true,
            provider: Some(provider),
            model: Some(model.to_owned()),
            available_providers: vec![provider],
        }
    }

    #[test]
    fn availability_reload_preserves_an_open_diff() {
        let mut state = GrammarState::from_config(&info(AiProviderId::Deepseek, "deepseek-chat"));
        state.result = Some(GrammarResult {
            original: "Teh".into(),
            corrected: "The".into(),
            provider: "deepseek".into(),
            model: "deepseek-chat".into(),
        });
        state.set_availability(&info(AiProviderId::Openrouter, "openai/gpt-4o-mini"));
        assert!(state.result().is_some(), "reload must not wipe a diff");
        assert_eq!(
            state.result().map(|result| result.corrected.as_str()),
            Some("The")
        );
    }
}
