//! The writer lock (ADR-0022, item 2): an operating-system lock on `.writer.lock` in the
//! bundle. The file is created once and never deleted; the kernel releases the lock when
//! its holder exits, however it exits.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fosscut_core::{ApiError, codes};

/// The lock file's name inside a bundle.
pub const LOCK_FILE: &str = ".writer.lock";

/// How long writers wait for each other before reporting the project busy.
pub(crate) const WAIT: Duration = Duration::from_secs(10);

/// Pause between attempts while another writer holds the lock.
pub(crate) const RETRY: Duration = Duration::from_millis(20);

/// Exclusive write access to a bundle, released on drop.
#[derive(Debug)]
pub struct WriterLock {
    file: File,
    path: PathBuf,
}

impl WriterLock {
    /// Takes the lock when no other writer holds it; `None` when one does.
    pub fn try_acquire(bundle: &Path) -> Result<Option<WriterLock>, ApiError> {
        let path = bundle.join(LOCK_FILE);
        let file = open(&path).map_err(|e| storage(bundle, &e))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(WriterLock { file, path })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(storage(bundle, &e)),
        }
    }

    /// Waits up to ten seconds for other writers, then reports the project busy.
    pub fn acquire(bundle: &Path) -> Result<WriterLock, ApiError> {
        WriterLock::acquire_within(bundle, WAIT)
    }

    pub(crate) fn acquire_within(bundle: &Path, wait: Duration) -> Result<WriterLock, ApiError> {
        let started = Instant::now();
        loop {
            if let Some(lock) = WriterLock::try_acquire(bundle)? {
                return Ok(lock);
            }
            if started.elapsed() >= wait {
                return Err(busy(bundle));
            }
            std::thread::sleep(RETRY);
        }
    }

    /// True while the locked file is still the one at the lock's path, so that no tool
    /// replaced or removed it under the holder (ADR-0022, item 7). Windows refuses to
    /// delete a file that is open without delete sharing, so only Unix needs to look.
    pub fn still_in_place(&self) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            match (self.file.metadata(), std::fs::metadata(&self.path)) {
                (Ok(held), Ok(current)) => {
                    held.dev() == current.dev() && held.ino() == current.ino()
                }
                _ => false,
            }
        }
        #[cfg(not(unix))]
        {
            self.path.exists()
        }
    }
}

fn open(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE, without FILE_SHARE_DELETE: nobody can delete
        // or rename the file while a writer has it open.
        options.share_mode(0x1 | 0x2);
    }
    options.open(path)
}

fn storage(bundle: &Path, error: &std::io::Error) -> ApiError {
    ApiError::new(
        "fosscut.project.storage",
        format!("cannot lock {}: {error}", bundle.display()),
    )
}

pub(crate) fn busy(bundle: &Path) -> ApiError {
    ApiError::new(
        codes::PROJECT_BUSY,
        format!("{} is being written by another process", bundle.display()),
    )
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
        let held = WriterLock::acquire(&dir).unwrap();
        assert!(WriterLock::try_acquire(&dir).unwrap().is_none());
        let Err(error) = WriterLock::acquire_within(&dir, Duration::from_millis(60)) else {
            panic!("a second writer must not get the lock");
        };
        assert_eq!(error.code, codes::PROJECT_BUSY);
        drop(held);
        assert!(
            dir.join(LOCK_FILE).exists(),
            "the lock file stays, so a lock never follows a deleted file"
        );
        let again = WriterLock::try_acquire(&dir).unwrap();
        assert!(again.is_some(), "a released lock is free at once");
        drop(again);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lock_left_by_a_crashed_writer_is_free() {
        // The kernel releases the lock with the file handle, whatever the holder did; an
        // old lock file with no holder never blocks anyone.
        let dir = bundle_dir("left");
        let file = std::fs::File::create(dir.join(LOCK_FILE)).unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        drop(file);
        let lock = WriterLock::acquire_within(&dir, Duration::ZERO).unwrap();
        assert!(lock.still_in_place());
        drop(lock);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_lock_file_is_noticed() {
        let dir = bundle_dir("replaced");
        let lock = WriterLock::acquire(&dir).unwrap();
        std::fs::remove_file(dir.join(LOCK_FILE)).unwrap();
        assert!(!lock.still_in_place(), "a deleted lock file is noticed");
        std::fs::write(dir.join(LOCK_FILE), b"").unwrap();
        assert!(
            !lock.still_in_place(),
            "a new file at the path is another file"
        );
        drop(lock);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn locking_a_missing_bundle_is_a_storage_error() {
        let dir = std::env::temp_dir().join(format!("fosscut-lock-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let Err(error) = WriterLock::acquire(&dir) else {
            panic!("a missing bundle cannot be locked");
        };
        assert_eq!(error.code, "fosscut.project.storage");
    }
}
