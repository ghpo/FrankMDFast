//! File-operation dialogs (ui-spec §5.13): New note, New folder, Move to…,
//! Delete confirmation, and inline rename (explorer row and pane title).
//! Forms and validation are `frankmd_workspace::dialogs`; this module owns
//! the inputs, renders, and dispatches the commands the forms produce.

use crate::scale::px;
use frankmd_core::hugo::HugoPathStyle;
use frankmd_core::new_note::NoteTemplate;
use frankmd_workspace::command::UserError;
use frankmd_workspace::dialogs::{
    CreationOrigin, DeleteConfirm, InlineRename, MoveForm, NewFolderForm, NewNoteForm, RenameTarget,
};
use frankmd_workspace::explorer::{Item, ItemKind, MovePlan};

use super::prelude::*;
use gpui_kit::component::input::InputEvent;
use gpui_kit::*;

/// Overlay ids (`dialog-<kind>` element ids).
pub(crate) const NEW_NOTE: &str = "new-note";
/// New folder dialog.
pub(crate) const NEW_FOLDER: &str = "new-folder";
/// Move to… dialog.
pub(crate) const MOVE: &str = "move";
/// Delete confirmation.
pub(crate) const DELETE: &str = "delete";

/// State of the file dialogs and their inputs.
pub(crate) struct FileDialogs {
    pub(crate) new_note: Option<NewNoteForm>,
    pub(crate) new_folder: Option<NewFolderForm>,
    pub(crate) move_form: Option<MoveForm>,
    /// Folder the moved item started in (the picker shows all folders until edited).
    pub(crate) move_origin: String,
    pub(crate) delete: Option<DeleteConfirm>,
    /// Delete dialog: the destructive button has keyboard focus (starts on Cancel).
    pub(crate) delete_on_confirm: bool,
    pub(crate) delete_error: Option<UserError>,
    pub(crate) rename: Option<InlineRename>,
    /// New note: the destination picker list is open.
    pub(crate) picking_folder: bool,
    pub(crate) note_name: Entity<InputState>,
    pub(crate) note_slug: Entity<InputState>,
    pub(crate) folder_name: Entity<InputState>,
    pub(crate) move_name: Entity<InputState>,
    pub(crate) move_folder: Entity<InputState>,
    pub(crate) rename_input: Entity<InputState>,
}

