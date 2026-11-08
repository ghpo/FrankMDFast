//! Level-b tests: search in files (ui-spec §5.3, F.16), back/forward and the
//! Ctrl+Tab switcher (§6.1, §5.2), changed/deleted-on-disk banners (§5.12)
//! with Compare…, splitters (§3.1) and the Settings scope/rows (§5.16).
#![allow(missing_docs, clippy::unwrap_used)]
// Fixtures simulate edits made outside FrankMD (that is what the watcher reports).
#![allow(clippy::disallowed_methods)]

extern crate gpui_kit as gpui;

mod support;

use std::time::Duration;

use frankmd_core::RelPath;
use frankmd_watch::WatchEvent;
use frankmd_workspace::content_search::SearchStatus;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use support::harness::{self, WIDE};
use support::shell::{Shell, VaultFixture, open_shell};
use support::vm::{BannerKind, FocusRegion, OverlayKind};

fn click(cx: &mut TestAppContext, shell: &Shell, id: &'static str) {
    harness::in_window(cx, shell.window, |window, cx| window.click(id, cx));
}

fn settle(cx: &mut TestAppContext, shell: &Shell) {
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    harness::in_window(cx, shell.window, |_, _| ());
}

fn external(cx: &mut TestAppContext, shell: &Shell, event: WatchEvent) {
    let view = shell.view.clone();
    harness::in_window(cx, shell.window, move |window, cx| {
        view.update(cx, |s, cx| s.external_event(event, window, cx))
    });
}

fn write_on_disk(shell: &Shell, path: &str, text: &str) {
    std::fs::write(shell.root.join(path), text).unwrap();
}

// ---- search in files ----------------------------------------------------------------------

// adapts: test/system/notes_test.rb:233 "content search finds text and opens the file at the line"
#[gpui_kit::test]
fn search_finds_matches_and_opens_at_the_line(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2 ctrl-shift-f");
    assert_eq!(shell.overlay(cx), Some(OverlayKind::SearchInFiles));
    assert!(
        harness::has_focus(cx, w, "search-input") || harness::is_shown(cx, w, "search-overlay")
    );
    harness::type_text(cx, w, "sh.p");
    settle(cx, &shell);
    let status = shell.view.read_with(cx, |s, _| s.content_search().status());
    assert_eq!(
        status,
        SearchStatus::Count {
            matches: 1,
            notes: 1,
            truncated: false
        }
    );
    assert!(harness::is_shown(
        cx,
        w,
        "search-result-projects/Launch list.md-3"
    ));
    harness::press(cx, w, "enter");
    assert_eq!(shell.overlay(cx), None);
    assert_eq!(
        shell.active_note(cx).as_deref(),
        Some("projects/Launch list.md")
    );
    assert_eq!(shell.focus_region(cx), Some(FocusRegion::Editor));
}

// ui-spec §5.3: `.*` off searches the literal text; Aa makes it case-sensitive.
#[gpui_kit::test]
fn search_toggles_change_matching(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2 ctrl-shift-f");
    harness::type_text(cx, w, "sh.p");
    settle(cx, &shell);
    click(cx, &shell, "search-regex");
    settle(cx, &shell);
    assert_eq!(
        shell.view.read_with(cx, |s, _| s.content_search().status()),
        SearchStatus::NoMatches
    );
    click(cx, &shell, "search-regex");
    click(cx, &shell, "search-case");
    let input = shell
        .view
        .read_with(cx, |s, _| s.content_search().query().to_owned());
    assert_eq!(input, "sh.p");
    harness::press(cx, w, "ctrl-a");
    harness::type_text(cx, w, "INBOX");
    settle(cx, &shell);
    assert_eq!(
        shell.view.read_with(cx, |s, _| s.content_search().status()),
        SearchStatus::NoMatches
    );
    click(cx, &shell, "search-case");
    settle(cx, &shell);
    assert!(matches!(
        shell.view.read_with(cx, |s, _| s.content_search().status()),
        SearchStatus::Count { .. }
    ));
}

