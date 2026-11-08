//! Level-b (headless GPUI) tests of the media and AI dialogs: Insert image,
//! Insert video, the media library pane and the grammar check, over a real
//! temporary vault (docs/testing/ui-test-plan.md image_picker, drop_images,
//! video_dialog, library, ai_grammar rows marked b / a + b).
#![allow(missing_docs, clippy::unwrap_used, clippy::disallowed_methods)]

extern crate gpui_kit as gpui;

#[path = "editor_support/mod.rs"]
mod editor_support;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use frankmd_app::dialogs::ai::{GrammarDialog, GrammarEvent};
use frankmd_app::dialogs::media::{self, MediaContext, MediaEvent, MediaHub, MediaInsert};
use frankmd_app::editor::EditorSettings;
use frankmd_media::vm::picker::{LocalFile, Source};
use frankmd_media::vm::video::VideoTab;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};

/// A 1×1 PNG (real image bytes: uploads are content-sniffed).
const PNG: [u8; 70] = [
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 218, 99, 100, 96, 248, 95, 15, 0, 2,
    135, 1, 128, 235, 71, 186, 146, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    window: AnyWindowHandle,
    hub: Rc<MediaHub>,
    events: Rc<RefCell<Vec<MediaEvent>>>,
}

fn context(dir: &Path) -> MediaContext {
    let root = dir.join("notes");
    let state = dir.join("state");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    MediaContext {
        vault_root: root,
        state_dir: state,
        env: frankmd_config::EnvSnapshot::default(),
        encryption: std::sync::Arc::new(frankmd_crypto::Session::new()),
        global_config: None,
    }
}

/// A window whose root view is one of the hub's surfaces.
fn open<V: gpui_kit::Render>(
    cx: &mut TestAppContext,
    pick: fn(&MediaHub) -> Entity<V>,
    seed: impl FnOnce(&Path),
) -> Fixture {
    cx.update(|cx| {
        gpui_kit::init(cx);
        media::init(cx);
    });
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    seed(&ctx.vault_root);
    let root = ctx.vault_root.clone();
    let hub_cell: Rc<RefCell<Option<Rc<MediaHub>>>> = Rc::default();
    let events: Rc<RefCell<Vec<MediaEvent>>> = Rc::default();
    let (cell, sink) = (Rc::clone(&hub_cell), Rc::clone(&events));
    let window = cx.update(|cx| {
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1280.), px(800.)),
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let (window, _) = gpui_kit::open_window(options, cx, move |window, cx| {
            let hub = Rc::new(MediaHub::new(&ctx, window, cx).unwrap());
            for emitter in [
                hub.picker.clone().into_any(),
                hub.video.clone().into_any(),
                hub.library.clone().into_any(),
            ] {
                let sink = Rc::clone(&sink);
                if let Ok(picker) = emitter.clone().downcast::<media::ImagePickerDialog>() {
                    cx.subscribe(&picker, move |_, ev: &MediaEvent, _| {
                        sink.borrow_mut().push(ev.clone())
                    })
                    .detach();
                } else if let Ok(video) = emitter.clone().downcast::<media::VideoDialogView>() {
                    cx.subscribe(&video, move |_, ev: &MediaEvent, _| {
                        sink.borrow_mut().push(ev.clone())
                    })
                    .detach();
                } else if let Ok(lib) = emitter.downcast::<media::MediaLibraryPane>() {
                    cx.subscribe(&lib, move |_, ev: &MediaEvent, _| {
                        sink.borrow_mut().push(ev.clone())
                    })
                    .detach();
                }
            }
            let view = pick(&hub);
            *cell.borrow_mut() = Some(hub);
            view
        })
        .expect("open window");
        window
    });
    let hub = hub_cell.borrow().clone().unwrap();
    Fixture {
        _dir: dir,
        root,
        window,
        hub,
        events,
    }
}

