//! Note attachments: audio/video/image links are never opened as notes; the
//! preview paperclip lists them and saves them through native prompts.

#![allow(missing_docs, clippy::unwrap_used)]
#![allow(clippy::disallowed_methods)]

extern crate gpui_kit as gpui;

mod support;

use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, point, px};
use support::harness::{self, WIDE};
use support::shell::{Shell, VaultFixture, open_shell};
use support::vm::OverlayKind;

fn click(cx: &mut TestAppContext, shell: &Shell, id: &'static str) {
    harness::in_window(cx, shell.window, |window, cx| window.click(id, cx));
}

fn settle(cx: &mut TestAppContext, shell: &Shell) {
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    harness::in_window(cx, shell.window, |_, _| ());
}

fn open(cx: &mut TestAppContext, shell: &Shell, path: &str) {
    let view = shell.view.clone();
    let path = path.to_owned();
    harness::in_window(cx, shell.window, move |window, cx| {
        view.update(cx, |shell, cx| {
            shell.open_argument(&path, window, cx);
        });
    });
}

#[gpui_kit::test]
fn audio_link_does_not_open_as_note_and_can_be_saved(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        WIDE,
        VaultFixture::sample_with(&[("with-audio.md", "[talk](images/talk.m4a)\n")]),
    );
    std::fs::create_dir_all(shell.root.join("images")).unwrap();
    let audio = b"fake m4a bytes\x00\x01\x02";
    std::fs::write(shell.root.join("images/talk.m4a"), audio).unwrap();

    open(cx, &shell, "with-audio.md");
    settle(cx, &shell);
    assert_eq!(shell.active_note(cx).as_deref(), Some("with-audio.md"));
    assert!(harness::is_shown(cx, shell.window, "preview-attachments"));

    harness::in_window(cx, shell.window, |window, cx| {
        window.click_at("preview-link-0-1", point(px(6.), px(6.)), cx);
    });
    assert_eq!(
        shell.overlay(cx),
        Some(OverlayKind::Dialog("attachments")),
        "clicking the audio link opens the attachment list"
    );
    assert_eq!(shell.active_note(cx).as_deref(), Some("with-audio.md"));
    harness::press(cx, shell.window, "escape");
    assert_eq!(shell.overlay(cx), None);

    click(cx, &shell, "preview-attachments");
    assert_eq!(shell.overlay(cx), Some(OverlayKind::Dialog("attachments")));
    assert_eq!(
        shell.active_note(cx).as_deref(),
        Some("with-audio.md"),
        "clicking the paperclip never opens the binary as a note"
    );
    assert!(harness::is_shown(cx, shell.window, "attachments-dialog"));
    assert!(harness::is_shown(cx, shell.window, "attachment-row-0"));

    let saved = shell.root.join("downloaded.m4a");
    click(cx, &shell, "attachment-save-0");
    cx.simulate_new_path_selection({
        let saved = saved.clone();
        move |_directory| Some(saved.clone())
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read(&saved).unwrap(), audio);

    let out = shell.root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    click(cx, &shell, "attachments-save-all");
    cx.simulate_path_prompt_response({
        let out = out.clone();
        move |_options| Some(vec![out.clone()])
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read(out.join("talk.m4a")).unwrap(), audio);
}
