//! The per-editor autosave + local-draft state machine
//! (**[COMPAT]** port of `autosave_controller.js`, keyed by [`DocId`]).
//!
//! Lifecycle, timers, saving and draft writes live here; recovery and the
//! content-loss guard are in [`recovery`].

mod recovery;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use frankmd_core::DocId;
use frankmd_core::guard::is_large_deletion;
use frankmd_core::registry::Remap;

use super::{
    AutosaveConfig, AutosaveEffect, DeletionPrep, SaveOutcome, SaveRequest, SaveStatus, SaveTicket,
};
use crate::draft_storage::{
    DraftBackend, DraftError, DraftErrorKind, DraftRecord, DraftStorage, ItemKind,
    RemoveDraftsError,
};
use crate::drafts::draft_subject;

/// The note the editor shows.
#[derive(Debug, Clone)]
struct ActiveDoc {
    doc: DocId,
    path: String,
}

/// What a debounced or immediate save captured (`captureSaveSnapshot`).
#[derive(Debug, Clone)]
struct SaveSnapshot {
    doc: DocId,
    file_version: u64,
    content: String,
    base_revision: Option<String>,
}

/// The save request currently in flight (at most one, `_isSaving`).
#[derive(Debug, Clone)]
struct InFlightSave {
    ticket: SaveTicket,
    snapshot: SaveSnapshot,
    /// Draft written for exactly this snapshot; removed only on success.
    saved_draft_revision: Option<String>,
    is_config: bool,
}

/// A debounced draft write (`_draftWriteTimeouts`), independent per document.
#[derive(Debug, Clone)]
struct PendingDraftWrite {
    deadline: Instant,
    content: String,
    base_revision: String,
}

/// A debounced legacy offline backup.
#[derive(Debug, Clone)]
struct PendingBackup {
    deadline: Instant,
    doc: DocId,
    content: String,
}

/// An open recovery dialog (`_pendingRecovery`): autosave and draft writes
/// for `doc` pause until it is resolved.
#[derive(Debug, Clone)]
struct PendingRecovery {
    doc: DocId,
}

/// `_contentLossState`: bound to one document *and* server baseline.
#[derive(Debug, Clone, Default)]
struct ContentLossState {
    doc: Option<DocId>,
    base_revision: Option<String>,
    warning_active: bool,
    override_active: bool,
    override_content: Option<String>,
}

/// The timer that fires next in [`AutosaveSession::tick`].
#[derive(Debug, Clone, Copy)]
enum Timer {
    DraftWrite(DocId),
    SaveDebounce,
    SaveMaxInterval,
    OfflineBackup,
    StatusClear,
}

/// Autosave + local-draft state machine for one editor.
pub struct AutosaveSession<B: DraftBackend> {
    drafts: DraftStorage<B>,
    config: AutosaveConfig,
    effects: Vec<AutosaveEffect>,
    editor_text: String,
    active: Option<ActiveDoc>,
    last_saved_content: Option<String>,
    base_revision: Option<String>,
    draft_revision: Option<String>,
    has_unsaved_changes: bool,
    is_offline: bool,
    /// The active note is encrypted: never write plaintext drafts/backups.
    encrypted: bool,
    /// Bumped whenever the active note is (re)loaded, cleared or deleted, so
    /// late responses for an earlier incarnation are recognised as stale.
    file_version: u64,
    next_ticket: u64,
    in_flight: Option<InFlightSave>,
    save_deadline: Option<Instant>,
    max_interval_deadline: Option<Instant>,
    scheduled_save: Option<SaveSnapshot>,
    offline_backup: Option<PendingBackup>,
    draft_writes: BTreeMap<DocId, PendingDraftWrite>,
    known_base_revisions: BTreeMap<DocId, String>,
    draft_persistence_failures: BTreeSet<DocId>,
    pending_recovery: Option<PendingRecovery>,
    content_loss: ContentLossState,
    draft_storage_error_visible: bool,
    status: SaveStatus,
    status_clear_deadline: Option<Instant>,
    unload_guard: bool,
}

