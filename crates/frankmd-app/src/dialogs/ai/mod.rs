//! AI dialogs: the grammar check (ui-spec §5.10). AI image generation lives
//! in the Insert image dialog's "AI generate" source
//! ([`crate::dialogs::media::picker`]).
//!
//! [`GrammarDialog`] renders `frankmd_ai::grammar::GrammarState`: the request
//! runs on the tokio bridge (aborted on Esc / close), the note is locked
//! read-only meanwhile, and the result is a side-by-side word diff with
//! per-change accept/reject and an Edit toggle. Only markdown notes are ever
//! sent (`.fed` and other files are refused before any network call).

use crate::scale::px;
use std::ops::Range;
use std::sync::Arc;

use frankmd_ai::grammar::{
    Availability, GrammarEffect, GrammarResult, GrammarState, Segment, StartError, View,
};
use frankmd_ai::{AiRequestError, NoteSource};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::dialogs::media::services::MediaServices;
use crate::dialogs::media::task::{Abort, spawn_net};
use crate::dialogs::media::widgets::t;
use crate::ui::i18n;

/// Key context of the result dialog.
pub const CONTEXT: &str = "GrammarDialog";

actions!(
    grammar_dialog,
    [
        /// Ctrl+Enter: apply.
        Apply,
        /// Esc: cancel.
        Cancel,
        /// Alt+↓.
        NextChange,
        /// Alt+↑.
        PrevChange,
        /// Y on the focused change.
        AcceptChange,
        /// N on the focused change.
        RejectChange,
        /// Alt+E.
        ToggleEdit,
    ]
);

pub(crate) fn init(cx: &mut App) {
    let ctx = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("ctrl-enter", Apply, ctx),
        KeyBinding::new("escape", Cancel, ctx),
        KeyBinding::new("alt-down", NextChange, ctx),
        KeyBinding::new("alt-up", PrevChange, ctx),
        KeyBinding::new("alt-e", ToggleEdit, ctx),
        KeyBinding::new("y", AcceptChange, Some("GrammarDiff")),
        KeyBinding::new("n", RejectChange, Some("GrammarDiff")),
    ]);
}

/// What the grammar check asks of the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrammarEvent {
    /// Save the pending autosave now.
    FlushAutosave,
    /// Lock (`true`) / unlock the editor.
    SetReadOnly(bool),
    /// Show (`Some(text)`) or remove the "Checking grammar…" pane banner; its
    /// Cancel button calls [`GrammarDialog::cancel_request`].
    Banner(Option<SharedString>),
    /// Open the result overlay (`dialog-grammar`).
    Show,
    /// Replace the whole document with this text (one undoable edit).
    Apply(String),
    /// The dialog closed.
    Closed,
    /// A status-line message (e.g. "No file open").
    Status {
        /// Text.
        text: SharedString,
        /// Error styling.
        error: bool,
    },
    /// Open Settings › Integrations.
    OpenSettings,
    /// Config became available while a start was pending: run it again.
    Retry,
}

/// The note the check reads: the editor text of `path` (identical to the
/// file once the autosave flush ran), served only if `path` is a contained,
/// existing vault file.
struct EditorNote {
    vault: frankmd_fs::Vault,
    path: String,
    text: String,
}

impl NoteSource for EditorNote {
    fn read_note(&self, path: &str) -> Option<String> {
        let contained = self.vault.safe_path(path, true).ok()?;
        (path == self.path || contained.rel().as_str() == self.path).then(|| self.text.clone())
    }
}

