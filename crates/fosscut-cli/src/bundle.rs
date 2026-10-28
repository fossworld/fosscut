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
                        .is_some_and(|age| age > LOCK_STALE);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > LOCK_WAIT {
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