impl<B: DraftBackend> std::fmt::Debug for AutosaveSession<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutosaveSession")
            .field("doc", &self.current_doc())
            .field("dirty", &self.has_unsaved_changes)
            .finish_non_exhaustive()
    }
}

fn storage_error(message: &str) -> DraftError {
    DraftError {
        kind: DraftErrorKind::Storage,
        message: message.to_owned(),
    }
}

impl<B: DraftBackend> AutosaveSession<B> {
    /// New session with no file.
    pub fn new(drafts: DraftStorage<B>, config: AutosaveConfig) -> Self {
        Self {
            drafts,
            config,
            effects: Vec::new(),
            editor_text: String::new(),
            active: None,
            last_saved_content: None,
            base_revision: None,
            draft_revision: None,
            has_unsaved_changes: false,
            is_offline: false,
            encrypted: false,
            file_version: 0,
            next_ticket: 0,
            in_flight: None,
            save_deadline: None,
            max_interval_deadline: None,
            scheduled_save: None,
            offline_backup: None,
            draft_writes: BTreeMap::new(),
            known_base_revisions: BTreeMap::new(),
            draft_persistence_failures: BTreeSet::new(),
            pending_recovery: None,
            content_loss: ContentLossState::default(),
            draft_storage_error_visible: false,
            status: SaveStatus::Hidden,
            status_clear_deadline: None,
            unload_guard: false,
        }
    }

    // ----- effects / drafts access -------------------------------------

    /// Drain queued effects.
    pub fn take_effects(&mut self) -> Vec<AutosaveEffect> {
        std::mem::take(&mut self.effects)
    }
    /// Draft store.
    pub fn drafts(&self) -> &DraftStorage<B> {
        &self.drafts
    }
    /// Mutable draft store (tests seed records and faults).
    pub fn drafts_mut(&mut self) -> &mut DraftStorage<B> {
        &mut self.drafts
    }

    // ----- editor mirror -------------------------------------------------

    /// Mirror the editor text without scheduling anything (programmatic change).
    pub fn set_editor_text(&mut self, text: &str) {
        text.clone_into(&mut self.editor_text);
    }
    /// Current mirrored editor text.
    pub fn editor_text(&self) -> &str {
        &self.editor_text
    }
    /// A user edit: mirror + [`Self::schedule_autosave`].
    pub fn editor_changed(&mut self, text: &str, now: Instant) {
        self.set_editor_text(text);
        self.schedule_autosave(now);
    }

    // ----- file lifecycle ------------------------------------------------

    /// `setFile(path, content, revision)`: `doc` was loaded from `path`
    /// (resets dirty, warning state and draft revision; remembers the server
    /// revision). Timers armed for the previous note stay armed but can no
    /// longer save it (their snapshot is stale).
    /// Mark the active note as encrypted (drafts are skipped).
    pub fn set_encrypted(&mut self, encrypted: bool) {
        self.encrypted = encrypted;
    }

    /// Load the active file as the autosave base.
    pub fn set_file(&mut self, doc: DocId, path: &str, content: &str, revision: Option<&str>) {
        self.dismiss_content_loss_warning();
        self.active = Some(ActiveDoc {
            doc,
            path: path.to_owned(),
        });
        self.last_saved_content = Some(content.to_owned());
        self.base_revision = revision.filter(|r| !r.is_empty()).map(str::to_owned);
        self.content_loss = ContentLossState {
            doc: Some(doc),
            base_revision: self.base_revision.clone(),
            ..ContentLossState::default()
        };
        self.draft_revision = None;
        if let Some(base) = &self.base_revision {
            self.known_base_revisions.insert(doc, base.clone());
        }
        self.has_unsaved_changes = false;
        self.file_version += 1;
        self.set_status(SaveStatus::Hidden);
        self.update_unload_guard();
    }

    /// `clearFile()`: no note is open any more.
    pub fn clear_file(&mut self) {
        self.clear_pending_timers();
        if let Some(doc) = self.current_doc() {
            self.draft_writes.remove(&doc);
        }
        self.invalidate_active();
    }

