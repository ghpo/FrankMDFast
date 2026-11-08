//! FrankMD desktop app: the only crate that links GPUI / gpui-kit.
//!
//! This crate is deliberately thin: it wires GUI-free crates to GPUI views.
//! Domain logic never lives here (see docs/architecture.md, "the GUI-free rule").
//! It is a library so headless GPUI tests in `tests/` can reach the views:
//! [`ui::Shell`] is the root view, [`ui::actions::install_keymap`] the keymap.

pub mod assets;
pub mod bridge;
pub mod cli;
pub mod dialogs;
pub mod editor;
pub mod encryption;
pub mod note_attachments;
pub mod preview;
pub mod remote;
pub mod scale;
pub mod ui;

/// App-owned files outside every vault (XDG state dir, IPC socket).
///
/// HP-004 routes filesystem *mutations of notes* through `frankmd-fs`, which
/// has no API for the app's own directories; these two helpers are the only
/// direct std::fs mutations in the app and never touch a vault.
pub(crate) mod app_fs {
    use std::path::Path;

    /// `mkdir -p` an app-owned directory (mode 0700 on Unix).
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    /// Remove a stale socket file of a previous instance.
    #[cfg(unix)]
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn remove_stale_socket(path: &Path) -> std::io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser as _;
use frankmd_config::EnvSnapshot;
use frankmd_config::dirs::AppDirs;
use gpui_kit::*;

/// Wayland app_id / X11 WM_CLASS; must match `frankmdfast.desktop` (OM-006).
pub const APP_ID: &str = "frankmdfast";

/// Parse the CLI and run the app until the last window closes.
pub fn run() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    init_tracing();
    tracing::info!(
        path = ?cli.path,
        locale = ?cli.locale,
        "starting FrankMDFast"
    );

    let env = EnvSnapshot::capture();
    let dirs = AppDirs::discover().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let prefs_file = dirs
        .state
        .join(frankmd_workspace::app_prefs::APP_PREFS_FILE);
    let prefs = frankmd_workspace::app_prefs::AppPrefsStore::open(&prefs_file)
        .get()
        .clone();
    let home = env.present("HOME").map(PathBuf::from);
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let notes_path = env.present("NOTES_PATH").map(PathBuf::from);
    let launch = cli::resolve_launch(
        cli.path.as_deref(),
        &cwd,
        notes_path.as_deref(),
        home.as_deref(),
    );

    // Remote access is opt-in (CLI or Settings › Remote). The GUI never runs
    // the server in-process: it spawns a detached `--serve-only` daemon so the
    // server survives closing the window.
    let remote_requested = cli.serve || cli.serve_only || prefs.remote_start;
    let real = launch
        .vault_root
        .canonicalize()
        .unwrap_or_else(|_| launch.vault_root.clone());
    let remote_state_dir = dirs.vault_state(&real);

    if cli.serve_only {
        let request = remote::build_request(&cli, &launch, &env, &dirs, &prefs)?;
        let plan = remote::prepare_request(&request)?;
        remote::write_running_status(
            &request.state_dir,
            std::process::id(),
            &plan.url,
            request.port,
            &request.bind_spec,
        );
        let runtime = frankmd_net::NetRuntime::new()?;
        tracing::info!(url = %plan.url, "serving without a window");
        let result = runtime
            .handle()
            .block_on(frankmd_remote::serve(plan.state, plan.addr));
        if let Err(err) = &result {
            remote::record_error(&request.state_dir, &err.to_string());
        }
        result?;
        return Ok(());
    }

    if remote_requested {
        match remote::spawn_daemon(&cli, &launch, &env, &dirs, &prefs) {
            Ok(status) => {
                tracing::info!(url = %status.url, pid = status.pid, "remote server running");
            }
            Err(err) => {
                tracing::error!(%err, "remote server not started");
                eprintln!("FrankMDFast: remote server not started: {err:#}");
                remote::record_error(&remote_state_dir, &format!("{err:#}"));
            }
        }
    }

    // Single instance (OM-021): hand the request to a running FrankMD.
    let socket = cli::socket_path(env.present("XDG_RUNTIME_DIR").map(Path::new), &dirs.state);
    if !cli.new_instance && !cli.serve && cli::forward(&socket, &launch) {
        tracing::info!("forwarded to the running instance");
        return Ok(());
    }

