//! Media library pane (ui-spec §5.15): replaces editor + preview while open.
//! Cards with usage, Insert / Copy path / Rename (S3) / Delete, a filter,
//! and a right-side detail sheet with the "Used in" list.

use crate::scale::px;
use std::sync::Arc;

use frankmd_media::MediaKind;
use frankmd_media::library::{Library, UsageResponse};
use frankmd_media::vm::Text;
use frankmd_media::vm::library::{
    Category, LibraryCard, LibraryEffect, LibraryPrompt, LoadState, MediaLibraryView, UsageLabel,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::MediaEvent;
use super::services::MediaServices;
use super::task::{Abort, spawn_net};
use super::widgets::{self, RemoteThumbs, t, tr};
use crate::preview::remote::RemoteImages;

/// Key context of the library pane.
pub const LIBRARY_CONTEXT: &str = "MediaLibrary";

actions!(
    media_library,
    [
        /// → next card.
        Next,
        /// ← previous card.
        Prev,
        /// ↓ next row.
        NextRow,
        /// ↑ previous row.
        PrevRow,
        /// Enter: detail sheet.
        Details,
        /// I: insert the focused card.
        InsertFocused,
        /// Delete: delete the focused card.
        DeleteFocused,
        /// /: focus the filter.
        FocusSearch,
        /// Esc: prompt → sheet → library.
        Close,
    ]
);

/// Grid columns assumed for Up/Down.
const COLUMNS: usize = 4;

pub(crate) fn init(cx: &mut App) {
    let ctx = Some(LIBRARY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("right", Next, ctx),
        KeyBinding::new("left", Prev, ctx),
        KeyBinding::new("down", NextRow, ctx),
        KeyBinding::new("up", PrevRow, ctx),
        KeyBinding::new("enter", Details, ctx),
        KeyBinding::new("i", InsertFocused, ctx),
        KeyBinding::new("delete", DeleteFocused, ctx),
        KeyBinding::new("/", FocusSearch, ctx),
        KeyBinding::new("escape", Close, ctx),
    ]);
    super::video::bind_keys(cx);
}

/// The media library view.
pub struct MediaLibraryPane {
    services: Arc<MediaServices>,
    vm: MediaLibraryView,
    note: Option<String>,
    focus: FocusHandle,
    focused: usize,
    filter: Entity<InputState>,
    rename: Entity<InputState>,
    remote: RemoteImages,
    detail_dims: Option<(String, Option<(u32, u32)>)>,
    jobs: Vec<(Option<Abort>, Task<()>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MediaEvent> for MediaLibraryPane {}

impl Focusable for MediaLibraryPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RemoteThumbs for MediaLibraryPane {
    fn remote(&mut self) -> &mut RemoteImages {
        &mut self.remote
    }

    fn remote_done(&mut self, _url: &str, _ok: bool, _cx: &mut Context<Self>) {}
}

impl MediaLibraryPane {
    /// Build (closed).
    pub fn new(services: Arc<MediaServices>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = t(cx, "media_dialogs.filter_media");
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let rename = cx.new(|cx| InputState::new(window, cx));
        let subs = vec![
            cx.subscribe_in(&filter, window, |this, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.vm.set_filter(&input.read(cx).value());
                    this.focused = 0;
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &rename,
                window,
                |this, input, ev: &InputEvent, window, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        let name = input.read(cx).value().to_string();
                        let effects = this.vm.accept_prompt(&name);
                        this.run(effects, window, cx);
                    }
                },
            ),
        ];
        let root = services.notes_root().to_path_buf();
        Self {
            services,
            vm: MediaLibraryView::new(root),
            note: None,
            focus: cx.focus_handle(),
            focused: 0,
            filter,
            rename,
            remote: RemoteImages::default(),
            detail_dims: None,
            jobs: Vec::new(),
            _subscriptions: subs,
        }
    }

    /// The view model.
    pub fn vm(&self) -> &MediaLibraryView {
        &self.vm
    }

