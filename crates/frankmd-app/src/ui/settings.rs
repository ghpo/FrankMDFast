//! Settings sheet (ui-spec §5.16): one place for every preference, a search
//! field filtering rows across categories, and a provenance chip on each row.
//! Changes apply live and are written to `.fed` through the workspace.
//!
//! Categories: Appearance, Editor, Writing, Preview, Files, Media,
//! Integrations, Remote, Encryption, Hugo, Diagnostics and About. The scope
//! toggle picks where edits go: *This vault* writes `.fed`, *All vaults* the
//! global `config.toml` (both through `frankmd-config`). Secrets are never
//! shown or echoed: a set secret reads "Set", and replacing one uses a
//! masked input that starts empty.

use std::path::PathBuf;

use crate::scale::px;
use frankmd_config::{Key, Provenance, Value};
use frankmd_workspace::Command;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::component::{Disableable as _, Selectable as _, StyledExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::i18n;
use super::shell::Shell;

/// Settings categories, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Category {
    #[default]
    Appearance,
    Editor,
    Writing,
    Preview,
    Files,
    Media,
    Integrations,
    Remote,
    Encryption,
    Hugo,
    Diagnostics,
    About,
}

impl Category {
    const ALL: [Category; 12] = [
        Self::Appearance,
        Self::Editor,
        Self::Writing,
        Self::Preview,
        Self::Files,
        Self::Media,
        Self::Integrations,
        Self::Remote,
        Self::Encryption,
        Self::Hugo,
        Self::Diagnostics,
        Self::About,
    ];

    fn slug(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Editor => "editor",
            Self::Writing => "writing",
            Self::Preview => "preview",
            Self::Files => "files",
            Self::Media => "media",
            Self::Integrations => "integrations",
            Self::Remote => "remote",
            Self::Encryption => "encryption",
            Self::Hugo => "hugo",
            Self::Diagnostics => "diagnostics",
            Self::About => "about",
        }
    }
}

/// What a row's control does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    /// On/off key.
    Switch(Key),
    /// Opens a palette picker (theme, language).
    Picker(&'static str),
    /// Split / Single.
    ViewMode,
    /// The vault path.
    NotesFolder,
    /// Editable text, committed on Enter / blur.
    Text(Key),
    /// A secret: "Set"/"Not set", replaced through a masked input.
    Secret(Key),
    /// One of a fixed set of values (`value`, label i18n key).
    Choice(Key, &'static [(&'static str, &'static str)]),
    /// One of a few integers, as toggle buttons (`value`, label).
    IntChoice(Key, &'static [(i64, ChoiceLabel)]),
    /// − / + through an ascending list of integers, shown with `suffix`.
    Steps(Key, &'static [i64], &'static str),
    /// Interface size (app preference, not a `.fed` key).
    UiScale,
    /// Hugo default folder (fixed by the backend, HP-014).
    HugoFolder,
    /// Opens `.hugo_template.md` as a note.
    HugoTemplate,
    /// "Storage for new media", bound to the media layer's storage model
    /// (saves immediately, disabled while saving, S3 only when configured).
    MediaStorage,
}

/// Label of an [`Control::IntChoice`] button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChoiceLabel {
    /// An i18n key.
    Key(&'static str),
    /// An i18n key taking `%{count}`.
    Count(&'static str, &'static str),
}

/// FrankMD's option lists (`_settings_panel.html.erb`).
const FONT_SIZES: &[i64] = &[12, 13, 14, 15, 16, 18, 20, 22, 24];
const EDITOR_WIDTHS: &[i64] = &[
    40, 48, 56, 64, 72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, 160, 168, 176, 184, 192, 200,
];
const PREVIEW_ZOOMS: &[i64] = &[50, 75, 90, 100, 110, 125, 150, 175, 200];
const INDENTS: &[(i64, ChoiceLabel)] = &[
    (2, ChoiceLabel::Count("settings.indent_spaces", "2")),
    (4, ChoiceLabel::Count("settings.indent_spaces", "4")),
    (8, ChoiceLabel::Count("settings.indent_spaces", "8")),
];
/// Relative numbering is not implemented yet: `2` in `.fed` shows as On.
const LINE_NUMBERS: &[(i64, ChoiceLabel)] = &[
    (0, ChoiceLabel::Key("settings.line_numbers_off")),
    (1, ChoiceLabel::Key("settings.line_numbers_absolute")),
];

/// Index of the largest option `<= current` (the first when none is).
fn option_index(options: impl IntoIterator<Item = i64>, current: i64) -> usize {
    options
        .into_iter()
        .enumerate()
        .filter(|&(_, v)| v <= current)
        .last()
        .map_or(0, |(i, _)| i)
}

/// The values − and + move to from `current` (`None` = button disabled).
/// A value between two options steps down to the lower one; an unset value
/// can only step down from the end.
fn step_targets(options: &[i64], current: Option<i64>) -> (Option<i64>, Option<i64>) {
    let Some(current) = current else {
        return (None, options.last().copied());
    };
    let i = option_index(options.iter().copied(), current);
    let down = if current > options[i] {
        Some(options[i])
    } else {
        i.checked_sub(1).map(|j| options[j])
    };
    let up = options.iter().copied().find(|&v| v > current);
    (down, up)
}

/// − value + control over `options`; `on_step` gets the chosen value.
fn stepper(
    id_slug: &str,
    options: &'static [i64],
    suffix: &'static str,
    current: Option<i64>,
    theme: &gpui_kit::component::theme::Theme,
    on_step: impl Fn(i64, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let (down, up) = step_targets(options, current);
    let shown = current.map_or_else(|| "—".to_owned(), |c| format!("{c}{suffix}"));
    let step = |id: &str, icon: &'static str, target: Option<i64>| {
        let on_step = on_step.clone();
        Button::new(SharedString::from(format!("setting-{id_slug}-{id}")))
            .small()
            .outline()
            .icon(crate::ui::icons::ph(icon))
            .disabled(target.is_none())
            .on_click(move |_, window, cx| {
                if let Some(value) = target {
                    on_step(value, window, cx);
                }
            })
    };
    h_flex()
        .gap_1()
        .items_center()
        .child(step("decrease", "minus", down))
        .child(
            div()
                .w(px(56.))
                .text_center()
                .font_family(theme.mono_font_family.clone())
                .child(shown),
        )
        .child(step("increase", "plus", up))
        .into_any_element()
}

/// Where Settings writes (ui-spec §5.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Scope {
    /// `.fed` in this vault.
    #[default]
    Vault,
    /// `~/.config/frankmdfast/config.toml`.
    Global,
}

const HUGO_STYLE_CHOICES: &[(&str, &str)] = &[
    ("dated", "settings_sheet.hugo.dated"),
    ("flat", "settings_sheet.hugo.flat"),
];
const AI_PROVIDER_CHOICES: &[(&str, &str)] = &[
    ("auto", "settings_sheet.integrations.auto"),
    ("openai", "settings_sheet.integrations.openai"),
    ("anthropic", "settings_sheet.integrations.anthropic"),
    ("openrouter", "settings_sheet.integrations.openrouter"),
    ("ollama", "settings_sheet.integrations.ollama"),
    ("gemini", "settings_sheet.integrations.gemini"),
];

const fn row(category: Category, label: &'static str, control: Control) -> RowSpec {
    RowSpec {
        category,
        label,
        description: None,
        control,
    }
}

struct RowSpec {
    category: Category,
    label: &'static str,
    description: Option<&'static str>,
    control: Control,
}

const ROWS: &[RowSpec] = &[
    RowSpec {
        category: Category::Appearance,
        label: "settings.theme",
        description: Some("settings.theme_description"),
        control: Control::Picker("theme.select"),
    },
    RowSpec {
        category: Category::Appearance,
        label: "settings.language",
        description: None,
        control: Control::Picker("language.select"),
    },
    RowSpec {
        category: Category::Appearance,
        label: "settings_sheet.ui_scale",
        description: Some("settings_sheet.ui_scale_description"),
        control: Control::UiScale,
    },
    RowSpec {
        category: Category::Editor,
        label: "settings.font",
        description: None,
        control: Control::Picker("font.select"),
    },
    RowSpec {
        category: Category::Editor,
        label: "settings.font_size",
        description: None,
        control: Control::Steps(Key::EditorFontSize, FONT_SIZES, "px"),
    },
    RowSpec {
        category: Category::Editor,
        label: "settings.editor_width",
        description: Some("settings.editor_width_description"),
        control: Control::Steps(Key::EditorWidth, EDITOR_WIDTHS, "ch"),
    },
    RowSpec {
        category: Category::Editor,
        label: "settings.line_numbers",
        description: None,
        control: Control::IntChoice(Key::EditorLineNumbers, LINE_NUMBERS),
    },
    RowSpec {
        category: Category::Editor,
        label: "settings.indent",
        description: Some("settings.indent_description"),
        control: Control::IntChoice(Key::EditorIndent, INDENTS),
    },
    RowSpec {
        category: Category::Editor,
        label: "settings.vim",
        description: Some("settings.vim_description"),
        control: Control::Switch(Key::VimMode),
    },
    RowSpec {
        category: Category::Writing,
        label: "settings.typewriter",
        description: Some("settings.typewriter_description"),
        control: Control::Switch(Key::TypewriterMode),
    },
    RowSpec {
        category: Category::Writing,
        label: "settings.cursor_recenter_on_click",
        description: Some("settings.cursor_recenter_on_click_description"),
        control: Control::Switch(Key::CursorRecenterOnClick),
    },
    RowSpec {
        category: Category::Writing,
        label: "settings.stats",
        description: Some("settings.stats_description"),
        control: Control::Switch(Key::ShowStats),
    },
    RowSpec {
        category: Category::Preview,
        label: "settings.view_mode",
        description: Some("settings.view_mode_description"),
        control: Control::ViewMode,
    },
    RowSpec {
        category: Category::Preview,
        label: "settings.preview_zoom",
        description: Some("settings.preview_zoom_description"),
        control: Control::Steps(Key::PreviewZoom, PREVIEW_ZOOMS, "%"),
    },
    RowSpec {
        category: Category::Preview,
        label: "settings.scroll_sync",
        description: Some("settings.scroll_sync_description"),
        control: Control::Switch(Key::ScrollSync),
    },
    RowSpec {
        category: Category::Files,
        label: "settings_sheet.notes_folder",
        description: Some("settings_sheet.notes_folder_description"),
        control: Control::NotesFolder,
    },
    // Media (§5.16)
    row(
        Category::Media,
        "settings_sheet.media.provider",
        Control::MediaStorage,
    ),
    row(
        Category::Media,
        "settings_sheet.media.images_path",
        Control::Text(Key::ImagesPath),
    ),
    row(
        Category::Media,
        "settings_sheet.media.image_types",
        Control::Text(Key::ImageUploadExtensions),
    ),
    row(
        Category::Media,
        "settings_sheet.media.video_types",
        Control::Text(Key::VideoUploadExtensions),
    ),
    row(
        Category::Media,
        "settings_sheet.media.s3_bucket",
        Control::Text(Key::AwsS3Bucket),
    ),
    row(
        Category::Media,
        "settings_sheet.media.s3_region",
        Control::Text(Key::AwsRegion),
    ),
    row(
        Category::Media,
        "settings_sheet.media.s3_endpoint",
        Control::Text(Key::AwsS3Endpoint),
    ),
    row(
        Category::Media,
        "settings_sheet.media.s3_path_style",
        Control::Switch(Key::AwsS3ForcePathStyle),
    ),
    row(
        Category::Media,
        "settings_sheet.media.s3_access_key",
        Control::Secret(Key::AwsAccessKeyId),
    ),
    row(
        Category::Media,
        "settings_sheet.media.s3_secret_key",
        Control::Secret(Key::AwsSecretAccessKey),
    ),
    // Integrations (§5.16, HP-020)
    row(
        Category::Integrations,
        "settings_sheet.integrations.ai_provider",
        Control::Choice(Key::AiProvider, AI_PROVIDER_CHOICES),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.ai_model",
        Control::Text(Key::AiModel),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.openai_key",
        Control::Secret(Key::OpenaiApiKey),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.openai_model",
        Control::Text(Key::OpenaiModel),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.anthropic_key",
        Control::Secret(Key::AnthropicApiKey),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.anthropic_model",
        Control::Text(Key::AnthropicModel),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.openrouter_key",
        Control::Secret(Key::OpenrouterApiKey),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.openrouter_model",
        Control::Text(Key::OpenrouterModel),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.gemini_key",
        Control::Secret(Key::GeminiApiKey),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.gemini_model",
        Control::Text(Key::GeminiModel),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.ollama_base",
        Control::Text(Key::OllamaApiBase),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.ollama_model",
        Control::Text(Key::OllamaModel),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.image_model",
        Control::Text(Key::ImageGenerationModel),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.youtube_key",
        Control::Secret(Key::YoutubeApiKey),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.google_key",
        Control::Secret(Key::GoogleApiKey),
    ),
    row(
        Category::Integrations,
        "settings_sheet.integrations.google_cse",
        Control::Text(Key::GoogleCseId),
    ),
    // Hugo (§5.16)
    row(
        Category::Hugo,
        "settings_sheet.hugo.path_style",
        Control::Choice(Key::HugoPathStyle, HUGO_STYLE_CHOICES),
    ),
    row(
        Category::Hugo,
        "settings_sheet.hugo.default_folder",
        Control::HugoFolder,
    ),
    row(
        Category::Hugo,
        "settings_sheet.hugo.template",
        Control::HugoTemplate,
    ),
];

/// Sheet state.
pub(crate) struct SettingsState {
    pub(crate) category: Category,
    pub(crate) search: Entity<InputState>,
    /// Where edits are written.
    pub(crate) scope: Scope,
    /// Inputs of the text / secret rows, created on first render.
    inputs: std::collections::HashMap<Key, Entity<InputState>>,
    /// Remote port input (Settings › Remote).
    remote_port: Option<Entity<InputState>>,
    /// Access-token input (Settings › Remote).
    remote_token: Option<Entity<InputState>>,
    /// Encryption password input.
    encryption_password: Option<Entity<InputState>>,
    /// Encryption confirmation input.
    encryption_confirm: Option<Entity<InputState>>,
    /// Seal backup import input.
    encryption_backup: Option<Entity<InputState>>,
    /// File chosen for import, waiting for overwrite confirmation.
    pending_seal_import: Option<PathBuf>,
    /// Background export/import tasks kept alive by the shell.
    pub(crate) encryption_jobs: Vec<Task<()>>,
    /// Result of the "encrypt attachments of encrypted notes" migration.
    pub(crate) encryption_repair: Option<(SharedString, bool)>,
    /// Secret rows whose "Change…" input is open.
    pub(crate) editing_secrets: std::collections::HashSet<Key>,
    /// Inline error of the last failed write, by key.
    pub(crate) errors: std::collections::HashMap<Key, SharedString>,
    subscriptions: Vec<Subscription>,
}

impl SettingsState {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> (Self, Subscription) {
        let search = cx.new(|cx| InputState::new(window, cx));
        let sub = cx.subscribe(&search, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        (
            Self {
                category: Category::default(),
                search,
                scope: Scope::default(),
                inputs: Default::default(),
                remote_port: None,
                remote_token: None,
                encryption_password: None,
                encryption_confirm: None,
                encryption_backup: None,
                pending_seal_import: None,
                encryption_jobs: Vec::new(),
                encryption_repair: None,
                editing_secrets: Default::default(),
                errors: Default::default(),
                subscriptions: Vec::new(),
            },
            sub,
        )
    }

    /// The input of a text/secret row (created with `initial` the first time).
    fn input_for(
        &mut self,
        key: Key,
        initial: &str,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Shell>,
    ) -> Entity<InputState> {
        if let Some(input) = self.inputs.get(&key) {
            return input.clone();
        }
        let initial = initial.to_owned();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(secret)
                .default_value(initial)
        });
        let sub = cx.subscribe_in(
            &input,
            window,
            move |shell, input, ev: &InputEvent, window, cx| {
                let commit = match ev {
                    InputEvent::PressEnter { .. } => true,
                    InputEvent::Blur => !secret,
                    _ => false,
                };
                if commit {
                    let value = input.read(cx).value().to_string();
                    shell.commit_setting_text(key, &value, secret, window, cx);
                }
            },
        );
        self.subscriptions.push(sub);
        self.inputs.insert(key, input.clone());
        input
    }

    /// Drop cached inputs so they re-read the config (after a write elsewhere).
    pub(crate) fn reset_inputs(&mut self) {
        self.inputs.clear();
        self.subscriptions.clear();
        self.editing_secrets.clear();
    }

    /// Focus the search field.
    pub(crate) fn open(&mut self, window: &mut Window, cx: &mut Context<Shell>) {
        self.reset_inputs();
        self.errors.clear();
        let placeholder = i18n::t(cx, "settings_sheet.search_placeholder");
        self.search.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
    }
}

