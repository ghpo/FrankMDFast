//! Entry point for level-b tests: the real app shell (`frankmd_app::ui::Shell`)
//! over a temporary vault, in a headless window with the registry keymap.
//!
//! Accessors read the shell's GUI-free view models (layout, overlays, banners,
//! status), so assertions are on state, plus element ids / focus for wiring
//! (gpui-kit can't assert rendered text).

use std::sync::Arc;

use frankmd_app::ui::{Shell as AppShell, ShellConfig};
use frankmd_config::EnvSnapshot;
use frankmd_config::store::ConfigStore;
use frankmd_fs::Vault;
use frankmd_fs::recent::RecentFilesService;
use frankmd_fs::service::NotesService;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Pixels, Size, TestAppContext};

use super::vm::{BannerKind, FocusRegion, LayoutState, OverlayKind, SaveState};

/// A vault to build in a temp dir before the shell opens it.
#[derive(Debug, Clone, Default)]
pub struct VaultFixture {
    /// `(relative path, content)`; folders are implied by paths.
    pub notes: Vec<(&'static str, &'static str)>,
    /// Shared recents (newest first), written to `.fed` `recent_files`.
    pub recents: Vec<&'static str>,
    /// Note to open at startup (`frankmd <path>`), if any.
    pub open: Option<&'static str>,
}

impl VaultFixture {
    /// A small vault: two folders, four notes, one open.
    pub fn sample() -> Self {
        Self {
            notes: vec![
                ("Inbox.md", "# Inbox\n"),
                ("blog/2026/Project brief.md", "# Project brief\n\nBody.\n"),
                ("projects/Launch list.md", "# Launch list\n\n- [ ] ship\n"),
                ("reference/Glossary.md", "# Glossary\n"),
            ],
            recents: vec!["projects/Launch list.md", "Inbox.md"],
            open: Some("blog/2026/Project brief.md"),
        }
    }

    /// The sample vault plus `extra` notes.
    pub fn sample_with(extra: &[(&'static str, &'static str)]) -> Self {
        let mut fixture = Self::sample();
        fixture.notes.extend_from_slice(extra);
        fixture
    }

    /// Same vault, nothing open (empty state with docked quick open, §5.2).
    pub fn sample_no_note() -> Self {
        Self {
            open: None,
            ..Self::sample()
        }
    }
}

/// Handle to an opened shell window.
pub struct Shell {
    /// The test window.
    pub window: AnyWindowHandle,
    /// The root view.
    pub view: Entity<AppShell>,
    /// Vault root on disk.
    pub root: std::path::PathBuf,
    /// Per-vault app state directory (drafts, seal).
    pub state_dir: std::path::PathBuf,
    _vault_dir: tempfile::TempDir,
    _state_dir: tempfile::TempDir,
}

fn build_vault(
    fixture: &VaultFixture,
) -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let vault_dir = tempfile::tempdir().expect("vault tempdir");
    let state_dir = tempfile::tempdir().expect("state tempdir");
    let root = vault_dir.path().join("notes");
    let vault = Vault::open(&root, state_dir.path()).expect("open vault");
    for (path, text) in &fixture.notes {
        NotesService::new(&vault)
            .write(path, text.as_bytes())
            .expect("write note");
    }
    let store = ConfigStore::open(vault.clone(), EnvSnapshot::default());
    let recents = RecentFilesService::new(&vault, &store);
    // Oldest first, one minute apart, so the newest-first order is unambiguous.
    let now = jiff::Timestamp::now();
    for (age, path) in fixture.recents.iter().enumerate().rev() {
        let minutes = i64::try_from(age).expect("small fixture");
        let at = now - jiff::SignedDuration::from_mins(minutes);
        recents.record(Some(path), at).expect("record recent");
    }
    (vault_dir, state_dir, root)
}