/// The grammar check view (banner state + result dialog).
pub struct GrammarDialog {
    services: Arc<MediaServices>,
    state: GrammarState,
    open: bool,
    text: String,
    focus: FocusHandle,
    diff_focus: FocusHandle,
    edit: Entity<TextareaState>,
    request: Option<(Option<Abort>, Task<()>)>,
    config: Option<Task<()>>,
    pending_start: bool,
    syncing: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GrammarEvent> for GrammarDialog {}

impl Focusable for GrammarDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl GrammarDialog {
    /// Build; AI availability is read once in the background (FE-439).
    pub fn new(services: Arc<MediaServices>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let edit = cx.new(|cx| TextareaState::new(window, cx));
        let subs = vec![
            cx.subscribe_in(&edit, window, |this, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) && !this.syncing {
                    this.state.set_edited(&input.read(cx).value());
                }
            }),
        ];
        let mut dialog = Self {
            services,
            state: GrammarState::default(),
            open: false,
            text: String::new(),
            focus: cx.focus_handle(),
            diff_focus: cx.focus_handle(),
            edit,
            request: None,
            config: None,
            pending_start: false,
            syncing: false,
            _subscriptions: subs,
        };
        dialog.reload_config(cx);
        dialog
    }

    /// Re-read the AI configuration (e.g. after `.fed` changed).
    pub fn reload_config(&mut self, cx: &mut Context<Self>) {
        let services = Arc::clone(&self.services);
        let work = cx
            .background_executor()
            .spawn(async move { services.ai().provider_info() });
        self.config = Some(cx.spawn(async move |this, cx| {
            let info = work.await;
            this.update(cx, |this, cx| {
                if !this.state.in_flight() {
                    this.state = GrammarState::from_config(&info);
                    let available =
                        !matches!(this.state.availability(), Availability::NotConfigured);
                    if this.pending_start {
                        this.pending_start = false;
                        if available {
                            // Start was requested while the dialog still held
                            // an old config. Re-run it with the fresh state.
                            this.open = false;
                            cx.emit(GrammarEvent::Closed);
                            cx.emit(GrammarEvent::Retry);
                        } else {
                            this.open = true;
                        }
                    } else if available
                        && this.open
                        && this.state.result().is_none()
                        && this.state.error().is_none()
                    {
                        // The setup screen was opened while the config was
                        // stale. Close it as soon as a provider is available;
                        // the user can invoke the check again.
                        this.open = false;
                        cx.emit(GrammarEvent::Closed);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The state (tests, palette availability).
    pub fn state(&self) -> &GrammarState {
        &self.state
    }

    /// The result dialog is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// A request is running.
    pub fn is_running(&self) -> bool {
        self.state.in_flight()
    }

    /// Ctrl+Shift+G: check `path` whose editor text is `text`.
    pub fn start(
        &mut self,
        path: Option<String>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.text = text;
        if !self.state.in_flight() {
            // Settings may have changed since this dialog was created; never
            // show the setup screen with a stale provider/model.
            self.state = GrammarState::from_config(&self.services.ai().provider_info());
        }
        self.pending_start = false;
        match self.state.start(path.as_deref(), &self.text) {
            Ok(effects) => self.run(effects, window, cx),
            Err(StartError::NotConfigured) => {
                // Show the setup immediately, but re-read once: if a provider
                // became available in the background, the reload closes the
                // setup and retries the check automatically.
                self.open = true;
                self.focus.focus(window, cx);
                cx.emit(GrammarEvent::Show);
                self.pending_start = true;
                self.reload_config(cx);
            }
            Err(error) => {
                if let Some(key) = error.key() {
                    cx.emit(GrammarEvent::Status {
                        text: t(cx, key),
                        error: true,
                    });
                }
            }
        }
        cx.notify();
    }

    /// Esc / the banner's Cancel: abort silently.
    pub fn cancel_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let effects = self.state.cancel();
        self.run(effects, window, cx);
    }

    fn run(&mut self, effects: Vec<GrammarEffect>, window: &mut Window, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                GrammarEffect::FlushAutosave => cx.emit(GrammarEvent::FlushAutosave),
                GrammarEffect::SetReadOnly(on) => cx.emit(GrammarEvent::SetReadOnly(on)),
                GrammarEffect::ShowBanner { provider, model } => {
                    let text = i18n::t_args(
                        cx,
                        "media_dialogs.grammar_checking",
                        &[("provider", &provider), ("model", &model)],
                    );
                    cx.emit(GrammarEvent::Banner(Some(text)));
                }
                GrammarEffect::HideBanner => cx.emit(GrammarEvent::Banner(None)),
                GrammarEffect::Request { id, path } => self.request(id, path, cx),
                GrammarEffect::Abort => self.request = None,
                GrammarEffect::ShowDialog => {
                    self.open = true;
                    self.sync_edit(window, cx);
                    self.focus.focus(window, cx);
                    self.diff_focus.focus(window, cx);
                    cx.emit(GrammarEvent::Show);
                }
            }
        }
        cx.notify();
    }

    fn request(&mut self, id: u64, path: String, cx: &mut Context<Self>) {
        let services = Arc::clone(&self.services);
        let note = EditorNote {
            vault: services.vault.clone(),
            path: path.clone(),
            text: self.text.clone(),
        };
        let (abort, work) = spawn_net(cx, async move {
            let service = services.ai();
            frankmd_ai::fix_grammar_note(&service, &note, &path).await
        });
        let task = cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                let result = match result {
                    Some(Ok(response)) => Ok(GrammarResult {
                        original: response.original,
                        corrected: response.fix.corrected,
                        provider: response.fix.provider.as_str().to_owned(),
                        model: response.fix.model,
                    }),
                    Some(Err(error)) => Err(error_text(cx, &error)),
                    None => Err(t(cx, "errors.failed_to_process_ai").to_string()),
                };
                let effects = this.state.finish(id, result);
                for effect in effects {
                    match effect {
                        GrammarEffect::SetReadOnly(on) => cx.emit(GrammarEvent::SetReadOnly(on)),
                        GrammarEffect::HideBanner => cx.emit(GrammarEvent::Banner(None)),
                        GrammarEffect::ShowDialog => {
                            this.open = true;
                            cx.emit(GrammarEvent::Show);
                        }
                        _ => {}
                    }
                }
                this.request = None;
                cx.notify();
            })
            .ok();
        });
        self.request = Some((abort, task));
    }

    fn sync_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .state
            .edited()
            .map(str::to_owned)
            .unwrap_or_else(|| self.state.merged_text());
        self.syncing = true;
        self.edit.update(cx, |s, cx| s.set_value(text, window, cx));
        self.syncing = false;
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.state.close();
        cx.emit(GrammarEvent::Closed);
        cx.notify();
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.state.apply() {
            self.open = false;
            cx.emit(GrammarEvent::Apply(text));
            cx.emit(GrammarEvent::Closed);
        }
        cx.notify();
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.clone();
        self.open = false;
        cx.emit(GrammarEvent::Closed);
        if let Ok(effects) = self.state.retry(&text) {
            self.run(effects, window, cx);
        }
    }

    fn toggle_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.toggle_edit();
        if self.state.view() == View::Edit {
            self.sync_edit(window, cx);
            self.edit.update(cx, |s, cx| s.focus(window, cx));
        } else {
            self.diff_focus.focus(window, cx);
        }
        cx.notify();
    }
}

