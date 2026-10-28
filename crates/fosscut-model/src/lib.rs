//! FossCut project documents (ADR-0004).
//!
//! A project is a directory bundle `Name.fosscut/`:
//!
//! ```text
//! project.json          manifest: format version, id, name, revision, timelines
//! media.json            asset table, one record per source file
//! timelines/<id>.json   one document per timeline
//! history/ops.jsonl     append-only operation log (written by fosscut-core)
//! ```
//!
//! Documents are sparse (defaults are omitted), references are typed, and nothing
//! machine-specific is stored besides path hints for media. All file access goes through
//! [`ProjectStore`], so the model builds for `wasm32-unknown-unknown` and can be backed
//! by browser storage or a server later.

mod bundle;
mod error;
mod id;
mod media;
mod project;
mod store;
mod timeline;
mod validate;

pub use bundle::{DocumentKey, MANIFEST_PATH, MEDIA_PATH, OPS_LOG_PATH, Project, TIMELINES_DIR};
pub use error::ModelError;
pub use id::{Id, IdKind};
pub use media::{
    Asset, AssetKind, AudioInfo, ContentIdentity, Media, MediaInfo, PathHints, VideoInfo,
};
pub use project::{Manifest, TimelineEntry};
pub use store::{DirStore, MemoryStore, ProjectStore, StoreError};
pub use timeline::{
    Align, Clip, ClipContent, Color, TextContent, TextStyle, Timeline, TimelineFormat, Track,
    TrackKind, Transform,
};
pub use validate::{Issue, validate};

/// Current project format version (semantic version, ADR-0004).
pub const FORMAT_VERSION: &str = "0.1.0";

/// JSON Schemas of the stored documents, keyed by file stem, for `cargo xtask schemas`.
pub fn document_schemas() -> Vec<(&'static str, schemars::Schema)> {
    vec![
        ("project", schemars::schema_for!(Manifest)),
        ("media", schemars::schema_for!(Media)),
        ("timeline", schemars::schema_for!(Timeline)),
    ]
}

pub(crate) fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}
