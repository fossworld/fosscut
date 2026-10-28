use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Errors from a [`ProjectStore`].
#[derive(Debug, Error)]
pub enum StoreError {
    /// The document does not exist.
    #[error("{0} not found")]
    NotFound(String),
    /// Any other storage failure.
    #[error("{path}: {message}")]
    Io {
        /// Document path inside the bundle.
        path: String,
        /// Failure description.
        message: String,
    },
}

/// Storage for the documents of one project bundle.
///
/// Paths are relative to the bundle root and use `/` separators. Desktop builds use
/// [`DirStore`]; tests and the future web build use [`MemoryStore`] or a browser-backed
/// implementation.
pub trait ProjectStore {
    /// Reads a whole document.
    fn read(&self, path: &str) -> Result<Vec<u8>, StoreError>;
    /// Replaces a document atomically: readers see the old or the new content, never a mix.
    fn write_atomic(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError>;
    /// Appends to a document, creating it if missing.
    fn append(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError>;
    /// True when the document exists.
    fn exists(&self, path: &str) -> bool;
}

/// An in-memory store for tests and non-file environments.
#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    files: BTreeMap<String, Vec<u8>>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    /// All stored paths, sorted.
    pub fn paths(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
}

impl ProjectStore for MemoryStore {
    fn read(&self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(path.to_string()))
    }

    fn write_atomic(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        self.files.insert(path.to_string(), data.to_vec());
        Ok(())
    }

    fn append(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        self.files
            .entry(path.to_string())
            .or_default()
            .extend_from_slice(data);
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
}

/// A store backed by a bundle directory on the local file system.
#[derive(Clone, Debug)]
pub struct DirStore {
    root: PathBuf,
}

impl DirStore {
    /// Uses `root` (the `.fosscut` directory) as the bundle root.
    pub fn new(root: impl Into<PathBuf>) -> DirStore {
        DirStore { root: root.into() }
    }

    /// The bundle directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn full_path(&self, path: &str) -> PathBuf {
        path.split('/')
            .fold(self.root.clone(), |acc, part| acc.join(part))
    }

    fn io_error(path: &str, error: std::io::Error) -> StoreError {
        if error.kind() == std::io::ErrorKind::NotFound {
            StoreError::NotFound(path.to_string())
        } else {
            StoreError::Io {
                path: path.to_string(),
                message: error.to_string(),
            }
        }
    }
}

impl ProjectStore for DirStore {
    fn read(&self, path: &str) -> Result<Vec<u8>, StoreError> {
        std::fs::read(self.full_path(path)).map_err(|e| Self::io_error(path, e))
    }

    fn write_atomic(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        let target = self.full_path(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Self::io_error(path, e))?;
        }
        let mut temporary = target.clone().into_os_string();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        let result = (|| {
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(data)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &target)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(|e| Self::io_error(path, e))
    }

    fn append(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        let target = self.full_path(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Self::io_error(path, e))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&target)
            .map_err(|e| Self::io_error(path, e))?;
        file.write_all(data)
            .and_then(|()| file.sync_all())
            .map_err(|e| Self::io_error(path, e))
    }

    fn exists(&self, path: &str) -> bool {
        self.full_path(path).exists()
    }
}
