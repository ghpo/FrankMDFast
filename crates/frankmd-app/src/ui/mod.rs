//! GPUI views. Each view renders from the workspace's view models and sends
//! registry commands; none owns domain state.

pub mod actions;
pub mod appearance;
mod attachments;
mod commands;
mod editor_host;
mod editor_pane;
mod encryption_actions;
mod explorer;
mod file_dialogs;
pub mod fonts;
mod help;
pub mod i18n;
pub(crate) mod icons;
mod locked_note;
mod media_host;
mod navigation;
mod overlay_view;
mod overlays;
mod palette;
mod prelude;
mod preview_export;
mod preview_region;
mod recovery;
mod render;
mod search;
mod settings;
mod shell;
mod sidebar;
mod status_line;
mod tasks;

pub use shell::{Shell, ShellConfig};
