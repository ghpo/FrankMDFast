//! The workspace store end to end over a temporary vault: open → edit →
//! debounced save, rename/delete during pending autosave, drafts across a
//! restart (DocId index + on-disk draft store), external changes.
//! Contract: architecture.md §3.1 / §9 item 4, HP-001/006/007.
#![allow(clippy::unwrap_used)]

use frankmd_config::EnvSnapshot;
use frankmd_config::store::ConfigStore;
use frankmd_core::{DocId, FakeClock, RelPath};
use frankmd_fs::Vault;
use frankmd_fs::service::NotesService;
use frankmd_watch::WatchEvent;
use frankmd_workspace::autosave::{AutosaveEffect, SaveStatus};
use frankmd_workspace::doc_index::DocIndex;
use frankmd_workspace::draft_storage::DraftBackend;
use frankmd_workspace::drafts::{FileDraftBackend, open_draft_storage};
use frankmd_workspace::{Command, Effect, Event, Workspace};

struct Fixture {
    _state_dir: tempfile::TempDir,
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    state: std::path::PathBuf,
    clock: FakeClock,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("notes");
        let state_dir = tempfile::tempdir().unwrap();
        let state = state_dir.path().to_path_buf();
        let fixture = Self {
            root,
            state,
            _state_dir: state_dir,
            _dir: dir,
            clock: FakeClock::default(),
        };
        fixture.service_vault();
        fixture
    }
    fn service_vault(&self) -> Vault {
        Vault::open(&self.root, &self.state).unwrap()
    }
    /// A file written by "someone else" (no self-write token).
    fn write_external(&self, path: &str, text: &str) {
        let vault = self.service_vault();
        NotesService::new(&vault)
            .write(path, text.as_bytes())
            .unwrap();
    }
    fn read(&self, path: &str) -> Option<String> {
        let vault = self.service_vault();
        NotesService::new(&vault)
            .read(path)
            .ok()
            .map(|f| String::from_utf8(f.bytes).unwrap())
    }
    fn workspace(&self) -> Workspace<FileDraftBackend> {
        // Deletes go to the Trash (ui-spec §5.13); never the user's real one in tests.
        let vault = self
            .service_vault()
            .with_trash(frankmd_fs::TrashBackend::Directory(
                self.state.join("trash"),
            ));
        let config = ConfigStore::open(vault.clone(), EnvSnapshot::default());
        let drafts = open_draft_storage(&self.state).unwrap();
        Workspace::new(
            vault,
            config,
            drafts,
            DocIndex::open(&self.state),
            Box::new(self.clock.clone()),
        )
    }
}

fn p(path: &str) -> RelPath {
    RelPath::parse(path).unwrap()
}

fn opened(events: &[Event]) -> (DocId, String) {
    events
        .iter()
        .find_map(|e| match e {
            Event::DocOpened { doc, text, .. } => Some((*doc, text.clone())),
            _ => None,
        })
        .expect("DocOpened")
}

fn tick_after(ws: &mut Workspace<FileDraftBackend>, clock: &FakeClock, ms: u64) -> Vec<Event> {
    clock.advance_ms(ms);
    ws.handle(Command::Tick).0
}

#[test]
fn edits_are_saved_2000ms_after_the_last_keystroke() {
    let f = Fixture::new();
    f.write_external("a.md", "hello");
    let mut ws = f.workspace();
    let (doc, text) = opened(&ws.handle(Command::OpenNote { path: p("a.md") }).0);
    assert_eq!(text, "hello");

    let (_, effects) = ws.handle(Command::Edited {
        doc,
        text: "hello world".into(),
    });
    assert!(
        effects.iter().any(|e| matches!(e, Effect::WakeAt(_))),
        "the app arms a timer"
    );
    assert!(ws.snapshot().dirty);
    assert!(
        tick_after(&mut ws, &f.clock, 1999)
            .iter()
            .all(|e| !matches!(e, Event::DocSaved { .. }))
    );
    let events = tick_after(&mut ws, &f.clock, 1);

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::DocSaved { doc: d, .. } if *d == doc))
    );
    assert_eq!(f.read("a.md").as_deref(), Some("hello world"));
    assert!(!ws.snapshot().dirty);
    assert_eq!(ws.autosave().status(), SaveStatus::Saved);
}

