//! Where the engine keeps regenerable, machine-specific data, and how it stays bounded.
//! Never inside a project bundle (ADR-0004, item 9).
//!
//! Analysis results (ADR-0019), proxies, remuxed copies and rendered frames (ADR-0032) each
//! have a folder of the cache directory, named by [`CacheKind::folder`]. Their entries are
//! files keyed by content, so deleting any of them only costs the time to make it again.
//! [`cache_info`] reports their sizes, [`clear`] empties them and [`enforce`] applies the
//! `cache.max_age_days` and `cache.limit_gb` policy (ADR-0031). None of them touches files
//! being written (names ending in `.partial`), the installed models, or anything outside
//! those folders.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use fosscut_model::{ContentIdentity, Id, Project};
use serde::{Deserialize, Serialize};

use crate::EngineError;
use crate::export::resolve;

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

/// One of the caches in the cache directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheKind {
    /// Analysis results by file content (ADR-0019), in `analysis/`.
    Analysis,
    /// Proxies for the preview ([`crate::proxy`]), in `proxies/`.
    Proxies,
    /// Remuxed copies for the preview ([`crate::remux`]), in `remux/`.
    Remux,
    /// Rendered preview frames ([`crate::frame_cache`]), in `frames/`.
    Frames,
}

impl CacheKind {
    /// Every cache, in the order [`cache_info`] reports them.
    pub const ALL: [CacheKind; 4] = [
        CacheKind::Analysis,
        CacheKind::Proxies,
        CacheKind::Remux,
        CacheKind::Frames,
    ];

    /// Its name, which is also its folder's: `analysis`, `proxies`, `remux` or `frames`.
    pub fn name(self) -> &'static str {
        match self {
            CacheKind::Analysis => "analysis",
            CacheKind::Proxies => "proxies",
            CacheKind::Remux => "remux",
            CacheKind::Frames => "frames",
        }
    }

    /// The cache named `name`.
    pub fn parse(name: &str) -> Result<CacheKind, EngineError> {
        CacheKind::ALL
            .into_iter()
            .find(|kind| kind.name() == name)
            .ok_or_else(|| EngineError::InvalidArguments {
                message: format!(
                    "there is no cache {name:?}; the caches are analysis, proxies, remux and frames"
                ),
            })
    }

    /// Its folder in the cache directory `cache`.
    pub fn folder(self, cache: &Path) -> PathBuf {
        cache.join(self.name())
    }
}

/// How much one cache holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheUsage {
    /// The cache.
    pub kind: CacheKind,
    /// Its folder.
    pub path: String,
    /// Bytes of its entries.
    pub bytes: u64,
    /// Number of entries (files).
    pub entries: u64,
}

/// What [`enforce`] removed and kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupReport {
    /// Entries removed.
    pub removed: u64,
    /// Bytes they held.
    pub freed_bytes: u64,
    /// Entries kept.
    pub kept: u64,
    /// Bytes they hold.
    pub kept_bytes: u64,
}

/// The cache policy of a host (ADR-0031, `cache.max_age_days` and `cache.limit_gb`), for
/// [`crate::jobs::Jobs::set_cache_policy`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachePolicy {
    /// The cache directory, normally [`cache_dir`].
    pub cache: PathBuf,
    /// Entries unused for longer are removed; 0 keeps them however old.
    pub max_age_days: u32,
    /// The oldest entries go beyond this many bytes in all; 0 sets no limit.
    pub limit_bytes: u64,
}

impl CachePolicy {
    /// Applies the policy now; see [`enforce`].
    pub fn enforce(&self) -> CleanupReport {
        enforce(&self.cache, self.max_age_days, self.limit_bytes)
    }
}

/// The size of every cache in `cache`, in [`CacheKind::ALL`] order. A cache that does not
/// exist yet is empty.
pub fn cache_info(cache: &Path) -> Vec<CacheUsage> {
    CacheKind::ALL
        .into_iter()
        .map(|kind| {
            let entries = entries(cache, &[kind]);
            CacheUsage {
                kind,
                path: kind.folder(cache).display().to_string(),
                bytes: entries.iter().map(|e| e.bytes).sum(),
                entries: entries.len() as u64,
            }
        })
        .collect()
}