fn value_text(value: Option<&Value>) -> String {
    match value {
        None => "—".to_owned(),
        Some(Value::Str(s)) if s.is_empty() => "—".to_owned(),
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Int(i)) => i.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(v) => v.to_json().to_string(),
    }
}

impl Shell {
    /// A row's source chip: only when something overrides the default
    /// (a "default" label on every row is noise).
    fn row_chip(&self, source: Provenance, cx: &App) -> Option<SharedString> {
        (source != Provenance::Default).then(|| self.provenance_chip(source, cx))
    }

    fn provenance_chip(&self, source: Provenance, cx: &App) -> SharedString {
        match source {
            Provenance::Fed => i18n::t(cx, "settings_sheet.source.fed"),
            Provenance::Env(name) => {
                i18n::t_args(cx, "settings_sheet.source.env", &[("name", name)])
            }
            Provenance::Default => i18n::t(cx, "settings_sheet.source.default"),
            Provenance::Global => i18n::t(cx, "settings_sheet.source.global"),
        }
    }

    /// Write `changes` to the scope the sheet is in.
    pub(crate) fn write_settings(
        &mut self,
        changes: Vec<(Key, Option<Value>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys: Vec<Key> = changes.iter().map(|(k, _)| *k).collect();
        let cmd = match self.settings.scope {
            Scope::Vault => Command::UpdateSettings { changes },
            Scope::Global => Command::UpdateGlobalSettings { changes },
        };
        let result = self.dispatch_checked(cmd, window, cx);
        for key in keys {
            match &result {
                Ok(()) => {
                    self.settings.errors.remove(&key);
                }
                Err(err) => {
                    let text = i18n::t(cx, err.key);
                    self.settings.errors.insert(key, text);
                }
            }
        }
        cx.notify();
    }

    /// A text/secret row was committed. Blank removes the key (back to the
    /// next layer); secrets are never echoed back into the input.
    fn commit_setting_text(
        &mut self,
        key: Key,
        raw: &str,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let trimmed = raw.trim();
        let current = self
            .core
            .as_ref()
            .and_then(|c| c.ws.snapshot().config.value(key));
        let value = if trimmed.is_empty() {
            None
        } else {
            match frankmd_config::value::cast(trimmed, key.spec().ty) {
                Ok(v) => v,
                Err(_) => {
                    let text = i18n::t(cx, "settings.save_failed");
                    self.settings.errors.insert(key, text);
                    cx.notify();
                    return;
                }
            }
        };
        if secret {
            self.settings.editing_secrets.remove(&key);
            self.settings.inputs.remove(&key);
            if value.is_none() {
                cx.notify();
                return;
            }
        } else if value == current {
            return;
        }
        self.write_settings(vec![(key, value)], window, cx);
    }

    /// Hugo "Edit template": open `.hugo_template.md`, creating it from the
    /// default template first (§5.16).
    fn edit_hugo_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.core.as_ref() else {
            return;
        };
        let service = frankmd_fs::service::NotesService::new(&core.vault);
        let path = frankmd_core::hugo::TEMPLATE_FILE;
        if !service.is_file(path).unwrap_or(false)
            && let Err(err) = service.create(path, frankmd_core::hugo::DEFAULT_TEMPLATE.as_bytes())
        {
            tracing::warn!(%err, "hugo template not created");
            return;
        }
        self.overlays.close();
        self.open_path(path, window, cx);
    }

