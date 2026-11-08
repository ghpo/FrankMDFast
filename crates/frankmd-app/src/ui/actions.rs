//! GPUI actions, one per command of the registry (`frankmd_workspace::commands`,
//! HP-018), and the default keymap generated from the registry's bindings.
//!
//! A shortcut, a palette row and (later) a vim ex command all end in the same
//! action, so they can never drift. The editor and preview panes handle the
//! editor-scoped actions (`Bold`, `Find`, ...) with `on_action`; whatever they
//! don't handle bubbles to the shell.

use std::rc::Rc;

use frankmd_workspace::commands::CommandRegistry;
use gpui_kit::{Action, App, InteractiveElement, KeyBinding, Window};

/// Callback that runs a command by registry id.
pub type CommandHandler = Rc<dyn Fn(&'static str, &mut Window, &mut App)>;

macro_rules! command_actions {
    ($($action:ident => $id:literal),* $(,)?) => {
        gpui_kit::actions!(frankmd, [$(#[doc = concat!("Registry command `", $id, "`.")] $action),*]);

        /// Every command id that has an action, in registry order.
        pub const COMMAND_IDS: &[&str] = &[$($id),*];

        /// The action of command `id`.
        pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
            match id {
                $($id => Some(Box::new($action)),)*
                _ => None,
            }
        }

        /// A key binding for command `id` (`keystroke` in GPUI notation).
        pub fn binding_for(id: &str, keystroke: &'static str, context: &str) -> Option<KeyBinding> {
            match id {
                $($id => Some(KeyBinding::new(keystroke, $action, Some(context))),)*
                _ => None,
            }
        }

        /// Attach one `on_action` listener per command to `el`; each calls
        /// `handler(command_id, ..)`.
        pub(crate) fn on_commands<E: InteractiveElement>(
            mut el: E,
            handler: CommandHandler,
        ) -> E {
            $(
                let h = Rc::clone(&handler);
                el = el.on_action(move |_: &$action, window: &mut Window, cx: &mut App| h($id, window, cx));
            )*
            el
        }
    };
}

command_actions!(
    ToggleCommandPalette => "app.command_palette",
    QuickOpen => "file.quick_open",
    RecentSwitcherNext => "file.recent_next",
    RecentSwitcherPrev => "file.recent_prev",
    SearchInFiles => "search.in_files",
    NewNote => "file.new_note",
    NewFolder => "file.new_folder",
    EncryptNote => "file.encrypt",
    DecryptNote => "file.decrypt",
    SaveNow => "file.save",
    CloseNote => "file.close",
    Quit => "app.quit",
    NavigateBack => "nav.back",
    NavigateForward => "nav.forward",
    ToggleSidebar => "view.toggle_sidebar",
    TogglePreview => "view.toggle_preview",
    ToggleTypewriter => "view.toggle_typewriter",
    ToggleScrollSync => "view.toggle_scroll_sync",
    ToggleZen => "view.toggle_zen",
    ToggleMediaLibrary => "view.media_library",
    OpenSettings => "app.settings",
    ToggleHelp => "help.toggle",
    ShowKeyboardShortcuts => "help.keyboard_shortcuts",
    OpenDiagnostics => "app.diagnostics",
    OpenRemoteSettings => "app.settings_remote",
    OpenEncryptionSettings => "app.settings_encryption",
    EncryptionToggle => "app.encryption_toggle",
    FocusSidebar => "focus.sidebar",
    FocusEditor => "focus.editor",
    FocusPreview => "focus.preview",
    FocusNextRegion => "focus.next_region",
    FocusPrevRegion => "focus.prev_region",
    Escape => "app.escape",
    SelectTheme => "theme.select",
    SelectLanguage => "language.select",
    SwitchVault => "vault.switch",
    ToggleVim => "editor.toggle_vim",
    Bold => "format.bold",
    Italic => "format.italic",
    Strikethrough => "format.strikethrough",
    InlineCode => "format.inline_code",
    InsertLink => "insert.link",
    TextFormatMenu => "format.menu",
    InsertMenu => "insert.menu",
    InsertImage => "insert.image",
    InsertVideo => "insert.video",
    InsertTable => "insert.table",
    InsertCodeBlock => "insert.code_block",
    EmojiPicker => "insert.emoji",
    ContextAction => "edit.context_action",
    EditTable => "table.edit",
    ToggleTask => "edit.toggle_task",
    Find => "find.find",
    Replace => "find.replace",
    FindNext => "find.next",
    FindPrev => "find.prev",
    GoToLine => "edit.go_to_line",
    ToggleLineNumbers => "view.line_numbers",
    EditorWidthIncrease => "view.width_increase",
    EditorWidthDecrease => "view.width_decrease",
    ZoomIn => "view.zoom_in",
    ZoomOut => "view.zoom_out",
    ZoomReset => "view.zoom_reset",
    GrammarCheck => "ai.grammar_check",
    RenameNote => "file.rename",
    MoveNote => "file.move",
    CopyContents => "file.copy_contents",
    CopyPath => "file.copy_path",
    RevealInExplorer => "file.reveal",
    DeleteNote => "file.delete",
    SelectPrev => "list.select_prev",
    SelectNext => "list.select_next",
    Confirm => "list.confirm",
    ListEscape => "list.escape",
);

/// Key context of the workspace root (global shortcuts).
pub const CX_WORKSPACE: &str = "Workspace";
/// Key context of the editor pane.
pub const CX_EDITOR: &str = "Editor";
/// Key context of overlay lists (palette, quick open, search).
pub const CX_OVERLAY: &str = "Overlay";

/// The default keymap, generated from [`CommandRegistry::bindings`].
pub fn default_keymap() -> Vec<KeyBinding> {
    CommandRegistry::builtin()
        .bindings()
        .into_iter()
        .filter_map(|b| binding_for(b.command, b.keystroke, b.context.name()))
        .collect()
}

/// Bind the default keymap globally.
pub fn install_keymap(cx: &mut App) {
    let mut bindings = default_keymap();
    if cfg!(target_os = "macos") {
        // The registry's `ctrl-q` maps the physical Ctrl key on every OS; on
        // macOS Quit is ⌘Q. The app menu's key equivalent is derived from
        // this binding.
        bindings.push(KeyBinding::new("cmd-q", Quit, Some(CX_WORKSPACE)));
    }
    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registry_command_has_an_action() {
        for command in CommandRegistry::builtin().commands() {
            assert!(
                action_for(command.id).is_some(),
                "no action for {}",
                command.id
            );
        }
        assert_eq!(
            COMMAND_IDS.len(),
            CommandRegistry::builtin().commands().len()
        );
    }

    #[test]
    fn every_binding_is_generated() {
        assert_eq!(
            default_keymap().len(),
            CommandRegistry::builtin().bindings().len()
        );
    }
}
