//! Opening project bundles on disk.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::{DirStore, Id, IdKind, Project, TimelineFormat};

const LOCK_FILE: &str = ".lock";
const LOCK_WAIT: Duration = Duration::from_secs(10);
const LOCK_STALE: Duration = Duration::from_secs(120);

/// Exclusive write access to a bundle, released on drop.
pub struct BundleLock {
    path: PathBuf,
}

impl BundleLock {
    /// Waits up to ten seconds for other writers; locks older than two minutes are stale.
    pub fn acquire(bundle: &Path) -> Result<BundleLock, ApiError> {
        BundleLock::acquire_with(bundle, LOCK_WAIT, LOCK_STALE)
    }

    fn acquire_with(
        bundle: &Path,
        wait: Duration,
        stale_after: Duration,
    ) -> Result<BundleLock, ApiError> {
        let path = bundle.join(LOCK_FILE);
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(BundleLock { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > stale_after);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > wait {
                        return Err(ApiError::new(
                            codes::PROJECT_BUSY,
                            format!("{} is locked by another process", bundle.display()),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return Err(ApiError::new(
                        "fosscut.project.storage",
                        format!("cannot lock {}: {e}", bundle.display()),
                    ));
                }
            }
        }
    }
}

impl Drop for BundleLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The bundle path from `--project`, or a single `*.fosscut` directory in the working directory.
pub fn bundle_path(explicit: Option<&Path>) -> Result<PathBuf, ApiError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let candidates: Vec<PathBuf> = std::fs::read_dir(".")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.extension().is_some_and(|ext| ext == "fosscut"))
                .collect()
        })
        .unwrap_or_default();
    match candidates.as_slice() {
        [single] => Ok(single.clone()),
        [] => Err(ApiError::new("fosscut.project.not_found", "no project given")
            .with_hint("Pass --project <path.fosscut>, set FOSSCUT_PROJECT, or run inside a directory with one .fosscut bundle.")),
        _ => Err(ApiError::new(codes::REQUEST_INVALID, "several .fosscut bundles in this directory")
            .with_hint("Choose one with --project.")),
    }
}

/// Opens an existing bundle.
pub fn open(bundle: &Path) -> Result<Session<DirStore>, ApiError> {
    if !bundle.join(fosscut_model::MANIFEST_PATH).exists() {
        return Err(ApiError::new(
            "fosscut.project.not_found",
            format!("{} is not a FossCut project", bundle.display()),
        )
        .with_hint("Create one with: fosscut project new <path.fosscut>"));
    }
    Session::open(DirStore::new(bundle))
}

/// Creates a new bundle directory with an empty primary timeline.
pub fn create(
    bundle: &Path,
    name: &str,
    format: TimelineFormat,
) -> Result<Session<DirStore>, ApiError> {
    if bundle.exists()
        && std::fs::read_dir(bundle)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Err(ApiError::new(
            codes::REQUEST_INVALID,
            format!("{} already exists and is not empty", bundle.display()),
        ));
    }
    std::fs::create_dir_all(bundle).map_err(|e| {
        ApiError::new(
            "fosscut.project.storage",
            format!("cannot create {}: {e}", bundle.display()),
        )
    })?;
    let project = Project::new(new_project_id(name), name, format);
    Session::create(DirStore::new(bundle), project)
}

fn new_project_id(name: &str) -> Id {
    let mut hasher = blake3::Hasher::new();
    hasher.update(name.as_bytes());
    hasher.update(&now_ms().unwrap_or_default().to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    let hash = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes[..5].copy_from_slice(&hash.as_bytes()[..5]);
    Id::new(IdKind::Project, u64::from_le_bytes(bytes))
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fosscut-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_held_lock_makes_other_writers_wait_then_fail() {
        let dir = bundle_dir("busy");
        let held = BundleLock::acquire(&dir).unwrap();
        let Err(error) = BundleLock::acquire_with(&dir, Duration::from_millis(120), LOCK_STALE)
        else {
            panic!("a second writer must not get the lock");
        };
        assert_eq!(error.code, codes::PROJECT_BUSY);
        drop(held);
        assert!(
            !dir.join(LOCK_FILE).exists(),
            "dropping the lock removes the file"
        );
        let again = BundleLock::acquire(&dir).unwrap();
        drop(again);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stale_locks_are_taken_over() {
        let dir = bundle_dir("stale");
        let file = std::fs::File::create(dir.join(LOCK_FILE)).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(600))
            .unwrap();
        drop(file);
        let lock = BundleLock::acquire_with(&dir, Duration::ZERO, LOCK_STALE).unwrap();
        drop(lock);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn locking_a_missing_bundle_is_a_storage_error() {
        let dir = std::env::temp_dir().join(format!("fosscut-lock-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let Err(error) = BundleLock::acquire(&dir) else {
            panic!("a missing bundle cannot be locked");
        };
        assert_eq!(error.code, "fosscut.project.storage");
    }
}