impl Fixture {
    fn with<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&MediaHub, &mut gpui_kit::Window, &mut gpui_kit::App) -> R,
    ) -> R {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            f(&self.hub, window, cx)
        })
        .unwrap()
    }

    fn press(&self, cx: &mut TestAppContext, keys: &str) {
        self.with(cx, |_, window, cx| {
            for key in keys.split_whitespace() {
                window.press(key, cx);
            }
        });
        cx.run_until_parked();
    }

    fn shown(&self, cx: &mut TestAppContext, id: &'static str) -> bool {
        self.with(cx, |_, window, _| {
            window.try_find(id).is_some_and(|e| e.visible())
        })
    }

    /// Let background + tokio work finish (real threads), up to ~3 s.
    fn settle(&self, cx: &mut TestAppContext, done: impl Fn(&Self, &mut TestAppContext) -> bool) {
        cx.executor().allow_parking();
        for _ in 0..300 {
            cx.run_until_parked();
            if done(self, cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("background work did not finish");
    }

    fn inserted(&self) -> Option<MediaInsert> {
        self.events.borrow().iter().find_map(|e| match e {
            MediaEvent::Insert(i) => Some(i.clone()),
            _ => None,
        })
    }
}

fn pasted() -> LocalFile {
    frankmd_app::dialogs::media::files::pasted_image("image/png", PNG.to_vec(), 1_730_000_000_000)
}

// adapts: image_picker_controller.test.js:59 + drop_images row 6 (paste arrives pre-selected) and :124 (insert)
#[gpui_kit::test]
fn pasted_image_uploads_and_inserts_note_relative_markdown(cx: &mut TestAppContext) {
    let f = open(cx, |h| h.picker.clone(), |_| {});
    f.with(cx, |hub, window, cx| {
        hub.picker.update(cx, |p, cx| {
            p.open_with_files(Some("blog/post.md".into()), vec![pasted()], window, cx)
        });
    });
    assert!(f.shown(cx, "dialog-insert-image"));
    let name = f.with(cx, |hub, _, cx| {
        hub.picker.read(cx).vm().selected_name().map(str::to_owned)
    });
    assert_eq!(name.as_deref(), Some("pasted-image-1730000000000.png"));
    f.press(cx, "ctrl-enter");
    f.settle(cx, |f, _| f.inserted().is_some());
    let Some(MediaInsert::Image(markdown)) = f.inserted() else {
        panic!()
    };
    let re = regex_lite(&markdown);
    assert!(re, "{markdown}");
    let stored: Vec<_> = std::fs::read_dir(f.root.join("images")).unwrap().collect();
    assert_eq!(stored.len(), 1, "one upload");
    assert!(!f.with(cx, |hub, _, cx| hub.picker.read(cx).is_open()));
}

/// `![pasted image 1730000000000](../images/<YYYYmmdd_HHMMSS>_<8 hex>_pasted-image-1730000000000.png)`.
fn regex_lite(markdown: &str) -> bool {
    let Some(rest) = markdown.strip_prefix("![pasted image 1730000000000](../images/") else {
        return false;
    };
    let Some(name) = rest.strip_suffix(")") else {
        return false;
    };
    let parts: Vec<&str> = name.splitn(4, '_').collect();
    parts.len() == 4
        && parts[0].len() == 8
        && parts[1].len() == 6
        && parts[2].len() == 8
        && parts[3] == "pasted-image-1730000000000.png"
}

// adapts: image_picker_controller.test.js:75 (sources keyboard-navigable; Ctrl+1…7, §6.4) + Esc closes
#[gpui_kit::test]
fn source_rail_keys_and_escape(cx: &mut TestAppContext) {
    let f = open(
        cx,
        |h| h.picker.clone(),
        |root| {
            std::fs::create_dir_all(root.join("images")).unwrap();
            std::fs::write(root.join("images/a.png"), PNG).unwrap();
        },
    );
    f.with(cx, |hub, window, cx| {
        hub.picker
            .update(cx, |p, cx| p.open(Some("n.md".into()), window, cx))
    });
    assert!(f.shown(cx, "upload-dropzone"));
    f.press(cx, "ctrl-2");
    assert_eq!(
        f.with(cx, |h, _, cx| h.picker.read(cx).vm().source()),
        Source::Library
    );
    f.settle(cx, |f, cx| {
        f.with(cx, |h, _, cx| {
            !h.picker.read(cx).vm().library_cards().is_empty()
        })
    });
    assert!(f.shown(cx, "picker-card-library-images/a.png"));
    f.press(cx, "ctrl-tab");
    assert_eq!(
        f.with(cx, |h, _, cx| h.picker.read(cx).vm().source()),
        Source::Folder
    );
    f.press(cx, "ctrl-7");
    assert_eq!(
        f.with(cx, |h, _, cx| h.picker.read(cx).vm().source()),
        Source::Ai
    );
    assert!(
        f.shown(cx, "ai-setup"),
        "AI without an OpenRouter key shows the setup notice"
    );
    f.press(cx, "escape");
    assert!(!f.shown(cx, "dialog-insert-image"));
    assert!(f.events.borrow().contains(&MediaEvent::Closed));
}

// adapts: drop_images rows 3-5/8 (dropzone wiring; Enter opens the chooser) — portal is manual (c)
#[gpui_kit::test]
fn dropzone_is_focused_on_open(cx: &mut TestAppContext) {
    let f = open(cx, |h| h.picker.clone(), |_| {});
    f.with(cx, |hub, window, cx| {
        hub.picker.update(cx, |p, cx| p.open(None, window, cx))
    });
    let focused = f.with(cx, |_, window, _| {
        window.try_find("upload-dropzone").and_then(|e| e.focused())
    });
    assert_eq!(focused, Some(true));
}

// adapts: video_dialog_controller.test.js rows 4, 9, 10, 15 (open, tabs, confirm with nothing closes)
#[gpui_kit::test]
fn video_dialog_tabs_and_empty_confirm(cx: &mut TestAppContext) {
    let f = open(cx, |h| h.video.clone(), |_| {});
    f.with(cx, |hub, window, cx| {
        hub.video.update(cx, |v, cx| v.open(window, cx))
    });
    assert!(f.shown(cx, "dialog-insert-video"));
    assert!(f.shown(cx, "video-dropzone"));
    f.press(cx, "ctrl-tab");
    assert_eq!(
        f.with(cx, |h, _, cx| h.video.read(cx).vm().tab()),
        VideoTab::Url
    );
    assert!(f.shown(cx, "video-url"));
    f.press(cx, "ctrl-enter");
    assert!(
        !f.shown(cx, "dialog-insert-video"),
        "nothing detected: Insert just closes"
    );
    assert!(f.events.borrow().contains(&MediaEvent::Closed));
}

// adapts: video_dialog rows 31-35 (drop a file → upload → insert the returned URL)
#[gpui_kit::test]
fn video_upload_inserts_the_stored_url(cx: &mut TestAppContext) {
    let f = open(cx, |h| h.video.clone(), |_| {});
    let dir = tempfile::tempdir().unwrap();
    // A real MP4 header (ftyp box) so the content check accepts it.
    let mut mp4 = vec![
        0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0, 0, 2, 0,
    ];
    mp4.extend_from_slice(b"isomiso2avc1mp41");
    let clip = dir.path().join("clip.mp4");
    std::fs::write(&clip, &mp4).unwrap();
    f.with(cx, |hub, window, cx| {
        hub.video.update(cx, |v, cx| v.open(window, cx))
    });
    let file = frankmd_app::dialogs::media::files::describe(&clip).unwrap();
    f.with(cx, |hub, window, cx| {
        hub.video
            .update(cx, |v, cx| v.drop_local_files(vec![file], window, cx));
    });
    f.settle(cx, |f, cx| {
        f.with(cx, |h, _, cx| h.video.read(cx).vm().can_insert())
    });
    f.press(cx, "ctrl-enter");
    let Some(MediaInsert::Block(embed)) = f.inserted() else {
        panic!("{:?}", f.events.borrow())
    };
    assert!(
        embed.starts_with("<video controls class=\"video-player\">\n  <source src=\"videos/"),
        "{embed}"
    );
    assert!(embed.contains("type=\"video/mp4\""));
}

// adapts: library_controller.test.js row 1-2, app_library rows 1/5 (open lists once; I inserts and closes)
#[gpui_kit::test]
fn library_lists_and_inserts_with_the_keyboard(cx: &mut TestAppContext) {
    let f = open(
        cx,
        |h| h.library.clone(),
        |root| {
            std::fs::create_dir_all(root.join("images")).unwrap();
            std::fs::write(root.join("images/photo (one).png"), PNG).unwrap();
        },
    );
    f.with(cx, |hub, window, cx| {
        hub.library.update(cx, |l, cx| {
            l.open(Some("folder/sub/note.md".into()), window, cx)
        })
    });
    f.settle(cx, |f, cx| {
        f.with(cx, |h, _, cx| !h.library.read(cx).vm().cards().is_empty())
    });
    assert!(f.shown(cx, "media-library"));
    assert!(f.shown(cx, "library-card-images/photo (one).png"));
    f.press(cx, "i");
    assert_eq!(
        f.inserted(),
        Some(MediaInsert::Image(
            "![photo (one).png](../../images/photo%20%28one%29.png)".into()
        ))
    );
    assert!(f.events.borrow().contains(&MediaEvent::CloseLibrary));
}

// adapts: library_controller.test.js row 8 (Esc closes the sheet, then the library)
#[gpui_kit::test]
fn library_escape_chain(cx: &mut TestAppContext) {
    let f = open(
        cx,
        |h| h.library.clone(),
        |root| {
            std::fs::create_dir_all(root.join("images")).unwrap();
            std::fs::write(root.join("images/a.png"), PNG).unwrap();
        },
    );
    f.with(cx, |hub, window, cx| {
        hub.library
            .update(cx, |l, cx| l.open(Some("n.md".into()), window, cx))
    });
    f.settle(cx, |f, cx| {
        f.with(cx, |h, _, cx| !h.library.read(cx).vm().cards().is_empty())
    });
    f.press(cx, "enter");
    assert!(f.shown(cx, "media-library-detail"));
    f.press(cx, "escape");
    assert!(!f.shown(cx, "media-library-detail"));
    f.press(cx, "escape");
    assert!(f.events.borrow().contains(&MediaEvent::CloseLibrary));
}

// adapts: ai_grammar_controller.test.js:119 (not configured → setup notice) + security (.fed refused)
#[gpui_kit::test]
fn grammar_setup_notice_and_fed_refusal(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        media::init(cx);
    });
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let events: Rc<RefCell<Vec<GrammarEvent>>> = Rc::default();
    let sink = Rc::clone(&events);
    let (window, dialog) = cx.update(|cx| {
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1280.), px(800.)),
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let services = frankmd_app::dialogs::media::MediaServices::new(&ctx).unwrap();
        let (window, dialog) = gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| GrammarDialog::new(services, window, cx))
        })
        .unwrap();
        cx.subscribe(&dialog, move |_, ev: &GrammarEvent, _| {
            sink.borrow_mut().push(ev.clone())
        })
        .detach();
        (window, dialog)
    });
    cx.executor().allow_parking();
    for _ in 0..50 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.update_window(window, |_, window, cx| {
        dialog.update(cx, |d, cx| {
            d.start(Some(".fed".into()), "secret = 1".into(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        matches!(
            events.borrow().last(),
            Some(GrammarEvent::Status { error: true, .. })
        ),
        ".fed is never sent"
    );
    cx.update_window(window, |_, window, cx| {
        dialog.update(cx, |d, cx| {
            d.start(Some("a.md".into()), "Teh text".into(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let shown = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find("grammar-setup")
                .is_some_and(|e| e.visible())
        })
        .unwrap();
    assert!(shown, "no provider configured: setup notice");
    assert!(events.borrow().contains(&GrammarEvent::Show));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(events.borrow().contains(&GrammarEvent::Closed));
}

// FE-313: apply_insert lands in the real editor as one edit with FrankMD's newline rules.
#[gpui_kit::test]
fn apply_insert_into_the_editor(cx: &mut TestAppContext) {
    let f = editor_support::open(cx, "abc", EditorSettings::default());
    f.with(cx, |pane, window, cx| {
        pane.update(cx, |p, cx| p.set_selection(3..3, cx));
        assert!(media::apply_insert(
            pane,
            &MediaInsert::Image("![a](b)".into()),
            None,
            window,
            cx
        ));
    });
    cx.run_until_parked();
    assert_eq!(f.text(cx), "abc\n![a](b)");
    f.with(cx, |pane, window, cx| {
        assert!(media::apply_insert(
            pane,
            &MediaInsert::Block("<video>".into()),
            None,
            window,
            cx
        ));
    });
    cx.run_until_parked();
    assert_eq!(f.text(cx), "abc\n![a](b)\n\n<video>");
}
