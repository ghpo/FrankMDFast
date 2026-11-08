//! The store: one authoritative state per vault (HP-002).
//!
//! [`Workspace::handle`] applies one [`Command`] at a time (the app feeds it
//! from a serialized queue, HP-003/005). File operations go straight through
//! `frankmd-fs` (they are short and must be ordered with the edits around
//! them); tree scans and renders are requested as [`Effect`]s and come back as
//! generation-tagged commands. Autosave and drafts are the DocId-keyed
//! [`AutosaveSession`]; its saves are executed here against the document's
//! *current* path, and a save never creates a file (HP-006).

use std::sync::Arc;

use frankmd_core::path::{is_config_file, normalize_note_path};
use frankmd_core::registry::DocRegistry;
use frankmd_core::tree::TreeNode;
use frankmd_core::{Clock, DocId, Generation, RelPath, Revision};
use frankmd_fs::FsError;
use frankmd_fs::service::NotesService;
use frankmd_watch::{SharedSelfWrites, WatchEvent};

use crate::autosave::{
    AutosaveConfig, AutosaveEffect, AutosaveSession, DeletionPrep, SaveOutcome, SaveRequest,
};
use crate::command::{Command, Effect, Event, NoteTemplate, UserError};
use crate::doc_index::DocIndex;
use crate::draft_storage::{DraftBackend, DraftStorage, ItemKind};
use crate::drafts::{FileDraftBackend, draft_subject};

/// The open document (one editor per window).
#[derive(Debug, Clone)]
pub struct OpenDoc {
    /// Stable id.
    pub id: DocId,
    /// Current path.
    pub path: RelPath,
    /// Revision of the bytes last read/written on disk.
    pub disk_revision: Revision,
    /// Generation of the latest edit (renders compare against it).
    pub edit_generation: Generation,
    /// The file was not valid UTF-8: shown lossily and never saved (HP-028).
    pub read_only: bool,
}

/// Immutable view handed to the UI.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Current tree.
    pub tree: Arc<Vec<TreeNode>>,
    /// Active document and its path.
    pub active: Option<(DocId, RelPath)>,
    /// Active document has unsaved changes.
    pub dirty: bool,
    /// Resolved config.
    pub config: Arc<frankmd_config::ResolvedConfig>,
}

/// Which file operation failed (selects the fallback error key).
#[derive(Debug, Clone, Copy)]
enum Op {
    Load,
    Save,
    Create,
    Rename,
    Delete,
}

fn user_error(op: Op, err: &FsError, folder: bool) -> UserError {
    let key = match (err, op) {
        (FsError::AlreadyExists(_), _) if folder => "errors.folder_already_exists",
        (FsError::AlreadyExists(_), _) => "errors.note_already_exists",
        (FsError::NotFound(_), Op::Load) => "errors.note_not_found",
        (FsError::NotFound(_), _) if folder => "errors.folder_no_longer_exists",
        (FsError::NotFound(_), _) => "errors.file_no_longer_exists",
        (FsError::RevisionConflict(_), _) => "errors.note_changed_since_loaded",
        (FsError::PermissionDenied(_), _) => "errors.permission_denied",
        (_, Op::Load) => "errors.failed_to_load",
        (_, Op::Save) => "errors.failed_to_save",
        (_, Op::Create) => "errors.failed_to_create",
        (_, Op::Rename) => "errors.failed_to_rename",
        (_, Op::Delete) => "errors.failed_to_delete",
    };
    UserError::new(key)
}

/// Shown when the outgoing draft could not be stored (the transition is refused).
const DRAFT_STORAGE_ERROR: &str = "status.draft_storage_error";

/// Output collected while handling one command.
#[derive(Debug, Default)]
struct Out {
    events: Vec<Event>,
    effects: Vec<Effect>,
}

impl Out {
    fn error(&mut self, error: UserError) {
        self.events.push(Event::Error(error));
    }
}

