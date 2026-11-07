//! The single command registry (HP-018, ui-spec §5.1 / §6): every command
//! the palette lists, its default key bindings and when it applies. The app
//! generates its GPUI keymap from [`CommandRegistry::bindings`], so a
//! shortcut, a palette row and a vim ex command always reach the same action.
//!
//! Titles are the English `Category: Name` strings; the `unavailable` reasons
//! are i18n keys (D-3).

use frankmd_core::text::utils::fuzzy_score;

/// Key context a binding lives in (GPUI key-context names).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyContext {
    /// Workspace root (global shortcuts).
    Workspace,
    /// The editor pane.
    Editor,
    /// Any overlay list (palette, quick open, search).
    Overlay,
}

impl KeyContext {
    /// GPUI key-context name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Editor => "Editor",
            Self::Overlay => "Overlay",
        }
    }
}

/// What a command needs to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// Always available.
    None,
    /// A note is open.
    Note,
    /// A markdown note is open.
    MarkdownNote,
    /// The caret is inside a table of a markdown note.
    CaretInTable,
}

impl Requirement {
    /// `None` when met, else the i18n key of the reason.
    fn unmet_reason(self, cx: &CommandContext) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Note if !cx.note_open && !cx.markdown_open => {
                Some("palette.unavailable.open_note")
            }
            Self::MarkdownNote if !cx.markdown_open => {
                Some("palette.unavailable.open_markdown_note")
            }
            Self::CaretInTable if !cx.markdown_open => {
                Some("palette.unavailable.open_markdown_note")
            }
            Self::CaretInTable if !cx.caret_in_table => Some("palette.unavailable.caret_in_table"),
            _ => None,
        }
    }
}

/// On/off state a toggle command shows (✓).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    /// Vim mode.
    Vim,
    /// Zen mode.
    Zen,
    /// Preview visible.
    Preview,
    /// Sidebar visible.
    Sidebar,
    /// Typewriter mode.
    Typewriter,
    /// Editor ↔ preview scroll sync.
    ScrollSync,
}

/// What the registry needs to know about the current UI to filter commands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandContext {
    /// A markdown note is open.
    pub markdown_open: bool,
    /// Caret is inside a table.
    pub caret_in_table: bool,
    /// Vim mode on (toggle state).
    pub vim: bool,
    /// Any note is open (markdown or not).
    pub note_open: bool,
    /// Zen mode on.
    pub zen: bool,
    /// Preview visible.
    pub preview_visible: bool,
    /// Sidebar visible.
    pub sidebar_visible: bool,
    /// Typewriter on.
    pub typewriter: bool,
    /// Scroll sync on.
    pub scroll_sync: bool,
}

impl CommandContext {
    fn toggle_state(&self, toggle: Toggle) -> bool {
        match toggle {
            Toggle::Vim => self.vim,
            Toggle::Zen => self.zen,
            Toggle::Preview => self.preview_visible,
            Toggle::Sidebar => self.sidebar_visible,
            Toggle::Typewriter => self.typewriter,
            Toggle::ScrollSync => self.scroll_sync,
        }
    }
}

/// One registered command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    /// Stable id, e.g. `view.toggle_zen`.
    pub id: &'static str,
    /// Palette category.
    pub category: &'static str,
    /// Palette name.
    pub name: &'static str,
    /// Extra search words.
    pub aliases: &'static [&'static str],
    /// Default bindings (GPUI keystroke notation); the first is displayed.
    pub keys: &'static [&'static str],
    /// Displayed shortcut when it differs from the first binding (or the
    /// command shares a contextual key, like `Ctrl+.`).
    pub shortcut_hint: Option<&'static str>,
    /// Key context of the bindings.
    pub context: KeyContext,
    /// When it applies.
    pub requires: Requirement,
    /// Toggle state shown as a check.
    pub toggle: Option<Toggle>,
    /// Listed in the palette (list-navigation actions are not).
    pub in_palette: bool,
}

const fn cmd(
    id: &'static str,
    category: &'static str,
    name: &'static str,
    keys: &'static [&'static str],
) -> CommandSpec {
    CommandSpec {
        id,
        category,
        name,
        aliases: &[],
        keys,
        shortcut_hint: None,
        context: KeyContext::Workspace,
        requires: Requirement::None,
        toggle: None,
        in_palette: true,
    }
}

impl CommandSpec {
    const fn editor(mut self) -> Self {
        self.context = KeyContext::Editor;
        self
    }
    const fn overlay(mut self) -> Self {
        self.context = KeyContext::Overlay;
        self.in_palette = false;
        self
    }
    const fn hidden(mut self) -> Self {
        self.in_palette = false;
        self
    }
    const fn needs(mut self, requirement: Requirement) -> Self {
        self.requires = requirement;
        self
    }
    const fn toggles(mut self, toggle: Toggle) -> Self {
        self.toggle = Some(toggle);
        self
    }
    const fn aka(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }
    const fn hint(mut self, shortcut: &'static str) -> Self {
        self.shortcut_hint = Some(shortcut);
        self
    }