    /// `prepareForTransition()`: flush the outgoing draft synchronously, then
    /// cancel timers. On failure the file stays active and timers keep running;
    /// a dirty file without a server revision cannot be anchored → error.
    pub fn prepare_for_transition(&mut self) -> Result<Option<DraftRecord>, DraftError> {
        let Some(doc) = self.current_doc() else {
            self.clear_pending_timers();
            return Ok(None);
        };
        let content = self.editor_text.clone();
        if self.last_saved_content.as_deref() != Some(content.as_str())
            && self.base_revision.is_none()
        {
            let error =
                storage_error("Cannot persist the outgoing draft without a server revision");
            self.draft_persistence_failures.insert(doc);
            self.update_unload_guard();
            self.show_draft_storage_error();
            return Err(error);
        }
        let base = self.base_revision.clone();
        let flushed = self.flush_draft_write(doc, &content, base.as_deref())?;
        // A transition must never let an outgoing save or backup timer inspect
        // the newly active editor: flush the draft first, then cancel timers.
        self.clear_pending_timers();
        Ok(flushed)
    }

    /// `prepareForFileDeletion(path)` (first step) for the active note `doc`.
    pub fn prepare_for_file_deletion(&mut self, doc: DocId) -> DeletionPrep {
        if self.current_doc() != Some(doc) {
            return DeletionPrep::Stale;
        }
        if let Err(error) = self.prepare_for_transition() {
            return DeletionPrep::Failed(error);
        }
        self.resume_file_deletion(doc)
    }

    /// Continue a [`DeletionPrep::AwaitSave`] after the in-flight save finished.
    pub fn resume_file_deletion(&mut self, doc: DocId) -> DeletionPrep {
        if self.current_doc() != Some(doc) {
            return DeletionPrep::Stale;
        }
        if self.in_flight.is_some() {
            return DeletionPrep::AwaitSave;
        }
        // The finished save may have scheduled a follow-up; the note is about
        // to be deleted, so stop it and use the latest acknowledged revision.
        self.clear_pending_timers();
        match self
            .known_base_revisions
            .get(&doc)
            .or(self.base_revision.as_ref())
        {
            Some(revision) => DeletionPrep::Ready {
                revision: revision.clone(),
            },
            None => DeletionPrep::Failed(storage_error(
                "Cannot delete the note without a known server revision",
            )),
        }
    }