/// "Failed to process text with AI: <error>" (the service's message is generic, HP-011).
fn error_text(cx: &App, error: &AiRequestError) -> String {
    let base = t(cx, "errors.failed_to_process_ai");
    let detail: String = match error {
        AiRequestError::BadRequest(key) | AiRequestError::NotFound(key) => t(cx, key).to_string(),
        AiRequestError::Forbidden => t(cx, "palette.unavailable.open_markdown_note").to_string(),
        AiRequestError::Service(e) => e.to_string(),
    };
    format!("{base}: {detail}")
}

// ----- rendering --------------------------------------------------------------------------

impl Render for GrammarDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().id("dialog-grammar-closed").into_any_element();
        }
        let theme = cx.theme().clone();
        let viewport = window.viewport_size();
        let badge = self
            .state
            .result()
            .map(|r| format!("{}: {}", r.provider, r.model));
        let body: AnyElement = if let Some(error) = self.state.error() {
            v_flex()
                .gap_2()
                .p_4()
                .child(
                    div()
                        .id("grammar-error")
                        .test_support()
                        .text_color(theme.danger)
                        .child(error.to_owned()),
                )
                .child(
                    Button::new("grammar-retry")
                        .small()
                        .label(t(cx, "media_dialogs.retry"))
                        .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx))),
                )
                .into_any_element()
        } else if self.state.result().is_some() {
            self.render_diff(&theme, viewport.width, cx)
                .into_any_element()
        } else {
            self.render_setup(&theme, cx).into_any_element()
        };
        let has_result = self.state.result().is_some();
        div()
            .id("dialog-grammar")
            .test_support()
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Apply, _, cx| this.apply(cx)))
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.close(cx)))
            .on_action(cx.listener(|this, _: &NextChange, _, cx| {
                this.state.focus_step(true);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PrevChange, _, cx| {
                this.state.focus_step(false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleEdit, window, cx| this.toggle_edit(window, cx)))
            .w((viewport.width * 0.9).min(px(1200.)))
            .h((viewport.height * 0.9).min(px(800.)))
            .flex()
            .flex_col()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(40.))
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t(cx, "dialogs.ai_diff.title")),
                    )
                    .when_some(badge, |el, badge| {
                        el.child(
                            div()
                                .id("grammar-badge")
                                .test_support()
                                .px_2()
                                .rounded_sm()
                                .text_xs()
                                .bg(theme.secondary)
                                .child(badge),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("grammar-close")
                            .ghost()
                            .xsmall()
                            .icon(crate::ui::icons::ph("x"))
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .child(
                h_flex()
                    .h(px(48.))
                    .px_4()
                    .gap_2()
                    .justify_end()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("grammar-cancel")
                            .label(t(cx, "common.cancel"))
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    )
                    .child(
                        Button::new("grammar-apply")
                            .primary()
                            .disabled(!has_result)
                            .label(format!(
                                "{}  Ctrl+↵",
                                t(cx, "dialogs.ai_diff.accept_changes")
                            ))
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    ),
            )
            .into_any_element()
    }
}