// adapts: content_search_controller.test.js "escape closes" + §5.3 focus restore
#[gpui_kit::test]
fn search_escape_restores_focus(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2 ctrl-shift-f escape");
    assert_eq!(shell.overlay(cx), None);
    assert_eq!(shell.focus_region(cx), Some(FocusRegion::Editor));
}

// ---- back / forward, Ctrl+Tab ------------------------------------------------------------

// ui-spec §6.1 + ui-test-plan workspace::navigation: back past the first entry → empty state.
#[gpui_kit::test]
fn back_and_forward_walk_the_note_history(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    click(cx, &shell, "explorer-row-Inbox.md");
    assert_eq!(shell.active_note(cx).as_deref(), Some("Inbox.md"));
    harness::press(cx, w, "ctrl-alt-left");
    assert_eq!(
        shell.active_note(cx).as_deref(),
        Some("blog/2026/Project brief.md")
    );
    harness::press(cx, w, "ctrl-alt-right");
    assert_eq!(shell.active_note(cx).as_deref(), Some("Inbox.md"));
    harness::press(cx, w, "ctrl-alt-left ctrl-alt-left");
    assert_eq!(
        shell.active_note(cx),
        None,
        "past the first note is the empty state"
    );
    assert!(harness::is_shown(cx, w, "empty-state"));
}

// ui-spec §5.2: Ctrl+Tab lists this session's MRU notes; it starts on the previous note.
#[gpui_kit::test]
fn ctrl_tab_switches_to_the_previous_note(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    click(cx, &shell, "explorer-row-Inbox.md");
    harness::press(cx, w, "ctrl-2 ctrl-tab");
    assert_eq!(
        shell.overlay(cx),
        Some(OverlayKind::Dialog("recent-switcher"))
    );
    assert!(harness::is_shown(
        cx,
        w,
        "recent-switcher-row-blog/2026/Project brief.md"
    ));
    harness::press(cx, w, "enter");
    assert_eq!(shell.overlay(cx), None);
    assert_eq!(
        shell.active_note(cx).as_deref(),
        Some("blog/2026/Project brief.md")
    );
    // Ctrl+Tab twice wraps back to the current note; Esc cancels.
    harness::press(cx, w, "ctrl-tab ctrl-tab escape");
    assert_eq!(shell.overlay(cx), None);
    assert_eq!(
        shell.active_note(cx).as_deref(),
        Some("blog/2026/Project brief.md")
    );
}

// ---- disk banners (§5.12) ----------------------------------------------------------------

// ui-spec §5.12 "Changed on disk" (new, FE-120): a dirty buffer gets the banner; Compare…
// shows disk vs editor; Keep my version keeps typing safe.
#[gpui_kit::test]
fn external_change_under_unsaved_edits_raises_the_banner_and_compare(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2");
    harness::type_text(cx, w, "local edit ");
    write_on_disk(
        &shell,
        "blog/2026/Project brief.md",
        "# Changed elsewhere\n",
    );
    let path = RelPath::parse("blog/2026/Project brief.md").unwrap();
    external(
        cx,
        &shell,
        WatchEvent::VaultChanged {
            paths: vec![path],
            removed: vec![],
        },
    );
    assert!(shell.banners(cx).contains(&BannerKind::ChangedOnDisk));
    click(cx, &shell, "banner-changed-on-disk-compare");
    assert_eq!(shell.overlay(cx), Some(OverlayKind::Dialog("recovery")));
    assert!(harness::is_shown(cx, w, "recovery-keep-mine"));
    click(cx, &shell, "recovery-keep-mine");
    assert_eq!(shell.overlay(cx), None);
    assert!(!shell.banners(cx).contains(&BannerKind::ChangedOnDisk));
    assert!(shell.editor_text_of(cx).contains("local edit"));
}