    /// Paths changed under live documents (rename/move of a file or folder,
    /// from `DocRegistry::rename`). Only the active note's path is updated:
    /// drafts, timers and pending saves are keyed by DocId and stay attached,
    /// dirty state is kept. Returns whether the active note moved.
    pub fn paths_changed(&mut self, remaps: &[Remap]) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        match remaps.iter().find(|r| r.id == active.doc) {
            Some(remap) => {
                remap.to.as_str().clone_into(&mut active.path);
                true
            }
            None => false,
        }
    }

    /// `deleteFile(path, kind)` for the documents `DocRegistry::remove`
    /// returned: drop their drafts, legacy backups and pending writes, and
    /// invalidate the active note if it is among them (even when cleanup
    /// fails). `Ok(removed)` counts the removed records.
    pub fn docs_deleted(&mut self, docs: &[DocId]) -> Result<usize, RemoveDraftsError> {
        let mut removed = 0;
        let mut first_error = None;
        for &doc in docs {
            self.draft_writes.remove(&doc);
            self.known_base_revisions.remove(&doc);
            self.draft_persistence_failures.remove(&doc);
            if self.offline_backup.as_ref().is_some_and(|b| b.doc == doc) {
                self.offline_backup = None;
            }
            if self.pending_recovery.as_ref().is_some_and(|p| p.doc == doc) {
                self.pending_recovery = None;
            }
            match self
                .drafts
                .remove_drafts(&draft_subject(doc), ItemKind::File)
            {
                Ok(n) => removed += n,
                Err(e) => {
                    removed += e.removed.unwrap_or(0);
                    first_error.get_or_insert(e.error);
                }
            }
        }
        if self.current_doc().is_some_and(|doc| docs.contains(&doc)) {
            self.clear_pending_timers();
            self.invalidate_active();
        }
        match first_error {
            Some(error) => {
                self.show_draft_storage_error();
                Err(RemoveDraftsError {
                    error,
                    removed: Some(removed),
                })
            }
            None => Ok(removed),
        }
    }

    /// `resumeAfterTransition()`: a rejected rename/delete lets the active file resume.
    pub fn resume_after_transition(&mut self, now: Instant) {
        if self.active.is_some() && self.has_unsaved_changes {
            self.schedule_autosave(now);
        }
    }

    fn invalidate_active(&mut self) {
        self.active = None;
        self.last_saved_content = None;
        self.base_revision = None;
        self.draft_revision = None;
        self.has_unsaved_changes = false;
        self.file_version += 1;
        self.dismiss_content_loss_warning();
        self.set_status(SaveStatus::Hidden);
        self.update_unload_guard();
    }

    // ----- timers --------------------------------------------------------

    /// `scheduleAutoSave()`: on every editor change.
    pub fn schedule_autosave(&mut self, now: Instant) {
        let Some(doc) = self.current_doc() else {
            return;
        };
        if self.has_pending_recovery(doc) {
            return;
        }
        if self.last_saved_content.as_deref() == Some(self.editor_text.as_str()) {
            self.clear_pending_timers();
            self.schedule_draft_write(now);
            self.has_unsaved_changes = false;
            self.set_status(SaveStatus::Hidden);
            return;
        }
        self.schedule_draft_write(now);
        if self.is_offline || self.content_loss_warning_active() {
            self.has_unsaved_changes = true;
            return;
        }
        if !self.has_unsaved_changes {
            self.has_unsaved_changes = true;
            self.set_status(SaveStatus::Unsaved);
        }
        self.scheduled_save = Some(self.capture_save_snapshot(doc));
        self.save_deadline = Some(now + self.config.save_debounce);
        if self.max_interval_deadline.is_none() {
            self.max_interval_deadline = Some(now + self.config.save_max_interval);
        }
    }

    /// `scheduleDraftWrite()`: debounce a local draft write for the current file.
    pub fn schedule_draft_write(&mut self, now: Instant) {
        let (Some(doc), Some(base)) = (self.current_doc(), self.base_revision.clone()) else {
            return;
        };
        if self.has_pending_recovery(doc) {
            return;
        }
        if self.last_saved_content.as_deref() == Some(self.editor_text.as_str()) {
            self.draft_writes.remove(&doc);
            if let Some(revision) = self.draft_revision.clone() {
                let _ = self.remove_draft_if_revision(doc, &revision);
            }
            return;
        }
        let write = PendingDraftWrite {
            deadline: now + self.config.draft_debounce,
            content: self.editor_text.clone(),
            base_revision: base,
        };
        self.draft_writes.insert(doc, write);
    }

    /// `scheduleOfflineBackup()`: only while offline.
    pub fn schedule_offline_backup(&mut self, now: Instant) {
        let Some(doc) = self.current_doc().filter(|_| self.is_offline) else {
            return;
        };
        self.offline_backup = Some(PendingBackup {
            deadline: now + self.config.offline_backup_debounce,
            doc,
            content: self.editor_text.clone(),
        });
    }

    /// Fire every timer due at `now` (draft writes, save debounce, max
    /// interval, offline backup, status linger), earliest first.
    pub fn tick(&mut self, now: Instant) {
        while let Some((deadline, timer)) = self.next_timer() {
            if deadline > now {
                break;
            }
            self.fire(timer);
        }
    }

    /// Earliest pending deadline (the app arms one timer for it).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.next_timer().map(|(deadline, _)| deadline)
    }

    fn next_timer(&self) -> Option<(Instant, Timer)> {
        let drafts = self
            .draft_writes
            .iter()
            .map(|(doc, w)| (w.deadline, Timer::DraftWrite(*doc)));
        let others = [
            self.save_deadline.map(|d| (d, Timer::SaveDebounce)),
            self.max_interval_deadline
                .map(|d| (d, Timer::SaveMaxInterval)),
            self.offline_backup
                .as_ref()
                .map(|b| (b.deadline, Timer::OfflineBackup)),
            self.status_clear_deadline.map(|d| (d, Timer::StatusClear)),
        ];
        drafts
            .chain(others.into_iter().flatten())
            .min_by_key(|(deadline, _)| *deadline)
    }

    fn fire(&mut self, timer: Timer) {
        match timer {
            Timer::DraftWrite(doc) => {
                if let Some(write) = self.draft_writes.remove(&doc) {
                    let _ = self.write_draft_snapshot(doc, &write.content, &write.base_revision);
                }
            }
            Timer::SaveDebounce => {
                self.save_deadline = None;
                if let Some(snapshot) = self
                    .scheduled_save
                    .clone()
                    .filter(|s| self.is_current_snapshot(s))
                {
                    self.save(Some(snapshot));
                }
            }
            Timer::SaveMaxInterval => {
                self.max_interval_deadline = None;
                let snapshot = self
                    .scheduled_save
                    .clone()
                    .filter(|s| self.is_current_snapshot(s));
                if let Some(snapshot) = snapshot.filter(|_| self.has_unsaved_changes) {
                    self.save(Some(snapshot));
                }
            }
            Timer::OfflineBackup => {
                if let Some(backup) = self.offline_backup.take() {
                    // offline_backup_controller: a failed backup is only a warning.
                    if !self.encrypted {
                        let _ = self
                            .drafts
                            .backup_save(&draft_subject(backup.doc), &backup.content);
                    }
                }
            }
            Timer::StatusClear => {
                self.status_clear_deadline = None;
                if self.status == SaveStatus::Saved {
                    self.set_status(SaveStatus::Hidden);
                }
            }
        }
    }

    fn clear_pending_timers(&mut self) {
        self.scheduled_save = None;
        self.save_deadline = None;
        self.max_interval_deadline = None;
        self.offline_backup = None;
    }

    // ----- saving --------------------------------------------------------

    /// `saveNow()`: emits [`AutosaveEffect::Save`] unless offline, recovering,
    /// unchanged, already saving, or a large deletion (→ banner + draft).
    pub fn save_now(&mut self) {
        self.save(None);
    }

    fn save(&mut self, snapshot: Option<SaveSnapshot>) {
        if self
            .current_doc()
            .is_some_and(|doc| self.has_pending_recovery(doc))
        {
            return;
        }
        if self.is_offline {
            self.has_unsaved_changes = true;
            return;
        }
        let Some(doc) = self.current_doc() else {
            return;
        };
        if snapshot
            .as_ref()
            .is_some_and(|s| !self.is_current_snapshot(s))
            || self.in_flight.is_some()
        {
            return;
        }
        let snapshot = snapshot.unwrap_or_else(|| self.capture_save_snapshot(doc));
        self.clear_pending_timers();

        let content = snapshot.content.clone();
        let saved_draft = self.flush_draft_write(doc, &content, snapshot.base_revision.as_deref());
        let saved_draft_revision = saved_draft
            .as_ref()
            .ok()
            .and_then(|d| d.as_ref())
            .map(|d| d.draft_revision.clone());

        if self.last_saved_content.as_deref() == Some(content.as_str()) {
            self.has_unsaved_changes = false;
            self.set_status(SaveStatus::Hidden);
            return;
        }
        let approved = self.content_loss_override()
            && self.content_loss.override_content.as_deref() == Some(&content);
        if !approved && is_large_deletion(self.last_saved_content.as_deref(), &content) {
            if saved_draft.is_ok() {
                self.show_content_loss_warning();
            }
            return;
        }

        let path = self.current_file().unwrap_or_default().to_owned();
        self.next_ticket += 1;
        let ticket = SaveTicket(self.next_ticket);
        let request = SaveRequest {
            ticket,
            doc,
            path: path.clone(),
            content,
            base_revision: snapshot.base_revision.clone(),
        };
        self.in_flight = Some(InFlightSave {
            ticket,
            snapshot,
            saved_draft_revision,
            is_config: path == ".fed",
        });
        self.effects.push(AutosaveEffect::Save(request));
    }

    /// Report the outcome of a save request. Responses for an earlier
    /// incarnation of the note (switched away, reloaded or deleted since)
    /// never touch the active state; a rename does not make them stale.
    pub fn save_finished(&mut self, ticket: SaveTicket, outcome: SaveOutcome, now: Instant) {
        let Some(save) = self.in_flight.take_if(|s| s.ticket == ticket) else {
            return;
        };
        let doc = save.snapshot.doc;
        let content = save.snapshot.content.clone();
        let revision = match outcome {
            SaveOutcome::Saved { revision } => revision,
            SaveOutcome::Failed => {
                if self.is_current_snapshot(&save.snapshot) {
                    self.set_status(SaveStatus::ErrorSaving);
                } else if self.active.is_some() && self.has_unsaved_changes {
                    self.schedule_autosave(now);
                }
                return;
            }
        };
        if let Some(revision) = &revision {
            self.known_base_revisions.insert(doc, revision.clone());
        }
        if !self.is_current_snapshot(&save.snapshot) {
            if self.active.is_some() && self.has_unsaved_changes {
                self.schedule_autosave(now);
            }
            return;
        }

        // Remove only the snapshot captured for this request; a newer edit may
        // already have replaced it while the request was in flight.
        if let Some(draft_revision) = &save.saved_draft_revision {
            let _ = self.remove_draft_if_revision(doc, draft_revision);
        }
        if let Some(revision) = &revision {
            self.rebase_newer_draft(doc, &content, revision);
        }

        self.last_saved_content = Some(content.clone());
        if let Some(revision) = revision {
            self.base_revision = Some(revision.clone());
            self.content_loss = ContentLossState {
                doc: Some(doc),
                base_revision: Some(revision),
                ..ContentLossState::default()
            };
        }
        self.set_content_loss_override(false);
        self.has_unsaved_changes = false;
        // offline_backup_controller#clear: a failure is only a warning.
        let _ = self.drafts.remove_backup(&draft_subject(doc));
        self.set_status(SaveStatus::Saved);
        self.status_clear_deadline = Some(now + self.config.saved_status_linger);
        if save.is_config {
            self.effects.push(AutosaveEffect::ConfigSaved);
        }

        if self.editor_text != content {
            self.has_unsaved_changes = true;
            if let Some(base) = self.base_revision.clone() {
                let fresh = self.editor_text.clone();
                let _ = self.write_draft_snapshot(doc, &fresh, &base);
            }
            if !self.is_offline {
                self.schedule_autosave(now);
            }
        }
    }

    /// A draft newer than the saved snapshot moves onto the new server baseline.
    fn rebase_newer_draft(&mut self, doc: DocId, saved_content: &str, new_revision: &str) {
        let subject = draft_subject(doc);
        let latest = match self.drafts.read_draft(&subject) {
            Ok(read) => read.draft,
            Err(_) => {
                if self.current_doc() == Some(doc) {
                    self.show_draft_storage_error();
                }
                return;
            }
        };
        let Some(latest) =
            latest.filter(|d| d.content != saved_content && d.base_revision != new_revision)
        else {
            return;
        };
        match self
            .drafts
            .write_draft(&subject, &latest.content, new_revision)
        {
            Ok(written) if self.current_doc() == Some(doc) => {
                self.draft_revision = Some(written.draft.draft_revision)
            }
            Ok(_) => {}
            Err(_) => {
                if self.current_doc() == Some(doc) {
                    self.show_draft_storage_error();
                }
            }
        }
    }

    fn capture_save_snapshot(&self, doc: DocId) -> SaveSnapshot {
        SaveSnapshot {
            doc,
            file_version: self.file_version,
            content: self.editor_text.clone(),
            base_revision: self.base_revision.clone(),
        }
    }

    fn is_current_snapshot(&self, snapshot: &SaveSnapshot) -> bool {
        self.current_doc() == Some(snapshot.doc) && snapshot.file_version == self.file_version
    }

    // ----- drafts --------------------------------------------------------

    /// `flushDraftWrite(path, content, baseRevision)` for `doc`: write (or,
    /// when the content equals the server text, drop) its draft now.
    pub fn flush_draft_write(
        &mut self,
        doc: DocId,
        content: &str,
        base_revision: Option<&str>,
    ) -> Result<Option<DraftRecord>, DraftError> {
        let Some(base_revision) = base_revision.filter(|b| !b.is_empty()) else {
            return Ok(None);
        };
        if self.has_pending_recovery(doc) {
            return Ok(None);
        }
        self.draft_writes.remove(&doc);

        if self.last_saved_content.as_deref() == Some(content) && self.current_doc() == Some(doc) {
            self.draft_persistence_failures.remove(&doc);
            self.update_unload_guard();
            if let Some(revision) = self.draft_revision.clone() {
                match self
                    .drafts
                    .remove_draft_if_revision(&draft_subject(doc), &revision)
                {
                    Err(error) => {
                        self.show_draft_storage_error();
                        return Err(error);
                    }
                    Ok(_) => {
                        self.draft_revision = None;
                        self.clear_draft_storage_error();
                    }
                }
                return Ok(None);
            }
            self.clear_draft_storage_error();
            return Ok(None);
        }
        self.write_draft_snapshot(doc, content, base_revision)
    }

    /// `writeDraftSnapshot({path, content, baseRevision})`. The latest
    /// acknowledged server revision wins over the captured baseline.
    fn write_draft_snapshot(
        &mut self,
        doc: DocId,
        content: &str,
        base_revision: &str,
    ) -> Result<Option<DraftRecord>, DraftError> {
        if self.encrypted || self.has_pending_recovery(doc) {
            return Ok(None);
        }
        let base = self
            .known_base_revisions
            .get(&doc)
            .cloned()
            .unwrap_or_else(|| base_revision.to_owned());
        let is_current = self.current_doc() == Some(doc);
        match self.drafts.write_draft(&draft_subject(doc), content, &base) {
            Err(error) => {
                self.draft_persistence_failures.insert(doc);
                self.update_unload_guard();
                if is_current {
                    self.show_draft_storage_error();
                }
                Err(error)
            }
            Ok(written) => {
                self.draft_persistence_failures.remove(&doc);
                self.update_unload_guard();
                if is_current {
                    self.draft_revision = Some(written.draft.draft_revision.clone());
                    self.clear_draft_storage_error();
                }
                Ok(Some(written.draft))
            }
        }
    }

    /// `flushActiveDraft()`: window hidden / app losing focus.
    pub fn flush_active_draft(&mut self) -> Result<Option<DraftRecord>, DraftError> {
        let Some(doc) = self.current_doc() else {
            return Ok(None);
        };
        let content = self.editor_text.clone();
        let base = self.base_revision.clone();
        self.flush_draft_write(doc, &content, base.as_deref())
    }

    /// `beforeunload`/window close: flush, and return `true` when the close
    /// must be warned about (the active draft could not be persisted).
    pub fn before_unload(&mut self) -> bool {
        let flushed = self.flush_active_draft();
        flushed.is_err()
            && self
                .current_doc()
                .is_some_and(|doc| self.draft_persistence_failures.contains(&doc))
    }

    fn remove_draft_if_revision(&mut self, doc: DocId, revision: &str) -> Result<bool, DraftError> {
        let result = self
            .drafts
            .remove_draft_if_revision(&draft_subject(doc), revision);
        let is_current = self.current_doc() == Some(doc);
        match &result {
            Err(_) if is_current => self.show_draft_storage_error(),
            Ok(true) if is_current && self.draft_revision.as_deref() == Some(revision) => {
                self.draft_revision = None
            }
            _ => {}
        }
        result
    }

    // ----- connection ----------------------------------------------------

    /// `onConnectionLost()`: the vault became unreachable.
    pub fn connection_lost(&mut self) {
        self.is_offline = true;
        // Preserve the latest editor snapshot before relying on the legacy backup.
        let _ = self.flush_active_draft();
        if self.save_deadline.take().is_some() {
            self.has_unsaved_changes = true;
        }
        self.max_interval_deadline = None;
        self.effects.push(AutosaveEffect::OfflineChanged(true));
        self.set_status(SaveStatus::Disconnected);
        if let Some(doc) = self.current_doc() {
            let content = self.editor_text.clone();
            if !content.is_empty() && self.last_saved_content.as_deref() != Some(content.as_str()) {
                let _ = self.drafts.backup_save(&draft_subject(doc), &content);
            }
        }
    }

    /// `onConnectionRestored()`: saves pending changes.
    pub fn connection_restored(&mut self) {
        self.is_offline = false;
        self.effects.push(AutosaveEffect::OfflineChanged(false));
        self.set_status(SaveStatus::Hidden);
        if self.has_unsaved_changes && self.active.is_some() {
            self.save_now();
        }
    }

    // ----- status / guards -----------------------------------------------

    fn set_status(&mut self, status: SaveStatus) {
        if self.status != status {
            self.status = status;
            self.effects.push(AutosaveEffect::Status(status));
        }
    }

    fn show_draft_storage_error(&mut self) {
        self.draft_storage_error_visible = true;
        self.set_status(SaveStatus::DraftStorageError);
    }

    fn clear_draft_storage_error(&mut self) {
        if !self.draft_storage_error_visible {
            return;
        }
        self.draft_storage_error_visible = false;
        let status = if self.has_unsaved_changes {
            SaveStatus::Unsaved
        } else {
            SaveStatus::Hidden
        };
        self.set_status(status);
    }

    fn update_unload_guard(&mut self) {
        let armed = self
            .current_doc()
            .is_some_and(|doc| self.draft_persistence_failures.contains(&doc));
        if armed != self.unload_guard {
            self.unload_guard = armed;
            self.effects.push(AutosaveEffect::UnloadGuard(armed));
        }
    }

    // ----- queries -------------------------------------------------------

    /// Active document.
    pub fn current_doc(&self) -> Option<DocId> {
        self.active.as_ref().map(|a| a.doc)
    }
    /// Path of the active note.
    pub fn current_file(&self) -> Option<&str> {
        self.active.as_ref().map(|a| a.path.as_str())
    }
    /// Dirty.
    pub fn has_unsaved_changes(&self) -> bool {
        self.has_unsaved_changes
    }
    /// A save request is in flight.
    pub fn is_saving(&self) -> bool {
        self.in_flight.is_some()
    }
    /// Offline (vault unreachable).
    pub fn is_offline(&self) -> bool {
        self.is_offline
    }
    /// Server text of the active note as last loaded/saved.
    pub fn last_saved_content(&self) -> Option<&str> {
        self.last_saved_content.as_deref()
    }
    /// Server revision of the active note.
    pub fn base_revision(&self) -> Option<&str> {
        self.base_revision.as_deref()
    }
    /// Draft revision of the active note's stored draft.
    pub fn draft_revision(&self) -> Option<&str> {
        self.draft_revision.as_deref()
    }
    /// Latest server-acknowledged revision for `doc`.
    pub fn known_base_revision(&self, doc: DocId) -> Option<&str> {
        self.known_base_revisions.get(&doc).map(String::as_str)
    }
    /// Debounced save armed (`saveTimeout`).
    pub fn save_timer_armed(&self) -> bool {
        self.save_deadline.is_some()
    }
    /// Max-interval save armed (`saveMaxIntervalTimeout`).
    pub fn max_interval_timer_armed(&self) -> bool {
        self.max_interval_deadline.is_some()
    }
    /// Number of pending debounced draft writes.
    pub fn pending_draft_writes(&self) -> usize {
        self.draft_writes.len()
    }
    /// Offline backup armed.
    pub fn offline_backup_armed(&self) -> bool {
        self.offline_backup.is_some()
    }
    /// A recovery dialog is pending for `doc`.
    pub fn has_pending_recovery(&self, doc: DocId) -> bool {
        self.pending_recovery.as_ref().is_some_and(|p| p.doc == doc)
    }
    /// Unload warning armed.
    pub fn unload_guard_active(&self) -> bool {
        self.unload_guard
    }
    /// Current status label.
    pub fn status(&self) -> SaveStatus {
        self.status
    }
}