/// The workspace for one vault.
pub struct Workspace<B: DraftBackend = FileDraftBackend> {
    vault: frankmd_fs::Vault,
    config: frankmd_config::store::ConfigStore,
    registry: DocRegistry,
    index: DocIndex,
    autosave: AutosaveSession<B>,
    open: Option<OpenDoc>,
    tree: Arc<Vec<TreeNode>>,
    tree_generation: Generation,
    self_writes: Option<SharedSelfWrites>,
    clock: Box<dyn Clock>,
}

impl<B: DraftBackend> std::fmt::Debug for Workspace<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("vault", &self.vault)
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

impl<B: DraftBackend> Workspace<B> {
    /// Create for an opened vault, its config store, its draft store and
    /// its persisted DocId index (both under the per-vault state dir).
    pub fn new(
        vault: frankmd_fs::Vault,
        config: frankmd_config::store::ConfigStore,
        drafts: DraftStorage<B>,
        index: DocIndex,
        clock: Box<dyn Clock>,
    ) -> Self {
        Self {
            vault,
            config,
            registry: DocRegistry::default(),
            index,
            autosave: AutosaveSession::new(drafts, AutosaveConfig::default()),
            open: None,
            tree: Arc::new(Vec::new()),
            tree_generation: Generation::default(),
            self_writes: None,
            clock,
        }
    }

    /// Share the watcher's self-write tokens so our own saves are not
    /// reported back as external changes.
    pub fn set_self_writes(&mut self, tokens: SharedSelfWrites) {
        self.self_writes = Some(tokens);
    }

    /// Request a fresh tree scan (startup, manual refresh).
    pub fn rescan(&mut self) -> Effect {
        self.tree_generation = self.tree_generation.next();
        Effect::ScanTree {
            generation: self.tree_generation,
        }
    }

    /// The autosave session (status, dirty state, draft store).
    pub fn autosave(&self) -> &AutosaveSession<B> {
        &self.autosave
    }

    /// The open note was encrypted/decrypted: skip plaintext drafts and drop
    /// the old draft when it becomes encrypted.
    pub fn set_open_encrypted(&mut self, encrypted: bool) {
        self.autosave.set_encrypted(encrypted);
        if encrypted && let Some(doc) = self.autosave.current_doc() {
            let _ = self.autosave.drafts_mut().remove_draft(&draft_subject(doc));
        }
    }

    /// The open document.
    pub fn open_doc(&self) -> Option<&OpenDoc> {
        self.open.as_ref()
    }

    /// Shared recent notes, newest first (missing files filtered out).
    pub fn recent_paths(&self) -> Vec<String> {
        frankmd_fs::recent::RecentFilesService::new(&self.vault, &self.config)
            .entries()
            .into_iter()
            .map(|r| r.path)
            .collect()
    }

    /// Clear the shared recents (empty state "Clear history…").
    pub fn clear_recents(&mut self) -> bool {
        frankmd_fs::recent::RecentFilesService::new(&self.vault, &self.config)
            .clear()
            .unwrap_or(false)
    }

    /// The vault this workspace edits.
    pub fn vault(&self) -> &frankmd_fs::Vault {
        &self.vault
    }

    /// Current path of `doc`.
    pub fn path_of(&self, doc: DocId) -> Option<&RelPath> {
        self.registry.path_of(doc)
    }