// ui-spec §5.12 "Changed on disk" → Reload replaces the buffer with the file.
#[gpui_kit::test]
fn changed_on_disk_reload_takes_the_file(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2");
    harness::type_text(cx, w, "local edit ");
    write_on_disk(
        &shell,
        "blog/2026/Project brief.md",
        "# Changed elsewhere\n",
    );
    let path = RelPath::parse("blog/2026/Project brief.md").unwrap();
    external(
        cx,
        &shell,
        WatchEvent::VaultChanged {
            paths: vec![path],
            removed: vec![],
        },
    );
    click(cx, &shell, "banner-changed-on-disk-reload");
    assert!(!shell.banners(cx).contains(&BannerKind::ChangedOnDisk));
}

// ui-spec §5.12 "Deleted on disk" (FE-024): Save again recreates the note.
#[gpui_kit::test]
fn deleted_on_disk_offers_save_again(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    std::fs::remove_file(shell.root.join("blog/2026/Project brief.md")).unwrap();
    let path = RelPath::parse("blog/2026/Project brief.md").unwrap();
    external(
        cx,
        &shell,
        WatchEvent::VaultChanged {
            paths: vec![path.clone()],
            removed: vec![path],
        },
    );
    assert!(shell.banners(cx).contains(&BannerKind::DeletedOnDisk));
    assert!(harness::is_shown(
        cx,
        shell.window,
        "banner-deleted-on-disk-save-again"
    ));
    click(cx, &shell, "banner-deleted-on-disk-close");
    assert_eq!(shell.active_note(cx), None);
}

// ---- splitters (§3.1) -----------------------------------------------------------------------

// ui-spec §3.1: a focused splitter moves by 16 px with ←/→; double-click resets; persisted.
#[gpui_kit::test]
fn sidebar_splitter_resizes_and_persists(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    let before = shell.view.read_with(cx, |s, _| s.sidebar_width());
    click(cx, &shell, "splitter-sidebar");
    harness::press(cx, w, "right right");
    let after = shell.view.read_with(cx, |s, _| s.sidebar_width());
    assert!((after - before - 32.0).abs() < 0.5, "{before} → {after}");
    assert!(
        shell.ui_state_json().contains(&format!("{after}")),
        "{}",
        shell.ui_state_json()
    );
    harness::in_window(cx, w, |window, cx| {
        window.double_click("splitter-sidebar", cx)
    });
    let reset = shell.view.read_with(cx, |s, _| s.sidebar_width());
    assert!((reset - before).abs() < 0.5);
}

// ui-spec §3.1: preview splitter keeps both panes above their minimums.
#[gpui_kit::test]
fn preview_splitter_is_clamped(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    click(cx, &shell, "splitter-preview");
    for _ in 0..60 {
        harness::press(cx, w, "right");
    }
    let (ratio, panes) = shell
        .view
        .read_with(cx, |s, _| (s.preview_ratio(), 1280.0 - s.sidebar_width()));
    assert!(
        ratio * panes >= 279.0,
        "preview keeps 280 px: {}",
        ratio * panes
    );
}

// ---- settings: Media / Integrations / Hugo and the scope toggle (§5.16) -------------------

#[gpui_kit::test]
fn settings_has_media_integrations_and_hugo_rows(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2 ctrl-,");
    click(cx, &shell, "settings-category-media");
    for id in [
        "setting-row-provider",
        "setting-row-images_path",
        "setting-row-s3_secret_key",
    ] {
        assert!(harness::is_shown(cx, w, id), "{id}");
    }
    click(cx, &shell, "settings-category-integrations");
    assert!(harness::is_shown(cx, w, "setting-row-openai_key"));
    assert!(harness::is_shown(cx, w, "setting-openai_key-state"));
    click(cx, &shell, "settings-category-hugo");
    click(cx, &shell, "setting-path_style-flat");
    assert!(
        std::fs::read_to_string(shell.root.join(".fed"))
            .unwrap()
            .contains("hugo_path_style = flat")
    );
}