/// Empties the caches `kinds` of `cache` (every cache when `kinds` is empty) and returns the
/// bytes freed. Entries that cannot be removed (held open on Windows, for example) stay.
pub fn clear(cache: &Path, kinds: &[CacheKind]) -> u64 {
    let kinds = if kinds.is_empty() {
        &CacheKind::ALL[..]
    } else {
        kinds
    };
    let mut freed = 0;
    for entry in entries(cache, kinds) {
        if std::fs::remove_file(&entry.path).is_ok() {
            freed += entry.bytes;
        }
    }
    for kind in kinds {
        remove_empty_folders(&kind.folder(cache));
    }
    freed
}

/// Applies the cache policy to every cache of `cache`: entries unused for more than
/// `max_age_days` days are removed (0 keeps them however old), then the least recently used
/// until all of them together hold at most `limit_bytes` (0 sets no limit). An entry was
/// last used when it was last modified or read, whichever is later; the engine updates
/// both whenever it uses a proxy, a remuxed copy or a rendered frame.
pub fn enforce(cache: &Path, max_age_days: u32, limit_bytes: u64) -> CleanupReport {
    enforce_at(cache, max_age_days, limit_bytes, SystemTime::now())
}

fn enforce_at(cache: &Path, max_age_days: u32, limit_bytes: u64, now: SystemTime) -> CleanupReport {
    let mut entries = entries(cache, &CacheKind::ALL);
    // Oldest first.
    entries.sort_by_key(|e| e.used);
    let mut report = CleanupReport::default();
    let oldest_kept = (max_age_days > 0)
        .then(|| now.checked_sub(Duration::from_secs(u64::from(max_age_days) * 86_400)))
        .flatten();
    let mut total: u64 = entries.iter().map(|e| e.bytes).sum();
    for entry in entries {
        let expired = oldest_kept.is_some_and(|limit| entry.used < limit);
        let over = limit_bytes > 0 && total > limit_bytes;
        if (expired || over) && std::fs::remove_file(&entry.path).is_ok() {
            report.removed += 1;
            report.freed_bytes += entry.bytes;
            total -= entry.bytes;
        } else {
            report.kept += 1;
            report.kept_bytes += entry.bytes;
        }
    }
    for kind in CacheKind::ALL {
        remove_empty_folders(&kind.folder(cache));
    }
    report
}

/// One file in a cache.
struct Entry {
    path: PathBuf,
    bytes: u64,
    used: SystemTime,
}

/// Every entry of the caches `kinds`: regular files below their folders, without following
/// symbolic links and without files being written.
fn entries(cache: &Path, kinds: &[CacheKind]) -> Vec<Entry> {
    let mut found = Vec::new();
    for kind in kinds {
        collect(&kind.folder(cache), &mut found);
    }
    found
}

fn collect(dir: &Path, found: &mut Vec<Entry>) {
    // A folder that is a link could lead outside the cache.
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return;
    }
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    for item in listing.flatten() {
        let Ok(metadata) = item.metadata() else {
            continue;
        };
        let path = item.path();
        if metadata.is_dir() {
            collect(&path, found);
        } else if metadata.is_file() && !is_temporary(&path) {
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let accessed = metadata.accessed().unwrap_or(modified);
            found.push(Entry {
                path,
                bytes: metadata.len(),
                used: modified.max(accessed),
            });
        }
    }
}

/// Removes the empty folders below `dir`, keeping `dir`.
fn remove_empty_folders(dir: &Path) {
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return;
    }
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    for item in listing.flatten() {
        if item.file_type().is_ok_and(|t| t.is_dir()) {
            let path = item.path();
            remove_empty_folders(&path);
            // Fails, as it should, while anything is left inside.
            let _ = std::fs::remove_dir(&path);
        }
    }
}