    /// Current snapshot.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tree: Arc::clone(&self.tree),
            active: self.open.as_ref().map(|o| (o.id, o.path.clone())),
            dirty: self.autosave.has_unsaved_changes(),
            config: self.config.current(),
        }
    }

    /// Handle one command. Returns events for views and effects for the runtime.
    pub fn handle(&mut self, cmd: Command) -> (Vec<Event>, Vec<Effect>) {
        let mut out = Out::default();
        let now = self.clock.instant();
        match cmd {
            Command::OpenNote { path } => self.open_note(&path, &mut out),
            Command::CloseNote => self.close_note(&mut out),
            Command::Edited { doc, text } => {
                if let Some(open) = self.open.as_mut().filter(|o| o.id == doc && !o.read_only) {
                    open.edit_generation = open.edit_generation.next();
                    let generation = open.edit_generation;
                    self.autosave.editor_changed(&text, now);
                    out.effects.push(Effect::Render { doc, generation });
                }
            }
            Command::SaveNow { doc } => {
                if self.autosave.current_doc() == Some(doc) {
                    self.autosave.save_now();
                }
            }
            Command::CreateNote {
                parent,
                name,
                template,
            } => self.create_note(&parent, &name, template, &mut out),
            Command::CreateHugoPost {
                parent,
                title,
                slug,
            } => self.create_hugo_post(&parent, &title, &slug, &mut out),
            Command::CreateFolder { parent, name } => self.create_folder(&parent, &name, &mut out),
            Command::Rename { from, to } => {
                let _ = self.rename(&from, &to, &mut out);
            }
            Command::MoveMany { items, dest } => self.move_many(items, &dest, &mut out),
            Command::Delete { path } => self.delete(&path, &mut out),
            Command::UpdateSettings { changes } => match self.config.update(&changes) {
                Ok(_) => out.events.push(Event::SettingsChanged),
                Err(_) => out.error(UserError::new("errors.failed_to_save")),
            },
            Command::UpdateGlobalSettings { changes } => {
                match self.config.update_global(&changes) {
                    Ok(_) => out.events.push(Event::SettingsChanged),
                    Err(_) => out.error(UserError::new("errors.failed_to_save")),
                }
            }
            Command::ResolveRecovery(resolution) => {
                self.autosave.recovery_resolved(resolution, now)
            }
            Command::RecoveryKeepBoth(request) => self.keep_both(request, now, &mut out),
            Command::ContentLossUndo {
                undid_editor_change,
            } => self.autosave.undo_content_loss(undid_editor_change),
            Command::ContentLossSaveAnyway => self.autosave.save_anyway_after_warning(),
            Command::FlushDrafts => {
                let _ = self.autosave.flush_active_draft();
            }
            Command::Tick => self.autosave.tick(now),
            Command::TreeScanned { generation, tree } => {
                if generation == self.tree_generation {
                    self.tree = Arc::new(tree);
                    out.events.push(Event::TreeChanged);
                }
            }
            Command::External(event) => self.external(event, &mut out),
        }
        self.drain_autosave(&mut out);
        if let Some(deadline) = self.autosave.next_deadline() {
            out.effects.push(Effect::WakeAt(deadline));
        }
        (out.events, out.effects)
    }

    // ----- autosave bridge -------------------------------------------------

    /// Execute queued saves and forward every other autosave effect.
    fn drain_autosave(&mut self, out: &mut Out) {
        loop {
            let effects = self.autosave.take_effects();
            if effects.is_empty() {
                return;
            }
            for effect in effects {
                match effect {
                    AutosaveEffect::Save(request) => self.perform_save(request, out),
                    other => out.events.push(Event::Autosave(other)),
                }
            }
        }
    }

    /// Write a save to the document's current path. `.fed` is never written
    /// from the editor (in-app `.fed` editing stays blocked).
    fn perform_save(&mut self, request: SaveRequest, out: &mut Out) {
        let now = self.clock.instant();
        let path = self.registry.path_of(request.doc).cloned();
        let result = match &path {
            Some(path) if !is_config_file(path.as_str()) => {
                self.expect_write(path, request.content.as_bytes());
                NotesService::new(&self.vault)
                    .update(path.as_str(), request.content.as_bytes())
                    .map_err(Some)
            }
            _ => Err(None),
        };
        let outcome = match result {
            Ok(revision) => {
                if let Some(open) = self.open.as_mut().filter(|o| o.id == request.doc) {
                    open.disk_revision = revision.clone();
                }
                out.events.push(Event::DocSaved {
                    doc: request.doc,
                    revision: revision.clone(),
                });
                SaveOutcome::Saved {
                    revision: Some(revision.0),
                }
            }
            Err(err) => {
                let error = err.map_or_else(
                    || UserError::new("errors.failed_to_save"),
                    |e| user_error(Op::Save, &e, false),
                );
                out.error(error);
                SaveOutcome::Failed
            }
        };
        self.autosave.save_finished(request.ticket, outcome, now);
    }

    fn expect_write(&self, path: &RelPath, bytes: &[u8]) {
        if let Some(tokens) = &self.self_writes {
            let mut tokens = tokens
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tokens.expect(path.as_str().into(), Revision::of(bytes));
        }
    }

    fn expect_removal(&self, path: &RelPath) {
        if let Some(tokens) = &self.self_writes {
            let mut tokens = tokens
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tokens.expect_removal(path.as_str().into());
        }
    }

    // ----- navigation ------------------------------------------------------

    fn open_note(&mut self, path: &RelPath, out: &mut Out) {
        if is_config_file(path.as_str()) {
            out.error(UserError::new("errors.failed_to_load"));
            return;
        }
        if !path.as_str().ends_with(frankmd_core::path::NOTE_EXT) {
            out.error(UserError::new("errors.not_a_note"));
            return;
        }
        if self.open.as_ref().is_some_and(|o| &o.path == path) {
            return;
        }
        if self.autosave.prepare_for_transition().is_err() {
            out.error(UserError::new(DRAFT_STORAGE_ERROR));
            return;
        }
        let file = match NotesService::new(&self.vault).read(path.as_str()) {
            Ok(file) => file,
            Err(err) => {
                out.error(user_error(Op::Load, &err, false));
                return;
            }
        };
        let text = String::from_utf8_lossy(&file.bytes).into_owned();
        self.autosave.set_encrypted(file.encrypted);
        let doc = self.registry.id_for(path);
        if let Some(previous) = self.index.record(path.as_str(), doc) {
            // Same note as in an earlier run: its drafts follow it to this run's id.
            let _ = self.autosave.drafts_mut().remap_drafts(
                &draft_subject(previous),
                &draft_subject(doc),
                ItemKind::File,
            );
        }
        self.autosave.set_editor_text(&text);
        self.autosave
            .set_file(doc, path.as_str(), &text, Some(&file.revision.0));
        let shown = if file.valid_utf8 {
            self.autosave.recover_draft(&text, &file.revision.0)
        } else {
            text
        };
        self.autosave.set_editor_text(&shown);
        self.open = Some(OpenDoc {
            id: doc,
            path: path.clone(),
            disk_revision: file.revision,
            edit_generation: Generation::default(),
            read_only: !file.valid_utf8,
        });
        self.record_recent(path);
        out.events.push(Event::DocOpened {
            doc,
            path: path.clone(),
            text: shown,
        });
        out.effects.push(Effect::Render {
            doc,
            generation: Generation::default(),
        });
    }

    /// Shared recents (`.fed` `recent_files`, FE-298…301): recorded on every
    /// successful open, newest first. A failed write only loses history.
    fn record_recent(&self, path: &RelPath) {
        let recents = frankmd_fs::recent::RecentFilesService::new(&self.vault, &self.config);
        let _ = recents.record(Some(path.as_str()), self.clock.now().timestamp());
    }

    fn close_note(&mut self, out: &mut Out) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let doc = open.id;
        if self.autosave.prepare_for_transition().is_err() {
            out.error(UserError::new(DRAFT_STORAGE_ERROR));
            return;
        }
        self.autosave.set_encrypted(false);
        self.autosave.clear_file();
        self.open = None;
        out.events.push(Event::DocClosed { doc });
    }

    // ----- file operations -------------------------------------------------

    fn create_note(&mut self, parent: &RelPath, name: &str, template: NoteTemplate, out: &mut Out) {
        let service = NotesService::new(&self.vault);
        let created = match template {
            NoteTemplate::Empty => {
                let raw = normalize_note_path(&join(parent, name));
                match RelPath::parse(&raw) {
                    Ok(path) if !is_config_file(path.as_str()) => {
                        self.expect_write(&path, b"");
                        service
                            .create(path.as_str(), b"")
                            .map(|_| path)
                            .map_err(|e| user_error(Op::Create, &e, false))
                    }
                    _ => Err(UserError::new("errors.failed_to_create")),
                }
            }
            NoteTemplate::Hugo => {
                let style_value = self
                    .config
                    .current()
                    .value(frankmd_config::Key::HugoPathStyle);
                let style = frankmd_core::hugo::HugoPathStyle::from_config(match &style_value {
                    Some(frankmd_config::Value::Str(s)) => Some(s.as_str()),
                    _ => None,
                });
                let parent = (!parent.is_root()).then(|| parent.as_str());
                service
                    .create_hugo_post(name, parent, style, self.clock.now())
                    .map_err(|e| user_error(Op::Create, &e, false))
                    .and_then(|post| {
                        RelPath::parse(&post.path)
                            .map_err(|_| UserError::new("errors.failed_to_create"))
                    })
            }
        };
        match created {
            Ok(path) => {
                out.events.push(Event::Reveal { path: path.clone() });
                out.effects.push(self.rescan());
                self.open_note(&path, out);
            }
            Err(error) => out.error(error),
        }
    }

    /// Hugo post with a user-edited slug: the same path and front matter as
    /// `NotesService::create_hugo_post`, with `slug` in place of the slugified
    /// title (a blank slug falls back to it).
    fn create_hugo_post(&mut self, parent: &RelPath, title: &str, slug: &str, out: &mut Out) {
        use frankmd_core::hugo;
        let slug = slug.trim();
        let slug = if slug.is_empty() {
            frankmd_core::slug::slugify(title)
        } else {
            frankmd_core::slug::slugify(slug)
        };
        if slug.is_empty() {
            out.error(UserError::new("errors.failed_to_create"));
            return;
        }
        let style_value = self
            .config
            .current()
            .value(frankmd_config::Key::HugoPathStyle);
        let style = hugo::HugoPathStyle::from_config(match &style_value {
            Some(frankmd_config::Value::Str(s)) => Some(s.as_str()),
            _ => None,
        });
        let now = self.clock.now();
        let relative = match style {
            hugo::HugoPathStyle::Flat => format!("{slug}.md"),
            hugo::HugoPathStyle::Dated => format!("{}/{slug}/index.md", hugo::date_path(&now)),
        };
        let service = NotesService::new(&self.vault);
        let template = service
            .hugo_template()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let content = hugo::render_template(
            template.as_deref().unwrap_or(hugo::DEFAULT_TEMPLATE),
            title,
            &slug,
            &hugo::format_date(&now),
        );
        let Ok(path) = RelPath::parse(&join(parent, &relative)) else {
            out.error(UserError::new("errors.failed_to_create"));
            return;
        };
        self.expect_write(&path, content.as_bytes());
        match service.create(path.as_str(), content.as_bytes()) {
            Ok(_) => {
                out.events.push(Event::Reveal { path: path.clone() });
                out.effects.push(self.rescan());
                self.open_note(&path, out);
            }
            Err(err) => out.error(user_error(Op::Create, &err, false)),
        }
    }

    /// "Keep both": the draft goes to a sibling copy (never overwriting a
    /// file), then the dialog resolves to the file on disk.
    fn keep_both(
        &mut self,
        request: crate::autosave::RecoveryRequest,
        now: std::time::Instant,
        out: &mut Out,
    ) {
        let Ok(path) = RelPath::parse(&request.path) else {
            out.error(UserError::new("errors.failed_to_create"));
            out.events
                .push(Event::Autosave(AutosaveEffect::OpenRecovery(request)));
            return;
        };
        let zone = self.clock.now().time_zone().clone();
        let service = NotesService::new(&self.vault);
        let mut created = None;
        for attempt in 1..=20 {
            let Some(copy) =
                crate::recovery_copy::copy_path(&path, request.backup_timestamp, &zone, attempt)
            else {
                break;
            };
            self.expect_write(&copy, request.backup_content.as_bytes());
            match service.create(copy.as_str(), request.backup_content.as_bytes()) {
                Ok(_) => {
                    created = Some(copy);
                    break;
                }
                Err(FsError::AlreadyExists(_)) => {}
                Err(err) => {
                    out.error(user_error(Op::Create, &err, false));
                    out.events
                        .push(Event::Autosave(AutosaveEffect::OpenRecovery(request)));
                    return;
                }
            }
        }
        let Some(copy) = created else {
            out.error(UserError::new("errors.note_already_exists"));
            out.events
                .push(Event::Autosave(AutosaveEffect::OpenRecovery(request)));
            return;
        };
        self.autosave.recovery_resolved(
            crate::autosave::RecoveryResolution::accept_server(&request),
            now,
        );
        out.events.push(Event::Reveal { path: copy });
        out.effects.push(self.rescan());
    }

    fn create_folder(&mut self, parent: &RelPath, name: &str, out: &mut Out) {
        let Ok(path) = RelPath::parse(&join(parent, name)) else {
            out.error(UserError::new("errors.failed_to_create"));
            return;
        };
        match NotesService::new(&self.vault).create_folder(path.as_str()) {
            Ok(()) => {
                out.events.push(Event::Reveal { path });
                out.effects.push(self.rescan());
            }
            Err(err) => out.error(user_error(Op::Create, &err, true)),
        }
    }

    /// Rename/move a file or folder; `false` when it failed (error reported).
    fn rename(&mut self, from: &RelPath, to: &RelPath, out: &mut Out) -> bool {
        if from == to {
            return true;
        }
        if is_config_file(from.as_str()) || is_config_file(to.as_str()) {
            out.error(UserError::new("errors.failed_to_rename"));
            return false;
        }
        let affects_active = self.open.as_ref().is_some_and(|o| o.path.starts_with(from));
        if affects_active && self.autosave.prepare_for_transition().is_err() {
            out.error(UserError::new(DRAFT_STORAGE_ERROR));
            return false;
        }
        let service = NotesService::new(&self.vault).with_recents(&self.config);
        let folder = service.is_dir(from.as_str()).unwrap_or(false);
        self.expect_removal(from);
        let renamed = if folder {
            service.rename_folder(from.as_str(), to.as_str())
        } else {
            service.rename(from.as_str(), to.as_str())
        };
        if let Err(err) = renamed {
            self.autosave.resume_after_transition(self.clock.instant());
            out.error(user_error(Op::Rename, &err, folder));
            return false;
        }
        let remaps = self.registry.rename(from, to);
        self.index.apply_remaps(&remaps);
        self.autosave.paths_changed(&remaps);
        if let Some(open) = self.open.as_mut()
            && let Some(remap) = remaps.iter().find(|r| r.id == open.id)
        {
            open.path = remap.to.clone();
        }
        self.autosave.resume_after_transition(self.clock.instant());
        out.events.push(Event::Remapped(remaps));
        out.events.push(Event::Reveal { path: to.clone() });
        out.effects.push(self.rescan());
        true
    }

    /// Sequential moves into `dest`, skipping items inside another selected
    /// item, stopping at the first failure (HP-029).
    fn move_many(&mut self, items: Vec<RelPath>, dest: &RelPath, out: &mut Out) {
        let roots: Vec<&RelPath> = items
            .iter()
            .filter(|item| {
                !items
                    .iter()
                    .any(|other| other != *item && item.starts_with(other))
            })
            .collect();
        for item in roots {
            let Some(name) = item.file_name() else {
                continue;
            };
            if item.parent().as_ref() == Some(dest)
                || (dest.is_root() && item.parent().is_none_or(|p| p.is_root()))
            {
                continue;
            }
            let Ok(to) = RelPath::parse(&join(dest, name)) else {
                out.error(UserError::new("errors.failed_to_move"));
                return;
            };
            if !self.rename(item, &to, out) {
                return;
            }
        }
    }

    fn delete(&mut self, path: &RelPath, out: &mut Out) {
        if is_config_file(path.as_str()) {
            out.error(UserError::new("errors.failed_to_delete"));
            return;
        }
        let service = NotesService::new(&self.vault);
        let folder = service.is_dir(path.as_str()).unwrap_or(false);
        let active = self
            .open
            .as_ref()
            .filter(|o| o.path.starts_with(path))
            .map(|o| (o.id, &o.path == path));
        let mut expected_revision = None;
        match active {
            Some((doc, true)) => match self.autosave.prepare_for_file_deletion(doc) {
                DeletionPrep::Ready { revision } => expected_revision = Some(revision),
                DeletionPrep::Stale | DeletionPrep::AwaitSave | DeletionPrep::Failed(_) => {
                    out.error(UserError::new(DRAFT_STORAGE_ERROR));
                    return;
                }
            },
            Some((_, false)) if self.autosave.prepare_for_transition().is_err() => {
                out.error(UserError::new(DRAFT_STORAGE_ERROR));
                return;
            }
            Some((_, false)) | None => {}
        }
        self.expect_removal(path);
        // ui-spec §5.13: delete moves to the Trash, so non-empty folders can
        // be deleted honestly (FE-267 fixed); recents under the path go too.
        let service = NotesService::new(&self.vault).with_recents(&self.config);
        let deleted = service.trash(path.as_str(), expected_revision.as_deref());
        if let Err(err) = deleted {
            self.autosave.resume_after_transition(self.clock.instant());
            out.error(user_error(Op::Delete, &err, folder));
            return;
        }
        let docs = self.registry.remove(path);
        self.index.remove(&docs);
        let _ = self.autosave.docs_deleted(&docs);
        if let Some(open) = self.open.take_if(|o| docs.contains(&o.id)) {
            out.events.push(Event::DocDeleted { doc: open.id });
        }
        out.effects.push(self.rescan());
    }

    // ----- watcher -----------------------------------------------------------

    fn external(&mut self, event: WatchEvent, out: &mut Out) {
        match event {
            WatchEvent::VaultChanged { paths, removed } => {
                out.effects.push(self.rescan());
                self.check_open_doc_on_disk(&paths, &removed, out);
            }
            WatchEvent::ConfigChanged => {
                if self.config.reload().is_ok() {
                    out.events.push(Event::SettingsChanged);
                }
            }
            other => out.events.push(Event::System(other)),
        }
    }

    /// External change under the open note: identical bytes → nothing; clean
    /// buffer → reload; dirty buffer → "changed on disk"; gone → "deleted on disk".
    fn check_open_doc_on_disk(&mut self, paths: &[RelPath], removed: &[RelPath], out: &mut Out) {
        let Some(open) = self.open.clone() else {
            return;
        };
        let lost_events = paths.iter().any(RelPath::is_root);
        if removed.contains(&open.path) {
            out.events.push(Event::DocDeletedOnDisk { doc: open.id });
            return;
        }
        if !lost_events && !paths.contains(&open.path) {
            return;
        }
        match NotesService::new(&self.vault).read(open.path.as_str()) {
            Err(FsError::NotFound(_)) => out.events.push(Event::DocDeletedOnDisk { doc: open.id }),
            Err(_) => {}
            Ok(file) if file.revision == open.disk_revision => {
                self.autosave.set_encrypted(file.encrypted);
            }
            Ok(file) if self.autosave.has_unsaved_changes() => {
                self.autosave.set_encrypted(file.encrypted);
                out.events.push(Event::DocConflict { doc: open.id })
            }
            Ok(file) => {
                self.autosave.set_encrypted(file.encrypted);
                let text = String::from_utf8_lossy(&file.bytes).into_owned();
                self.autosave.set_editor_text(&text);
                self.autosave
                    .set_file(open.id, open.path.as_str(), &text, Some(&file.revision.0));
                if let Some(o) = self.open.as_mut() {
                    o.disk_revision = file.revision;
                    o.read_only = !file.valid_utf8;
                }
                out.events.push(Event::DocReloaded { doc: open.id, text });
            }
        }
    }
}

/// `parent/name` (just `name` at the root).
fn join(parent: &RelPath, name: &str) -> String {
    if parent.is_root() {
        name.to_owned()
    } else {
        format!("{}/{name}", parent.as_str())
    }
}
