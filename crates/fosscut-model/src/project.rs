use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Id;

/// The project manifest (`project.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    /// Format version of the bundle (semantic version).
    pub format_version: String,
    /// Project id (`project_…`).
    pub id: Id,
    /// Project name.
    pub name: String,
    /// Increments on every committed change (ADR-0006).
    pub revision: u64,
    /// Next number for new object ids; ids are never reused.
    pub next_id: u64,
    /// The timeline that opens by default and that exports target unless told otherwise.
    pub primary_timeline: Id,
    /// Timelines in the project, in display order.
    pub timelines: Vec<TimelineEntry>,
}

/// A timeline listed in the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TimelineEntry {
    /// Timeline id; its document is `timelines/<id>.json`.
    pub id: Id,
    /// Timeline name.
    pub name: String,
}
