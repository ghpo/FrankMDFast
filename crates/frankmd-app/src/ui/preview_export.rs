//! Preview header actions: copy the rendered note, save a PDF and print.

use std::path::{Path, PathBuf};

use super::prelude::*;
use gpui_kit::*;

impl Shell {
    /// Copy the rendered preview as rich HTML and plain text.
    pub(crate) fn copy_preview(&mut self, cx: &mut Context<Self>) {
        let Some((html, plain)) = self.preview_export_payload(cx) else {
            return;
        };
        match arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_html(html, Some(plain.clone())))
        {
            Ok(()) => {
                let text = i18n::t(cx, "preview.copied_formatted");
                self.show_message(&text, false, cx);
            }
            Err(err) => {
                tracing::warn!(%err, "rich clipboard unavailable; copying plain text");
                cx.write_to_clipboard(ClipboardItem::new_string(plain));
                let text = i18n::t(cx, "preview.copied_plain");
                self.show_message(&text, true, cx);
            }
        }
        cx.notify();
    }

    /// Prompt for a destination and render the preview to PDF.
    pub(crate) fn save_preview_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((html, _)) = self.preview_export_payload(cx) else {
            return;
        };
        let Some(title) = self.preview_title() else {
            return;
        };
        let directory = attachment_default_dir(&self.vault_root);
        let suggested = format!("{title}.pdf");
        let rx = cx.prompt_for_new_path(&directory, Some(&suggested));
        let html_path = self.write_preview_html(&html);
        let task = cx.spawn_in(window, async move |this, cx| {
            let destination = match rx.await {
                Ok(Ok(Some(path))) => path,
                Ok(Err(err)) => {
                    this.update_in(cx, |shell, _window, cx| {
                        let text =
                            i18n::t_args(cx, "preview.pdf_failed", &[("error", &err.to_string())]);
                        shell.show_message(&text, true, cx);
                    })
                    .ok();
                    return;
                }
                _ => return,
            };
            let result = match html_path {
                Ok(html_path) => {
                    cx.background_executor()
                        .spawn(async move { render_pdf(&html_path, &destination) })
                        .await
                }
                Err(err) => Err(err),
            };
            this.update_in(cx, |shell, _window, cx| {
                match result {
                    Ok(()) => {
                        let text = i18n::t(cx, "preview.pdf_saved");
                        shell.show_message(&text, false, cx);
                    }
                    Err(err) => {
                        let text = i18n::t_args(cx, "preview.pdf_failed", &[("error", &err)]);
                        shell.show_message(&text, true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.attachment_jobs.push(task);
    }

    /// Render a temporary PDF and open it in the default viewer. The user can
    /// print from the viewer when no CUPS printer is configured.
    pub(crate) fn print_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((html, _)) = self.preview_export_payload(cx) else {
            return;
        };
        let Some(title) = self.preview_title() else {
            return;
        };
        let html_path = self.write_preview_html(&html);
        let pdf_path = self
            .state_dir
            .join("export")
            .join(format!("{title}-print.pdf"));
        let task = cx.spawn_in(window, async move |this, cx| {
            let pdf_for_render = pdf_path.clone();
            let result = match html_path {
                Ok(html_path) => {
                    cx.background_executor()
                        .spawn(async move { render_pdf(&html_path, &pdf_for_render) })
                        .await
                }
                Err(err) => Err(err),
            };
            this.update_in(cx, |shell, _window, cx| match result {
                Ok(()) => match open_path(&pdf_path) {
                    Ok(()) => {
                        let text = i18n::t(cx, "preview.print_opened");
                        shell.show_message(&text, false, cx);
                    }
                    Err(err) => {
                        let text = i18n::t_args(cx, "preview.print_failed", &[("error", &err)]);
                        shell.show_message(&text, true, cx);
                    }
                },
                Err(err) => {
                    let text = i18n::t_args(cx, "preview.print_failed", &[("error", &err)]);
                    shell.show_message(&text, true, cx);
                }
            })
            .ok();
        });
        self.attachment_jobs.push(task);
    }

    fn preview_export_payload(&self, cx: &Context<Self>) -> Option<(String, String)> {
        self.active_path()?;
        let title = self.preview_title()?;
        let html = self.preview.read(cx).export_html(&title);
        let plain = self.preview.read(cx).export_plain_text();
        Some((html, plain))
    }

    fn preview_title(&self) -> Option<String> {
        self.active_path().map(|path| {
            path.rsplit('/')
                .next()
                .unwrap_or(&path)
                .trim_end_matches(".md")
                .to_owned()
        })
    }

    fn write_preview_html(&self, html: &str) -> Result<PathBuf, String> {
        let dir = self.state_dir.join("export");
        crate::app_fs::ensure_private_dir(&dir).map_err(|err| err.to_string())?;
        let path = dir.join(format!(
            "preview-{}-{}.html",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_millis())
        ));
        frankmd_fs::atomic::atomic_replace(
            &path,
            html.as_bytes(),
            frankmd_fs::atomic::ModePolicy::Fixed(0o600),
        )
        .map_err(|err| err.to_string())?;
        Ok(path)
    }
}

#[allow(clippy::disallowed_methods)]
fn render_pdf(html: &Path, destination: &Path) -> Result<(), String> {
    let url = url::Url::from_file_path(html).map_err(|_| "invalid preview path".to_owned())?;
    let profile = std::env::temp_dir().join(format!(
        "frankmd-chromium-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_millis())
    ));
    let output = format!("--print-to-pdf={}", destination.display());
    let status = std::process::Command::new("chromium")
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-pdf-header-footer")
        .arg("--allow-file-access-from-files")
        .arg("--no-first-run")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--virtual-time-budget=2000")
        .arg(output)
        .arg(url.as_str())
        .status()
        .map_err(|err| format!("Chromium: {err}"))?;
    let _ = std::fs::remove_dir_all(&profile);
    if !status.success() || !destination.is_file() {
        return Err(format!("Chromium exited with {status}"));
    }
    Ok(())
}

fn open_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("cmd");
    #[cfg(target_os = "windows")]
    command.arg("/C").arg("start").arg("");
    command
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn attachment_default_dir(vault_root: &Path) -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let downloads = home.join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
        return home;
    }
    vault_root.parent().unwrap_or(vault_root).to_path_buf()
}
