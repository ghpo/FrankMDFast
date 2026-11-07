//! # frankmd-remote
//!
//! Optional remote access for FrankMDFast: a small HTTP + SSE server that
//! serves the same notes vault to a browser running on another machine on the
//! same **Tailscale** network.
//!
//! Design rules (mirroring the app's architecture):
//! - **GUI-free.** This crate never links GPUI; it reuses the same domain
//!   crates (`frankmd-fs`, `frankmd-config`, `frankmd-markdown`, ...).
//! - **Opt-in and off by default.** [`RemoteConfig`] must be built explicitly
//!   by the app (`--serve`).
//! - **Tailscale-only by default.** [`bind`] refuses a wildcard bind unless
//!   the operator passes `--serve-bind any`. A token of at least 32 bytes is
//!   mandatory; there is no unauthenticated mode.
//! - **Stateless requests.** Like the original Rails FrankMD, every handler
//!   re-reads configuration and the filesystem; there is no shadow copy. Live
//!   change notifications are pushed over SSE from `frankmd-watch`.

pub mod auth;
pub mod bind;
pub mod error;
pub mod events;
pub mod handlers;
pub mod routes;
pub mod state;
pub mod web;

pub use auth::{AuthConfig, AuthError, AuthVia};
pub use bind::{BindSpec, is_tailscale_addr, resolve_bind, tailscale_ipv4};
pub use error::ApiError;
pub use routes::{build_router, serve, serve_until};
pub use state::{RemoteConfig, ServerState};

/// Default port for the remote web UI.
pub const DEFAULT_PORT: u16 = 8787;