    /// `Category: Name`.
    pub fn title(&self) -> String {
        format!("{}: {}", self.category, self.name)
    }

    /// The shortcut as printed (`Ctrl+Shift+P`, `F11`).
    pub fn shortcut(&self) -> Option<String> {
        self.shortcut_hint
            .map(str::to_owned)
            .or_else(|| self.keys.first().map(|k| display_keystroke(k)))
    }
}

use Requirement::{CaretInTable, MarkdownNote, Note};

/// Every built-in command, in palette order (ui-spec §6.1, §6.2, §6.4).
pub const BUILTIN: &[CommandSpec] = &[
    // Global (§6.1)
    cmd(
        "app.command_palette",
        "View",
        "Command palette",
        &["ctrl-shift-p"],
    ),
    cmd("file.quick_open", "File", "Quick open", &["ctrl-p"]).aka(&["find file", "go to file"]),
    cmd(
        "file.recent_next",
        "File",
        "Switch to recent note",
        &["ctrl-tab"],
    ),
    cmd(
        "file.recent_prev",
        "File",
        "Switch to previous recent note",
        &["ctrl-shift-tab"],
    )
    .hidden(),
    cmd(
        "search.in_files",
        "Search",
        "Search in files",
        &["ctrl-shift-f"],
    )
    .aka(&["grep", "find in files"]),
    cmd("file.new_note", "File", "New note", &["ctrl-n"]),
    cmd("file.new_folder", "File", "New folder", &["ctrl-shift-n"]),
    cmd("file.save", "File", "Save", &["ctrl-s"]).needs(Note),
    cmd("file.close", "File", "Close note", &["ctrl-w"]).needs(Note),
    cmd("app.quit", "App", "Quit", &["ctrl-q"]).aka(&["exit"]),
    cmd("nav.back", "Go", "Back", &["ctrl-alt-left"]),
    cmd("nav.forward", "Go", "Forward", &["ctrl-alt-right"]),
    cmd("view.toggle_sidebar", "View", "Sidebar", &["ctrl-e"])
        .toggles(Toggle::Sidebar)
        .aka(&["explorer"]),
    cmd("view.toggle_preview", "View", "Preview", &["ctrl-shift-v"]).toggles(Toggle::Preview),
    cmd(
        "view.toggle_typewriter",
        "View",
        "Typewriter mode",
        &["ctrl-\\"],
    )
    .toggles(Toggle::Typewriter)
    .needs(MarkdownNote),
    cmd(
        "view.toggle_scroll_sync",
        "View",
        "Scroll sync",
        &["ctrl-shift-\\", "ctrl-|"],
    )
    .toggles(Toggle::ScrollSync),
    cmd("view.toggle_zen", "View", "Zen mode", &["f11"])
        .toggles(Toggle::Zen)
        .aka(&["focus", "distraction free"]),
    cmd(
        "view.media_library",
        "View",
        "Media library",
        &["ctrl-shift-m"],
    )
    .aka(&["images"]),
    cmd("app.settings", "App", "Settings", &["ctrl-,"]).aka(&["preferences", "font", "config"]),
    cmd("help.toggle", "Help", "Help", &["f1"]),
    cmd(
        "help.keyboard_shortcuts",
        "Help",
        "Keyboard shortcuts",
        &["ctrl-/"],
    )
    .aka(&["keymap"]),
    cmd("app.diagnostics", "App", "Diagnostics", &["ctrl-shift-o"]).aka(&["logs"]),
    cmd("app.settings_remote", "App", "Remote access settings", &[]).aka(&[
        "remote",
        "serve",
        "tailscale",
        "web",
    ]),
    cmd("focus.sidebar", "Focus", "Sidebar", &["ctrl-1"]),
    cmd("focus.editor", "Focus", "Editor", &["ctrl-2"]),
    cmd("focus.preview", "Focus", "Preview", &["ctrl-3"]),
    cmd("focus.next_region", "Focus", "Next region", &["f6"]),
    cmd(
        "focus.prev_region",
        "Focus",
        "Previous region",
        &["shift-f6"],
    ),
    cmd("app.escape", "App", "Escape", &["escape"]).hidden(),
    // Pickers (palette sub-modes, no default shortcut)
    cmd("theme.select", "Theme", "Select…", &[]).aka(&["color", "dark", "light"]),
    cmd("language.select", "Language", "Select…", &[]).aka(&["locale"]),
    cmd("vault.switch", "Vault", "Switch…", &[]).aka(&["folder", "open folder"]),
    cmd("editor.toggle_vim", "Editor", "Vim mode", &[]).toggles(Toggle::Vim),
    // Editor (§6.2)
    cmd("format.bold", "Format", "Bold", &["ctrl-b"])
        .editor()
        .needs(MarkdownNote),
    cmd("format.italic", "Format", "Italic", &["ctrl-i"])
        .editor()
        .needs(MarkdownNote),
    cmd(
        "format.strikethrough",
        "Format",
        "Strikethrough",
        &["ctrl-shift-s"],
    )
    .editor()
    .needs(MarkdownNote),
    cmd("format.inline_code", "Format", "Inline code", &["ctrl-`"])
        .editor()
        .needs(MarkdownNote),
    cmd("insert.link", "Insert", "Link", &["ctrl-k"])
        .editor()
        .needs(MarkdownNote),
    cmd("format.menu", "Format", "Text format menu", &["ctrl-m"])
        .editor()
        .needs(MarkdownNote),
    cmd("insert.menu", "Insert", "Insert menu", &["alt-i"])
        .editor()
        .needs(MarkdownNote),
    cmd("insert.image", "Insert", "Image…", &["ctrl-shift-i"])
        .editor()
        .needs(MarkdownNote),
    cmd("insert.video", "Insert", "Video…", &[]).needs(MarkdownNote),
    cmd("insert.table", "Insert", "Table…", &[]).needs(MarkdownNote),
    cmd("insert.code_block", "Insert", "Code block…", &[]).needs(MarkdownNote),
    cmd("insert.emoji", "Insert", "Emoji", &["ctrl-shift-e"])
        .editor()
        .needs(MarkdownNote),
    cmd("edit.context_action", "Edit", "Context action", &["ctrl-."])
        .editor()
        .needs(Note),
    cmd("table.edit", "Table", "Edit", &[])
        .hint("Ctrl+.")
        .needs(CaretInTable),
    cmd("edit.toggle_task", "Edit", "Toggle task", &["ctrl-enter"])
        .editor()
        .needs(MarkdownNote),
    cmd("find.find", "Edit", "Find", &["ctrl-f"])
        .editor()
        .needs(Note),
    cmd("find.replace", "Edit", "Replace", &["ctrl-h"])
        .editor()
        .needs(Note),
    cmd("find.next", "Edit", "Find next", &["f3"])
        .editor()
        .needs(Note),
    cmd("find.prev", "Edit", "Find previous", &["shift-f3"])
        .editor()
        .needs(Note),
    cmd("edit.go_to_line", "Edit", "Go to line", &["ctrl-g"])
        .editor()
        .needs(Note),
    cmd("view.line_numbers", "View", "Line numbers", &["ctrl-l"])
        .editor()
        .needs(Note),
    cmd(
        "view.width_increase",
        "View",
        "Increase editor width",
        &["ctrl-shift-=", "ctrl-+"],
    )
    .editor()
    .needs(Note),
    cmd(
        "view.width_decrease",
        "View",
        "Decrease editor width",
        &["ctrl-shift--", "ctrl-_"],
    )
    .editor()
    .needs(Note),
    cmd("view.zoom_in", "View", "Zoom in", &["ctrl-="])
        .editor()
        .aka(&["font size"]),
    cmd("view.zoom_out", "View", "Zoom out", &["ctrl--"])
        .editor()
        .aka(&["font size"]),
    cmd("view.zoom_reset", "View", "Reset zoom", &["ctrl-0"]).editor(),
    cmd(
        "ai.grammar_check",
        "Check",
        "Grammar check with AI",
        &["ctrl-shift-g"],
    )
    .editor()
    .needs(MarkdownNote),
    cmd("file.rename", "Document", "Rename", &["f2"])
        .editor()
        .needs(Note),
    cmd("file.move", "Document", "Move to…", &[])
        .needs(Note)
        .aka(&["folder"]),
    cmd("file.copy_contents", "Document", "Copy contents", &[]).needs(Note),
    cmd("file.copy_path", "Document", "Copy path", &[]).needs(Note),
    cmd("file.reveal", "Document", "Reveal in explorer", &[]).needs(Note),
    cmd("file.delete", "Document", "Delete…", &[]).needs(Note),
    // Overlay lists (§6.4)
    cmd("list.select_prev", "List", "Previous", &["up"]).overlay(),
    cmd("list.select_next", "List", "Next", &["down"]).overlay(),
    cmd("list.confirm", "List", "Confirm", &["enter"]).overlay(),
    cmd("list.escape", "List", "Close", &["escape"]).overlay(),
];

