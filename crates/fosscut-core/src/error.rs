use std::fmt;

use fosscut_model::ModelError;
use fosscut_time::TimeError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An error returned by a command or query (ADR-0006).
///
/// `code` is stable and part of the public API; `message` is for people; `hint` tells an
/// agent how to recover; `details` carries structured context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApiError {
    /// Stable error code, for example `fosscut.clip.overlap`.
    pub code: String,
    /// Human-readable description.
    pub message: String,
    /// How to recover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Structured context such as conflicting ids or the current revision.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub details: serde_json::Value,
}

impl ApiError {
    /// An error with `code` and `message`; the catalogue hint is filled in when known.
    pub fn new(code: &str, message: impl Into<String>) -> ApiError {
        let hint = error_catalogue()
            .into_iter()
            .find(|e| e.code == code)
            .map(|e| e.hint.to_string());
        ApiError {
            code: code.to_string(),
            message: message.into(),
            hint,
            details: serde_json::Value::Null,
        }
    }

    /// Replaces the hint.
    pub fn with_hint(mut self, hint: impl Into<String>) -> ApiError {
        self.hint = Some(hint.into());
        self
    }

    /// Attaches structured details.
    pub fn with_details(mut self, details: serde_json::Value) -> ApiError {
        self.details = details;
        self
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ApiError {}

impl From<TimeError> for ApiError {
    fn from(error: TimeError) -> ApiError {
        ApiError::new(error.code(), error.to_string())
    }
}

impl From<fosscut_model::StoreError> for ApiError {
    fn from(error: fosscut_model::StoreError) -> ApiError {
        ApiError::from(ModelError::Store(error))
    }
}

impl From<ModelError> for ApiError {
    fn from(error: ModelError) -> ApiError {
        let details = match &error {
            ModelError::Invalid(issues) => serde_json::to_value(issues).unwrap_or_default(),
            _ => serde_json::Value::Null,
        };
        ApiError::new(error.code(), error.to_string()).with_details(details)
    }
}

/// Stable error codes (ADR-0006). The catalogue is published through `capabilities`.
pub mod codes {
    /// The request envelope is malformed.
    pub const REQUEST_INVALID: &str = "fosscut.request.invalid";
    /// A command's arguments do not match its schema.
    pub const INVALID_ARGUMENTS: &str = "fosscut.request.invalid_arguments";
    /// A `request_id` was reused with different commands.
    pub const REQUEST_REUSED: &str = "fosscut.request.reused";
    /// No command with this name exists.
    pub const UNKNOWN_COMMAND: &str = "fosscut.command.unknown";
    /// No query with this name exists.
    pub const UNKNOWN_QUERY: &str = "fosscut.query.unknown";
    /// `expected_revision` does not match the project.
    pub const REVISION_CONFLICT: &str = "fosscut.revision.conflict";
    /// A destructive operation was requested without `allow_destructive`.
    pub const GUARD_DESTRUCTIVE: &str = "fosscut.guard.destructive";
    /// There is no transaction to undo.
    pub const NOTHING_TO_UNDO: &str = "fosscut.history.nothing_to_undo";
    /// There is no transaction to redo.
    pub const NOTHING_TO_REDO: &str = "fosscut.history.nothing_to_redo";
    /// The project changed outside the operation log, so history cannot be applied.
    pub const HISTORY_DIVERGED: &str = "fosscut.history.diverged";
    /// The operation log is unreadable.
    pub const HISTORY_CORRUPT: &str = "fosscut.history.corrupt";
    /// The timeline does not exist.
    pub const TIMELINE_NOT_FOUND: &str = "fosscut.timeline.not_found";
    /// The track does not exist.
    pub const TRACK_NOT_FOUND: &str = "fosscut.track.not_found";
    /// The track is locked.
    pub const TRACK_LOCKED: &str = "fosscut.track.locked";
    /// The clip does not fit the kind of the target track.
    pub const TRACK_KIND_MISMATCH: &str = "fosscut.track.kind_mismatch";
    /// The clip does not exist.
    pub const CLIP_NOT_FOUND: &str = "fosscut.clip.not_found";
    /// The clip would overlap another clip.
    pub const CLIP_OVERLAP: &str = "fosscut.clip.overlap";
    /// The clip would read outside its asset.
    pub const CLIP_SOURCE_RANGE: &str = "fosscut.clip.source_range";
    /// A split point is not strictly inside the clip.
    pub const CLIP_SPLIT_OUTSIDE: &str = "fosscut.clip.split_outside";
    /// A clip would be shorter than one frame or start before zero.
    pub const CLIP_INVALID_RANGE: &str = "fosscut.clip.invalid_range";
    /// The property has keyframes, so its plain value cannot be set.
    pub const CLIP_PROPERTY_ANIMATED: &str = "fosscut.clip.property_animated";
    /// No keyframe of the property is at that time.
    pub const KEYFRAME_NOT_FOUND: &str = "fosscut.keyframe.not_found";
    /// A keyframe time lies outside its clip.
    pub const KEYFRAME_OUTSIDE: &str = "fosscut.keyframe.outside";
    /// No effect type has this name.
    pub const EFFECT_UNKNOWN_TYPE: &str = "fosscut.effect.unknown_type";
    /// The clip has no effect with this id.
    pub const EFFECT_NOT_FOUND: &str = "fosscut.effect.not_found";
    /// No built-in look has this name.
    pub const LOOK_UNKNOWN: &str = "fosscut.look.unknown";
    /// The clip has no mask with this id.
    pub const MASK_NOT_FOUND: &str = "fosscut.mask.not_found";
    /// No timeline has a marker with this id.
    pub const MARKER_NOT_FOUND: &str = "fosscut.marker.not_found";
    /// No transition type has this name.
    pub const TRANSITION_UNKNOWN_TYPE: &str = "fosscut.transition.unknown_type";
    /// No clip starts where the clip ends, so there is no cut to transition over.
    pub const TRANSITION_NO_NEIGHBOR: &str = "fosscut.transition.no_neighbor";
    /// The asset does not exist.
    pub const ASSET_NOT_FOUND: &str = "fosscut.asset.not_found";
    /// Clips show the asset, so it cannot be removed alone.
    pub const ASSET_IN_USE: &str = "fosscut.asset.in_use";
    /// The file is not the asset's media.
    pub const ASSET_IDENTITY_MISMATCH: &str = "fosscut.asset.identity_mismatch";
    /// Media information is missing or inconsistent with the asset kind.
    pub const MEDIA_INCOMPLETE: &str = "fosscut.media.incomplete_info";
    /// Text is empty.
    pub const TEXT_EMPTY: &str = "fosscut.text.empty";
    /// Caption words are empty, out of order, overlapping or outside their page.
    pub const CAPTIONS_INVALID_WORDS: &str = "fosscut.captions.invalid_words";
    /// No caption preset has this name.
    pub const CAPTIONS_UNKNOWN_PRESET: &str = "fosscut.captions.unknown_preset";
    /// No media prober (ffprobe) is available to the front end.
    pub const PROBE_UNAVAILABLE: &str = "fosscut.media.probe_unavailable";
    /// The media file could not be probed.
    pub const PROBE_FAILED: &str = "fosscut.media.probe_failed";
    /// The project is being written by another process.
    pub const PROJECT_BUSY: &str = "fosscut.project.busy";
    /// The result would violate a project format rule.
    pub const MODEL_INVALID: &str = "fosscut.model.invalid";
    /// Internal failure; please report it.
    pub const INTERNAL: &str = "fosscut.internal";
}

/// A documented error code.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ErrorCode {
    /// The code.
    pub code: &'static str,
    /// What it means.
    pub meaning: &'static str,
    /// How to recover.
    pub hint: &'static str,
}

/// Every error code the core can return, with recovery hints.
pub fn error_catalogue() -> Vec<ErrorCode> {
    use codes::*;
    let entry = |code, meaning, hint| ErrorCode {
        code,
        meaning,
        hint,
    };
    vec![
        entry(
            REQUEST_INVALID,
            "The request envelope is malformed.",
            "Send {\"commands\": [{\"command\": name, \"args\": {...}}]} or a single {\"command\", \"args\"} object.",
        ),
        entry(
            INVALID_ARGUMENTS,
            "Arguments do not match the command schema.",
            "Read the command's input schema with capabilities (names: [the command]) and correct the field the message names.",
        ),
        entry(
            REQUEST_REUSED,
            "The request_id was already used for different commands.",
            "Use a new request_id for a new request; reuse an id only to retry the identical request.",
        ),
        entry(
            UNKNOWN_COMMAND,
            "No command has this name.",
            "List commands with capabilities.",
        ),
        entry(
            UNKNOWN_QUERY,
            "No query has this name.",
            "List queries with capabilities.",
        ),
        entry(
            REVISION_CONFLICT,
            "The project changed since expected_revision.",
            "Read the project again, re-plan the edit against the current revision, then retry with the new revision.",
        ),
        entry(
            GUARD_DESTRUCTIVE,
            "The operation would remove primary content.",
            "Ask the user to confirm, then retry with allow_destructive: true.",
        ),
        entry(
            NOTHING_TO_UNDO,
            "There is no committed transaction to undo.",
            "Check the history query.",
        ),
        entry(
            NOTHING_TO_REDO,
            "There is no undone transaction to redo.",
            "A new commit clears the redo history.",
        ),
        entry(
            HISTORY_DIVERGED,
            "Documents differ from what the operation log expects.",
            "Do not undo further; inspect the project and report the problem.",
        ),
        entry(
            HISTORY_CORRUPT,
            "The operation log cannot be read.",
            "Restore history/ops.jsonl from version control or a backup.",
        ),
        entry(
            TIMELINE_NOT_FOUND,
            "The timeline does not exist.",
            "List timelines with project.info.",
        ),
        entry(
            TRACK_NOT_FOUND,
            "The track does not exist.",
            "Read the timeline to get current track ids.",
        ),
        entry(
            TRACK_LOCKED,
            "The track is locked.",
            "Ask the user before unlocking it.",
        ),
        entry(
            TRACK_KIND_MISMATCH,
            "The clip kind does not fit the track kind.",
            "Use a video track for video and images, an audio track for audio and a text track for text.",
        ),
        entry(
            CLIP_NOT_FOUND,
            "The clip does not exist.",
            "Read the timeline to get current clip ids.",
        ),
        entry(
            CLIP_OVERLAP,
            "The clip would overlap another clip on the track.",
            "Choose a free position (details.free_from), another track, or remove the other clip first.",
        ),
        entry(
            CLIP_SOURCE_RANGE,
            "The clip would read beyond the start or end of its media.",
            "Shorten the clip or move its source in-point; details give the asset duration.",
        ),
        entry(
            CLIP_SPLIT_OUTSIDE,
            "The split point is not strictly inside the clip.",
            "Choose a time between the clip's start and end (exclusive).",
        ),
        entry(
            CLIP_INVALID_RANGE,
            "The clip would be empty or start before zero.",
            "Use a positive duration of at least one frame and a start of zero or later.",
        ),
        entry(
            CLIP_PROPERTY_ANIMATED,
            "The property has keyframes, so a plain value would be ignored.",
            "Set the value at a time with keyframe.set, or remove its keyframes with keyframe.remove (all: true) first.",
        ),
        entry(
            KEYFRAME_NOT_FOUND,
            "The property has no keyframe at that time.",
            "Read the clip's keyframes with timeline.read (detail full) and use one of their times.",
        ),
        entry(
            KEYFRAME_OUTSIDE,
            "The keyframe time is outside the clip.",
            "Use a time from the clip's start to its end.",
        ),
        entry(
            EFFECT_UNKNOWN_TYPE,
            "No effect type has this name.",
            "List the effect types and their parameters with capabilities (under effects).",
        ),
        entry(
            EFFECT_NOT_FOUND,
            "The clip has no effect with this id.",
            "Read the clip's effects with timeline.read; effect ids look like effect_0007.",
        ),
        entry(
            TRANSITION_UNKNOWN_TYPE,
            "No transition type has this name.",
            "List the transitions with capabilities (under transitions).",
        ),
        entry(
            TRANSITION_NO_NEIGHBOR,
            "No clip starts where the clip ends, so there is no cut to transition over.",
            "Move the next clip on the track so it starts where this clip ends, then add the transition.",
        ),
        entry(
            MASK_NOT_FOUND,
            "The clip has no mask with this id.",
            "Read the clip's masks with timeline.read; mask ids look like mask_0007.",
        ),
        entry(
            MARKER_NOT_FOUND,
            "No timeline has a marker with this id.",
            "Read the markers with timeline.read; marker ids look like marker_0007.",
        ),
        entry(
            LOOK_UNKNOWN,
            "No built-in look has this name.",
            "List the looks with capabilities (under looks).",
        ),
        entry(
            ASSET_NOT_FOUND,
            "The asset does not exist.",
            "List assets with media.list.",
        ),
        entry(
            ASSET_IN_USE,
            "Clips show the asset, so it cannot be removed alone.",
            "Remove the clips in the error's details first, or pass remove_clips: true with allow_destructive.",
        ),
        entry(
            ASSET_IDENTITY_MISMATCH,
            "The file is not the asset's media: its size or content differs.",
            "Find the original file, or import the new one and swap it in with media.replace.",
        ),
        entry(
            MEDIA_INCOMPLETE,
            "Media information is missing for this kind of asset.",
            "Import the file with a front end that probes it (fosscut media import).",
        ),
        entry(TEXT_EMPTY, "Text is empty.", "Provide non-empty text."),
        entry(
            CAPTIONS_INVALID_WORDS,
            "Caption words are empty, out of order, overlapping or outside their page.",
            "Give words in order with start before end and no overlaps; details name the word.",
        ),
        entry(
            CAPTIONS_UNKNOWN_PRESET,
            "No caption preset has this name.",
            "capabilities lists the caption presets.",
        ),
        entry(
            PROBE_UNAVAILABLE,
            "No media prober is available.",
            "Install FFmpeg so that ffprobe is on PATH, or set FOSSCUT_FFPROBE to its location.",
        ),
        entry(
            PROBE_FAILED,
            "The media file could not be read.",
            "Check that the path exists and is a supported audio, video or image file.",
        ),
        entry(
            PROJECT_BUSY,
            "Another process is writing the project.",
            "Wait for the other command to finish, then retry with the same request_id.",
        ),
        entry(
            MODEL_INVALID,
            "The change would break a project format rule.",
            "Read details for the violated rule; this usually indicates a bug in the command.",
        ),
        entry(
            "fosscut.time.unit_required",
            "A time has no explicit unit.",
            "Use {\"frames\": n}, {\"seconds\": \"s\"}, {\"timecode\": \"HH:MM:SS:FF\"} or {\"flicks\": n}.",
        ),
        entry(
            "fosscut.time.invalid_timecode",
            "A timecode is malformed for the timeline rate.",
            "Use HH:MM:SS:FF with a frame field below the frame rate.",
        ),
        entry(
            "fosscut.time.invalid_seconds",
            "A seconds value is malformed.",
            "Use a decimal number such as \"3.9333\".",
        ),
        entry(
            "fosscut.time.invalid_rate",
            "A frame rate is malformed.",
            "Use an integer or a fraction such as \"30000/1001\".",
        ),
        entry(
            "fosscut.time.overflow",
            "A time value is out of range.",
            "Use a smaller value.",
        ),
        entry(
            "fosscut.project.not_found",
            "No project exists at this location.",
            "Create one with fosscut project new.",
        ),
        entry(
            "fosscut.project.unsupported_version",
            "The project format version is not supported.",
            "Update FossCut.",
        ),
        entry(
            "fosscut.project.malformed",
            "A project document is not valid JSON for its schema.",
            "Restore the document from version control or a backup.",
        ),
        entry(
            "fosscut.project.invalid",
            "The project violates format rules.",
            "Read details for the list of issues.",
        ),
        entry(
            "fosscut.project.storage",
            "Reading or writing project files failed.",
            "Check permissions and free disk space.",
        ),
        entry(
            INTERNAL,
            "Internal failure.",
            "Report this as a bug with the request that caused it.",
        ),
    ]
}