impl FileDialogs {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Shell>) -> (Self, Vec<Subscription>) {
        let input = |cx: &mut Context<Shell>, window: &mut Window, placeholder: &str| {
            let placeholder = i18n::t(cx, placeholder);
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let note_name = input(cx, window, "dialogs.new_note.name_placeholder");
        let note_slug = input(cx, window, "dialogs.new_note.slug_placeholder");
        let folder_name = input(cx, window, "dialogs.new_item.folder_placeholder");
        let move_name = input(cx, window, "dialogs.rename.placeholder");
        let move_folder = input(cx, window, "dialogs.move.folder_placeholder");
        let rename_input = input(cx, window, "dialogs.rename.placeholder");
        let subs = vec![
            cx.subscribe_in(
                &note_name,
                window,
                |shell, input, ev, window, cx| match ev {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if let Some(form) = shell.files.new_note.as_mut() {
                            form.set_name(&value);
                            form.set_error(None);
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => shell.submit_new_note(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &note_slug,
                window,
                |shell, input, ev, window, cx| match ev {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if let Some(form) = shell.files.new_note.as_mut()
                            && value != form.slug()
                        {
                            form.set_slug(&value);
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => shell.submit_new_note(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &folder_name,
                window,
                |shell, input, ev, window, cx| match ev {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if let Some(form) = shell.files.new_folder.as_mut() {
                            form.set_name(&value);
                            form.set_error(None);
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => shell.submit_new_folder(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &move_name,
                window,
                |shell, input, ev, window, cx| match ev {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if let Some(form) = shell.files.move_form.as_mut() {
                            form.set_name(&value);
                            form.set_error(None);
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => shell.submit_move(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &move_folder,
                window,
                |shell, input, ev, window, cx| match ev {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if let Some(form) = shell.files.move_form.as_mut() {
                            form.set_folder(value.trim().trim_matches('/'));
                            form.set_error(None);
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => shell.submit_move(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &rename_input,
                window,
                |shell, input, ev, window, cx| match ev {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if let Some(rename) = shell.files.rename.as_mut() {
                            rename.set_value(&value);
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => shell.commit_inline_rename(window, cx),
                    // Blur commits (F.14), unless the rename already ended.
                    InputEvent::Blur => {
                        if shell.files.rename.is_some() {
                            shell.commit_inline_rename(window, cx);
                        }
                    }
                    InputEvent::Focus => {}
                },
            ),
        ];
        (
            Self {
                new_note: None,
                new_folder: None,
                move_form: None,
                move_origin: String::new(),
                delete: None,
                delete_on_confirm: false,
                delete_error: None,
                rename: None,
                picking_folder: false,
                note_name,
                note_slug,
                folder_name,
                move_name,
                move_folder,
                rename_input,
            },
            subs,
        )
    }
}

impl Shell {
    // ----- plumbing ---------------------------------------------------------------------

    /// Dispatch `cmd`; the first user-facing error it raised, if any. The
    /// error also reaches the status line as usual.
    pub(crate) fn dispatch_checked(
        &mut self,
        cmd: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), UserError> {
        self.last_error = None;
        self.dispatch(cmd, window, cx);
        match self.last_error.take() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn hugo_style(&self) -> HugoPathStyle {
        let value = self
            .core
            .as_ref()
            .and_then(|c| c.ws.snapshot().config.value(Key::HugoPathStyle));
        HugoPathStyle::from_config(value.as_ref().and_then(Value::as_str))
    }

    /// Every folder path of the tree ("" first = root), for the pickers.
    pub(crate) fn folder_paths(&self) -> Vec<String> {
        let mut out = vec![String::new()];
        out.extend(
            self.tree_paths()
                .into_iter()
                .filter(|p| self.kind_of(p) == ItemKind::Folder),
        );
        out
    }

    fn set_input(
        input: &Entity<InputState>,
        value: &str,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.to_owned();
        input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            if focus {
                input.focus(window, cx);
            }
        });
    }

    fn err_text(cx: &App, err: &UserError) -> SharedString {
        let args: Vec<(&str, &str)> = err.args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        i18n::t_args(cx, err.key, &args)
    }

    // ----- New note (F.8, §5.13) ------------------------------------------------------

    /// Open the New note dialog; `name` pre-fills it (quick open's
    /// "Ctrl+N creates <query>").
    pub(crate) fn open_new_note(
        &mut self,
        origin: CreationOrigin,
        name: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.core.is_none() {
            return;
        }
        if let CreationOrigin::FolderContext(folder) = &origin {
            // #268: the destination is revealed before creating into it.
            self.explorer.expand(folder);
            self.reveal_in_tree(folder);
        }
        let name = name.unwrap_or("");
        self.files.new_note = Some(NewNoteForm::new(origin, self.hugo_style()).with_name(name));
        self.files.picking_folder = false;
        self.open_overlay(OverlayKind::Dialog(NEW_NOTE), window, cx);
        let (name_input, slug_input) = (self.files.note_name.clone(), self.files.note_slug.clone());
        let slug = self
            .files
            .new_note
            .as_ref()
            .map(|f| f.slug().to_owned())
            .unwrap_or_default();
        Self::set_input(&slug_input, &slug, false, window, cx);
        Self::set_input(&name_input, name, true, window, cx);
    }

    /// The global New note: destination from the explorer selection (F.7),
    /// or the focused folder row when the sidebar has focus (§5.14).
    pub(crate) fn new_note_global(
        &mut self,
        name: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folder = self
            .focused_tree_folder(window, cx)
            .unwrap_or_else(|| self.explorer.creation_folder());
        self.open_new_note(
            CreationOrigin::Global {
                selected_folder: folder,
            },
            name,
            window,
            cx,
        );
    }

    /// The cursor row when the sidebar is focused and it is a folder.
    fn focused_tree_folder(&self, window: &Window, cx: &App) -> Option<String> {
        if !self.sidebar_focus.contains_focused(window, cx) || self.explorer.selection().len() > 1 {
            return None;
        }
        self.explorer
            .cursor()
            .filter(|c| self.kind_of(c) == ItemKind::Folder)
            .map(str::to_owned)
    }

    fn set_note_template(
        &mut self,
        template: NoteTemplate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.files.new_note.as_mut() {
            form.set_template(template);
        }
        let input = self.files.note_name.clone();
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// "Change…" picked a destination: an explicit folder, like the folder
    /// context menu (so Hugo posts go there too).
    fn pick_note_destination(
        &mut self,
        folder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(old) = self.files.new_note.take() else {
            return;
        };
        let origin = if folder.is_empty() {
            CreationOrigin::Global {
                selected_folder: String::new(),
            }
        } else {
            CreationOrigin::FolderContext(folder)
        };
        let mut form = NewNoteForm::new(origin, self.hugo_style()).with_name(old.name());
        form.set_template(old.template());
        let slug = old.slug().to_owned();
        if slug != form.slug() {
            form.set_slug(&slug);
        }
        self.files.new_note = Some(form);
        self.files.picking_folder = false;
        let input = self.files.note_name.clone();
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn submit_new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cmd) = self.files.new_note.as_ref().and_then(NewNoteForm::submit) else {
            return;
        };
        match self.dispatch_checked(cmd, window, cx) {
            Ok(()) => {
                self.files.new_note = None;
                if self.overlays.is_open(&OverlayKind::Dialog(NEW_NOTE)) {
                    self.overlays.close();
                }
                // F.8: the new (empty) note opens with the editor focused.
                self.focus_region(FocusRegion::Editor, window, cx);
            }
            Err(err) => {
                if let Some(form) = self.files.new_note.as_mut() {
                    form.set_error(Some(err));
                }
            }
        }
        cx.notify();
    }

    // ----- New folder (F.9) -------------------------------------------------------------

    pub(crate) fn open_new_folder(
        &mut self,
        parent: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.core.is_none() {
            return;
        }
        if !parent.is_empty() {
            self.explorer.expand(parent);
            self.reveal_in_tree(parent);
        }
        self.files.new_folder = Some(NewFolderForm::new(parent));
        self.open_overlay(OverlayKind::Dialog(NEW_FOLDER), window, cx);
        let input = self.files.folder_name.clone();
        Self::set_input(&input, "", true, window, cx);
    }

    pub(crate) fn new_folder_global(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let folder = self
            .focused_tree_folder(window, cx)
            .unwrap_or_else(|| self.explorer.creation_folder());
        self.open_new_folder(&folder, window, cx);
    }

    pub(crate) fn submit_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.files.new_folder.as_ref() else {
            return;
        };
        let Some(cmd) = form.submit() else {
            return;
        };
        let created = format!(
            "{}{}",
            if form.parent.is_empty() {
                String::new()
            } else {
                format!("{}/", form.parent)
            },
            form.name().trim()
        );
        match self.dispatch_checked(cmd, window, cx) {
            Ok(()) => {
                self.files.new_folder = None;
                // F.9: the new folder is expanded, not selected or opened.
                self.explorer.expand(&created);
                self.persist_expanded();
                self.close_overlay(window, cx);
            }
            Err(err) => {
                if let Some(form) = self.files.new_folder.as_mut() {
                    form.set_error(Some(err));
                }
            }
        }
        cx.notify();
    }

    // ----- Move to… (§5.13) -----------------------------------------------------------

    pub(crate) fn open_move(
        &mut self,
        path: &str,
        kind: ItemKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if frankmd_core::path::is_config_file(path) {
            return;
        }
        let form = MoveForm::new(path, kind);
        let (name, folder) = (form.name().to_owned(), form.folder().to_owned());
        self.files.move_origin = folder.clone();
        self.files.move_form = Some(form);
        self.open_overlay(OverlayKind::Dialog(MOVE), window, cx);
        let (name_input, folder_input) =
            (self.files.move_name.clone(), self.files.move_folder.clone());
        Self::set_input(&folder_input, &folder, false, window, cx);
        Self::set_input(&name_input, &name, true, window, cx);
        name_input.update(cx, |input, cx| input.select_all(window, cx));
    }

    pub(crate) fn submit_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.files.move_form.as_ref() else {
            return;
        };
        let result = match form.submit() {
            Ok(None) => Ok(()),
            Ok(Some(cmd)) => self.dispatch_checked(cmd, window, cx),
            Err(err) => Err(err),
        };
        match result {
            Ok(()) => {
                self.files.move_form = None;
                self.close_overlay(window, cx);
            }
            Err(err) => {
                if let Some(form) = self.files.move_form.as_mut() {
                    form.set_error(Some(err));
                }
            }
        }
        cx.notify();
    }

    /// Drag & drop result: sequential moves, stop at the first failure;
    /// the destination is revealed (F.12, §5.14).
    pub(crate) fn run_move_plan(
        &mut self,
        plan: MovePlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items: Vec<RelPath> = plan
            .items
            .iter()
            .filter_map(|p| RelPath::parse(p).ok())
            .collect();
        if items.is_empty() {
            return;
        }
        let dest = if plan.dest.is_empty() {
            RelPath::root()
        } else {
            match RelPath::parse(&plan.dest) {
                Ok(dest) => dest,
                Err(_) => return,
            }
        };
        if !plan.dest.is_empty() {
            self.explorer.expand(&plan.dest);
            self.reveal_in_tree(&plan.dest);
        }
        let _ = self.dispatch_checked(Command::MoveMany { items, dest }, window, cx);
        cx.notify();
    }

    // ----- Delete (F.11, §5.13) ---------------------------------------------------------

    pub(crate) fn open_delete(
        &mut self,
        items: Vec<Item>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = items.into_iter().map(|i| (i.path, i.kind)).collect();
        let Some(confirm) = DeleteConfirm::new(items) else {
            return;
        };
        self.files.delete = Some(confirm);
        self.files.delete_on_confirm = !DeleteConfirm::initial_focus_cancel();
        self.files.delete_error = None;
        self.open_overlay(OverlayKind::Dialog(DELETE), window, cx);
    }

    /// Delete the open note (palette "Document: Delete…").
    pub(crate) fn delete_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.active_path() {
            self.open_delete(
                vec![Item {
                    path,
                    kind: ItemKind::File,
                    encrypted: false,
                }],
                window,
                cx,
            );
        }
    }

    pub(crate) fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirm) = self.files.delete.take() else {
            return;
        };
        for (cmd, (path, _)) in confirm.commands().into_iter().zip(confirm.items.iter()) {
            match self.dispatch_checked(cmd, window, cx) {
                Ok(()) => self.explorer.remove(path),
                Err(err) => {
                    // First failure stops the rest (F.11); the dialog stays with the error.
                    self.files.delete_error = Some(err);
                    let rest: Vec<(String, ItemKind)> = confirm
                        .items
                        .iter()
                        .skip_while(|(p, _)| p != path)
                        .cloned()
                        .collect();
                    self.files.delete = DeleteConfirm::new(rest);
                    cx.notify();
                    return;
                }
            }
        }
        self.close_overlay(window, cx);
        cx.notify();
    }

    fn delete_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match keystroke.key.as_str() {
            "tab" | "left" | "right" => {
                self.files.delete_on_confirm = !self.files.delete_on_confirm;
            }
            "enter" if keystroke.modifiers.control => self.confirm_delete(window, cx),
            "enter" | "space" => {
                if self.files.delete_on_confirm {
                    self.confirm_delete(window, cx);
                } else {
                    self.close_overlay(window, cx);
                }
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    // ----- inline rename (F.14, §5.13) ----------------------------------------------

    /// Start renaming `path` inline (explorer row or pane title).
    pub(crate) fn start_inline_rename(
        &mut self,
        target: RenameTarget,
        path: &str,
        kind: ItemKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(rename) = InlineRename::start(target, path, kind) else {
            return;
        };
        let value = rename.value().to_owned();
        self.files.rename = Some(rename);
        let input = self.files.rename_input.clone();
        Self::set_input(&input, &value, true, window, cx);
        if target == RenameTarget::Row {
            input.update(cx, |input, cx| input.select_all(window, cx));
        }
        cx.notify();
    }

    /// Enter / blur: rename; errors stay inline and keep editing.
    pub(crate) fn commit_inline_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.files.rename.as_ref() else {
            return;
        };
        let target = rename.target;
        let result = match rename.commit() {
            Ok(None) => Ok(()),
            Ok(Some(cmd)) => {
                // End editing first: the rename re-renders the row/title.
                let pending = self.files.rename.take();
                let result = self.dispatch_checked(cmd, window, cx);
                if result.is_err() {
                    self.files.rename = pending;
                }
                result
            }
            Err(err) => Err(err),
        };
        match result {
            Ok(()) => {
                self.files.rename = None;
                match target {
                    RenameTarget::Row => self.sidebar_focus.focus(window, cx),
                    RenameTarget::Title => self.focus_region(FocusRegion::Editor, window, cx),
                }
            }
            Err(err) => {
                if let Some(rename) = self.files.rename.as_mut() {
                    rename.set_error(Some(err));
                }
            }
        }
        cx.notify();
    }

    /// Esc: leave the name as it was.
    pub(crate) fn cancel_inline_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.files.rename.take() else {
            return;
        };
        match rename.target {
            RenameTarget::Row => self.sidebar_focus.focus(window, cx),
            RenameTarget::Title => self.focus_region(FocusRegion::Editor, window, cx),
        }
        cx.notify();
    }

    /// F2 / title click on the open note (HP-030: same command as the explorer).
    pub(crate) fn rename_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.active_path() {
            self.start_inline_rename(RenameTarget::Title, &path, ItemKind::File, window, cx);
        }
    }

    /// Inline rename error under the pane header (§3.4) or the explorer row.
    pub(crate) fn rename_error(&self) -> Option<&UserError> {
        self.files.rename.as_ref().and_then(InlineRename::error)
    }

    // ----- rendering ----------------------------------------------------------------

    fn dialog_frame(
        id: &'static str,
        title: SharedString,
        width: f32,
        focus: Option<&FocusHandle>,
        theme: &Theme,
        close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement + ParentElement + InteractiveElement + Styled {
        v_flex()
            .id(id)
            .test_support()
            .when_some(focus, |el, f| el.track_focus(f))
            .w(px(width))
            .max_w(relative(1.))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(40.))
                    .px_4()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().font_semibold().child(title))
                    .child(
                        Button::new(SharedString::from(format!("{id}-close")))
                            .ghost()
                            .xsmall()
                            .icon(crate::ui::icons::ph("x"))
                            .on_click(move |_, window, cx| close(window, cx)),
                    ),
            )
    }

    fn footer(
        &self,
        id: &'static str,
        primary: SharedString,
        enabled: bool,
        danger: bool,
        on_primary: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let this = cx.entity().downgrade();
        let cancel = this.clone();
        let theme = cx.theme().clone();
        h_flex()
            .h(px(48.))
            .px_4()
            .gap_2()
            .justify_end()
            .border_t_1()
            .border_color(theme.border)
            .child(
                Button::new(SharedString::from(format!("{id}-cancel")))
                    .small()
                    .label(i18n::t(cx, "common.cancel"))
                    .on_click(move |_, window, cx| {
                        let _ = cancel.update(cx, |shell, cx| shell.close_overlay(window, cx));
                    }),
            )
            .child({
                let button = Button::new(SharedString::from(format!("{id}-confirm")))
                    .small()
                    .disabled(!enabled)
                    .label(format!("{primary}  Ctrl+↵"))
                    .on_click(move |_, window, cx| {
                        let _ = this.update(cx, |shell, cx| on_primary(shell, window, cx));
                    });
                if danger {
                    button.danger()
                } else {
                    button.primary()
                }
            })
            .into_any_element()
    }

    fn close_fn(&self, cx: &mut Context<Self>) -> impl Fn(&mut Window, &mut App) + 'static {
        let this = cx.entity().downgrade();
        move |window, cx| {
            let _ = this.update(cx, |shell, cx| shell.close_overlay(window, cx));
        }
    }

    fn error_row(text: Option<SharedString>, theme: &Theme) -> impl IntoElement {
        div()
            .id("dialog-error")
            .test_support()
            .text_xs()
            .text_color(theme.danger)
            .children(text)
    }

    /// One of this module's dialogs, by overlay id.
    pub(crate) fn render_file_dialog(
        &mut self,
        kind: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match kind {
            NEW_NOTE => self.render_new_note(window, cx).into_any_element(),
            NEW_FOLDER => self.render_new_folder(cx).into_any_element(),
            MOVE => self.render_move(cx).into_any_element(),
            DELETE => self.render_delete(cx).into_any_element(),
            _ => div().into_any_element(),
        }
    }

    fn render_new_note(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let Some(form) = self.files.new_note.as_ref() else {
            return div().into_any_element();
        };
        let today = self.clock.now().date();
        let root = i18n::t(cx, "dialogs.new_note.root");
        let slot = i18n::t(cx, "dialogs.new_note.name_slot");
        let preview = form.preview(today, &root, &slot);
        let template = form.template();
        let can_submit = form.can_submit();
        let destination = form.destination();
        let error = form.error().map(|e| Self::err_text(cx, e));
        let this = cx.entity().downgrade();
        let segment = |id: &'static str,
                       t: NoteTemplate,
                       title: &str,
                       subtitle: &str,
                       cx: &mut Context<Self>| {
            let this = this.clone();
            v_flex()
                .id(id)
                .test_support()
                .flex_1()
                .p_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(if template == t {
                    theme.primary
                } else {
                    theme.border
                })
                .when(template == t, |el| el.bg(theme.primary.opacity(0.08)))
                .child(div().font_medium().child(i18n::t(cx, title)))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(i18n::t(cx, subtitle)),
                )
                .on_click(move |_, window, cx| {
                    let _ = this.update(cx, |shell, cx| shell.set_note_template(t, window, cx));
                })
        };
        let empty = segment(
            "dialog-new-note-empty",
            NoteTemplate::Empty,
            "dialogs.new_note.empty_title",
            "dialogs.new_note.empty_subtitle",
            cx,
        );
        let hugo = segment(
            "dialog-new-note-hugo",
            NoteTemplate::Hugo,
            "dialogs.new_note.hugo_title",
            "dialogs.new_note.hugo_subtitle",
            cx,
        );
        let picking = self.files.picking_folder;
        let change = {
            let this = this.clone();
            Button::new("dialog-new-note-change")
                .ghost()
                .xsmall()
                .label(i18n::t(cx, "dialogs.new_note.change"))
                .on_click(move |_, _, cx| {
                    let _ = this.update(cx, |shell, cx| {
                        shell.files.picking_folder = !shell.files.picking_folder;
                        cx.notify();
                    });
                })
        };
        let picker = picking.then(|| {
            let rows = self.folder_paths().into_iter().map(|folder| {
                let this = this.clone();
                let label = if folder.is_empty() {
                    i18n::t(cx, "dialogs.new_item.root_location").to_string()
                } else {
                    format!("{folder}/")
                };
                div()
                    .id(SharedString::from(format!("dialog-folder-{folder}")))
                    .test_support()
                    .px_2()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .hover(|s| s.bg(theme.list_hover))
                    .child(label)
                    .on_click(move |_, window, cx| {
                        let folder = folder.clone();
                        let _ = this.update(cx, |shell, cx| {
                            shell.pick_note_destination(folder, window, cx)
                        });
                    })
            });
            v_flex()
                .id("dialog-new-note-folders")
                .max_h(px(160.))
                .overflow_y_scroll()
                .border_1()
                .border_color(theme.border)
                .rounded(theme.radius)
                .children(rows.collect::<Vec<_>>())
        });
        let where_label = if destination.is_empty() {
            i18n::t(cx, "dialogs.new_item.root_location").to_string()
        } else {
            format!("{destination}/")
        };
        let path_color = if preview.empty {
            theme.muted_foreground.opacity(0.6)
        } else if preview.valid {
            theme.muted_foreground
        } else {
            theme.danger
        };
        let body = v_flex()
            .p_4()
            .gap_3()
            .child(Input::new(&self.files.note_name).id("dialog-new-note-name"))
            .child(h_flex().gap_2().child(empty).child(hugo))
            .when(template == NoteTemplate::Hugo, |el| {
                el.child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(i18n::t(cx, "dialogs.new_note.slug")),
                        )
                        .child(Input::new(&self.files.note_slug).id("dialog-new-note-slug")),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(div().text_color(theme.muted_foreground).child(format!(
                        "{} {where_label}",
                        i18n::t(cx, "dialogs.new_item.creating_in")
                    )))
                    .child(change),
            )
            .children(picker)
            .child(
                div()
                    .id("dialog-new-note-path")
                    .test_support()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(path_color)
                    .child(preview.path),
            )
            .child(Self::error_row(error, &theme));
        let close = self.close_fn(cx);
        let footer = self.footer(
            "dialog-new-note",
            i18n::t(cx, "common.create"),
            can_submit,
            false,
            |shell, window, cx| shell.submit_new_note(window, cx),
            cx,
        );
        Self::dialog_frame(
            "dialog-new-note",
            i18n::t(cx, "dialogs.new_note.title"),
            480.,
            None,
            &theme,
            close,
        )
        .child(body)
        .child(footer)
        .into_any_element()
    }

    fn render_new_folder(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let Some(form) = self.files.new_folder.as_ref() else {
            return div().into_any_element();
        };
        let subtitle = if form.subtitle_root() {
            i18n::t(cx, "dialogs.new_item.root_location").to_string()
        } else {
            form.parent.clone()
        };
        let error = form.error().map(|e| Self::err_text(cx, e));
        let can_submit = form.can_submit();
        let close = self.close_fn(cx);
        let footer = self.footer(
            "dialog-new-folder",
            i18n::t(cx, "common.create"),
            can_submit,
            false,
            |shell, window, cx| shell.submit_new_folder(window, cx),
            cx,
        );
        Self::dialog_frame(
            "dialog-new-folder",
            i18n::t(cx, "dialogs.new_item.new_folder"),
            400.,
            None,
            &theme,
            close,
        )
        .child(
            v_flex()
                .p_4()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(format!(
                            "{} {subtitle}",
                            i18n::t(cx, "dialogs.new_item.creating_in")
                        )),
                )
                .child(Input::new(&self.files.folder_name).id("dialog-new-folder-name"))
                .child(Self::error_row(error, &theme)),
        )
        .child(footer)
        .into_any_element()
    }

    fn render_move(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let Some(form) = self.files.move_form.as_ref() else {
            return div().into_any_element();
        };
        let error = form.error().map(|e| Self::err_text(cx, e));
        let target = form.target().ok();
        // The picker filters by what the user typed; the prefilled parent shows all.
        let original = self.files.move_origin.clone();
        let query = if form.folder() == original {
            String::new()
        } else {
            form.folder().to_lowercase()
        };
        let this = cx.entity().downgrade();
        let folders: Vec<_> = self
            .folder_paths()
            .into_iter()
            .filter(|f| query.is_empty() || f.to_lowercase().contains(&query))
            .take(50)
            .map(|folder| {
                let this = this.clone();
                let label = if folder.is_empty() {
                    i18n::t(cx, "dialogs.new_item.root_location").to_string()
                } else {
                    format!("{folder}/")
                };
                div()
                    .id(SharedString::from(format!("dialog-move-folder-{folder}")))
                    .test_support()
                    .px_2()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .hover(|s| s.bg(theme.list_hover))
                    .child(label)
                    .on_click(move |_, window, cx| {
                        let folder = folder.clone();
                        let _ = this.update(cx, |shell, cx| {
                            let input = shell.files.move_folder.clone();
                            Self::set_input(&input, &folder, false, window, cx);
                            if let Some(form) = shell.files.move_form.as_mut() {
                                form.set_folder(&folder);
                            }
                            cx.notify();
                        });
                    })
            })
            .collect();
        let close = self.close_fn(cx);
        let footer = self.footer(
            "dialog-move",
            i18n::t(cx, "dialogs.move.confirm"),
            target.is_some(),
            false,
            |shell, window, cx| shell.submit_move(window, cx),
            cx,
        );
        Self::dialog_frame(
            "dialog-move",
            i18n::t(cx, "dialogs.move.title"),
            480.,
            None,
            &theme,
            close,
        )
        .child(
            v_flex()
                .p_4()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(i18n::t(cx, "dialogs.move.folder")),
                )
                .child(Input::new(&self.files.move_folder).id("dialog-move-folder"))
                .child(
                    v_flex()
                        .id("dialog-move-folders")
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .border_1()
                        .border_color(theme.border)
                        .rounded(theme.radius)
                        .children(folders),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(i18n::t(cx, "dialogs.rename.placeholder")),
                )
                .child(Input::new(&self.files.move_name).id("dialog-move-name"))
                .when_some(target, |el, target| {
                    el.child(
                        div()
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(target),
                    )
                })
                .child(Self::error_row(error, &theme)),
        )
        .child(footer)
        .into_any_element()
    }

    fn render_delete(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let Some(confirm) = self.files.delete.as_ref() else {
            return div().into_any_element();
        };
        let (key, args) = confirm.message();
        let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let message = i18n::t_args(cx, key, &args);
        let error = self
            .files
            .delete_error
            .as_ref()
            .map(|e| Self::err_text(cx, e));
        let on_confirm = self.files.delete_on_confirm;
        let this = cx.entity().downgrade();
        let cancel = this.clone();
        let close = self.close_fn(cx);
        let focus = self.overlay_focus.clone();
        Self::dialog_frame(
            "dialog-delete",
            i18n::t(cx, "dialogs.delete.title"),
            440.,
            Some(&focus),
            &theme,
            close,
        )
        .on_key_down({
            let this = this.clone();
            move |ev: &KeyDownEvent, window, cx| {
                let _ = this.update(cx, |shell, cx| {
                    if shell.delete_key(&ev.keystroke, window, cx) {
                        cx.stop_propagation();
                    }
                });
            }
        })
        .child(
            v_flex()
                .p_4()
                .gap_2()
                .child(
                    div()
                        .id("dialog-delete-message")
                        .test_support()
                        .child(message),
                )
                .child(Self::error_row(error, &theme)),
        )
        .child(
            h_flex()
                .h(px(48.))
                .px_4()
                .gap_2()
                .justify_end()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    Button::new("dialog-delete-cancel")
                        .small()
                        .selected(!on_confirm)
                        .label(i18n::t(cx, "common.cancel"))
                        .on_click(move |_, window, cx| {
                            let _ = cancel.update(cx, |shell, cx| shell.close_overlay(window, cx));
                        }),
                )
                .child(
                    Button::new("dialog-delete-confirm")
                        .small()
                        .danger()
                        .selected(on_confirm)
                        .label(i18n::t(cx, "common.delete"))
                        .on_click(move |_, window, cx| {
                            let _ = this.update(cx, |shell, cx| shell.confirm_delete(window, cx));
                        }),
                ),
        )
        .into_any_element()
    }
}
