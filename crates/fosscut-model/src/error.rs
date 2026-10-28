use thiserror::Error;

use crate::{Issue, StoreError};

/// Errors from loading, saving or validating a project.
#[derive(Debug, Error)]
pub enum ModelError {
    /// The storage backend failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A document is not valid JSON or does not match the schema.
    #[error("{path}: {message}")]
    Json {
        /// Document path inside the bundle.
        path: String,
        /// Parser message.
        message: String,
    },
    /// The project was written by a newer or unknown format version.
    #[error(
        "project format {found} is not supported by this FossCut (supports {supported}); update FossCut"
    )]
    UnsupportedVersion {
        /// Version found in `project.json`.
        found: String,
        /// Version this build reads and writes.
        supported: &'static str,
    },
    /// The project parsed but violates format rules.
    #[error("project is invalid: {}", .0.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("; "))]
    Invalid(Vec<Issue>),
}

impl ModelError {
    /// The stable error code used in FossCut API responses (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            ModelError::Store(StoreError::NotFound(_)) => "fosscut.project.not_found",
            ModelError::Store(_) => "fosscut.project.storage",
            ModelError::Json { .. } => "fosscut.project.malformed",
            ModelError::UnsupportedVersion { .. } => "fosscut.project.unsupported_version",
            ModelError::Invalid(_) => "fosscut.project.invalid",
        }
    }
}