/// Opens the app shell on `fixture` at `size`, with the default keymap bound.
pub fn open_shell(cx: &mut TestAppContext, size: Size<Pixels>, fixture: VaultFixture) -> Shell {
    cx.update(gpui_kit::init);
    cx.update(frankmd_app::ui::actions::install_keymap);
    cx.update(frankmd_app::dialogs::media::init);
    let (vault_dir, state_dir, root) = build_vault(&fixture);
    let config = ShellConfig {
        vault_root: root.clone(),
        state_dir: state_dir.path().join("vault"),
        config_dir: state_dir.path().join("config"),
        env: EnvSnapshot::default(),
        open: fixture.open.map(str::to_owned),
        clock: Arc::new(frankmd_core::SystemClock),
        omarchy: None,
        watch: false,
        locale: Some("en".into()),
        // Deletes go to the Trash (ui-spec §5.13); never the user's real one in tests.
        trash_dir: Some(state_dir.path().join("trash")),
        global_config: Some(state_dir.path().join("global").join("config.toml")),
        app_prefs: None,
        encryption: None,
    };
    let (window, view) = super::harness::open_window(cx, size, move |window, cx| {
        cx.new(|cx| AppShell::new(config, window, cx))
    });
    let window: AnyWindowHandle = window.into();
    cx.run_until_parked();
    super::harness::in_window(cx, window, |_, _| ());
    Shell {
        window,
        view,
        root,
        state_dir: state_dir.path().join("vault"),
        _vault_dir: vault_dir,
        _state_dir: state_dir,
    }
}

impl Shell {
    /// Layout view-model.
    pub fn layout(&self, cx: &mut TestAppContext) -> LayoutState {
        self.view.read_with(cx, |s, _| s.layout().clone())
    }
    /// Focused region (from the shell's focus handles).
    pub fn focus_region(&self, cx: &mut TestAppContext) -> Option<FocusRegion> {
        let view = self.view.clone();
        super::harness::in_window(cx, self.window, move |window, cx| {
            view.read(cx).focused_region(window, cx)
        })
    }
    /// Open overlay.
    pub fn overlay(&self, cx: &mut TestAppContext) -> Option<OverlayKind> {
        self.view.read_with(cx, |s, _| s.overlay())
    }
    /// Visible banners, top to bottom.
    pub fn banners(&self, cx: &mut TestAppContext) -> Vec<BannerKind> {
        self.view.read_with(cx, |s, _| s.banners())
    }
    /// Save-state slot.
    pub fn save_state(&self, cx: &mut TestAppContext) -> SaveState {
        self.view.read_with(cx, |s, _| s.save_state())
    }
    /// Active note path.
    pub fn active_note(&self, cx: &mut TestAppContext) -> Option<String> {
        self.view.read_with(cx, |s, _| s.active_path())
    }
    /// Raise a banner through the shell's banner path (vault probe, storage
    /// fallback and content-loss conditions all end in it).
    pub fn simulate(&self, cx: &mut TestAppContext, condition: BannerKind) {
        self.view.update(cx, |s, cx| s.raise_banner(condition, cx));
        super::harness::in_window(cx, self.window, |_, _| ());
    }
    /// The per-vault UI state file (sidebar width, expansion, …), or "".
    pub fn ui_state_json(&self) -> String {
        std::fs::read_to_string(self._state_dir.path().join("vault").join("ui_state.json"))
            .unwrap_or_default()
    }
    /// Where deleted items went (the test Trash).
    pub fn trash_dir(&self) -> std::path::PathBuf {
        self._state_dir.path().join("trash")
    }
    /// The global settings file of this test.
    pub fn global_config(&self) -> std::path::PathBuf {
        self._state_dir.path().join("global").join("config.toml")
    }
    /// Current editor text.
    pub fn editor_text_of(&self, cx: &mut TestAppContext) -> String {
        self.view.read_with(cx, |s, cx| s.editor_text(cx))
    }
    /// Read a note from disk.
    pub fn read_note(&self, path: &str) -> Option<String> {
        let state = tempfile::tempdir().ok()?;
        let vault = Vault::open(&self.root, state.path()).ok()?;
        NotesService::new(&vault)
            .read(path)
            .ok()
            .map(|f| String::from_utf8_lossy(&f.bytes).into_owned())
    }
}