    /// The sheet.
    pub(crate) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let query = self.settings.search.read(cx).value().to_lowercase();
        let config = self.core.as_ref().map(|c| c.ws.snapshot().config);
        let this = cx.entity().downgrade();

        let categories = Category::ALL.iter().map(|cat| {
            let cat = *cat;
            let selected = cat == self.settings.category && query.is_empty();
            let this = this.clone();
            div()
                .id(SharedString::from(format!(
                    "settings-category-{}",
                    cat.slug()
                )))
                .test_support()
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .rounded(theme.radius)
                .when(selected, |el| el.bg(theme.selection))
                .child(i18n::t(
                    cx,
                    &format!("settings_sheet.categories.{}", cat.slug()),
                ))
                .on_click(move |_, _, cx| {
                    let _ = this.update(cx, |shell, cx| {
                        shell.settings.category = cat;
                        cx.notify();
                    });
                })
        });
        let categories: Vec<_> = categories.collect();

        let mut body: Vec<AnyElement> = Vec::new();
        let rows: Vec<&RowSpec> = ROWS
            .iter()
            .filter(|r| {
                if query.is_empty() {
                    r.category == self.settings.category
                } else {
                    i18n::t(cx, r.label).to_lowercase().contains(&query)
                        || r.description
                            .is_some_and(|d| i18n::t(cx, d).to_lowercase().contains(&query))
                }
            })
            .collect();
        for row in &rows {
            body.push(
                self.render_setting_row(row, config.as_deref(), &theme, window, cx)
                    .into_any_element(),
            );
        }
        if query.is_empty()
            && self.settings.category == Category::Diagnostics
            && let Some(config) = config.as_deref()
        {
            {
                body.push(
                    div()
                        .pt_2()
                        .font_semibold()
                        .child(i18n::t(cx, "settings_sheet.config_sources"))
                        .into_any_element(),
                );
                for entry in config.entries() {
                    let value = if entry.sensitive && !entry.value.is_null() {
                        "****".to_owned()
                    } else {
                        entry.value.to_string()
                    };
                    let source = self.provenance_chip(entry.source, cx);
                    body.push(
                        h_flex()
                            .id(SharedString::from(format!(
                                "config-source-{}",
                                entry.key.name()
                            )))
                            .test_support()
                            .h(px(24.))
                            .gap_3()
                            .text_xs()
                            .child(div().w(px(200.)).truncate().child(entry.key.name()))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .font_family(theme.mono_font_family.clone())
                                    .child(value),
                            )
                            .child(div().text_color(theme.muted_foreground).child(source))
                            .into_any_element(),
                    );
                }
            }
        }
        if query.is_empty() && self.settings.category == Category::About {
            body.push(
                v_flex()
                    .id("settings-about")
                    .test_support()
                    .gap_2()
                    .items_center()
                    .pt_4()
                    .child(
                        svg()
                            .path("brand/icon.svg")
                            .size(px(64.))
                            .text_color(theme.primary),
                    )
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child(i18n::t(cx, "app.name")),
                    )
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(i18n::t(cx, "home.description")),
                    )
                    .child(div().text_xs().child(i18n::t_args(
                        cx,
                        "settings_sheet.version",
                        &[("version", env!("CARGO_PKG_VERSION"))],
                    )))
                    .into_any_element(),
            );
        }
        if query.is_empty() && self.settings.category == Category::Remote {
            body.push(
                self.render_remote_panel(&theme, config.as_deref(), window, cx)
                    .into_any_element(),
            );
        }
        if query.is_empty() && self.settings.category == Category::Encryption {
            body.push(
                self.render_encryption_panel(&theme, window, cx)
                    .into_any_element(),
            );
        }
        if body.is_empty() {
            body.push(
                div()
                    .text_color(theme.muted_foreground)
                    .child(i18n::t(cx, "settings_sheet.no_results"))
                    .into_any_element(),
            );
        }

        let close_this = this.clone();
        v_flex()
            .id("settings-sheet")
            .test_support()
            .track_focus(&self.overlay_focus)
            .w(px(960.))
            .h(px(680.))
            .max_w(relative(1.))
            .max_h(relative(1.))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .h(px(44.))
                    .px_4()
                    .gap_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().font_semibold().child(i18n::t(cx, "settings.title")))
                    .child(div().flex_1())
                    .child(
                        div()
                            .w(px(280.))
                            .child(Input::new(&self.settings.search).id("settings-search")),
                    )
                    .child(
                        Button::new("settings-close")
                            .ghost()
                            .small()
                            .icon(crate::ui::icons::ph("x"))
                            .on_click(move |_, window, cx| {
                                let _ = close_this
                                    .update(cx, |shell, cx| shell.close_overlay(window, cx));
                            }),
                    ),
            )
            .child(self.render_scope_toggle(&theme, cx))
            .child(
                h_flex()
                    .flex_1()
                    .items_start()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .w(px(200.))
                            .h_full()
                            .p_2()
                            .gap_1()
                            .bg(theme.sidebar)
                            .children(categories),
                    )
                    .child(
                        v_flex()
                            .id("settings-rows")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .p_4()
                            .gap_1()
                            .overflow_y_scroll()
                            .children(body),
                    ),
            )
    }

    /// Settings › Remote: how to reach this vault from another device, plus
    /// the live status of the server started in this session.
    fn render_remote_panel(
        &mut self,
        theme: &gpui_kit::component::theme::Theme,
        config: Option<&frankmd_config::ResolvedConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let vault = self
            .core
            .as_ref()
            .map(|c| c.vault.root().display().to_string())
            .unwrap_or_default();
        let quoted = shell_quote(&vault);
        let status = crate::remote::status(&self.state_dir);
        let tailscale = crate::remote::tailscale_ipv4_cached();
        let port = status
            .as_ref()
            .map_or(frankmd_remote::DEFAULT_PORT, |s| s.port);
        let url = status
            .as_ref()
            .map(|s| s.url.clone())
            .filter(|url| !url.is_empty())
            .or_else(|| tailscale.map(|ip| format!("http://{ip}:{port}")))
            .unwrap_or_else(|| format!("http://<tailscale-ip>:{port}"));
        let token_entry = config.map(|c| c.get(Key::AuthToken));
        let token_source = token_entry
            .as_ref()
            .and_then(|entry| entry.value.as_ref().map(|_| entry.source));
        let env_override = matches!(token_source, Some(Provenance::Env(_)));

        let prefs = self.app_prefs.get().clone();
        if self.settings.remote_port.is_none() {
            let input = cx
                .new(|cx| InputState::new(window, cx).default_value(prefs.remote_port.to_string()));
            let sub = cx.subscribe_in(
                &input,
                window,
                |shell, input, ev: &InputEvent, _window, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let text = input.read(cx).value().to_string();
                        shell.commit_remote_port(&text, cx);
                    }
                },
            );
            self.settings.subscriptions.push(sub);
            self.settings.remote_port = Some(input);
        }
        let port_input = self.settings.remote_port.clone().expect("created above");

        if self.settings.remote_token.is_none() {
            let placeholder = i18n::t(cx, "settings_sheet.remote.token_placeholder");
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(placeholder)
            });
            self.settings.remote_token = Some(input);
        }
        let token_input = self.settings.remote_token.clone().expect("created above");
        let save_path = match self.settings.scope {
            Scope::Vault => self.vault_root.join(".fed").display().to_string(),
            Scope::Global => self
                .global_config
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        };

        let (status_text, status_color) = match &status {
            Some(s) if s.running => (
                i18n::t_args(cx, "settings_sheet.remote.running", &[("url", &s.url)]),
                theme.success,
            ),
            Some(_) if token_source.is_none() => (
                i18n::t(cx, "settings_sheet.remote.token_required"),
                theme.warning,
            ),
            Some(s) => (
                i18n::t_args(
                    cx,
                    "settings_sheet.remote.start_failed",
                    &[("error", s.error.as_deref().unwrap_or(""))],
                ),
                theme.danger,
            ),
            None => (
                i18n::t(cx, "settings_sheet.remote.not_running"),
                theme.muted_foreground,
            ),
        };
        let (token_text, token_color) = match token_source {
            Some(Provenance::Env(name)) => (
                i18n::t_args(cx, "settings_sheet.remote.token_set_env", &[("name", name)]),
                theme.success,
            ),
            Some(Provenance::Fed) => (
                i18n::t(cx, "settings_sheet.remote.token_set_fed"),
                theme.success,
            ),
            Some(Provenance::Global) => (
                i18n::t(cx, "settings_sheet.remote.token_set_global"),
                theme.success,
            ),
            _ => (
                i18n::t(cx, "settings_sheet.remote.token_unset"),
                theme.warning,
            ),
        };
        let (tailscale_text, tailscale_color) = match tailscale {
            Some(ip) => (
                i18n::t_args(
                    cx,
                    "settings_sheet.remote.tailscale",
                    &[("ip", &ip.to_string())],
                ),
                theme.muted_foreground,
            ),
            None => (
                i18n::t(cx, "settings_sheet.remote.no_tailscale"),
                theme.warning,
            ),
        };

        let title = |text: SharedString| div().text_sm().font_semibold().child(text);
        let paragraph = |text: SharedString| {
            div()
                .w_full()
                .min_w_0()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let code = |id: &'static str, text: String| {
            div()
                .id(id)
                .test_support()
                .w_full()
                .font_family(theme.mono_font_family.clone())
                .text_xs()
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .bg(theme.muted)
                .child(text)
        };
        let code_row = |id: &'static str, text: String| code(id, text).flex_1().min_w_0();
        let copy = |id: &'static str, text: String| {
            let this = cx.entity().downgrade();
            let label = i18n::t(cx, "settings_sheet.remote.copy");
            Button::new(id)
                .small()
                .outline()
                .label(label)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    let message = i18n::t(cx, "settings_sheet.remote.copied");
                    let _ = this.update(cx, |shell, cx| shell.show_message(&message, false, cx));
                })
        };

        let start_command = format!("frankmdfast --serve {quoted}");
        let headless_command = format!("frankmdfast --serve-only {quoted}");

        let command_row = |id: &'static str, command: String| {
            let copy_id = match id {
                "settings-remote-start" => "settings-remote-copy-start",
                "settings-remote-headless" => "settings-remote-copy-headless",
                _ => "settings-remote-copy-url",
            };
            h_flex()
                .gap_2()
                .items_center()
                .child(code_row(id, command.clone()))
                .child(copy(copy_id, command))
        };

        let toggle_this = cx.entity().downgrade();
        let toggle = Switch::new("settings-remote-start")
            .checked(prefs.remote_start)
            .on_click(move |checked, _, cx| {
                let enabled = *checked;
                let _ = toggle_this.update(cx, |shell, cx| shell.set_remote_start(enabled, cx));
            });
        let bind_tailscale = {
            let this = cx.entity().downgrade();
            Button::new("settings-remote-bind-tailscale")
                .small()
                .selected(prefs.remote_bind == "tailscale")
                .label(i18n::t(cx, "settings_sheet.remote.bind_tailscale"))
                .on_click(move |_, _, cx| {
                    let _ = this.update(cx, |shell, cx| shell.set_remote_bind("tailscale", cx));
                })
        };
        let bind_loopback = {
            let this = cx.entity().downgrade();
            Button::new("settings-remote-bind-loopback")
                .small()
                .selected(prefs.remote_bind == "loopback")
                .label(i18n::t(cx, "settings_sheet.remote.bind_loopback"))
                .on_click(move |_, _, cx| {
                    let _ = this.update(cx, |shell, cx| shell.set_remote_bind("loopback", cx));
                })
        };

        let running = status.as_ref().is_some_and(|status| status.running);
        let start_stop = if running {
            let this = cx.entity().downgrade();
            Button::new("settings-remote-stop-now")
                .small()
                .outline()
                .label(i18n::t(cx, "settings_sheet.remote.stop_server"))
                .on_click(move |_, _, cx| {
                    let _ = this.update(cx, |shell, cx| shell.stop_remote_now(cx));
                })
        } else {
            let this = cx.entity().downgrade();
            Button::new("settings-remote-start-now")
                .small()
                .primary()
                .label(i18n::t(cx, "settings_sheet.remote.start_server"))
                .on_click(move |_, _, cx| {
                    let _ = this.update(cx, |shell, cx| shell.start_remote_now(cx));
                })
        };
        let login_this = cx.entity().downgrade();
        let login_switch = Switch::new("settings-remote-login")
            .checked(prefs.remote_login)
            .on_click(move |checked, _, cx| {
                let _ = login_this.update(cx, |shell, cx| shell.set_remote_login(*checked, cx));
            });

        let generate_this = cx.entity().downgrade();
        let generate = Button::new("settings-remote-token-generate")
            .small()
            .outline()
            .label(i18n::t(cx, "settings_sheet.remote.token_generate"))
            .on_click(move |_, window, cx| {
                let _ =
                    generate_this.update(cx, |shell, cx| shell.generate_remote_token(window, cx));
            });
        let save_this = cx.entity().downgrade();
        let save = Button::new("settings-remote-token-save")
            .small()
            .primary()
            .label(i18n::t(cx, "settings_sheet.remote.token_save"))
            .on_click(move |_, window, cx| {
                let _ = save_this.update(cx, |shell, cx| shell.save_remote_token(window, cx));
            });
        let copy_token_this = cx.entity().downgrade();
        let copy_token = Button::new("settings-remote-token-copy")
            .small()
            .outline()
            .label(i18n::t(cx, "settings_sheet.remote.token_copy"))
            .on_click(move |_, _, cx| {
                let _ = copy_token_this.update(cx, |shell, cx| shell.copy_remote_token(cx));
            });

        v_flex()
            .id("settings-remote")
            .test_support()
            .w_full()
            .min_w_0()
            .gap_1()
            .pt_1()
            .child(title(i18n::t(cx, "settings_sheet.remote.title")))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .flex_wrap()
                    .child(
                        h_flex().gap_2().items_center().child(toggle).child(
                            div()
                                .text_sm()
                                .child(i18n::t(cx, "settings_sheet.remote.start_on_launch")),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(i18n::t(cx, "settings_sheet.remote.bind")),
                    )
                    .child(bind_tailscale)
                    .child(bind_loopback)
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(i18n::t(cx, "settings_sheet.remote.port")),
                    )
                    .child(
                        div()
                            .w(px(96.))
                            .child(Input::new(&port_input).id("settings-remote-port")),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(i18n::t(cx, "settings_sheet.remote.restart_hint")),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .font_semibold()
                            .child(i18n::t(cx, "settings_sheet.remote.status")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(status_color)
                            .child(status_text),
                    ),
            )
            .child(div().text_xs().text_color(token_color).child(token_text))
            .child(
                div()
                    .text_xs()
                    .text_color(tailscale_color)
                    .child(tailscale_text),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .items_center()
                    .flex_wrap()
                    .child(start_stop)
                    .child(login_switch)
                    .child(
                        div()
                            .text_sm()
                            .child(i18n::t(cx, "settings_sheet.remote.start_at_login")),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(i18n::t(cx, "settings_sheet.remote.login_hint")),
            )
            .when(prefs.remote_login && env_override, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(i18n::t(cx, "settings_sheet.remote.login_token_warning")),
                )
            })
            .child(title(i18n::t(cx, "settings_sheet.remote.token_title")))
            .child(paragraph(i18n::t(cx, "settings_sheet.remote.token_help")))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&token_input).id("settings-remote-token-input")),
                    )
                    .child(generate)
                    .child(save)
                    .child(copy_token),
            )
            .child(
                div()
                    .text_xs()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.muted_foreground)
                    .child(i18n::t_args(
                        cx,
                        "settings_sheet.remote.token_save_path",
                        &[("path", &save_path)],
                    )),
            )
            .when(env_override, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(i18n::t(cx, "settings_sheet.remote.token_env_override")),
                )
            })
            .child(title(i18n::t(cx, "settings_sheet.remote.how_to")))
            .child(paragraph(i18n::t(cx, "settings_sheet.remote.step_start")))
            .child(command_row("settings-remote-start", start_command))
            .child(paragraph(i18n::t(
                cx,
                "settings_sheet.remote.step_headless",
            )))
            .child(command_row("settings-remote-headless", headless_command))
            .child(paragraph(i18n::t(cx, "settings_sheet.remote.step_open")))
            .child(command_row("settings-remote-url", url))
            .child(title(i18n::t(cx, "settings_sheet.remote.security")))
            .child(paragraph(i18n::t(
                cx,
                "settings_sheet.remote.security_text",
            )))
            .into_any_element()
    }

    /// Settings › Remote: remember the app-launch preference and apply it now.
    pub(crate) fn set_remote_start(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.app_prefs.set_remote_start(enabled);
        if enabled {
            self.start_remote_action(cx);
        } else {
            self.stop_remote_action(cx);
        }
        cx.notify();
    }

    /// Settings › Remote: start the server now (systemd unit or detached daemon).
    pub(crate) fn start_remote_now(&mut self, cx: &mut Context<Self>) {
        self.start_remote_action(cx);
        cx.notify();
    }

    /// Settings › Remote: stop the server now, keeping the login unit enabled.
    pub(crate) fn stop_remote_now(&mut self, cx: &mut Context<Self>) {
        self.stop_remote_action(cx);
        cx.notify();
    }

    fn start_remote_action(&mut self, cx: &mut Context<Self>) {
        let result = if crate::remote::unit_enabled() {
            crate::remote::start_unit().map_err(|err| SharedString::from(format!("{err:#}")))
        } else {
            self.remote_request(cx).and_then(|request| {
                crate::remote::spawn_daemon_request(&request)
                    .map(|_| ())
                    .map_err(|err| SharedString::from(format!("{err:#}")))
            })
        };
        match result {
            Ok(()) => {
                let text = i18n::t(cx, "settings_sheet.remote.started");
                self.show_message(&text, false, cx);
            }
            Err(err) => {
                crate::remote::record_error(&self.state_dir, &err);
                self.show_message(&err, true, cx);
            }
        }
    }

    fn stop_remote_action(&mut self, cx: &mut Context<Self>) {
        let result = if crate::remote::unit_enabled() {
            crate::remote::stop_unit().map_err(|err| SharedString::from(format!("{err:#}")))
        } else {
            crate::remote::stop_daemon(&self.state_dir);
            Ok(())
        };
        match result {
            Ok(()) => {
                let text = i18n::t(cx, "settings_sheet.remote.stopped");
                self.show_message(&text, false, cx);
            }
            Err(err) => {
                crate::remote::record_error(&self.state_dir, &err);
                self.show_message(&err, true, cx);
            }
        }
    }

    /// Settings › Remote: enable/disable the systemd user unit that starts at login.
    pub(crate) fn set_remote_login(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled {
            let env_token = self.core.as_ref().is_some_and(|core| {
                matches!(
                    core.ws.snapshot().config.get(Key::AuthToken).source,
                    Provenance::Env(_)
                )
            });
            if env_token {
                let message = i18n::t(cx, "settings_sheet.remote.login_token_warning");
                self.show_message(&message, true, cx);
                cx.notify();
                return;
            }
            let result = self.remote_request(cx).and_then(|request| {
                crate::remote::enable_unit(&self.state_dir, &self.config_dir, &request)
                    .map_err(|err| SharedString::from(format!("{err:#}")))
            });
            match result {
                Ok(()) => {
                    self.app_prefs.set_remote_login(true);
                    let message = i18n::t(cx, "settings_sheet.remote.login_enabled");
                    self.show_message(&message, false, cx);
                }
                Err(err) => {
                    self.show_message(&err, true, cx);
                }
            }
        } else {
            let result = crate::remote::disable_unit(&self.state_dir)
                .map_err(|err| SharedString::from(format!("{err:#}")));
            match result {
                Ok(()) => {
                    self.app_prefs.set_remote_login(false);
                    let message = i18n::t(cx, "settings_sheet.remote.login_disabled");
                    self.show_message(&message, false, cx);
                }
                Err(err) => {
                    self.show_message(&err, true, cx);
                }
            }
        }
        cx.notify();
    }

    /// Settings › Remote bind choice.
    pub(crate) fn set_remote_bind(&mut self, bind: &str, cx: &mut Context<Self>) {
        self.app_prefs.set_remote_bind(bind);
        self.refresh_remote(cx);
        cx.notify();
    }

    /// Settings › Remote port (committed on Enter/blur).
    pub(crate) fn commit_remote_port(&mut self, text: &str, cx: &mut Context<Self>) {
        let Ok(port) = text.trim().parse::<u16>() else {
            let message = i18n::t(cx, "settings_sheet.remote.invalid_port");
            self.show_message(&message, true, cx);
            return;
        };
        if port == 0 {
            let message = i18n::t(cx, "settings_sheet.remote.invalid_port");
            self.show_message(&message, true, cx);
            return;
        }
        self.app_prefs.set_remote_port(port);
        self.refresh_remote(cx);
        cx.notify();
    }

    /// Generate a 32-byte hex token, put it in the field and copy it.
    pub(crate) fn generate_remote_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut bytes = [0u8; 32];
        if let Err(err) = getrandom::fill(&mut bytes) {
            tracing::error!(%err, "could not generate a remote token");
            let message = i18n::t(cx, "settings_sheet.remote.token_generate_failed");
            self.show_message(&message, true, cx);
            return;
        }
        use std::fmt::Write as _;
        let mut token = String::with_capacity(64);
        for byte in bytes {
            let _ = write!(token, "{byte:02x}");
        }
        if let Some(input) = self.settings.remote_token.clone() {
            input.update(cx, |state, cx| state.set_value(&token, window, cx));
        }
        cx.write_to_clipboard(ClipboardItem::new_string(token));
        let message = i18n::t(cx, "settings_sheet.remote.token_generated");
        self.show_message(&message, false, cx);
        cx.notify();
    }

    /// Copy the effective token (field first, otherwise the saved/env one).
    pub(crate) fn copy_remote_token(&mut self, cx: &mut Context<Self>) {
        let from_input = self
            .settings
            .remote_token
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_owned())
            .filter(|token| !token.is_empty());
        let token = from_input.or_else(|| {
            self.core
                .as_ref()
                .and_then(|c| c.ws.snapshot().config.value(Key::AuthToken))
                .and_then(|value| value.as_str().map(str::to_owned))
        });
        match token {
            Some(token) => {
                cx.write_to_clipboard(ClipboardItem::new_string(token));
                let message = i18n::t(cx, "settings_sheet.remote.token_copied");
                self.show_message(&message, false, cx);
            }
            None => {
                let message = i18n::t(cx, "settings_sheet.remote.token_none");
                self.show_message(&message, true, cx);
            }
        }
        cx.notify();
    }

    /// Save the token field to the selected scope and restart the server.
    pub(crate) fn save_remote_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.settings.remote_token.clone() else {
            return;
        };
        let token = input.read(cx).value().trim().to_owned();
        if token.len() < 32 {
            let message = i18n::t(cx, "settings_sheet.remote.token_too_short");
            self.show_message(&message, true, cx);
            return;
        }
        let changes = vec![(Key::AuthToken, Some(Value::Str(token.clone())))];
        let cmd = match self.settings.scope {
            Scope::Vault => Command::UpdateSettings { changes },
            Scope::Global => Command::UpdateGlobalSettings { changes },
        };
        match self.dispatch_checked(cmd, window, cx) {
            Ok(()) => {
                cx.write_to_clipboard(ClipboardItem::new_string(token));
                let message = i18n::t(cx, "settings_sheet.remote.token_saved");
                self.show_message(&message, false, cx);
                self.refresh_remote(cx);
            }
            Err(err) => {
                let message = i18n::t(cx, err.key);
                self.show_message(&message, true, cx);
            }
        }
        cx.notify();
    }

    /// Restart after bind/port/token changes: rewrite the login unit, or
    /// restart the detached daemon.
    fn refresh_remote(&mut self, cx: &mut Context<Self>) {
        if self.app_prefs.get().remote_login || crate::remote::unit_enabled() {
            let result = self.remote_request(cx).and_then(|request| {
                crate::remote::restart_unit(&self.config_dir, &request)
                    .map_err(|err| SharedString::from(format!("{err:#}")))
            });
            match result {
                Ok(()) => {
                    let text = i18n::t(cx, "settings_sheet.remote.started");
                    self.show_message(&text, false, cx);
                }
                Err(err) => {
                    crate::remote::record_error(&self.state_dir, &err);
                    self.show_message(&err, true, cx);
                }
            }
            return;
        }
        if !crate::remote::is_running(&self.state_dir) && !self.app_prefs.get().remote_start {
            return;
        }
        crate::remote::stop_daemon(&self.state_dir);
        self.start_remote_action(cx);
    }

    /// Build the request used to spawn the daemon or write the login unit.
    fn remote_request(
        &self,
        cx: &Context<Self>,
    ) -> Result<crate::remote::RemoteRequest, SharedString> {
        let Some(core) = self.core.as_ref() else {
            return Err(i18n::t(cx, "settings_sheet.remote.vault_unavailable"));
        };
        let snapshot = core.ws.snapshot();
        let Some(token) = snapshot
            .config
            .value(Key::AuthToken)
            .and_then(|value| value.as_str().map(str::to_owned))
            .filter(|token| !token.trim().is_empty())
        else {
            return Err(i18n::t(cx, "settings_sheet.remote.token_required"));
        };
        let prefs = self.app_prefs.get().clone();
        Ok(crate::remote::RemoteRequest {
            vault_root: self.vault_root.clone(),
            state_dir: self.state_dir.clone(),
            env: snapshot.config.env().clone(),
            bind_spec: prefs.remote_bind.clone(),
            bind_any: false,
            port: prefs.remote_port,
            token,
            locale: self.locale_override.map(|locale| locale.code().to_owned()),
        })
    }

    /// Settings › Encryption: create/unlock/lock, change the password and back
    /// up/import the seal.
    fn render_encryption_panel(
        &mut self,
        theme: &gpui_kit::component::theme::Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.settings.encryption_password.is_none() {
            let password = cx.new(|cx| InputState::new(window, cx).masked(true));
            let confirm = cx.new(|cx| InputState::new(window, cx).masked(true));
            let backup = cx.new(|cx| InputState::new(window, cx));
            self.settings.encryption_password = Some(password);
            self.settings.encryption_confirm = Some(confirm);
            self.settings.encryption_backup = Some(backup);
        }
        let password_input = self.settings.encryption_password.clone().expect("created");
        let confirm_input = self.settings.encryption_confirm.clone().expect("created");
        let backup_input = self.settings.encryption_backup.clone().expect("created");

        let has_seal = self.encryption.seal().is_some();
        let unlocked = self.encryption.is_unlocked();
        let (status_text, status_color) = if !has_seal {
            (
                i18n::t(cx, "settings_sheet.encryption.status_unconfigured"),
                theme.warning,
            )
        } else if unlocked {
            (
                i18n::t(cx, "settings_sheet.encryption.status_unlocked"),
                theme.success,
            )
        } else {
            (
                i18n::t(cx, "settings_sheet.encryption.status_locked"),
                theme.muted_foreground,
            )
        };

        let title = |text: SharedString| div().text_sm().font_semibold().child(text);
        let paragraph = |text: SharedString| {
            div()
                .w_full()
                .min_w_0()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let label = |text: SharedString| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };

        let input_row = |id: &'static str, input: &Entity<InputState>, button: Button| {
            h_flex()
                .w_full()
                .min_w_0()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w_0().child(Input::new(input).id(id)))
                .child(button)
        };

        let create_this = cx.entity().downgrade();
        let create = Button::new("encryption-create")
            .small()
            .primary()
            .label(i18n::t(cx, "settings_sheet.encryption.create"))
            .on_click(move |_, window, cx| {
                let _ = create_this.update(cx, |shell, cx| shell.create_encryption_key(window, cx));
            });
        let unlock_this = cx.entity().downgrade();
        let unlock = Button::new("encryption-unlock")
            .small()
            .primary()
            .label(i18n::t(cx, "settings_sheet.encryption.unlock"))
            .on_click(move |_, window, cx| {
                let _ = unlock_this.update(cx, |shell, cx| shell.unlock_encryption(window, cx));
            });
        let lock_this = cx.entity().downgrade();
        let lock = Button::new("encryption-lock")
            .small()
            .outline()
            .label(i18n::t(cx, "settings_sheet.encryption.lock"))
            .on_click(move |_, window, cx| {
                let _ = lock_this.update(cx, |shell, cx| shell.lock_encryption(window, cx));
            });
        let change_this = cx.entity().downgrade();
        let change = Button::new("encryption-change")
            .small()
            .outline()
            .label(i18n::t(cx, "settings_sheet.encryption.change"))
            .on_click(move |_, window, cx| {
                let _ = change_this
                    .update(cx, |shell, cx| shell.change_encryption_password(window, cx));
            });
        let repair_this = cx.entity().downgrade();
        let repair_attachments = Button::new("encryption-repair-attachments")
            .small()
            .outline()
            .label(i18n::t(cx, "settings_sheet.encryption.attachment_repair"))
            .on_click(move |_, window, cx| {
                let _ = repair_this.update(cx, |shell, cx| {
                    shell.encrypt_attachments_of_encrypted_notes(window, cx)
                });
            });
        let copy_this = cx.entity().downgrade();
        let copy_backup = Button::new("encryption-copy-backup")
            .small()
            .outline()
            .label(i18n::t(cx, "settings_sheet.encryption.backup_copy"))
            .on_click(move |_, _, cx| {
                let _ = copy_this.update(cx, |shell, cx| shell.copy_seal_backup(cx));
            });
        let export_this = cx.entity().downgrade();
        let export_file = Button::new("encryption-export-file")
            .small()
            .outline()
            .label(i18n::t(cx, "settings_sheet.encryption.export_file"))
            .on_click(move |_, window, cx| {
                let _ = export_this.update(cx, |shell, cx| shell.export_seal_file(window, cx));
            });
        let import_file_this = cx.entity().downgrade();
        let import_file = Button::new("encryption-import-file")
            .small()
            .outline()
            .label(i18n::t(cx, "settings_sheet.encryption.import_file"))
            .on_click(move |_, window, cx| {
                let _ = import_file_this.update(cx, |shell, cx| shell.import_seal_file(window, cx));
            });
        let import_this = cx.entity().downgrade();
        let import = Button::new("encryption-import")
            .small()
            .outline()
            .label(i18n::t(cx, "settings_sheet.encryption.import"))
            .on_click(move |_, window, cx| {
                let _ = import_this.update(cx, |shell, cx| shell.import_seal_backup(window, cx));
            });

        v_flex()
            .id("settings-encryption")
            .test_support()
            .w_full()
            .min_w_0()
            .gap_2()
            .pt_1()
            .child(title(i18n::t(cx, "settings_sheet.encryption.title")))
            .child(paragraph(i18n::t(cx, "settings_sheet.encryption.intro")))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(i18n::t(cx, "settings_sheet.encryption.status")),
                    )
                    .child(div().text_sm().text_color(status_color).child(status_text)),
            )
            .when(!has_seal, |el| {
                el.child(label(i18n::t(cx, "settings_sheet.encryption.password")))
                    .child(input_row("encryption-password", &password_input, create))
                    .child(label(i18n::t(cx, "settings_sheet.encryption.confirm")))
                    .child(
                        div()
                            .w_full()
                            .child(Input::new(&confirm_input).id("encryption-confirm")),
                    )
            })
            .when(has_seal && !unlocked, |el| {
                el.child(label(i18n::t(cx, "settings_sheet.encryption.password")))
                    .child(input_row("encryption-password", &password_input, unlock))
            })
            .when(has_seal && unlocked, |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .flex_wrap()
                        .child(lock)
                        .child(change)
                        .child(repair_attachments),
                )
                .child(label(i18n::t(
                    cx,
                    "settings_sheet.encryption.attachment_repair_hint",
                )))
                .when_some(
                    self.settings.encryption_repair.clone(),
                    |el, (text, error)| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(if error { theme.danger } else { theme.success })
                                .child(text),
                        )
                    },
                )
                .child(label(i18n::t(cx, "settings_sheet.encryption.new_password")))
                .child(
                    div()
                        .w_full()
                        .child(Input::new(&password_input).id("encryption-password")),
                )
                .child(label(i18n::t(cx, "settings_sheet.encryption.confirm")))
                .child(
                    div()
                        .w_full()
                        .child(Input::new(&confirm_input).id("encryption-confirm")),
                )
            })
            .child(title(i18n::t(cx, "settings_sheet.encryption.backup_title")))
            .child(paragraph(i18n::t(
                cx,
                "settings_sheet.encryption.backup_hint",
            )))
            .when(has_seal, |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(copy_backup)
                        .child(export_file),
                )
            })
            .child(import_file)
            .child(label(i18n::t(
                cx,
                "settings_sheet.encryption.backup_import",
            )))
            .child(input_row("encryption-backup", &backup_input, import))
            .when_some(self.settings.pending_seal_import.clone(), |el, path| {
                let confirm_path = path.clone();
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(i18n::t_args(
                            cx,
                            "settings_sheet.encryption.confirm_replace",
                            &[("path", &path.display().to_string())],
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("encryption-import-confirm")
                                .small()
                                .primary()
                                .label(i18n::t(cx, "settings_sheet.encryption.import"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.apply_seal_import(confirm_path.clone(), window, cx)
                                })),
                        )
                        .child(
                            Button::new("encryption-import-cancel")
                                .small()
                                .label(i18n::t(cx, "common.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings.pending_seal_import = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .w_full()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(i18n::t(cx, "settings_sheet.encryption.warning")),
            )
            .into_any_element()
    }

    fn encryption_password(&self, cx: &Context<Self>) -> String {
        self.settings
            .encryption_password
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default()
    }

    fn encryption_confirm(&self, cx: &Context<Self>) -> String {
        self.settings
            .encryption_confirm
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default()
    }

    fn clear_encryption_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            self.settings.encryption_password.clone(),
            self.settings.encryption_confirm.clone(),
            self.settings.encryption_backup.clone(),
        ]
        .into_iter()
        .flatten()
        {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    pub(crate) fn create_encryption_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.encryption_password(cx);
        let confirm = self.encryption_confirm(cx);
        if password != confirm {
            let message = i18n::t(cx, "settings_sheet.encryption.password_mismatch");
            self.show_message(&message, true, cx);
            return;
        }
        if crate::encryption::validate_password(&password).is_err() {
            let message = i18n::t(cx, "settings_sheet.encryption.password_short");
            self.show_message(&message, true, cx);
            return;
        }
        match self.encryption.create(&password) {
            Ok(seal) => {
                if let Err(err) = crate::encryption::save_seal(&self.state_dir, &seal) {
                    let message = SharedString::from(format!("{err:#}"));
                    self.show_message(&message, true, cx);
                    return;
                }
                self.clear_encryption_inputs(window, cx);
                let message = i18n::t(cx, "settings_sheet.encryption.created");
                self.show_message(&message, false, cx);
                self.refresh_remote(cx);
            }
            Err(err) => {
                let message = SharedString::from(err.to_string());
                self.show_message(&message, true, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn unlock_encryption(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.encryption_password(cx);
        match self.encryption.unlock(&password) {
            Ok(()) => {
                let message = i18n::t(cx, "settings_sheet.encryption.unlocked");
                self.show_message(&message, false, cx);
            }
            Err(err) => {
                let message = i18n::t(cx, "settings_sheet.encryption.wrong_password");
                let _ = err;
                self.show_message(&message, true, cx);
            }
        }
        self.clear_encryption_inputs(window, cx);
        if self.encryption.is_unlocked()
            && let Some(path) = self.locked_note.take()
        {
            self.open_path(&path, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn lock_encryption(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lock_vault(window, cx);
    }

    pub(crate) fn change_encryption_password(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.encryption.is_unlocked() {
            let message = i18n::t(cx, "settings_sheet.encryption.status_locked");
            self.show_message(&message, true, cx);
            return;
        }
        let password = self.encryption_password(cx);
        let confirm = self.encryption_confirm(cx);
        if password != confirm {
            let message = i18n::t(cx, "settings_sheet.encryption.password_mismatch");
            self.show_message(&message, true, cx);
            return;
        }
        if crate::encryption::validate_password(&password).is_err() {
            let message = i18n::t(cx, "settings_sheet.encryption.password_short");
            self.show_message(&message, true, cx);
            return;
        }
        let root = self.vault_root.clone();
        let result = self.encryption.rekey(&password, |old, new| {
            crate::encryption::rekey_files(&root, old, new).map(|_| ())
        });
        match result {
            Ok(seal) => {
                if let Err(err) = crate::encryption::save_seal(&self.state_dir, &seal) {
                    let message = SharedString::from(format!("{err:#}"));
                    self.show_message(&message, true, cx);
                    return;
                }
                self.clear_encryption_inputs(window, cx);
                let message = i18n::t(cx, "settings_sheet.encryption.changed");
                self.show_message(&message, false, cx);
                self.refresh_remote(cx);
            }
            Err(err) => {
                let message = SharedString::from(err.to_string());
                self.show_message(&message, true, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn copy_seal_backup(&mut self, cx: &mut Context<Self>) {
        match crate::encryption::seal_backup(&self.encryption) {
            Some(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                let message = i18n::t(cx, "settings_sheet.encryption.backup_copied");
                self.show_message(&message, false, cx);
            }
            None => {
                let message = i18n::t(cx, "settings_sheet.encryption.status_unconfigured");
                self.show_message(&message, true, cx);
            }
        }
        cx.notify();
    }

    fn seal_file_name(&self) -> String {
        let mut name = String::from("frankmd-seal-");
        for ch in self.vault_name.chars() {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                name.push(ch);
            } else if !name.ends_with('-') {
                name.push('-');
            }
        }
        name.push_str(".json");
        name
    }

    pub(crate) fn export_seal_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(seal) = self.encryption.seal() else {
            let message = i18n::t(cx, "settings_sheet.encryption.status_unconfigured");
            self.show_message(&message, true, cx);
            return;
        };
        let directory = self
            .vault_root
            .parent()
            .unwrap_or(&self.vault_root)
            .to_path_buf();
        let suggested = self.seal_file_name();
        let rx = cx.prompt_for_new_path(&directory, Some(&suggested));
        let task = cx.spawn_in(window, async move |this, cx| {
            let path = match rx.await {
                Ok(Ok(Some(path))) => path,
                Ok(Err(err)) => {
                    this.update_in(cx, |shell, _window, cx| {
                        let message = SharedString::from(format!(
                            "{}: {err:#}",
                            i18n::t(cx, "settings_sheet.encryption.export_failed")
                        ));
                        shell.show_message(&message, true, cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                _ => return,
            };
            let result = cx
                .background_executor()
                .spawn(async move { crate::encryption::export_seal_to(&path, &seal) })
                .await;
            this.update_in(cx, |shell, _window, cx| {
                match result {
                    Ok(()) => {
                        let message = i18n::t(cx, "settings_sheet.encryption.export_saved");
                        shell.show_message(&message, false, cx);
                    }
                    Err(err) => {
                        let message = SharedString::from(format!(
                            "{}: {err:#}",
                            i18n::t(cx, "settings_sheet.encryption.export_failed")
                        ));
                        shell.show_message(&message, true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.settings.encryption_jobs.push(task);
    }

    pub(crate) fn import_seal_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = Some(i18n::t(cx, "settings_sheet.encryption.import_file"));
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt,
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            let path = match rx.await {
                Ok(Ok(Some(mut paths))) => match paths.pop() {
                    Some(path) => path,
                    None => return,
                },
                Ok(Err(err)) => {
                    this.update_in(cx, |shell, _window, cx| {
                        let message = SharedString::from(format!(
                            "{}: {err:#}",
                            i18n::t(cx, "settings_sheet.encryption.import_failed")
                        ));
                        shell.show_message(&message, true, cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                _ => return,
            };
            this.update_in(cx, |shell, window, cx| {
                if shell.encryption.seal().is_some() {
                    shell.settings.pending_seal_import = Some(path);
                    cx.notify();
                } else {
                    shell.apply_seal_import(path, window, cx);
                }
            })
            .ok();
        });
        self.settings.encryption_jobs.push(task);
    }

    pub(crate) fn apply_seal_import(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let seal = match crate::encryption::read_seal_file(&path) {
            Ok(seal) => seal,
            Err(err) => {
                let message = SharedString::from(format!(
                    "{}: {err:#}",
                    i18n::t(cx, "settings_sheet.encryption.import_failed")
                ));
                self.show_message(&message, true, cx);
                return;
            }
        };
        if let Err(err) = crate::encryption::save_seal(&self.state_dir, &seal) {
            let message = SharedString::from(format!("{err:#}"));
            self.show_message(&message, true, cx);
            return;
        }
        self.encryption.set_seal(Some(seal));
        self.settings.pending_seal_import = None;
        self.lock_vault(window, cx);
        self.refresh_remote(cx);
        let message = i18n::t(cx, "settings_sheet.encryption.backup_imported");
        self.show_message(&message, false, cx);
        cx.notify();
    }

    pub(crate) fn import_seal_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .settings
            .encryption_backup
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        match crate::encryption::import_seal(&self.encryption, &text) {
            Ok(seal) => {
                if let Err(err) = crate::encryption::save_seal(&self.state_dir, &seal) {
                    let message = SharedString::from(format!("{err:#}"));
                    self.show_message(&message, true, cx);
                    return;
                }
                self.clear_encryption_inputs(window, cx);
                let message = i18n::t(cx, "settings_sheet.encryption.backup_imported");
                self.show_message(&message, false, cx);
                self.refresh_remote(cx);
            }
            Err(err) => {
                let message = SharedString::from(format!("{err:#}"));
                self.show_message(&message, true, cx);
            }
        }
        cx.notify();
    }

    fn render_setting_row(
        &mut self,
        row: &RowSpec,
        config: Option<&frankmd_config::ResolvedConfig>,
        theme: &gpui_kit::component::theme::Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity().downgrade();
        let id_slug = row.label.rsplit('.').next().unwrap_or(row.label).to_owned();
        let this_reset = this.clone();
        let row_key = match row.control {
            Control::Switch(k)
            | Control::Text(k)
            | Control::Secret(k)
            | Control::Choice(k, _)
            | Control::IntChoice(k, _)
            | Control::Steps(k, _, _) => Some(k),
            Control::Picker("font.select") => Some(Key::EditorFont),
            Control::ViewMode => Some(Key::ViewMode),
            Control::MediaStorage => Some(Key::MediaStorageProvider),
            _ => None,
        };
        let label = i18n::t(cx, row.label);
        let description = row.description.map(|d| i18n::t(cx, d));
        let resolved = |key: Key| config.map(|c| c.get(key));
        let mut chip: Option<SharedString> = None;
        let control: AnyElement = match row.control {
            Control::Switch(key) => {
                let r = resolved(key);
                let on = r
                    .as_ref()
                    .and_then(|r| r.value.as_ref())
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                chip = r.and_then(|r| self.row_chip(r.source, cx));
                Switch::new(SharedString::from(format!("setting-{id_slug}")))
                    .checked(on)
                    .on_click(move |checked, window, cx| {
                        let checked = *checked;
                        let _ = this.update(cx, |shell, cx| {
                            shell.write_settings(
                                vec![(key, Some(Value::Bool(checked)))],
                                window,
                                cx,
                            )
                        });
                    })
                    .into_any_element()
            }
            Control::Picker(command) => {
                let text = match command {
                    "font.select" => {
                        let r = resolved(Key::EditorFont);
                        chip = r.as_ref().and_then(|r| self.row_chip(r.source, cx));
                        let id = r
                            .as_ref()
                            .and_then(|r| r.value.as_ref())
                            .and_then(Value::as_str);
                        let id = id.unwrap_or("cascadia-code");
                        super::fonts::family_of(id).unwrap_or(id).to_owned()
                    }
                    "theme.select" => match &self.theme.omarchy_name {
                        Some(name) => {
                            i18n::t_args(cx, "palette.follow_omarchy", &[("theme", name)])
                                .to_string()
                        }
                        None => self.theme.id.clone(),
                    },
                    _ => {
                        let name = i18n::language_name(cx, i18n::current(cx).locale());
                        let explicit = config
                            .is_some_and(|c| c.get(Key::Locale).source != Provenance::Default);
                        if explicit {
                            name.to_string()
                        } else {
                            // Following Omarchy / LANG: say so, like the theme row.
                            i18n::t_args(cx, "palette.system_language", &[("language", &name)])
                                .to_string()
                        }
                    }
                };
                Button::new(SharedString::from(format!("setting-{id_slug}")))
                    .small()
                    .outline()
                    .label(format!("{text} ▾"))
                    .on_click(move |_, window, cx| {
                        let _ = this.update(cx, |shell, cx| {
                            shell.overlays.close();
                            if command == "font.select" {
                                // Not a registry command: only reachable from here.
                                use frankmd_workspace::overlay::{OverlayKind, PaletteMode};
                                let palette = OverlayKind::Palette(PaletteMode::Font);
                                shell.open_overlay(palette, window, cx);
                            } else {
                                shell.run_command(command, window, cx);
                            }
                        });
                    })
                    .into_any_element()
            }
            Control::ViewMode => {
                let single = self.layout.view_mode == frankmd_workspace::layout::ViewMode::Single;
                chip = resolved(Key::ViewMode).and_then(|r| self.row_chip(r.source, cx));
                let this2 = this.clone();
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("setting-view-mode-split")
                            .small()
                            .selected(!single)
                            .label(i18n::t(cx, "settings.view_mode_split"))
                            .on_click(move |_, window, cx| {
                                let _ = this.update(cx, |shell, cx| {
                                    shell.write_settings(
                                        vec![(Key::ViewMode, Some(Value::Str("split".into())))],
                                        window,
                                        cx,
                                    );
                                });
                            }),
                    )
                    .child(
                        Button::new("setting-view-mode-single")
                            .small()
                            .selected(single)
                            .label(i18n::t(cx, "settings.view_mode_single"))
                            .on_click(move |_, window, cx| {
                                let _ = this2.update(cx, |shell, cx| {
                                    shell.write_settings(
                                        vec![(Key::ViewMode, Some(Value::Str("single".into())))],
                                        window,
                                        cx,
                                    );
                                });
                            }),
                    )
                    .into_any_element()
            }
            Control::NotesFolder => div()
                .text_xs()
                .child(self.vault_root.display().to_string())
                .into_any_element(),
            Control::Text(key) => {
                let r = resolved(key);
                chip = r.as_ref().and_then(|r| self.row_chip(r.source, cx));
                let current = r
                    .as_ref()
                    .and_then(|r| r.value.as_ref())
                    .map(|v| match v {
                        Value::Str(s) => s.clone(),
                        other => value_text(Some(other)),
                    })
                    .unwrap_or_default();
                let input = self.settings.input_for(key, &current, false, window, cx);
                div()
                    .w(px(260.))
                    .child(
                        Input::new(&input)
                            .id(SharedString::from(format!("setting-{id_slug}")))
                            .small(),
                    )
                    .into_any_element()
            }
            Control::Secret(key) => {
                let r = resolved(key);
                chip = r.as_ref().and_then(|r| self.row_chip(r.source, cx));
                let set = r.as_ref().is_some_and(|r| {
                    r.value
                        .as_ref()
                        .is_some_and(|v| !matches!(v, Value::Str(s) if s.is_empty()))
                });
                if self.settings.editing_secrets.contains(&key) {
                    // Starts empty: the stored secret is never put back into the UI.
                    let input = self.settings.input_for(key, "", true, window, cx);
                    div()
                        .w(px(260.))
                        .child(
                            Input::new(&input)
                                .id(SharedString::from(format!("setting-{id_slug}")))
                                .small(),
                        )
                        .into_any_element()
                } else {
                    let change = this.clone();
                    let clear = this.clone();
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .id(SharedString::from(format!("setting-{id_slug}-state")))
                                .test_support()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(i18n::t(
                                    cx,
                                    if set {
                                        "settings_sheet.secret_set"
                                    } else {
                                        "settings_sheet.secret_not_set"
                                    },
                                )),
                        )
                        .child(
                            Button::new(SharedString::from(format!("setting-{id_slug}-change")))
                                .small()
                                .outline()
                                .label(i18n::t(cx, "settings_sheet.secret_change"))
                                .on_click(move |_, window, cx| {
                                    let _ = change.update(cx, |shell, cx| {
                                        shell.settings.editing_secrets.insert(key);
                                        shell.settings.inputs.remove(&key);
                                        cx.notify();
                                        let _ = window;
                                    });
                                }),
                        )
                        .when(set, |el| {
                            el.child(
                                Button::new(SharedString::from(format!("setting-{id_slug}-clear")))
                                    .small()
                                    .ghost()
                                    .label(i18n::t(cx, "settings_sheet.secret_clear"))
                                    .on_click(move |_, window, cx| {
                                        let _ = clear.update(cx, |shell, cx| {
                                            shell.write_settings(vec![(key, None)], window, cx)
                                        });
                                    }),
                            )
                        })
                        .into_any_element()
                }
            }
            Control::Choice(key, choices) => {
                let r = resolved(key);
                chip = r.as_ref().and_then(|r| self.row_chip(r.source, cx));
                let current = r
                    .as_ref()
                    .and_then(|r| r.value.as_ref())
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let current = current.unwrap_or_else(|| choices[0].0.to_owned());
                let buttons: Vec<_> = choices
                    .iter()
                    .map(|(value, label)| {
                        let this = this.clone();
                        let value = *value;
                        Button::new(SharedString::from(format!("setting-{id_slug}-{value}")))
                            .small()
                            .selected(current == value)
                            .label(i18n::t(cx, label))
                            .on_click(move |_, window, cx| {
                                let _ = this.update(cx, |shell, cx| {
                                    shell.write_settings(
                                        vec![(key, Some(Value::Str(value.to_owned())))],
                                        window,
                                        cx,
                                    )
                                });
                            })
                    })
                    .collect();
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .children(buttons)
                    .into_any_element()
            }
            Control::IntChoice(key, options) => {
                let r = resolved(key);
                chip = r.as_ref().and_then(|r| self.row_chip(r.source, cx));
                let current = r
                    .as_ref()
                    .and_then(|r| r.value.as_ref())
                    .and_then(Value::as_int)
                    .unwrap_or(options[0].0);
                let selected = option_index(options.iter().map(|o| o.0), current);
                let buttons: Vec<_> = options
                    .iter()
                    .enumerate()
                    .map(|(i, &(value, label))| {
                        let this = this.clone();
                        let text = match label {
                            ChoiceLabel::Key(k) => i18n::t(cx, k),
                            ChoiceLabel::Count(k, n) => i18n::t_args(cx, k, &[("count", n)]),
                        };
                        Button::new(SharedString::from(format!("setting-{id_slug}-{value}")))
                            .small()
                            .selected(i == selected)
                            .label(text)
                            .on_click(move |_, window, cx| {
                                let _ = this.update(cx, |shell, cx| {
                                    shell.write_settings(
                                        vec![(key, Some(Value::Int(i128::from(value))))],
                                        window,
                                        cx,
                                    )
                                });
                            })
                    })
                    .collect();
                h_flex().gap_1().children(buttons).into_any_element()
            }
            Control::Steps(key, options, suffix) => {
                let r = resolved(key);
                chip = r.as_ref().and_then(|r| self.row_chip(r.source, cx));
                let current = r
                    .as_ref()
                    .and_then(|r| r.value.as_ref())
                    .and_then(Value::as_int);
                stepper(
                    &id_slug,
                    options,
                    suffix,
                    current,
                    theme,
                    move |value, window, cx| {
                        let _ = this.update(cx, |shell, cx| {
                            shell.write_settings(
                                vec![(key, Some(Value::Int(i128::from(value))))],
                                window,
                                cx,
                            )
                        });
                    },
                )
            }
            Control::UiScale => {
                let current = i64::from(crate::scale::percent());
                stepper(
                    &id_slug,
                    crate::scale::STEPS,
                    "%",
                    Some(current),
                    theme,
                    move |value, window, cx| {
                        let _ = this.update(cx, |shell, cx| {
                            shell.set_ui_scale(u16::try_from(value).unwrap_or(100), window, cx)
                        });
                    },
                )
            }
            Control::MediaStorage => match self.storage_row(cx) {
                Some((provider, disabled, s3_reason)) => {
                    use frankmd_media::vm::storage::Provider;
                    let local = this.clone();
                    let s3 = this.clone();
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("setting-provider-local")
                                .small()
                                .selected(provider == Provider::Local)
                                .disabled(disabled)
                                .label(i18n::t(cx, "settings_sheet.media.local"))
                                .on_click(move |_, _, cx| {
                                    let _ = local.update(cx, |shell, cx| {
                                        shell.request_storage(Provider::Local, cx)
                                    });
                                }),
                        )
                        .child(
                            Button::new("setting-provider-s3")
                                .small()
                                .selected(provider == Provider::S3)
                                .disabled(disabled || s3_reason.is_some())
                                .label(i18n::t(cx, "settings_sheet.media.s3"))
                                .on_click(move |_, _, cx| {
                                    let _ = s3.update(cx, |shell, cx| {
                                        shell.request_storage(Provider::S3, cx)
                                    });
                                }),
                        )
                        .when_some(s3_reason, |el, reason| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(reason),
                            )
                        })
                        .into_any_element()
                }
                None => div()
                    .child(value_text(
                        resolved(Key::MediaStorageProvider)
                            .as_ref()
                            .and_then(|r| r.value.as_ref()),
                    ))
                    .into_any_element(),
            },
            Control::HugoFolder => div()
                .font_family(theme.mono_font_family.clone())
                .child("blog/")
                .into_any_element(),
            Control::HugoTemplate => {
                let this = this.clone();
                Button::new("setting-hugo-template")
                    .small()
                    .outline()
                    .label(i18n::t(cx, "settings_sheet.hugo.edit_template"))
                    .on_click(move |_, window, cx| {
                        let _ = this.update(cx, |shell, cx| shell.edit_hugo_template(window, cx));
                    })
                    .into_any_element()
            }
        };
        let error = row_key.and_then(|k| self.settings.errors.get(&k).cloned());
        // "Overridden in this vault" dot + Reset to global (§5.16), in the global scope.
        let overridden = self.settings.scope == Scope::Global
            && row_key.is_some_and(|k| config.is_some_and(|c| c.in_vault_file(k)));
        let reset = overridden.then(|| {
            let this = this_reset.clone();
            let key = row_key.unwrap_or(Key::Theme);
            Button::new(SharedString::from(format!("setting-{id_slug}-reset")))
                .xsmall()
                .ghost()
                .label(i18n::t(cx, "settings_sheet.reset_to_global"))
                .on_click(move |_, window, cx| {
                    let _ = this.update(cx, |shell, cx| {
                        shell.update_settings(vec![(key, None)], window, cx);
                        shell.settings.reset_inputs();
                    });
                })
        });
        h_flex()
            .id(SharedString::from(format!("setting-row-{id_slug}")))
            .test_support()
            .min_h(px(44.))
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(div().child(label))
                            .when(overridden, |el| {
                                el.child(div().text_color(theme.primary).child("●"))
                            }),
                    )
                    .when_some(description, |el, d| {
                        el.child(div().text_xs().text_color(theme.muted_foreground).child(d))
                    })
                    .when_some(error, |el, e| {
                        el.child(div().text_xs().text_color(theme.danger).child(e))
                    }),
            )
            .children(reset)
            .when_some(chip, |el, c| {
                el.child(
                    div()
                        .text_xs()
                        .px_1()
                        .rounded(theme.radius)
                        .bg(theme.secondary)
                        .text_color(theme.muted_foreground)
                        .child(c),
                )
            })
            .child(control)
    }

    /// `Scope: [ All vaults | This vault (<name>) ]` (§5.16).
    fn render_scope_toggle(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let scope = self.settings.scope;
        let this = cx.entity().downgrade();
        let button = |id: &'static str, label: SharedString, target: Scope| {
            let this = this.clone();
            let button = Button::new(id).xsmall().label(label);
            let button = if scope == target {
                button.primary()
            } else {
                button.ghost()
            };
            button.on_click(move |_, _, cx| {
                let _ = this.update(cx, |shell, cx| {
                    shell.settings.scope = target;
                    shell.settings.reset_inputs();
                    cx.notify();
                });
            })
        };
        let global = button(
            "settings-scope-global",
            i18n::t(cx, "settings_sheet.scope_global"),
            Scope::Global,
        );
        let vault = button(
            "settings-scope-vault",
            i18n::t_args(
                cx,
                "settings_sheet.scope_vault",
                &[("vault", &self.vault_name)],
            ),
            Scope::Vault,
        );
        h_flex()
            .id("settings-scope")
            .test_support()
            .h(px(36.))
            .px_4()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(i18n::t(cx, "settings_sheet.scope")),
            )
            .child(global)
            .child(vault)
    }
}

/// Wrap a path in single quotes unless every character is shell-safe.
fn shell_quote(path: &str) -> String {
    if !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~'))
    {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::{EDITOR_WIDTHS, FONT_SIZES, option_index, step_targets};

    #[test]
    fn steps_move_to_the_neighbouring_option() {
        assert_eq!(step_targets(FONT_SIZES, Some(14)), (Some(13), Some(15)));
        assert_eq!(step_targets(FONT_SIZES, Some(12)), (None, Some(13)));
        assert_eq!(step_targets(FONT_SIZES, Some(24)), (Some(22), None));
    }

    #[test]
    fn a_value_between_options_steps_to_its_neighbours() {
        // 17 (hand-edited .fed) is between 16 and 18.
        assert_eq!(step_targets(FONT_SIZES, Some(17)), (Some(16), Some(18)));
        // Off the ends: back into range.
        assert_eq!(step_targets(FONT_SIZES, Some(40)), (Some(24), None));
        assert_eq!(step_targets(FONT_SIZES, Some(8)), (None, Some(12)));
        assert_eq!(step_targets(EDITOR_WIDTHS, Some(75)), (Some(72), Some(80)));
    }

    #[test]
    fn unset_steps_down_from_the_end_only() {
        assert_eq!(step_targets(FONT_SIZES, None), (None, Some(24)));
    }

    #[test]
    fn choice_picks_the_largest_option_not_above_the_value() {
        // Line numbers: 2 (relative, not implemented) shows as On (1).
        assert_eq!(option_index([0, 1], 2), 1);
        assert_eq!(option_index([2, 4, 8], 4), 1);
        assert_eq!(option_index([2, 4, 8], 0), 0);
    }
}