#[test]
fn rename_during_a_pending_autosave_saves_to_the_new_path_and_never_recreates_the_old_one() {
    let f = Fixture::new();
    f.write_external("old.md", "text");
    let mut ws = f.workspace();
    let (doc, _) = opened(&ws.handle(Command::OpenNote { path: p("old.md") }).0);
    ws.handle(Command::Edited {
        doc,
        text: "text edited".into(),
    });

    let (events, _) = ws.handle(Command::Rename {
        from: p("old.md"),
        to: p("new.md"),
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Remapped(r) if r.iter().any(|m| m.id == doc)))
    );
    assert_eq!(
        ws.snapshot().active,
        Some((doc, p("new.md"))),
        "same DocId, new path"
    );
    assert!(
        ws.snapshot().dirty,
        "a rename never makes pending edits look saved"
    );

    tick_after(&mut ws, &f.clock, 2000);
    assert_eq!(f.read("new.md").as_deref(), Some("text edited"));
    assert_eq!(
        f.read("old.md"),
        None,
        "HP-006: a save never creates a file"
    );
}

#[test]
fn folder_rename_carries_the_open_note_and_its_draft() {
    let f = Fixture::new();
    f.write_external("docs/a.md", "a");
    let mut ws = f.workspace();
    let (doc, _) = opened(
        &ws.handle(Command::OpenNote {
            path: p("docs/a.md"),
        })
        .0,
    );
    ws.handle(Command::Edited {
        doc,
        text: "a draft".into(),
    });
    tick_after(&mut ws, &f.clock, 300); // draft written, save still pending

    ws.handle(Command::Rename {
        from: p("docs"),
        to: p("archive"),
    });

    assert_eq!(ws.path_of(doc), Some(&p("archive/a.md")));
    // The transition paused autosave; resuming restarts the 2 s debounce.
    tick_after(&mut ws, &f.clock, 2000);
    assert_eq!(f.read("archive/a.md").as_deref(), Some("a draft"));
}

#[test]
fn deleting_the_open_note_clears_it_and_its_draft() {
    let f = Fixture::new();
    f.write_external("gone.md", "x");
    let mut ws = f.workspace();
    let (doc, _) = opened(&ws.handle(Command::OpenNote { path: p("gone.md") }).0);
    ws.handle(Command::Edited {
        doc,
        text: "unsaved".into(),
    });
    tick_after(&mut ws, &f.clock, 300);

    let (events, _) = ws.handle(Command::Delete { path: p("gone.md") });

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::DocDeleted { doc: d } if *d == doc))
    );
    assert_eq!(ws.snapshot().active, None);
    assert_eq!(f.read("gone.md"), None);
    tick_after(&mut ws, &f.clock, 5000);
    assert_eq!(f.read("gone.md"), None, "no late save recreates it");
    assert!(
        ws.autosave().drafts().backend().is_empty().unwrap(),
        "draft removed"
    );
}

#[test]
fn a_draft_survives_a_restart_and_is_restored_when_the_baseline_matches() {
    let f = Fixture::new();
    f.write_external("n.md", "server text");
    {
        let mut ws = f.workspace();
        let (doc, _) = opened(&ws.handle(Command::OpenNote { path: p("n.md") }).0);
        ws.handle(Command::Edited {
            doc,
            text: "unsaved local edit".into(),
        });
        tick_after(&mut ws, &f.clock, 300); // draft persisted; the app "crashes" before the save
    }
    assert_eq!(f.read("n.md").as_deref(), Some("server text"));

    let mut ws = f.workspace();
    let (events, _) = ws.handle(Command::OpenNote { path: p("n.md") });
    let (_, text) = opened(&events);

    assert_eq!(
        text, "unsaved local edit",
        "auto-restored without a recovery dialog"
    );
    assert!(ws.snapshot().dirty);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Autosave(AutosaveEffect::OpenRecovery(_))))
    );
}

#[test]
fn a_draft_whose_file_changed_meanwhile_opens_the_recovery_dialog() {
    let f = Fixture::new();
    f.write_external("n.md", "v1");
    {
        let mut ws = f.workspace();
        let (doc, _) = opened(&ws.handle(Command::OpenNote { path: p("n.md") }).0);
        ws.handle(Command::Edited {
            doc,
            text: "my draft".into(),
        });
        tick_after(&mut ws, &f.clock, 300);
    }
    f.write_external("n.md", "v2 from a sync tool");

    let mut ws = f.workspace();
    let (events, _) = ws.handle(Command::OpenNote { path: p("n.md") });

    assert_eq!(opened(&events).1, "v2 from a sync tool");
    let recovery = events.iter().find_map(|e| match e {
        Event::Autosave(AutosaveEffect::OpenRecovery(r)) => Some(r.clone()),
        _ => None,
    });
    assert_eq!(
        recovery.map(|r| (r.path, r.backup_content)),
        Some(("n.md".into(), "my draft".into()))
    );
}

fn vault_changed(paths: &[&str], removed: &[&str]) -> Command {
    Command::External(WatchEvent::VaultChanged {
        paths: paths.iter().map(|s| p(s)).collect(),
        removed: removed.iter().map(|s| p(s)).collect(),
    })
}

