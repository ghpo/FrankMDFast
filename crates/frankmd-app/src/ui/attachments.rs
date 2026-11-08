//! Local note attachments: the paperclip list in the preview header and the
//! "Save as…" / "Save all…" flow.
//!
//! The preview scanner finds every local non-note file referenced by the
//! note (Markdown links, images, `<video>/<audio>/<source>` and reference
//! definitions). This module only turns those references into save actions;
//! it never opens a binary file as a Markdown note.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::prelude::*;
use crate::scale::px;
use gpui_kit::*;

/// One row in the attachment dialog.
#[derive(Debug, Clone)]
pub(crate) struct AttachmentRow {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) relative: String,
    pub(crate) size: u64,
    pub(crate) encrypted: bool,
}

/// The open attachment dialog.
pub(crate) struct AttachmentsState {
    pub(crate) rows: Vec<AttachmentRow>,
    pub(crate) selected: Option<PathBuf>,
    pub(crate) message: Option<(SharedString, bool)>,
}

/// An encrypted attachment waiting for the unlock dialog.
#[derive(Debug, Clone)]
pub(crate) enum PendingAttachmentAction {
    One(PathBuf),
    All,
}

impl Shell {
    /// Open the attachment list for the current note, optionally highlighting
    /// the file whose link was clicked.
    pub(crate) fn open_attachments(
        &mut self,
        focus: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let references = self.preview.read(cx).attachments().to_vec();
        let mut rows = Vec::new();
        let mut seen = HashSet::new();
        for reference in references {
            if seen.insert(reference.path.clone())
                && let Some(row) = self.attachment_row(&reference.path)
            {
                rows.push(row);
            }
        }
        if let Some(path) = focus.as_ref()
            && path.starts_with(&self.vault_root)
            && path.file_name().is_none_or(|name| name != ".fed")
            && !path.to_string_lossy().ends_with(".md")
            && path.is_file()
            && seen.insert(path.clone())
            && let Some(row) = self.attachment_row(path)
        {
            rows.push(row);
        }
        if rows.is_empty() {
            let message = i18n::t(cx, "attachments.empty");
            self.show_message(&message, false, cx);
            return;
        }
        let selected = focus.filter(|path| rows.iter().any(|row| &row.path == path));
        self.attachments = Some(AttachmentsState {
            rows,
            selected,
            message: None,
        });
        self.open_overlay(OverlayKind::Dialog("attachments"), window, cx);
    }

    fn attachment_row(&self, path: &Path) -> Option<AttachmentRow> {
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let relative = path
            .strip_prefix(&self.vault_root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| relative.clone());
        Some(AttachmentRow {
            path: path.to_path_buf(),
            name,
            relative,
            size: metadata.len(),
            encrypted: crate::encryption::media_is_encrypted(path),
        })
    }

    /// Save one attachment through the native save dialog.
    pub(crate) fn save_attachment(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self
            .attachments
            .as_ref()
            .and_then(|state| state.rows.get(index))
            .cloned()
        else {
            return;
        };
        if row.encrypted && !self.encryption.is_unlocked() {
            if self.encryption.seal().is_none() {
                self.set_attachment_message("attachments.locked", true, cx);
                return;
            }
            self.pending_attachment = Some(PendingAttachmentAction::One(row.path));
            self.open_unlock(window, cx);
            return;
        }
        self.prompt_save_attachment(row.path, row.name, window, cx);
    }

    /// Save every attachment into a user-chosen folder.
    pub(crate) fn save_all_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.attachments.as_ref() else {
            return;
        };
        let rows: Vec<(PathBuf, String)> = state
            .rows
            .iter()
            .map(|row| (row.path.clone(), row.name.clone()))
            .collect();
        if rows.is_empty() {
            return;
        }
        if state.rows.iter().any(|row| row.encrypted) && !self.encryption.is_unlocked() {
            if self.encryption.seal().is_none() {
                self.set_attachment_message("attachments.locked", true, cx);
                return;
            }
            self.pending_attachment = Some(PendingAttachmentAction::All);
            self.open_unlock(window, cx);
            return;
        }

