use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Id, TextPreset};

/// The project manifest (`project.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    /// Text styles saved for reuse, sorted by name (ADR-0027).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_presets: Vec<TextPreset>,
}

/// A timeline listed in the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TimelineEntry {
    /// Timeline id; its document is `timelines/<id>.json`.
    pub id: Id,
    /// Timeline name.
    pub name: String,
    /// What the timeline is for: an edit of its own, or the inside of compound clips.
    #[serde(default, skip_serializing_if = "is_sequence")]
    pub role: TimelineRole,
}

/// What a timeline is for (ADR-0025).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimelineRole {
    /// An edit of its own.
    #[default]
    Sequence,
    /// The inside of compound clips, which show it.
    Compound,
}

fn is_sequence(role: &TimelineRole) -> bool {
    *role == TimelineRole::Sequence
}