impl GrammarDialog {
    fn render_setup(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let cards: [(&str, &[&str]); 6] = [
            (
                "dialogs.ai_diff.ollama_title",
                &[
                    "ollama_api_base = http://localhost:11434",
                    "ollama_model = llama3.2",
                    "Run: ollama pull llama3.2",
                ],
            ),
            (
                "dialogs.ai_diff.openrouter_title",
                &[
                    "openrouter_api_key = sk-or-...",
                    "openrouter_model = openai/gpt-4o-mini",
                ],
            ),
            (
                "dialogs.ai_diff.anthropic_title",
                &[
                    "anthropic_api_key = sk-ant-...",
                    "anthropic_model = claude-sonnet-4-20250514",
                ],
            ),
            (
                "dialogs.ai_diff.gemini_title",
                &["gemini_api_key = ...", "gemini_model = gemini-2.0-flash"],
            ),
            (
                "dialogs.ai_diff.openai_title",
                &["openai_api_key = sk-...", "openai_model = gpt-4o-mini"],
            ),
            (
                "dialogs.ai_diff.provider_selection",
                &[
                    "ai_provider = anthropic",
                    "ai_model = claude-sonnet-4-20250514",
                ],
            ),
        ];
        v_flex()
            .id("grammar-setup")
            .test_support()
            .p_4()
            .gap_2()
            .overflow_y_scroll()
            .size_full()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t(cx, "media_dialogs.grammar_needs_provider")),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t(cx, "dialogs.ai_diff.config_description")),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t(cx, "dialogs.ai_diff.priority_note")),
            )
            .child(div().grid().grid_cols(2).gap_2().children(cards.iter().map(
                |(title, lines)| {
                    v_flex()
                        .p_2()
                        .gap_1()
                        .rounded(theme.radius)
                        .bg(theme.secondary)
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t(cx, title)),
                        )
                        .children(lines.iter().map(|line| {
                            div()
                                .text_xs()
                                .font_family(theme.mono_font_family.clone())
                                .child(*line)
                        }))
                },
            )))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("grammar-reload-config")
                            .small()
                            .outline()
                            .label(t(cx, "media_dialogs.retry"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reload_config(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("grammar-open-settings")
                            .small()
                            .label(t(cx, "media_dialogs.open_settings"))
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(GrammarEvent::OpenSettings)),
                            ),
                    ),
            )
    }

    fn render_diff(
        &self,
        theme: &Theme,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let (left, left_hl, right, right_hl) = diff_texts(&self.state, theme);
        let (current, total) = self.state.counter();
        let focused = self.state.focused();
        let edit_mode = self.state.view() == View::Edit;
        let side_by_side = width >= px(900.);
        let mono = theme.mono_font_family.clone();
        let column = |id: &'static str, title: SharedString, body: AnyElement| {
            v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(title),
                )
                .child(
                    div()
                        .id(id)
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .p_2()
                        .rounded(theme.radius)
                        .bg(theme.secondary)
                        .overflow_y_scroll()
                        .child(body),
                )
        };
        let left_el = div()
            .font_family(mono.clone())
            .text_sm()
            .child(StyledText::new(left).with_highlights(left_hl))
            .into_any_element();
        let right_el = if edit_mode {
            Textarea::new(&self.edit).h_full().into_any_element()
        } else {
            div()
                .font_family(mono.clone())
                .text_sm()
                .child(StyledText::new(right).with_highlights(right_hl))
                .into_any_element()
        };
        let original_title: SharedString = format!(
            "{} {}",
            t(cx, "dialogs.ai_diff.original"),
            t(cx, "dialogs.ai_diff.deletions_hint")
        )
        .into();
        let corrected_title: SharedString = format!(
            "{} {}",
            t(cx, "dialogs.ai_diff.corrected"),
            t(cx, "dialogs.ai_diff.additions_hint")
        )
        .into();
        let hunks = self.state.hunks().to_vec();
        v_flex()
            .size_full()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().child(i18n::t_args(
                        cx,
                        "media_dialogs.change_counter",
                        &[
                            ("current", &current.to_string()),
                            ("total", &total.to_string()),
                        ],
                    )))
                    .child(div().flex_1())
                    .child(
                        Button::new("grammar-toggle-edit")
                            .xsmall()
                            .selected(edit_mode)
                            .label(t(cx, self.state.toggle_label_key()))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_edit(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(side_by_side, |el| el.flex_row())
                    .when(!side_by_side, |el| el.flex_col())
                    .gap_3()
                    .child(column("grammar-original", original_title, left_el))
                    .child(column("grammar-corrected", corrected_title, right_el)),
            )
            .when(!edit_mode && !hunks.is_empty(), |el| {
                el.child(
                    v_flex()
                        .id("grammar-hunks")
                        .test_support()
                        .key_context("GrammarDiff")
                        .track_focus(&self.diff_focus)
                        .on_action(cx.listener(|this, _: &AcceptChange, _, cx| {
                            let i = this.state.focused();
                            this.state.set_accepted(i, true);
                            cx.notify();
                        }))
                        .on_action(cx.listener(|this, _: &RejectChange, _, cx| {
                            let i = this.state.focused();
                            this.state.set_accepted(i, false);
                            cx.notify();
                        }))
                        .max_h(px(140.))
                        .overflow_y_scroll()
                        .gap_1()
                        .children(hunks.into_iter().enumerate().map(|(i, hunk)| {
                            h_flex()
                                .id(SharedString::from(format!("grammar-hunk-{i}")))
                                .gap_2()
                                .px_2()
                                .rounded_sm()
                                .when(i == focused, |el| el.bg(theme.secondary))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.state.focus(i);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .font_family(mono.clone())
                                        .truncate()
                                        .child(format!(
                                            "{} → {}",
                                            hunk.deleted.trim(),
                                            hunk.inserted.trim()
                                        )),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("grammar-accept-{i}")))
                                        .xsmall()
                                        .selected(hunk.accepted)
                                        .icon(crate::ui::icons::ph("check"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.state.set_accepted(i, true);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("grammar-reject-{i}")))
                                        .xsmall()
                                        .selected(!hunk.accepted)
                                        .icon(crate::ui::icons::ph("x"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.state.set_accepted(i, false);
                                            cx.notify();
                                        })),
                                )
                        })),
                )
            })
    }
}

