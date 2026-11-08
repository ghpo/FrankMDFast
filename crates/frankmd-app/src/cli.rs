//! Command line (`frankmd [path]`, the native `fed [path]`) and single
//! instance (OM-021).
//!
//! `path` may be a notes folder (opens it) or a note (opens its vault: the
//! configured `NOTES_PATH` when the note is inside it, else the note's
//! folder, then the note). A second invocation forwards its request to the
//! running instance over a user-private Unix socket and exits; the primary
//! focuses the matching window or opens a new one.

#[cfg(unix)]
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};

/// FrankMD command line.
#[derive(Debug, Clone, clap::Parser)]
#[command(
    name = "frankmdfast",
    version,
    about = "Fast Markdown notes and blog editor"
)]
pub struct Cli {
    /// Notes folder or note to open (falls back to `NOTES_PATH`, like `fed <path>`).
    pub path: Option<PathBuf>,
    /// Override the UI locale (debugging/testing).
    #[arg(long)]
    pub locale: Option<String>,
    /// Always start a new instance (don't forward to a running one).
    #[arg(long)]
    pub new_instance: bool,
    /// Serve this vault over HTTP on the Tailscale network (opens the window too).
    #[arg(long)]
    pub serve: bool,
    /// Serve without opening a window (headless remote server).
    #[arg(long)]
    pub serve_only: bool,
    /// Bind target: `tailscale` (default), `loopback`, `any`, or an IP address.
    #[arg(long)]
    pub serve_bind: Option<String>,
    /// Allow binding a non-Tailscale, non-loopback address (dangerous).
    #[arg(long)]
    pub serve_bind_any: bool,
    /// Remote port (default 8787, or FRANKMD_REMOTE_PORT).
    #[arg(long)]
    pub serve_port: Option<u16>,
    /// Access token (>= 32 bytes). Prefer FRANKMD_AUTH_TOKEN or `.fed auth_token`
    /// (a flag is visible in `ps`).
    #[arg(long)]
    pub serve_token: Option<String>,
}

/// What to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Notes root.
    pub vault_root: PathBuf,
    /// Note to open, as given (absolute paths are resolved inside the vault later).
    pub open: Option<String>,
}

/// Resolve `path` (already absolute or relative to `cwd`) against the
/// configured notes root (`NOTES_PATH`) and `~/notes` as the last default.
pub fn resolve_launch(
    path: Option<&Path>,
    cwd: &Path,
    notes_path: Option<&Path>,
    home: Option<&Path>,
) -> Launch {
    let default_root = || {
        notes_path
            .map(Path::to_path_buf)
            .or_else(|| home.map(|h| h.join("notes")))
            .unwrap_or_else(|| cwd.join("notes"))
    };
    let Some(path) = path else {
        return Launch {
            vault_root: default_root(),
            open: None,
        };
    };
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let is_note = abs.extension().is_some_and(|e| e == "md") || abs.is_file();
    if !is_note {
        return Launch {
            vault_root: abs,
            open: None,
        };
    }
    if let Some(root) = notes_path.filter(|root| abs.starts_with(root)) {
        return Launch {
            vault_root: root.to_path_buf(),
            open: Some(abs.display().to_string()),
        };
    }
    // Opening a note from the desktop/recent list must still use its vault,
    // not the note's immediate folder. Prefer the default ~/notes root, then
    // the nearest ancestor that has a `.fed`.
    if let Some(root) = home
        .map(|home| home.join("notes"))
        .filter(|root| abs.starts_with(root))
    {
        return Launch {
            vault_root: root,
            open: Some(abs.display().to_string()),
        };
    }
    if let Some(root) = nearest_config_root(&abs) {
        return Launch {
            vault_root: root,
            open: Some(abs.display().to_string()),
        };
    }
    let parent = abs.parent().map_or_else(default_root, Path::to_path_buf);
    Launch {
        vault_root: parent,
        open: Some(abs.display().to_string()),
    }
}

/// The nearest ancestor of a note that owns a `.fed` file.
fn nearest_config_root(note: &Path) -> Option<PathBuf> {
    note.parent()?
        .ancestors()
        .find(|dir| dir.join(".fed").is_file())
        .map(Path::to_path_buf)
}

/// Socket of the primary instance (`$XDG_RUNTIME_DIR/frankmdfast.sock`).
pub fn socket_path(runtime_dir: Option<&Path>, state_dir: &Path) -> PathBuf {
    runtime_dir.map_or_else(
        || state_dir.join("frankmdfast.sock"),
        |d| d.join("frankmdfast.sock"),
    )
}

/// One forwarded request: vault root, then the note (may be empty), newline-separated.
#[cfg_attr(not(unix), allow(dead_code))]
fn encode(launch: &Launch) -> String {
    format!(
        "{}\n{}\n",
        launch.vault_root.display(),
        launch.open.as_deref().unwrap_or("")
    )
}