#[test]
fn external_change_reloads_a_clean_note_and_flags_a_dirty_one() {
    let f = Fixture::new();
    f.write_external("n.md", "one");
    let mut ws = f.workspace();
    let (doc, _) = opened(&ws.handle(Command::OpenNote { path: p("n.md") }).0);

    // Identical bytes (e.g. our own echo): nothing happens.
    let (events, _) = ws.handle(vault_changed(&["n.md"], &[]));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::DocReloaded { .. } | Event::DocConflict { .. }))
    );

    f.write_external("n.md", "two");
    let (events, _) = ws.handle(vault_changed(&["n.md"], &[]));
    assert!(
        events.iter().any(
            |e| matches!(e, Event::DocReloaded { doc: d, text } if *d == doc && text == "two")
        )
    );

    ws.handle(Command::Edited {
        doc,
        text: "two, edited".into(),
    });
    f.write_external("n.md", "three");
    let (events, _) = ws.handle(vault_changed(&["n.md"], &[]));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::DocConflict { doc: d } if *d == doc)),
        "changed on disk banner"
    );
}

#[test]
fn external_delete_raises_deleted_on_disk() {
    let f = Fixture::new();
    f.write_external("n.md", "one");
    let mut ws = f.workspace();
    let (doc, _) = opened(&ws.handle(Command::OpenNote { path: p("n.md") }).0);

    let (events, effects) = ws.handle(vault_changed(&["n.md"], &["n.md"]));

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::DocDeletedOnDisk { doc: d } if *d == doc))
    );
    assert!(effects.iter().any(|e| matches!(e, Effect::ScanTree { .. })));
}

#[test]
fn stale_tree_scans_are_dropped() {
    let f = Fixture::new();
    let mut ws = f.workspace();
    let Effect::ScanTree { generation: old } = ws.rescan() else {
        unreachable!()
    };
    let Effect::ScanTree { generation: new } = ws.rescan() else {
        unreachable!()
    };

    let (events, _) = ws.handle(Command::TreeScanned {
        generation: old,
        tree: vec![],
    });
    assert!(events.is_empty(), "older generation discarded (HP-003)");
    let (events, _) = ws.handle(Command::TreeScanned {
        generation: new,
        tree: vec![],
    });
    assert!(events.iter().any(|e| matches!(e, Event::TreeChanged)));
}

// security: in-app `.fed` editing stays blocked.
#[test]
fn the_config_file_cannot_be_opened_in_the_editor() {
    let f = Fixture::new();
    let mut ws = f.workspace();
    let (events, _) = ws.handle(Command::OpenNote { path: p(".fed") });
    assert!(events.iter().any(|e| matches!(e, Event::Error(_))));
    assert_eq!(ws.snapshot().active, None);
}

#[test]
#[allow(clippy::disallowed_methods)]
fn binary_attachment_is_not_opened_as_a_note() {
    let f = Fixture::new();
    std::fs::write(f.root.join("talk.m4a"), b"not a markdown note").unwrap();
    let mut ws = f.workspace();
    let (events, _) = ws.handle(Command::OpenNote {
        path: p("talk.m4a"),
    });
    assert!(
        events.iter().any(|e| matches!(e, Event::Error(_))),
        "attachment paths are rejected"
    );
    assert!(
        !events.iter().any(|e| matches!(e, Event::DocOpened { .. })),
        "binary files are never read as notes"
    );
    assert_eq!(ws.snapshot().active, None);
}

#[test]
fn create_note_appends_md_refuses_duplicates_and_opens_it() {
    let f = Fixture::new();
    let mut ws = f.workspace();
    let cmd = || Command::CreateNote {
        parent: p("ideas"),
        name: "first".into(),
        template: frankmd_workspace::command::NoteTemplate::Empty,
    };
    let (events, _) = ws.handle(cmd());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Reveal { path } if path == &p("ideas/first.md")))
    );
    assert_eq!(
        ws.snapshot().active.map(|(_, path)| path),
        Some(p("ideas/first.md"))
    );

    let (events, _) = ws.handle(cmd());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Error(err) if err.key == "errors.note_already_exists"))
    );
}

#[test]
fn move_many_skips_descendants_of_selected_folders() {
    let f = Fixture::new();
    f.write_external("a/one.md", "1");
    f.write_external("b.md", "2");
    let mut ws = f.workspace();
    ws.handle(Command::CreateFolder {
        parent: RelPath::root(),
        name: "dest".into(),
    });

    ws.handle(Command::MoveMany {
        items: vec![p("a"), p("a/one.md"), p("b.md")],
        dest: p("dest"),
    });

    assert_eq!(f.read("dest/a/one.md").as_deref(), Some("1"));
    assert_eq!(f.read("dest/b.md").as_deref(), Some("2"));
}