/// True for a file being written: its name ends in `.partial`, as every cache and export
/// writer of the engine names its temporary files.
pub(crate) fn is_temporary(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(".partial"))
}

/// A temporary name for writing `path`, beside it and unique to this process and call, so
/// writers never meet; moving it to `path` makes the entry appear complete.
pub(crate) fn temporary_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}-{n}.partial", std::process::id()))
}

/// Marks `path` as used now, for [`enforce`]; a file that cannot be updated keeps its times.
pub(crate) fn touch(path: &Path) {
    let now = SystemTime::now();
    if let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_times(
            std::fs::FileTimes::new()
                .set_accessed(now)
                .set_modified(now),
        );
    }
}

/// The folder-safe name of a file content: the identity's hex digits (an edited media
/// table cannot point outside the cache with them) and its size, as the analysis cache
/// names it.
pub(crate) fn content_name(identity: &ContentIdentity) -> String {
    let hash: String = identity
        .sample_blake3
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(64)
        .collect();
    format!("{hash}-{}", identity.size)
}

/// The media file of `asset` and its content identity: the one recorded at import, or
/// measured now when the media table has none.
pub(crate) fn source_of(
    project: &Project,
    bundle: &Path,
    asset: &Id,
) -> Result<(PathBuf, ContentIdentity), EngineError> {
    let recorded = project
        .media
        .get(asset)
        .ok_or_else(|| EngineError::AssetNotFound {
            asset: asset.to_string(),
        })?
        .identity
        .clone();
    let path = resolve(project, bundle, asset)?;
    let identity = match recorded {
        Some(identity) => identity,
        None => crate::import::identity(&path).map_err(|e| EngineError::Failed {
            code: e.code,
            message: e.message,
        })?,
    };
    Ok((path, identity))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fosscut-cache-policy-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes `bytes` bytes at `path`, last used `days` days ago.
    fn file(path: &Path, bytes: usize, days: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7u8; bytes]).unwrap();
        let at = SystemTime::now() - Duration::from_secs(days * 86_400);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_accessed(at).set_modified(at))
            .unwrap();
    }

    #[test]
    fn kinds_have_names_and_folders() {
        for kind in CacheKind::ALL {
            assert_eq!(CacheKind::parse(kind.name()).unwrap(), kind);
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::Value::String(kind.name().into())
            );
        }
        assert_eq!(
            CacheKind::Proxies.folder(Path::new("/c")),
            Path::new("/c/proxies")
        );
        assert_eq!(
            CacheKind::parse("models").unwrap_err().code(),
            "fosscut.request.invalid_arguments"
        );
    }

    #[test]
    fn info_counts_entries_and_clear_empties_the_chosen_caches() {
        let cache = folder("clear");
        file(&cache.join("analysis/ab-1/silences@1-x.json"), 100, 0);
        file(&cache.join("proxies/ab-1/half@1.mov"), 1_000, 0);
        file(&cache.join("frames/ab/abcd.rgba"), 10, 0);
        file(&cache.join("frames/ab/cdef.rgba"), 20, 0);
        file(&cache.join("frames/ab/.cdef.rgba.1-0.partial"), 5, 0);
        file(&cache.join("models/ggml-tiny.en.bin"), 50, 0);
        let info = cache_info(&cache);
        let usage = |kind| info.iter().find(|u| u.kind == kind).cloned().unwrap();
        assert_eq!(info.len(), 4);
        assert_eq!(
            (
                usage(CacheKind::Frames).bytes,
                usage(CacheKind::Frames).entries
            ),
            (30, 2),
            "the file being written is not an entry"
        );
        assert_eq!(
            usage(CacheKind::Remux).entries,
            0,
            "a missing cache is empty"
        );
        assert_eq!(usage(CacheKind::Proxies).bytes, 1_000);

        assert_eq!(clear(&cache, &[CacheKind::Frames]), 30);
        assert!(cache.join("frames/ab/.cdef.rgba.1-0.partial").exists());
        assert!(cache.join("proxies/ab-1/half@1.mov").exists());
        assert_eq!(clear(&cache, &[]), 1_100, "everything else");
        assert!(!cache.join("proxies/ab-1").exists(), "empty folders go");
        assert!(
            cache.join("models/ggml-tiny.en.bin").exists(),
            "models are not a cache"
        );
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn the_policy_removes_old_entries_then_the_oldest_beyond_the_limit() {
        let cache = folder("policy");
        file(&cache.join("analysis/a/old.json"), 100, 40);
        file(&cache.join("proxies/a/half@1.mov"), 1_000, 10);
        file(&cache.join("remux/a@1.mov"), 1_000, 5);
        file(&cache.join("frames/aa/new.rgba"), 1_000, 1);
        file(&cache.join("frames/aa/newest.rgba"), 1_000, 0);
        file(&cache.join("frames/aa/.x.1-2.partial"), 1_000, 400);
        file(&cache.join("models/m.bin"), 1_000, 400);

        // Nothing is old enough and there is room: nothing changes.
        let kept = enforce(&cache, 60, 10_000);
        assert_eq!((kept.removed, kept.kept, kept.kept_bytes), (0, 5, 4_100));

        // Older than 30 days goes.
        let aged = enforce(&cache, 30, 10_000);
        assert_eq!((aged.removed, aged.freed_bytes), (1, 100));
        assert!(!cache.join("analysis/a/old.json").exists());

        // Beyond 2 500 bytes, the least recently used go first.
        let limited = enforce(&cache, 0, 2_500);
        assert_eq!((limited.removed, limited.freed_bytes), (2, 2_000));
        assert_eq!(limited.kept_bytes, 2_000);
        assert!(!cache.join("proxies/a/half@1.mov").exists());
        assert!(!cache.join("remux/a@1.mov").exists());
        assert!(cache.join("frames/aa/new.rgba").exists());
        assert!(cache.join("frames/aa/newest.rgba").exists());

        // A use makes an entry recent again.
        touch(&cache.join("frames/aa/new.rgba"));
        enforce(&cache, 0, 1_000);
        assert!(cache.join("frames/aa/new.rgba").exists());
        assert!(!cache.join("frames/aa/newest.rgba").exists());

        // 0 days and 0 bytes keep everything.
        let unlimited = enforce_at(
            &cache,
            0,
            0,
            SystemTime::now() + Duration::from_secs(1_000 * 86_400),
        );
        assert_eq!(unlimited.removed, 0);
        assert!(
            cache.join("frames/aa/.x.1-2.partial").exists(),
            "files being written are never touched"
        );
        assert!(
            cache.join("models/m.bin").exists(),
            "models are not a cache"
        );
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn links_out_of_the_cache_are_not_followed() {
        let cache = folder("links");
        let outside = folder("links-outside");
        file(&outside.join("precious.mov"), 1_000, 400);
        std::fs::create_dir_all(cache.join("proxies")).unwrap();
        std::os::unix::fs::symlink(&outside, cache.join("proxies/elsewhere")).unwrap();
        std::os::unix::fs::symlink(&outside, cache.join("remux")).unwrap();
        assert_eq!(enforce(&cache, 1, 1).removed, 0);
        assert_eq!(clear(&cache, &[]), 0);
        assert!(outside.join("precious.mov").exists());
        std::fs::remove_dir_all(&cache).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn temporary_names_are_unique_partials_beside_the_entry() {
        let entry = Path::new("/c/proxies/ab-1/half@1.mov");
        let (a, b) = (temporary_path(entry), temporary_path(entry));
        assert_ne!(a, b);
        assert_eq!(a.parent(), entry.parent());
        assert!(is_temporary(&a) && !is_temporary(entry));
        let identity = ContentIdentity {
            size: 12,
            sample_blake3: "ab/../cd".into(),
        };
        assert_eq!(content_name(&identity), "abcd-12", "only hex digits");
    }
}
