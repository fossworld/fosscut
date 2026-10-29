//! Where the engine keeps regenerable, machine-specific data. Never inside a project
//! bundle (ADR-0004, item 9).

use std::path::PathBuf;

/// FossCut's per-user cache directory: `FOSSCUT_CACHE_DIR` when set, otherwise `fosscut`
/// in the platform's cache directory (`~/Library/Caches` on macOS, `%LOCALAPPDATA%` on
/// Windows, `$XDG_CACHE_HOME` or `~/.cache` elsewhere), or in the system temporary
/// directory when none of those is known.
pub fn cache_dir() -> PathBuf {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var("FOSSCUT_CACHE_DIR") {
        return dir;
    }
    let base = if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library").join("Caches"))
    } else if cfg!(windows) {
        var("LOCALAPPDATA")
    } else {
        var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("fosscut")
}

/// Where models are installed (ADR-0019, item 5): `FOSSCUT_MODELS_DIR` when set, otherwise
/// `models` in [`cache_dir`]. Separate so tests and CI can share one installed model while
/// keeping other caches apart.
pub fn models_dir() -> PathBuf {
    std::env::var_os("FOSSCUT_MODELS_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| cache_dir().join("models"))
}