// ui-spec §5.16: secrets are never echoed: "Set" + a masked input that starts empty.
#[gpui_kit::test]
fn secrets_are_masked_and_replaced_through_an_empty_input(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    std::fs::write(
        shell.root.join(".fed"),
        "openai_api_key = sk-secret-value\n",
    )
    .unwrap();
    external(cx, &shell, WatchEvent::ConfigChanged);
    let w = shell.window;
    harness::press(cx, w, "ctrl-2 ctrl-,");
    click(cx, &shell, "settings-category-integrations");
    click(cx, &shell, "setting-openai_key-change");
    let value = harness::in_window(cx, w, |window, _| {
        window
            .find("setting-openai_key")
            .value()
            .unwrap_or_default()
            .to_string()
    });
    assert!(
        !value.contains("sk-secret"),
        "the stored secret is not put in the input: {value:?}"
    );
    harness::in_window(cx, w, |window, cx| window.click("setting-openai_key", cx));
    harness::type_text(cx, w, "sk-new");
    harness::press(cx, w, "enter");
    let fed = std::fs::read_to_string(shell.root.join(".fed")).unwrap();
    assert!(fed.contains("openai_api_key = sk-new"), "{fed}");
}

// ui-spec §5.16 scope toggle: "All vaults" writes the global file, `.fed` untouched.
#[gpui_kit::test]
fn global_scope_writes_the_global_config(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2 ctrl-,");
    click(cx, &shell, "settings-scope-global");
    click(cx, &shell, "settings-category-hugo");
    click(cx, &shell, "setting-path_style-flat");
    let global = std::fs::read_to_string(shell.global_config()).unwrap();
    assert!(global.contains("hugo_path_style"), "{global}");
    // `.fed` may carry the template's commented `# hugo_path_style = …` line, never a value.
    let fed = std::fs::read_to_string(shell.root.join(".fed")).unwrap_or_default();
    assert!(
        !fed.lines()
            .any(|l| l.trim_start().starts_with("hugo_path_style")),
        "{fed}"
    );
}

// ---- recovery dialog (§5.11) ---------------------------------------------------------------

fn recovery_request(
    path: &str,
    file: &str,
    draft: &str,
) -> frankmd_workspace::autosave::RecoveryRequest {
    frankmd_workspace::autosave::RecoveryRequest {
        path: path.into(),
        server_content: file.into(),
        backup_content: draft.into(),
        backup_timestamp: 1_793_800_920_000.0,
        source: frankmd_workspace::autosave::RecoverySource::Draft,
        draft_revision: None,
        conflict_id: None,
    }
}