        let prompt = Some(i18n::t(cx, "attachments.choose_folder"));
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt,
        });
        let session = Arc::clone(&self.encryption);
        let total = rows.len();
        let task = cx.spawn_in(window, async move |this, cx| {
            let directory = match rx.await {
                Ok(Ok(Some(mut paths))) => match paths.pop() {
                    Some(path) => path,
                    None => return,
                },
                Ok(Err(err)) => {
                    this.update_in(cx, |shell, _window, cx| {
                        shell.set_attachment_message(
                            &format!("{}: {err:#}", i18n::t(cx, "attachments.save_failed")),
                            true,
                            cx,
                        );
                    })
                    .ok();
                    return;
                }
                _ => return,
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut saved = 0usize;
                    let mut error = None;
                    for (source, name) in rows {
                        let destination = unique_path(&directory, &name);
                        match crate::encryption::save_attachment(&session, &source, &destination) {
                            Ok(()) => saved += 1,
                            Err(err) if error.is_none() => {
                                error = Some(format!("{}: {err:#}", name));
                            }
                            Err(_) => {}
                        }
                    }
                    (saved, error)
                })
                .await;
            this.update_in(cx, |shell, _window, cx| {
                let (saved, error) = result;
                match error {
                    None => {
                        let text = i18n::t_args(
                            cx,
                            "attachments.saved_all",
                            &[("count", &saved.to_string())],
                        );
                        shell.set_attachment_message(&text, false, cx);
                    }
                    Some(error) => {
                        let text = i18n::t_args(
                            cx,
                            "attachments.saved_partial",
                            &[
                                ("saved", &saved.to_string()),
                                ("total", &total.to_string()),
                                ("error", &error),
                            ],
                        );
                        shell.set_attachment_message(&text, true, cx);
                    }
                }
            })
            .ok();
        });
        self.attachment_jobs.push(task);
    }

    /// After a successful unlock, continue the save the user asked for.
    pub(crate) fn resume_pending_attachment(
        &mut self,
        pending: Option<PendingAttachmentAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match pending {
            Some(PendingAttachmentAction::One(path)) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "attachment".to_owned());
                self.prompt_save_attachment(path, name, window, cx);
            }
            Some(PendingAttachmentAction::All) => self.save_all_attachments(window, cx),
            None => {}
        }
    }

    fn prompt_save_attachment(
        &mut self,
        source: PathBuf,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let directory = attachment_default_dir(&self.vault_root);
        let rx = cx.prompt_for_new_path(&directory, Some(&name));
        let session = Arc::clone(&self.encryption);
        let task = cx.spawn_in(window, async move |this, cx| {
            let destination = match rx.await {
                Ok(Ok(Some(path))) => path,
                Ok(Err(err)) => {
                    this.update_in(cx, |shell, _window, cx| {
                        shell.set_attachment_message(
                            &format!("{}: {err:#}", i18n::t(cx, "attachments.save_failed")),
                            true,
                            cx,
                        );
                    })
                    .ok();
                    return;
                }
                _ => return,
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::encryption::save_attachment(&session, &source, &destination)
                })
                .await;
            this.update_in(cx, |shell, _window, cx| match result {
                Ok(()) => {
                    let text = i18n::t(cx, "attachments.saved_one");
                    shell.set_attachment_message(&text, false, cx);
                }
                Err(err) => {
                    let text = format!("{}: {err:#}", i18n::t(cx, "attachments.save_failed"));
                    shell.set_attachment_message(&text, true, cx);
                }
            })
            .ok();
        });
        self.attachment_jobs.push(task);
    }

    fn set_attachment_message(&mut self, text: &str, error: bool, cx: &mut Context<Self>) {
        if let Some(state) = self.attachments.as_mut() {
            state.message = Some((SharedString::from(text.to_owned()), error));
        }
        self.show_message(text, error, cx);
        cx.notify();
    }

    /// The attachment dialog content (`OverlayKind::Dialog("attachments")`).
    pub(super) fn render_attachments_dialog(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(state) = self.attachments.as_ref() else {
            return div().into_any_element();
        };
        let rows = state.rows.clone();
        let selected = state.selected.clone();
        let message = state.message.clone();
        let count = rows.len();
        let focus = self.overlay_focus.clone();
        let close = cx.entity().downgrade();
        let save_all = cx.entity().downgrade();
        let mut list = v_flex()
            .id("attachments-list")
            .gap_1()
            .max_h(px(420.))
            .overflow_y_scroll();
        for (index, row) in rows.into_iter().enumerate() {
            let locked = row.encrypted && !self.encryption.is_unlocked();
            let is_selected = selected.as_ref() == Some(&row.path);
            let save = cx.entity().downgrade();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("attachment-row-{index}")))
                    .test_support()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded(theme.radius)
                    .when(is_selected, |el| el.bg(theme.selection))
                    .child(
                        crate::ui::icons::ph(if row.encrypted { "lock" } else { "file" })
                            .with_size(px(18.))
                            .text_color(if locked {
                                theme.warning
                            } else {
                                theme.muted_foreground
                            }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(row.name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(row.relative.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(frankmd_core::text::stats::format_file_size(row.size)),
                    )
                    .when(row.encrypted, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(if locked {
                                    theme.warning
                                } else {
                                    theme.muted_foreground
                                })
                                .child(i18n::t(
                                    cx,
                                    if locked {
                                        "attachments.locked"
                                    } else {
                                        "attachments.encrypted"
                                    },
                                )),
                        )
                    })
                    .child(
                        Button::new(SharedString::from(format!("attachment-save-{index}")))
                            .xsmall()
                            .label(i18n::t(cx, "attachments.save_as"))
                            .on_click(move |_, window, cx| {
                                let _ = save.update(cx, |shell, cx| {
                                    shell.save_attachment(index, window, cx)
                                });
                            }),
                    ),
            );
        }
        v_flex()
            .id("attachments-dialog")
            .test_support()
            .key_context(actions::CX_OVERLAY)
            .track_focus(&focus)
            .w(px(660.))
            .max_w(relative(1.))
            .p_4()
            .gap_3()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().font_semibold().child(i18n::t_args(
                        cx,
                        "attachments.title_count",
                        &[("count", &count.to_string())],
                    )))
                    .child(div().flex_1())
                    .child(
                        Button::new("attachments-close")
                            .ghost()
                            .xsmall()
                            .icon(crate::ui::icons::ph("x"))
                            .tooltip(i18n::t(cx, "attachments.close"))
                            .on_click(move |_, window, cx| {
                                let _ =
                                    close.update(cx, |shell, cx| shell.close_overlay(window, cx));
                            }),
                    ),
            )
            .child(list)
            .when_some(message, |el, (text, error)| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(if error { theme.danger } else { theme.success })
                        .child(text),
                )
            })
            .child(
                h_flex().justify_end().gap_2().child(
                    Button::new("attachments-save-all")
                        .primary()
                        .label(i18n::t(cx, "attachments.save_all"))
                        .on_click(move |_, window, cx| {
                            let _ = save_all
                                .update(cx, |shell, cx| shell.save_all_attachments(window, cx));
                        }),
                ),
            )
            .into_any_element()
    }
}

fn attachment_default_dir(vault_root: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
    {
        return path;
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let downloads = home.join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
        return home;
    }
    vault_root.parent().unwrap_or(vault_root).to_path_buf()
}

fn unique_path(directory: &Path, name: &str) -> PathBuf {
    let candidate = directory.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| name.to_owned());
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());
    for suffix in 1..=9999 {
        let file = match &extension {
            Some(extension) => format!("{stem} ({suffix}).{extension}"),
            None => format!("{stem} ({suffix})"),
        };
        let candidate = directory.join(file);
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(name)
}

#[allow(clippy::disallowed_methods)]
#[cfg(test)]
mod tests {
    #[test]
    fn collision_gets_a_suffix() {
        let dir = std::env::temp_dir().join(format!("frankmd-attachments-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("talk.m4a"), b"x").expect("write");
        assert_eq!(
            super::unique_path(&dir, "talk.m4a"),
            dir.join("talk (1).m4a")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
