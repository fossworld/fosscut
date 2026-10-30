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
    /// Schema, the request and time schemas, and the error codes (large).
    #[serde(default)]
    pub detail: Detail,
    /// Only these commands and queries, each with its argument JSON Schema, for example
    /// `["clip.add", "text.add"]`. Ask for the commands you are about to use.
    #[serde(default)]
    pub names: Vec<String>,
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

/// Arguments of `media_analyze`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MediaAnalyzeArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// The asset to analyze, for example `asset_0001` (from `media_list`).
    pub asset: String,
    /// `loudness`, `silences`, `scenes` and/or `transcript`. When omitted, every analyzer the
    /// asset allows except `transcript`, which needs an installed speech model (see `models`).
    #[serde(default)]
    pub analyzers: Vec<String>,
    /// Analyzer options, for example `{"silences": {"threshold_dbfs": -35, "min_duration_ms":
    /// 400}}` or `{"transcript": {"language": "en", "min_pause_ms": 250}}`; defaults when
    /// omitted.
    #[serde(default)]
    pub options: fosscut_engine::analysis::Options,
    /// Measure again even if results are cached.
    #[serde(default)]
    pub refresh: bool,
    /// Seconds to wait for a measurement to finish before answering, 0 to 60 (default 30).
    pub wait_seconds: Option<u64>,
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

/// Arguments of `models`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ModelsArgs {
    /// The model to install, for example `whisper-base.en`; omit to list the models.
    pub install: Option<String>,
    /// `true` only after the user agreed to the download size and license.
    #[serde(default)]
    pub confirm: bool,
    /// Install from this local file instead of downloading; it is verified the same way.
    pub from: Option<String>,
}

/// What the `captions` tool does.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CaptionsAction {
    /// Caption the clips of a track from their cached transcripts.
    Generate,
    /// Add caption pages from an SRT file, one page per cue.
    Import,
    /// Write a caption track to an SRT file.
    Export,
}

/// Arguments of `captions`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CaptionsArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// `generate` (from transcripts), `import` (an SRT file) or `export` (to an SRT file).
    pub action: CaptionsAction,
    /// The SRT file to import or to write.
    pub file: Option<String>,
    /// Timeline; the primary timeline when omitted.
    pub timeline: Option<String>,
    /// Caption track; the first caption track (created when adding) when omitted.
    pub track: Option<String>,
    /// generate: the track whose clips are captioned; the first video or audio track with
    /// sound when omitted.
    pub source_track: Option<String>,
    /// A built-in look: `bold-pop`, `highlight`, `karaoke` or `boxed` (capabilities lists
    /// them with descriptions).
    pub preset: Option<String>,
    /// Changes to the caption track's look as a JSON merge patch, for example
    /// `{"placement": "middle", "size": 0.07}`.
    pub style: Option<serde_json::Value>,
    /// generate: paging limits, for example `{"max_words": 3, "max_lines": 1}`.
    pub paging: Option<serde_json::Value>,
    /// generate: the transcript options the media was analyzed with (`model`, `language`,
    /// `min_pause_ms`), if not the defaults.
    #[serde(default)]
    pub transcript: fosscut_engine::analysis::TranscriptOptions,
    /// Replace the caption track's existing captions; also needs allow_destructive.
    #[serde(default)]
    pub replace: bool,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
    /// Report what would change without committing it (or, for export, without writing).
    #[serde(default)]
    pub dry_run: bool,
    /// Retry key for generate and import.
    pub request_id: Option<String>,
    /// Allow replacing captions or overwriting an existing file. Ask the user first.
    #[serde(default)]
    pub allow_destructive: bool,
    /// Why, recorded in the project's history.
    pub reason: Option<String>,
}

/// What the `color` tool does.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColorAction {
    /// Correct white balance, brightness and levels.
    Auto,
    /// Bring the clip close to a reference clip.
    Match,
}

/// Arguments of the `color` tool.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct ColorArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// `auto` or `match`.
    pub action: ColorAction,
    /// The video or image clip to correct.
    pub clip: String,
    /// match: the clip whose look to match.
    pub reference: Option<String>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
    /// Report what would change without committing it.
    #[serde(default)]
    pub dry_run: bool,
    /// Retry key.
    pub request_id: Option<String>,
    /// Why, recorded in the project's history.
    pub reason: Option<String>,
}