/// One palette row.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteRow {
    /// Stable id, e.g. `view.toggle_zen`.
    pub id: &'static str,
    /// `Category: Name` as displayed.
    pub title: String,
    /// Printed shortcut, e.g. `F11`.
    pub shortcut: Option<String>,
    /// Toggle commands: current state (✓).
    pub checked: Option<bool>,
    /// `Some(reason i18n key)` for the trailing "Unavailable" section.
    pub unavailable: Option<&'static str>,
}

/// One default key binding generated from the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// GPUI keystroke notation (`ctrl-shift-p`).
    pub keystroke: &'static str,
    /// Command id.
    pub command: &'static str,
    /// Key context.
    pub context: KeyContext,
}

/// The single command registry.
#[derive(Debug, Clone)]
pub struct CommandRegistry {
    commands: &'static [CommandSpec],
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl CommandRegistry {
    /// Every built-in command.
    pub fn builtin() -> Self {
        Self { commands: BUILTIN }
    }

    /// All commands in registry order.
    pub fn commands(&self) -> &[CommandSpec] {
        self.commands
    }

    /// Look up a command.
    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// The default keymap: one row per (keystroke, command, context).
    pub fn bindings(&self) -> Vec<Binding> {
        self.commands
            .iter()
            .flat_map(|c| {
                c.keys.iter().map(move |k| Binding {
                    keystroke: k,
                    command: c.id,
                    context: c.context,
                })
            })
            .collect()
    }

    /// Whether `id` applies in `cx`.
    pub fn is_available(&self, id: &str, cx: &CommandContext) -> bool {
        self.get(id)
            .is_some_and(|c| c.requires.unmet_reason(cx).is_none())
    }

    /// Fuzzy palette query. Every query word must fuzzy-match one word of
    /// the title or aliases; best score first. Commands that don't apply are
    /// hidden, unless *only* such commands match: then they are listed with
    /// their reason. An empty query lists recently used commands first.
    pub fn query(&self, query: &str, cx: &CommandContext, recent: &[&str]) -> Vec<PaletteRow> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut scored: Vec<(u32, usize, &CommandSpec)> = self
            .commands
            .iter()
            .enumerate()
            .filter(|(_, c)| c.in_palette)
            .filter_map(|(i, c)| {
                if words.is_empty() {
                    let rank = recent
                        .iter()
                        .position(|r| *r == c.id)
                        .map_or(0, |pos| u32::MAX - pos as u32);
                    return Some((rank, i, c));
                }
                match_score(c, &words).map(|score| (score, i, c))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        let row = |c: &CommandSpec, unavailable: Option<&'static str>| PaletteRow {
            id: c.id,
            title: c.title(),
            shortcut: c.shortcut(),
            checked: c.toggle.map(|t| cx.toggle_state(t)),
            unavailable,
        };
        let available: Vec<PaletteRow> = scored
            .iter()
            .filter(|(_, _, c)| c.requires.unmet_reason(cx).is_none())
            .map(|(_, _, c)| row(c, None))
            .collect();
        if !available.is_empty() || words.is_empty() {
            return available;
        }
        scored
            .iter()
            .filter_map(|(_, _, c)| {
                c.requires
                    .unmet_reason(cx)
                    .map(|reason| row(c, Some(reason)))
            })
            .collect()
    }
}

/// Sum of the best per-word fuzzy scores, `None` if a word matches nothing.
fn match_score(command: &CommandSpec, words: &[String]) -> Option<u32> {
    let haystack = std::iter::once(command.category)
        .chain(std::iter::once(command.name))
        .chain(command.aliases.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let tokens: Vec<&str> = haystack
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    words.iter().try_fold(0u32, |total, word| {
        let best = tokens
            .iter()
            .map(|t| fuzzy_score(t, word))
            .max()
            .unwrap_or(0);
        (best > 0).then(|| total.saturating_add(best))
    })
}

/// `ctrl-shift-p` → `Ctrl+Shift+P`, `f11` → `F11`, `ctrl-\` → `Ctrl+\`.
pub fn display_keystroke(keystroke: &str) -> String {
    let (modifiers, key) = match keystroke.rsplit_once('-') {
        // `ctrl--` / `ctrl-shift--`: the key itself is `-`.
        Some((mods, "")) => (mods.trim_end_matches('-'), "-"),
        Some((mods, key)) => (mods, key),
        None => ("", keystroke),
    };
    let mut parts: Vec<String> = modifiers
        .split('-')
        .filter(|m| !m.is_empty())
        .map(|m| match m {
            "ctrl" => "Ctrl".to_owned(),
            "shift" => "Shift".to_owned(),
            "alt" => "Alt".to_owned(),
            other => other.to_owned(),
        })
        .collect();
    let key = match key {
        "left" => "←".to_owned(),
        "right" => "→".to_owned(),
        "up" => "↑".to_owned(),
        "down" => "↓".to_owned(),
        "enter" => "Enter".to_owned(),
        "escape" => "Esc".to_owned(),
        "tab" => "Tab".to_owned(),
        k => k.to_uppercase(),
    };
    parts.push(key);
    parts.join("+")
}