    let (tx, rx) = async_channel::unbounded::<cli::Launch>();
    if let Err(err) = cli::listen(&socket, move |l| {
        let _ = tx.send_blocking(l);
    }) {
        tracing::warn!(%err, "single-instance socket unavailable");
    }

    let locale = cli.locale.clone();
    // Resolved before the event loop starts: fc-match is a process spawn.
    let (ui_font, mono_font) = ui::fonts::resolve_system_fonts();
    let ctx = LaunchContext {
        dirs,
        env,
        home,
        locale,
    };
    // macOS relaunch with every window closed (Dock/Launchpad/Finder click):
    // reopen the last vault instead of sitting there as a zombie (#3).
    let reopen = (ctx.clone(), launch.clone());
    let app = gpui_kit::application().with_assets(assets::FrankAssets);
    app.on_reopen(move |cx| reopen.0.open(reopen.1.clone(), cx));
    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_global(bridge::theme::SystemFonts {
            ui: ui_font.map(Into::into),
            mono: mono_font.map(Into::into),
        });
        ui::actions::install_keymap(cx);
        dialogs::media::init(cx);
        match frankmd_net::NetRuntime::new() {
            Ok(rt) => {
                let rt = Arc::new(rt);
                cx.set_global(bridge::tokio::NetRuntimeGlobal(Arc::clone(&rt)));
            }
            Err(err) => tracing::error!(%err, "network runtime unavailable"),
        }
        // Without a menu bar macOS has no ⌘Q key equivalent (#4); the
        // Quit item's ⌘Q comes from the keymap binding above.
        cx.set_menus(vec![Menu {
            name: "FrankMDFast".into(),
            items: vec![MenuItem::action("Quit FrankMDFast", ui::actions::Quit)],
            disabled: false,
        }]);
        cx.activate(true);
        ctx.open(launch, cx);
        let ctx = ctx.clone();
        cx.spawn(async move |cx| {
            while let Ok(launch) = rx.recv().await {
                cx.update(|cx| ctx.open(launch, cx));
            }
        })
        .detach();
    });
    Ok(())
}

/// What every window of this process shares.
#[derive(Clone)]
struct LaunchContext {
    dirs: AppDirs,
    env: EnvSnapshot,
    home: Option<PathBuf>,
    locale: Option<String>,
}

impl LaunchContext {
    /// Focus the window already showing `launch.vault_root` (opening the
    /// note there), or open a new window for it.
    fn open(&self, launch: cli::Launch, cx: &mut App) {
        for handle in cx.windows() {
            let Some(root) = handle.downcast::<gpui_kit::base::Root>() else {
                continue;
            };
            let found = root.update(cx, |root, window, cx| {
                let Ok(shell) = root.view().clone().downcast::<ui::Shell>() else {
                    return false;
                };
                let same = shell.read(cx).vault_root() == launch.vault_root;
                if same {
                    window.activate_window();
                    if let Some(open) = launch.open.clone() {
                        shell.update(cx, |shell, cx| shell.open_argument(&open, window, cx));
                    }
                }
                same
            });
            if found.unwrap_or(false) {
                return;
            }
        }
        let real = launch
            .vault_root
            .canonicalize()
            .unwrap_or_else(|_| launch.vault_root.clone());
        let state_dir = self.dirs.vault_state(&real);
        let encryption = crate::encryption::load_session(&state_dir);
        let config = ui::ShellConfig {
            state_dir,
            config_dir: self.dirs.config.clone(),
            encryption: Some(encryption),
            vault_root: launch.vault_root,
            env: self.env.clone(),
            open: launch.open,
            clock: Arc::new(frankmd_core::SystemClock),
            omarchy: self
                .home
                .as_deref()
                .map(frankmd_theme::OmarchyPaths::from_home),
            watch: true,
            locale: self.locale.clone(),
            trash_dir: None,
            global_config: Some(frankmd_config::global::global_config_path(&self.dirs)),
            app_prefs: Some(
                self.dirs
                    .state
                    .join(frankmd_workspace::app_prefs::APP_PREFS_FILE),
            ),
        };
        let options = WindowOptions {
            app_id: Some(APP_ID.into()),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.), px(800.)),
                cx,
            ))),
            ..gpui_kit::component::TitleBar::window_options()
        };
        if let Err(err) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ui::Shell::new(config, window, cx))
        }) {
            tracing::error!(%err, "failed to open window");
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_env("FRANKMD_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