#[cfg_attr(not(unix), allow(dead_code))]
fn decode(lines: &[String]) -> Option<Launch> {
    let root = lines.first().filter(|l| !l.is_empty())?;
    let open = lines.get(1).filter(|l| !l.is_empty()).cloned();
    Some(Launch {
        vault_root: PathBuf::from(root),
        open,
    })
}

/// Hand `launch` to a running instance. `true` when one took it.
#[cfg(unix)]
pub fn forward(socket: &Path, launch: &Launch) -> bool {
    use std::os::unix::net::UnixStream;
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return false;
    };
    stream.write_all(encode(launch).as_bytes()).is_ok() && stream.flush().is_ok()
}

/// Not supported off Unix: always start a new instance.
#[cfg(not(unix))]
pub fn forward(_socket: &Path, _launch: &Launch) -> bool {
    false
}

/// Become the primary instance: accept forwarded requests on a background
/// thread and pass them to `deliver`. A stale socket file is replaced.
#[cfg(unix)]
pub fn listen(socket: &Path, deliver: impl Fn(Launch) + Send + 'static) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::UnixListener;
    if let Some(dir) = socket.parent().filter(|d| !d.exists()) {
        crate::app_fs::ensure_private_dir(dir)?;
    }
    crate::app_fs::remove_stale_socket(socket)?;
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    std::thread::Builder::new()
        .name("frankmd-ipc".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let lines: Vec<String> = BufReader::new(stream)
                    .lines()
                    .take(2)
                    .map_while(Result::ok)
                    .collect();
                if let Some(launch) = decode(&lines) {
                    deliver(launch);
                }
            }
        })?;
    Ok(())
}

/// Not supported off Unix.
#[cfg(not(unix))]
pub fn listen(_socket: &Path, _deliver: impl Fn(Launch) + Send + 'static) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn folder_argument_is_the_vault() {
        let l = resolve_launch(
            Some(Path::new("blog")),
            Path::new("/home/u"),
            None,
            Some(Path::new("/home/u")),
        );
        assert_eq!(
            l,
            Launch {
                vault_root: "/home/u/blog".into(),
                open: None
            }
        );
    }

    #[test]
    fn no_argument_uses_notes_path_then_home_notes() {
        let l = resolve_launch(
            None,
            Path::new("/x"),
            Some(Path::new("/n")),
            Some(Path::new("/home/u")),
        );
        assert_eq!(l.vault_root, PathBuf::from("/n"));
        let l = resolve_launch(None, Path::new("/x"), None, Some(Path::new("/home/u")));
        assert_eq!(l.vault_root, PathBuf::from("/home/u/notes"));
    }

    #[test]
    fn note_inside_notes_path_opens_in_that_vault() {
        let l = resolve_launch(
            Some(Path::new("/n/blog/a.md")),
            Path::new("/x"),
            Some(Path::new("/n")),
            None,
        );
        assert_eq!(
            l,
            Launch {
                vault_root: "/n".into(),
                open: Some("/n/blog/a.md".into())
            }
        );
    }

    #[test]
    fn note_inside_home_notes_uses_home_notes_without_notes_path() {
        let l = resolve_launch(
            Some(Path::new("/home/u/notes/blog/a.md")),
            Path::new("/"),
            None,
            Some(Path::new("/home/u")),
        );
        assert_eq!(
            l,
            Launch {
                vault_root: "/home/u/notes".into(),
                open: Some("/home/u/notes/blog/a.md".into())
            }
        );
    }

    #[test]
    fn note_uses_nearest_fed_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let note = vault.join("sub/a.md");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        std::fs::write(vault.join(".fed"), "").unwrap();
        std::fs::write(&note, "# a\n").unwrap();
        let l = resolve_launch(Some(&note), Path::new("/"), None, None);
        assert_eq!(
            l,
            Launch {
                vault_root: vault,
                open: Some(note.display().to_string())
            }
        );
    }

    #[test]
    fn note_elsewhere_opens_its_folder() {
        let l = resolve_launch(
            Some(Path::new("/tmp/x/a.md")),
            Path::new("/"),
            Some(Path::new("/n")),
            None,
        );
        assert_eq!(
            l,
            Launch {
                vault_root: "/tmp/x".into(),
                open: Some("/tmp/x/a.md".into())
            }
        );
    }

    #[test]
    fn requests_round_trip() {
        let l = Launch {
            vault_root: "/n".into(),
            open: Some("a.md".into()),
        };
        let lines: Vec<String> = encode(&l).lines().map(str::to_owned).collect();
        assert_eq!(decode(&lines), Some(l));
        assert_eq!(
            decode(&["/n".into(), String::new()]),
            Some(Launch {
                vault_root: "/n".into(),
                open: None
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_second_instance_forwards_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("s.sock");
        let launch = Launch {
            vault_root: "/n".into(),
            open: None,
        };
        assert!(!forward(&socket, &launch), "nobody listening yet");
        let (tx, rx) = std::sync::mpsc::channel();
        listen(&socket, move |l| {
            let _ = tx.send(l);
        })
        .unwrap();
        assert!(forward(&socket, &launch));
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            launch
        );
    }
}