/// Highlighted ranges of a diff column.
type Highlights = Vec<(Range<usize>, HighlightStyle)>;

/// Left (equal + deletions) and right (equal + insertions, or the original
/// words of rejected changes) texts with their highlight ranges.
fn diff_texts(state: &GrammarState, theme: &Theme) -> (String, Highlights, String, Highlights) {
    let deleted = HighlightStyle {
        color: Some(theme.danger),
        background_color: Some(theme.danger.opacity(0.2)),
        strikethrough: Some(StrikethroughStyle {
            thickness: px(1.),
            color: Some(theme.danger),
        }),
        ..HighlightStyle::default()
    };
    let inserted = HighlightStyle {
        color: Some(theme.success),
        background_color: Some(theme.success.opacity(0.2)),
        ..HighlightStyle::default()
    };
    let (mut left, mut right) = (String::new(), String::new());
    let (mut lh, mut rh) = (Vec::new(), Vec::new());
    for segment in state.segments() {
        match segment {
            Segment::Equal(text) => {
                left.push_str(text);
                right.push_str(text);
            }
            Segment::Change(i) => {
                let Some(hunk) = state.hunks().get(*i) else {
                    continue;
                };
                let start = left.len();
                left.push_str(&hunk.deleted);
                if left.len() > start {
                    lh.push((start..left.len(), deleted));
                }
                let start = right.len();
                if hunk.accepted {
                    right.push_str(&hunk.inserted);
                    if right.len() > start {
                        rh.push((start..right.len(), inserted));
                    }
                } else {
                    right.push_str(&hunk.deleted);
                }
            }
        }
    }
    (left, lh, right, rh)
}