// adapts: test/system/draft_recovery_test.rb:12 "shows the recovery diff and restores" +
// §5.11 (changed: word diff side by side; Esc does nothing; Keep both is new)
#[gpui_kit::test]
fn recovery_dialog_shows_the_diff_and_keep_both_writes_a_copy(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    let request = recovery_request(
        "blog/2026/Project brief.md",
        "# Project brief\n\nBody.\n",
        "# Project brief\n\nNew body.\n",
    );
    let view = shell.view.clone();
    harness::in_window(cx, w, move |window, cx| {
        view.update(cx, |s, cx| s.open_recovery(request, window, cx))
    });
    assert!(harness::is_shown(cx, w, "dialog-recovery"));
    assert!(harness::is_shown(cx, w, "recovery-file"));
    assert!(harness::is_shown(cx, w, "recovery-draft"));
    harness::press(cx, w, "escape");
    assert_eq!(
        shell.overlay(cx),
        Some(OverlayKind::Dialog("recovery")),
        "Esc does nothing (FE-146)"
    );
    click(cx, &shell, "recovery-keep-both");
    assert_eq!(shell.overlay(cx), None);
    let copies: Vec<String> = std::fs::read_dir(shell.root.join("blog/2026"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("Project brief (draft "))
        .collect();
    assert_eq!(copies.len(), 1, "{copies:?}");
    let copy = shell
        .read_note(&format!("blog/2026/{}", copies[0]))
        .unwrap();
    assert_eq!(copy, "# Project brief\n\nNew body.\n");
}

// §5.11: below 900 px the diff is unified.
#[gpui_kit::test]
fn recovery_diff_is_unified_when_narrow(cx: &mut TestAppContext) {
    let shell = open_shell(cx, support::harness::MEDIUM, VaultFixture::sample());
    let w = shell.window;
    harness::resize(cx, w, 860.);
    let request = recovery_request("blog/2026/Project brief.md", "a b c", "a x c");
    let view = shell.view.clone();
    harness::in_window(cx, w, move |window, cx| {
        view.update(cx, |s, cx| s.open_recovery(request, window, cx))
    });
    assert!(harness::is_shown(cx, w, "recovery-unified"));
    click(cx, &shell, "recovery-restore-draft");
    assert_eq!(shell.overlay(cx), None);
}

// Settings › Remote explains how to open this vault from another device.
#[gpui_kit::test]
fn settings_explains_remote_access(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2 ctrl-,");
    click(cx, &shell, "settings-category-remote");
    for id in [
        "settings-remote",
        "settings-remote-bind-tailscale",
        "settings-remote-bind-loopback",
        "settings-remote-headless",
        "settings-remote-token-input",
        "settings-remote-token-generate",
        "settings-remote-token-save",
        "settings-remote-token-copy",
        "settings-remote-url",
        "settings-remote-copy-url",
    ] {
        // Lower rows live below the fold; existence is what matters here.
        let present = harness::in_window(cx, w, |window, _| window.try_find(id).is_some());
        assert!(present, "{id}");
    }
}

// Settings › Remote can generate and persist an access token without editing .fed.
#[gpui_kit::test]
fn settings_can_generate_and_save_a_remote_token(cx: &mut TestAppContext) {
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2 ctrl-,");
    click(cx, &shell, "settings-category-remote");
    click(cx, &shell, "settings-remote-token-generate");
    click(cx, &shell, "settings-remote-token-save");

    let fed = std::fs::read_to_string(shell.root.join(".fed")).expect("read .fed");
    let token = fed
        .lines()
        .find_map(|line| line.strip_prefix("auth_token = "))
        .expect("auth_token written");
    assert_eq!(token.trim().len(), 64, "{fed}");
    assert!(token.trim().chars().all(|c| c.is_ascii_hexdigit()), "{fed}");
}

// Settings › Encryption can create the vault key (and persist the seal).
#[gpui_kit::test]
fn settings_can_create_an_encryption_key(cx: &mut TestAppContext) {
    const PASSWORD: &str = "uma senha longa e unica para o teste";
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    harness::press(cx, w, "ctrl-2 ctrl-,");
    click(cx, &shell, "settings-category-encryption");
    click(cx, &shell, "encryption-password");
    harness::type_text(cx, w, PASSWORD);
    click(cx, &shell, "encryption-confirm");
    harness::type_text(cx, w, PASSWORD);
    click(cx, &shell, "encryption-create");
    let seal =
        std::fs::read_to_string(shell.state_dir.join("encryption.json")).expect("seal written");
    let seal = frankmd_crypto::SealFile::decode(&seal).expect("seal json");
    assert!(seal.unlock(PASSWORD).is_ok());
    assert!(seal.unlock("wrong password").is_err());

    // Per-note encrypt/decrypt through the registry command (same path as the
    // tree context menu).
    click(cx, &shell, "settings-close");
    let active = shell
        .view
        .read_with(cx, |shell, _| shell.active_path())
        .expect("note open");
    let run = |cx: &mut TestAppContext, id: &'static str| {
        let view = shell.view.clone();
        harness::in_window(cx, shell.window, move |window, cx| {
            view.update(cx, |s, cx| s.run_command(id, window, cx))
        });
    };
    run(cx, "file.encrypt");
    let encrypted = std::fs::read(shell.root.join(&active)).expect("encrypted file");
    assert!(frankmd_crypto::is_note_encrypted(&encrypted));
    run(cx, "file.decrypt");
    let plain = std::fs::read(shell.root.join(&active)).expect("plain file");
    assert!(!frankmd_crypto::is_note_encrypted(&plain));
}

