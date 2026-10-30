//! Live sessions (ADR-0022): a host holds a project's session in memory and is its only
//! writer, and the CLI, the MCP server and the desktop app reach it over a local socket,
//! so every edit appears live in one place and undoes there.
//!
//! - [`Host`] serves one bundle: the desktop app embeds it, `fosscut host` runs it alone.
//! - [`crate::project::Projects`] decides per call whether a host runs and forwards to it,
//!   so front ends need no other change.
//! - A bundle's `.writer.lock` ([`WriterLock`]) guarantees one writer; discovery records in
//!   the user's [`runtime_dir`] tell clients where the host listens.

mod auth;
mod client;
mod host;
mod lock;
mod record;
mod transport;
mod wire;

use std::time::Duration;

use fosscut_core::ApiError;

pub(crate) use client::{CallError, Connection, OpenError};
pub use client::{Subscription, watch};
pub use host::{Change, ClientInfo, Host, HostOptions};
pub use lock::{LOCK_FILE, WriterLock};
pub(crate) use lock::{RETRY, WAIT, busy};
pub use record::{HostName, Record, runtime_dir};
pub(crate) use record::{bundle_key, read as read_record, remove as remove_record};

/// The wire protocol's version (ADR-0022, item 6). Changing a message's shape bumps it.
pub const PROTOCOL: u32 = 1;

/// Another host already serves the bundle.
pub const ALREADY_HOSTED: &str = "fosscut.session.already_hosted";
/// The session key proof failed.
pub const UNAUTHORIZED: &str = "fosscut.session.unauthorized";
/// The host speaks another protocol version, or a message broke the protocol.
pub const PROTOCOL_MISMATCH: &str = "fosscut.session.protocol";
/// The host cannot be reached, or was lost during a request.
pub const UNAVAILABLE: &str = "fosscut.session.unavailable";
/// Files of the bundle changed under the host, which stopped writing.
pub const DIVERGED: &str = "fosscut.session.diverged";

/// Largest message after the handshake.
pub(crate) const MAX_MESSAGE: usize = 64 << 20;
/// Largest message before authentication.
pub(crate) const MAX_HANDSHAKE: usize = 4 << 10;
/// How long a client waits for an answer.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a connection may take to authenticate.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Most connections a host serves at once.
pub(crate) const MAX_CONNECTIONS: usize = 64;
/// Most connections a host lets wait in the handshake at once.
pub(crate) const MAX_UNAUTHENTICATED: usize = 16;

/// A session error with the engine catalogue's hint.
pub(crate) fn error(code: &str, message: impl Into<String>) -> ApiError {
    let api = ApiError::new(code, message);
    if api.hint.is_some() {
        return api;
    }
    match crate::engine_error_catalogue()
        .into_iter()
        .find(|entry| entry.code == code)
    {
        Some(entry) => api.with_hint(entry.hint),
        None => api,
    }
}

/// A short runtime directory for one test: Unix socket paths are limited to about 100
/// bytes, and macOS's temporary directory is long.
#[cfg(test)]
pub(crate) fn test_runtime(name: &str) -> std::path::PathBuf {
    let base = if cfg!(unix) {
        std::path::PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let dir = base.join(format!("fc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