    fn media_encrypted(&self, path: &str) -> bool {
        let abs = self.services.notes_root().join(path);
        let Ok(mut file) = std::fs::File::open(abs) else {
            return false;
        };
        use std::io::Read as _;
        let mut prefix = [0u8; 11];
        let n = file.read(&mut prefix).unwrap_or(0);
        prefix[..n].starts_with(frankmd_crypto::MEDIA_MAGIC)
    }

    fn toggle_media_encryption(
        &mut self,
        path: &str,
        encrypt: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.services.encryption.is_unlocked() {
            cx.emit(MediaEvent::Status {
                text: t(cx, "settings_sheet.encryption.locked_hint"),
                error: true,
            });
            return;
        }
        let abs = self.services.notes_root().join(path);
        let result = if encrypt {
            crate::encryption::encrypt_media_file(&self.services.encryption, &abs)
        } else {
            crate::encryption::decrypt_media_file(&self.services.encryption, &abs)
        };
        match result {
            Ok(()) => {
                let key = if encrypt {
                    "library.media_encrypted"
                } else {
                    "library.media_decrypted"
                };
                cx.emit(MediaEvent::Status {
                    text: t(cx, key),
                    error: false,
                });
                let effects = self.vm.open();
                self.run(effects, window, cx);
            }
            Err(err) => {
                cx.emit(MediaEvent::Status {
                    text: SharedString::from(format!("{err:#}")),
                    error: true,
                });
            }
        }
        cx.notify();
    }

    /// Open (lists media once per open). `note` = the open note's vault path.
    pub fn open(&mut self, note: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.note = note;
        let effects = self.vm.open();
        self.focused = 0;
        self.focus.focus(window, cx);
        self.run(effects, window, cx);
    }

    /// The open note changed while the library is open.
    pub fn set_note(&mut self, note: Option<String>) {
        self.note = note;
    }