fn create_encryption_key_for_test(cx: &mut TestAppContext, shell: &Shell, password: &str) {
    harness::press(cx, shell.window, "ctrl-2 ctrl-,");
    click(cx, shell, "settings-category-encryption");
    click(cx, shell, "encryption-password");
    harness::type_text(cx, shell.window, password);
    click(cx, shell, "encryption-confirm");
    harness::type_text(cx, shell.window, password);
    click(cx, shell, "encryption-create");
    cx.run_until_parked();
}

fn run_shell_command(cx: &mut TestAppContext, shell: &Shell, id: &'static str) {
    let view = shell.view.clone();
    harness::in_window(cx, shell.window, move |window, cx| {
        view.update(cx, |s, cx| s.run_command(id, window, cx))
    });
}

#[gpui_kit::test]
fn settings_export_and_import_seal_file(cx: &mut TestAppContext) {
    const PASSWORD: &str = "uma senha longa e unica para o teste";
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    create_encryption_key_for_test(cx, &shell, PASSWORD);

    let exported = shell.state_dir.join("exported-seal.json");
    click(cx, &shell, "encryption-export-file");
    cx.simulate_new_path_selection({
        let exported = exported.clone();
        move |_directory| Some(exported.clone())
    });
    cx.run_until_parked();
    let exported_text = std::fs::read_to_string(&exported).expect("seal exported");
    let exported_seal = frankmd_crypto::SealFile::decode(&exported_text).expect("seal json");

    click(cx, &shell, "encryption-import-file");
    cx.simulate_path_prompt_response({
        let exported = exported.clone();
        move |_options| Some(vec![exported.clone()])
    });
    cx.run_until_parked();
    assert!(
        harness::is_shown(cx, w, "encryption-import-confirm"),
        "existing seal asks for confirmation"
    );
    click(cx, &shell, "encryption-import-confirm");
    cx.run_until_parked();

    let state_text =
        std::fs::read_to_string(shell.state_dir.join("encryption.json")).expect("state seal");
    assert_eq!(
        frankmd_crypto::SealFile::decode(&state_text).unwrap(),
        exported_seal
    );
}

#[gpui_kit::test]
fn locked_note_shows_padlock_and_unlocks(cx: &mut TestAppContext) {
    const PASSWORD: &str = "uma senha longa e unica para o teste";
    let shell = open_shell(cx, WIDE, VaultFixture::sample());
    let w = shell.window;
    create_encryption_key_for_test(cx, &shell, PASSWORD);
    click(cx, &shell, "settings-close");

    let active = shell
        .view
        .read_with(cx, |shell, _| shell.active_path())
        .expect("note open");
    run_shell_command(cx, &shell, "file.encrypt");
    cx.run_until_parked();
    assert!(shell.view.read_with(cx, |shell, _| {
        shell.active_path().as_deref() == Some(active.as_str())
    }));
    let row_lock: &'static str = Box::leak(format!("explorer-lock-{active}").into_boxed_str());
    let lock_present = harness::in_window(cx, w, |window, _| window.try_find(row_lock).is_some());
    assert!(lock_present, "tree row shows a padlock");

    // Locking turns the open encrypted note into the padlock view.
    run_shell_command(cx, &shell, "app.encryption_toggle");
    cx.run_until_parked();
    assert!(harness::is_shown(cx, w, "locked-note"));
    assert!(
        shell
            .view
            .read_with(cx, |shell, _| shell.active_path())
            .is_none()
    );

    // The dock button opens the quick unlock dialog; the note reopens.
    run_shell_command(cx, &shell, "app.encryption_toggle");
    assert!(harness::is_shown(cx, w, "unlock-dialog"));
    click(cx, &shell, "unlock-password");
    harness::type_text(cx, w, PASSWORD);
    click(cx, &shell, "unlock-confirm");
    cx.run_until_parked();
    assert!(!harness::is_shown(cx, w, "locked-note"));
    assert_eq!(
        shell.view.read_with(cx, |shell, _| shell.active_path()),
        Some(active)
    );
}
