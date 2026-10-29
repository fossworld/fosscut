//! Tool arguments. Their doc comments become the JSON Schema descriptions agents read.

use fosscut_core::CommandCall;
use fosscut_time::TimeInput;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// How much `capabilities` and `timeline_read` include.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    /// Names and summaries only.
    #[default]
    Outline,
    /// Everything except schemas.
    Normal,
    /// Everything, including JSON Schemas.
    Full,
}

impl Detail {
    /// The query argument value.
    pub fn as_str(self) -> &'static str {
        match self {
            Detail::Outline => "outline",
            Detail::Normal => "normal",
            Detail::Full => "full",
        }
    }
}

/// Arguments of `capabilities`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CapabilitiesArgs {
    /// `outline` (default) lists names and summaries; `full` adds every command's JSON
    /// Schema, the request and time schemas, and the error codes.
    #[serde(default)]
    pub detail: Detail,
}

/// Arguments of `project_new`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectNewArgs {
    /// Bundle directory to create, for example `Promo.fosscut`. It must not exist or must
    /// be empty.
    pub path: String,
    /// Project name; the directory name when omitted.
    pub name: Option<String>,
    /// Timeline width in pixels (default 1920; 1080 for vertical video).
    pub width: Option<u32>,
    /// Timeline height in pixels (default 1080; 1920 for vertical video).
    pub height: Option<u32>,
    /// Frame rate, for example `30`, `25` or `30000/1001` (default 30).
    pub rate: Option<String>,
}

/// Arguments of `project_open`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectOpenArgs {
    /// Path of an existing `.fosscut` bundle.
    pub path: String,
}

/// Arguments of tools that only read a project.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ProjectArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
}

/// Arguments of `timeline_read`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TimelineReadArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// Timeline id; the primary timeline when omitted.
    pub timeline: Option<String>,
    /// `outline` counts clips per track, `normal` (default) lists every clip, `full` adds
    /// styles and transforms.
    pub detail: Option<Detail>,
}

/// Arguments of `media_import`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MediaImportArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// Media files to import: video, audio or images. All are added in one revision.
    pub files: Vec<String>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
    /// Report what would be added without changing the project.
    #[serde(default)]
    pub dry_run: bool,
    /// Retry key: repeating it with the same files returns the first result.
    pub request_id: Option<String>,
    /// Why, recorded in the project's history.
    pub reason: Option<String>,
}

/// Arguments of `edit`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// Commands applied in order as one transaction and one undo step, for example
    /// `[{"command": "clip.split", "args": {"clip": "clip_0003", "at": {"timecode": "00:00:03:28"}}}]`.
    /// `capabilities` with `detail: full` gives each command's argument schema.
    pub commands: Vec<CommandCall>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision; pass
    /// the revision from your last read.
    pub expected_revision: Option<u64>,
    /// Validate and report the changes without committing them.
    #[serde(default)]
    pub dry_run: bool,
    /// Retry key: repeating it with the same commands returns the first result instead of
    /// applying them twice.
    pub request_id: Option<String>,
    /// Allow destructive changes such as removing clips from the main video track. Ask the
    /// user first.
    #[serde(default)]
    pub allow_destructive: bool,
    /// Why, recorded in the project's history.
    pub reason: Option<String>,
}

/// Arguments of `undo` and `redo`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct HistoryStepArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
    /// Why, recorded in the project's history.
    pub reason: Option<String>,
}

/// Arguments of `history`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct HistoryArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// Most recent entries to list (default 20).
    pub limit: Option<usize>,
}

/// Arguments of `query`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// Query name from `capabilities`, for example `history.list`.
    pub name: String,
    /// Query arguments matching its schema.
    pub args: Option<Value>,
}

/// Arguments of `frame_grab`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FrameArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// The moment to show, with a unit: `{"seconds": "1.5"}`, `{"frames": 45}`,
    /// `{"timecode": "00:00:01:15"}` or `{"flicks": n}`.
    pub time: TimeInput,
    /// Timeline id; the primary timeline when omitted.
    pub timeline: Option<String>,
    /// Image width in pixels (default 640); the height keeps the aspect ratio.
    pub width: Option<u32>,
}

/// Arguments of `contact_sheet`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SheetArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// Frames to show, 1 to 100 (default 12), from the middle of equal parts of the
    /// timeline.
    pub count: Option<u32>,
    /// Width of each frame in pixels (default 240).
    pub width: Option<u32>,
    /// Timeline id; the primary timeline when omitted.
    pub timeline: Option<String>,
}

/// Arguments of `export`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// File to write, for example `Promo.mp4`.
    pub output: String,
    /// `mp4-h264`, `mp4-hevc`, `mp4-av1`, `mov-prores`, `webm-vp9` or `m4a-aac`. When
    /// omitted the extension decides: `.mov` ProRes, `.webm` VP9, `.m4a` audio only, and
    /// anything else the most compatible video preset this machine can encode.
    pub preset: Option<String>,
    /// Timeline id; the primary timeline when omitted.
    pub timeline: Option<String>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision, so the
    /// export matches the edit you checked.
    pub expected_revision: Option<u64>,
    /// Check everything and report frames, duration and encoders without writing.
    #[serde(default)]
    pub dry_run: bool,
    /// Retry key: repeating it returns the running or finished export.
    pub request_id: Option<String>,
    /// Replace the output if it exists. Ask the user first.
    #[serde(default)]
    pub allow_destructive: bool,
}

/// Arguments of `job_status` and `job_cancel`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct JobArgs {
    /// Job id from `export`, for example `job_0001`.
    pub job: String,
    /// Seconds to wait for the job to finish before answering, 0 to 60 (default 0).
    pub wait_seconds: Option<u64>,
}
