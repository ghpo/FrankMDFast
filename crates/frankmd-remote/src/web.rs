//! Embedded web client (no build step, no CDN: it must work on an offline
//! Tailscale network).

/// The SPA shell.
pub const INDEX_HTML: &str = include_str!("../web/index.html");
/// The client script.
pub const APP_JS: &str = include_str!("../web/app.js");
/// The stylesheet.
pub const STYLE_CSS: &str = include_str!("../web/style.css");
/// The login page.
pub const LOGIN_HTML: &str = include_str!("../web/login.html");
/// The tiny login-page script (shows `?error=invalid`).
pub const LOGIN_JS: &str = include_str!("../web/login.js");