    /// Close (the shell restores the editor).
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if !self.vm.is_open() {
            return;
        }
        self.vm.close();
        self.jobs.clear();
        cx.emit(MediaEvent::CloseLibrary);
        cx.notify();
    }

    fn run(&mut self, effects: Vec<LibraryEffect>, window: &mut Window, cx: &mut Context<Self>) {
        for effect in effects {
            let services = Arc::clone(&self.services);
            match effect {
                LibraryEffect::Load { generation } => {
                    let (abort, work) = spawn_net(cx, async move {
                        Library::new(&services.vault, &services.api.env)
                            .with_recovery(&services.api.recovery)
                            .list()
                            .await
                            .map_err(|e| {
                                e.i18n_key()
                                    .map_or(Text::key("library.load_failed"), Text::key)
                            })
                    });
                    let task = cx.spawn(async move |this, cx| {
                        let result = work
                            .await
                            .unwrap_or_else(|| Err(Text::key("library.load_failed")));
                        this.update(cx, |this, cx| {
                            this.vm.load_result(generation, result);
                            cx.notify();
                        })
                        .ok();
                    });
                    self.jobs.push((abort, task));
                }
                LibraryEffect::LoadUsage { generation } => {
                    let work = cx.background_executor().spawn(async move {
                        match Library::new(&services.vault, &services.api.env)
                            .usage(&services.usage, None)
                        {
                            Ok(UsageResponse::Counts(counts)) => Ok(counts),
                            _ => Err(Text::key("library.usage_scan_failed")),
                        }
                    });
                    let task = cx.spawn(async move |this, cx| {
                        let result = work.await;
                        this.update(cx, |this, cx| {
                            this.vm.usage_result(generation, result);
                            cx.notify();
                        })
                        .ok();
                    });
                    self.jobs.push((None, task));
                }
                LibraryEffect::LoadUsageNotes { path } => {
                    let key = path.clone();
                    let work = cx.background_executor().spawn(async move {
                        match Library::new(&services.vault, &services.api.env)
                            .usage(&services.usage, Some(&path))
                        {
                            Ok(UsageResponse::Notes(notes)) => Ok(notes),
                            _ => Err(Text::key("library.usage_scan_failed")),
                        }
                    });
                    let task = cx.spawn(async move |this, cx| {
                        let result = work.await;
                        this.update(cx, |this, cx| {
                            this.vm.usage_notes_result(&key, result);
                            cx.notify();
                        })
                        .ok();
                    });
                    self.jobs.push((None, task));
                }
                LibraryEffect::WriteClipboard(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    let effects = self.vm.copy_result(true);
                    self.run(effects, window, cx);
                }
                LibraryEffect::Delete { path } => {
                    let key = path.clone();
                    let (abort, work) = spawn_net(cx, async move {
                        Library::new(&services.vault, &services.api.env)
                            .with_recovery(&services.api.recovery)
                            .delete(&path)
                            .await
                            .map_err(|e| {
                                e.i18n_key()
                                    .map_or(Text::key("library.delete_failed"), Text::key)
                            })
                    });
                    let task = cx.spawn_in(window, async move |this, cx| {
                        let result = work
                            .await
                            .unwrap_or_else(|| Err(Text::key("library.delete_failed")));
                        this.update_in(cx, |this, window, cx| {
                            let effects = this.vm.delete_result(&key, result);
                            this.run(effects, window, cx);
                        })
                        .ok();
                    });
                    self.jobs.push((abort, task));
                }
                LibraryEffect::Rename { path, name } => {
                    let key = path.clone();
                    let (abort, work) = spawn_net(cx, async move {
                        Library::new(&services.vault, &services.api.env)
                            .with_recovery(&services.api.recovery)
                            .rename(&path, &name)
                            .await
                            .map_err(|e| {
                                e.i18n_key()
                                    .map_or(Text::key("library.rename_failed"), Text::key)
                            })
                    });
                    let task = cx.spawn_in(window, async move |this, cx| {
                        let result = work
                            .await
                            .unwrap_or_else(|| Err(Text::key("library.rename_failed")));
                        this.update_in(cx, |this, window, cx| {
                            let effects = this.vm.rename_result(&key, result);
                            this.run(effects, window, cx);
                        })
                        .ok();
                    });
                    self.jobs.push((abort, task));
                }
                LibraryEffect::Insert(insert) => {
                    self.jobs.clear();
                    cx.emit(MediaEvent::Insert(insert));
                    cx.emit(MediaEvent::CloseLibrary);
                }
                LibraryEffect::OpenNote(note) => {
                    self.jobs.clear();
                    cx.emit(MediaEvent::OpenNote(note));
                    cx.emit(MediaEvent::CloseLibrary);
                }
                LibraryEffect::Status(text, error) => {
                    let text = tr(cx, &text);
                    cx.emit(MediaEvent::Status { text, error });
                }
            }
        }
        cx.notify();
    }

    fn focused_path(&self) -> Option<String> {
        self.vm.cards().get(self.focused).map(|c| c.path.clone())
    }

    fn move_focus(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.vm.cards().len();
        if n == 0 {
            return;
        }
        let next = (self.focused as isize + delta).rem_euclid(n as isize);
        self.focused = usize::try_from(next).unwrap_or(0);
        cx.notify();
    }

    fn open_detail(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let effects = self.vm.open_detail(path);
        let local = self
            .vm
            .detail()
            .filter(|c| c.kind == MediaKind::Image && !c.can_rename);
        if let Some(card) = local {
            let file = self.services.notes_root().join(&card.path);
            let key = card.path.clone();
            let work = cx.background_executor().spawn(async move {
                let d = frankmd_media::server_folder::image_dimensions(&file);
                d.width.zip(d.height)
            });
            let task = cx.spawn(async move |this, cx| {
                let dims = work.await;
                this.update(cx, |this, cx| {
                    this.detail_dims = Some((key, dims));
                    cx.notify();
                })
                .ok();
            });
            self.jobs.push((None, task));
        }
        self.run(effects, window, cx);
    }

    fn request_rename(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = self.vm.request_rename(path) {
            self.rename.update(cx, |s, cx| {
                s.set_value(name, window, cx);
                s.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn escape(&mut self, cx: &mut Context<Self>) {
        if self.vm.prompt().is_some() {
            self.vm.cancel_prompt();
        } else if self.vm.detail().is_some() {
            self.vm.close_detail();
        } else {
            self.close(cx);
            return;
        }
        cx.notify();
    }
}

impl Render for MediaLibraryPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let cards = self.vm.cards();
        // Cards are rendered = visible: the first local one starts the usage scan.
        if let Some(first) = cards.iter().find(|c| !c.can_rename) {
            let effects = self.vm.mark_visible(&first.path);
            if !effects.is_empty() {
                self.run(effects, window, cx);
            }
        }
        for card in &cards {
            if let frankmd_media::vm::picker::Thumb::Url(url) = &card.thumb
                && card.kind == MediaKind::Image
            {
                widgets::fetch_remote(self, url, cx);
            }
        }
        let (images, videos) = self.vm.counts();
        let category = self.vm.category();
        let focused = self.focused;
        h_flex()
            .id("media-library")
            .test_support()
            .key_context(LIBRARY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Next, _, cx| this.move_focus(1, cx)))
            .on_action(cx.listener(|this, _: &Prev, _, cx| this.move_focus(-1, cx)))
            .on_action(
                cx.listener(|this, _: &NextRow, _, cx| this.move_focus(COLUMNS as isize, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PrevRow, _, cx| this.move_focus(-(COLUMNS as isize), cx)),
            )
            .on_action(cx.listener(|this, _: &Details, window, cx| {
                if let Some(path) = this.focused_path() {
                    this.open_detail(&path, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &InsertFocused, window, cx| {
                if let Some(path) = this.focused_path() {
                    let effects = this.vm.insert(&path, this.note.as_deref());
                    this.run(effects, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &DeleteFocused, _, cx| {
                if let Some(path) = this.focused_path() {
                    this.vm.request_delete(&path);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.filter.update(cx, |s, cx| s.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &Close, _, cx| this.escape(cx)))
            .size_full()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        h_flex()
                            .id("media-library-header")
                            .h(px(40.))
                            .px_3()
                            .gap_2()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t(cx, "library.title")),
                            )
                            .child(
                                Button::new("library-tab-images")
                                    .xsmall()
                                    .selected(category == Category::Images)
                                    .label(format!("{} {images}", t(cx, "library.images")))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.vm.set_category(Category::Images);
                                        this.focused = 0;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("library-tab-videos")
                                    .xsmall()
                                    .selected(category == Category::Videos)
                                    .label(format!("{} {videos}", t(cx, "library.videos")))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.vm.set_category(Category::Videos);
                                        this.focused = 0;
                                        cx.notify();
                                    })),
                            )
                            .child(div().w(px(220.)).child(Input::new(&self.filter).small()))
                            .child(div().flex_1())
                            .child(
                                Button::new("library-close")
                                    .ghost()
                                    .xsmall()
                                    .icon(crate::ui::icons::ph("x"))
                                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                            ),
                    )
                    .when_some(self.vm.prompt().cloned(), |el, prompt| {
                        el.child(self.render_prompt(&prompt, &theme, cx))
                    })
                    .child(
                        div()
                            .id("media-library-grid")
                            .test_support()
                            .flex_1()
                            .min_h_0()
                            .p_3()
                            .overflow_y_scroll()
                            .child(match self.vm.load_state().clone() {
                                LoadState::Loading | LoadState::Idle => v_flex()
                                    .gap_2()
                                    .child(Progress::new("library-loading").loading(true))
                                    .child(t(cx, "library.loading"))
                                    .into_any_element(),
                                LoadState::Failed(error) => {
                                    widgets::error_line(tr(cx, &error), &theme).into_any_element()
                                }
                                LoadState::Loaded if cards.is_empty() => div()
                                    .text_color(theme.muted_foreground)
                                    .child(
                                        self.vm
                                            .empty_text()
                                            .map(|t| tr(cx, &t))
                                            .unwrap_or_default(),
                                    )
                                    .into_any_element(),
                                LoadState::Loaded => div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_3()
                                    .children(cards.into_iter().enumerate().map(|(i, card)| {
                                        self.render_card(card, i == focused, &theme, cx)
                                    }))
                                    .into_any_element(),
                            }),
                    ),
            )
            .when_some(self.vm.detail(), |el, card| {
                el.child(self.render_detail(card, &theme, cx))
            })
    }
}

impl MediaLibraryPane {
    fn render_card(
        &self,
        card: LibraryCard,
        focused: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let path = card.path.clone();
        let encrypted = self.media_encrypted(&path);
        let (p1, p2, p3, p4, p5) = (
            path.clone(),
            path.clone(),
            path.clone(),
            path.clone(),
            path.clone(),
        );
        v_flex()
            .id(SharedString::from(format!("library-card-{}", card.path)))
            .test_support()
            .w(px(160.))
            .gap_1()
            .p_1()
            .rounded(theme.radius)
            .border_2()
            .border_color(if focused { theme.accent } else { theme.border })
            .child(
                div()
                    .id(SharedString::from(format!("library-preview-{}", card.path)))
                    .w_full()
                    .h(px(100.))
                    .rounded(theme.radius)
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_detail(&p1, window, cx)),
                    )
                    .child(if encrypted {
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.secondary)
                            .child("⚿")
                            .into_any_element()
                    } else if card.kind == MediaKind::Video {
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.secondary)
                            .child("▶")
                            .into_any_element()
                    } else {
                        widgets::thumb_element(Some(&card.thumb), &self.remote, theme)
                    }),
            )
            .child(div().text_sm().truncate().child(card.name.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} · {}", card.size, card.date)),
            )
            .child(self.usage_line(card.usage, &path, theme, cx))
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        Button::new(SharedString::from(format!("library-insert-{path}")))
                            .xsmall()
                            .primary()
                            .label(t(cx, "library.insert"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let effects = this.vm.insert(&p2, this.note.as_deref());
                                this.run(effects, window, cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("library-copy-{path}")))
                            .xsmall()
                            .label(t(cx, "media_dialogs.copy_path"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let effects = this.vm.copy_path(&p3);
                                this.run(effects, window, cx);
                            })),
                    )
                    .when(card.can_rename, |el| {
                        el.child(
                            Button::new(SharedString::from(format!("library-rename-{path}")))
                                .xsmall()
                                .label(t(cx, "library.rename"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.request_rename(&p4, window, cx)
                                })),
                        )
                    })
                    .child({
                        let pe = path.clone();
                        let label = if encrypted {
                            t(cx, "library.decrypt")
                        } else {
                            t(cx, "library.encrypt")
                        };
                        Button::new(SharedString::from(format!("library-crypto-{path}")))
                            .xsmall()
                            .label(label)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.toggle_media_encryption(&pe, !encrypted, window, cx)
                            }))
                    })
                    .child(
                        Button::new(SharedString::from(format!("library-delete-{path}")))
                            .xsmall()
                            .danger()
                            .label(t(cx, "library.delete"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.vm.request_delete(&p5);
                                cx.notify();
                            })),
                    ),
            )
    }

    fn usage_line(
        &self,
        usage: UsageLabel,
        path: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text = tr(cx, &usage.text());
        match usage {
            UsageLabel::Used(_) => {
                let path = path.to_owned();
                div()
                    .id(SharedString::from(format!("library-usage-{path}")))
                    .text_xs()
                    .underline()
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_detail(&path, window, cx)),
                    )
                    .child(text)
                    .into_any_element()
            }
            _ => div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element(),
        }
    }

    fn render_prompt(
        &self,
        prompt: &LibraryPrompt,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let body: AnyElement = match prompt {
            LibraryPrompt::Delete { name, .. } => div()
                .flex_1()
                .text_sm()
                .child(crate::ui::i18n::t_args(
                    cx,
                    "library.delete_confirm",
                    &[("name", name)],
                ))
                .into_any_element(),
            LibraryPrompt::Rename { .. } => h_flex()
                .flex_1()
                .gap_2()
                .child(div().text_sm().child(t(cx, "library.rename_prompt")))
                .child(div().flex_1().child(Input::new(&self.rename).small()))
                .into_any_element(),
        };
        let destructive = matches!(prompt, LibraryPrompt::Delete { .. });
        let accept_label = if destructive {
            t(cx, "library.delete")
        } else {
            t(cx, "library.rename")
        };
        let accept = Button::new("library-prompt-accept")
            .small()
            .label(accept_label)
            .on_click(cx.listener(|this, _, window, cx| {
                let text = this.rename.read(cx).value().to_string();
                let effects = this.vm.accept_prompt(&text);
                this.run(effects, window, cx);
            }));
        h_flex()
            .id("library-prompt")
            .test_support()
            .gap_2()
            .p_2()
            .bg(theme.secondary)
            .child(body)
            .child(
                Button::new("library-prompt-cancel")
                    .small()
                    .label(t(cx, "common.cancel"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.vm.cancel_prompt();
                        cx.notify();
                    })),
            )
            .child(if destructive {
                accept.danger()
            } else {
                accept.primary()
            })
    }

    fn render_detail(
        &self,
        card: LibraryCard,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let encrypted = self.media_encrypted(&card.path);
        let dims = self
            .detail_dims
            .as_ref()
            .filter(|(p, _)| *p == card.path)
            .and_then(|(_, d)| *d);
        let metadata = self.vm.detail_metadata(dims).unwrap_or_default();
        let usage = tr(cx, &card.usage.text());
        let notes: Option<Vec<String>> = self.vm.detail_notes().map(<[String]>::to_vec);
        let (p_insert, p_copy, p_delete) =
            (card.path.clone(), card.path.clone(), card.path.clone());
        v_flex()
            .id("media-library-detail")
            .test_support()
            .w(px(420.))
            .h_full()
            .p_3()
            .gap_2()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .overflow_y_scroll()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(card.name.clone()),
                    )
                    .child(
                        Button::new("library-detail-close")
                            .ghost()
                            .xsmall()
                            .icon(crate::ui::icons::ph("x"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.vm.close_detail();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{usage} · {metadata}")),
            )
            .child(
                div()
                    .w_full()
                    .h(px(280.))
                    .rounded(theme.radius)
                    .overflow_hidden()
                    .child(if encrypted {
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.secondary)
                            .child("⚿")
                            .into_any_element()
                    } else if card.kind == MediaKind::Video {
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.secondary)
                            .child("▶")
                            .into_any_element()
                    } else {
                        widgets::thumb_element(Some(&card.thumb), &self.remote, theme)
                    }),
            )
            .when_some(notes, |el, notes| {
                el.child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t(cx, "library.usage_dialog_title")),
                )
                .child(if notes.is_empty() {
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(t(cx, "library.usage_dialog_empty"))
                        .into_any_element()
                } else {
                    v_flex()
                        .gap_1()
                        .children(notes.into_iter().map(|note| {
                            let target = note.clone();
                            Button::new(SharedString::from(format!("library-note-{note}")))
                                .ghost()
                                .xsmall()
                                .label(note)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let effects = this.vm.open_note(&target);
                                    this.run(effects, window, cx);
                                }))
                        }))
                        .into_any_element()
                })
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("library-detail-delete")
                            .small()
                            .danger()
                            .label(t(cx, "library.delete"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.vm.request_delete(&p_delete);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("library-detail-copy")
                            .small()
                            .label(t(cx, "library.copy_path"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let effects = this.vm.copy_path(&p_copy);
                                this.run(effects, window, cx);
                            })),
                    )
                    .child(
                        Button::new("library-detail-insert")
                            .small()
                            .primary()
                            .label(t(cx, "library.insert"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let effects = this.vm.insert(&p_insert, this.note.as_deref());
                                this.run(effects, window, cx);
                            })),
                    ),
            )
    }
}
