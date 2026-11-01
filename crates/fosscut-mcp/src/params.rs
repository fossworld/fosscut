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
    /// Media files to import: video, audio or images, or web compositions (a folder with an
    /// index.html, or an .html file). All are added in one revision.
    pub files: Vec<String>,
    /// Instead of adding it, the one file given is the new place of this moved asset.
    #[serde(default)]
    pub relink: Option<String>,
    /// For web compositions: width, height, fps and duration, for values the page's
    /// `<meta name="fosscut:composition" content="width=1080, height=1920, fps=30,
    /// duration=6">` tag does not give.
    #[serde(default)]
    pub composition: Option<fosscut_engine::web::CompositionOptions>,
    /// Render a composition of the user's own Remotion project (the one folder in files)
    /// and import the file, as a background job (follow it with job_status). Only after the
    /// user agreed to Remotion's licence: without confirm it fails with the note to tell
    /// them.
    #[serde(default)]
    pub remotion: Option<RemotionArgs>,
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

/// A Remotion render for `media_import` (ADR-0039).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemotionArgs {
    /// The composition id.
    pub composition: String,
    /// Input props for the composition.
    #[serde(default)]
    pub props: Option<Value>,
    /// The entry point relative to the project, such as `src/index.ts`; Remotion's usual
    /// places when omitted.
    #[serde(default)]
    pub entry: Option<String>,
    /// True once the user agreed to run their Remotion under its licence.
    #[serde(default)]
    pub confirm: bool,
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
    /// A preset from `capabilities` (`export_presets`), for example `mp4-h264`,
    /// `mov-prores-hq`, `gif`, `wav` or `mp3`. When omitted the extension decides (`.mov`
    /// ProRes, `.webm` VP9, `.gif`, `.m4a`, `.wav`, `.flac` and `.mp3` audio), and anything
    /// else the most compatible video preset this machine can encode.
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
    /// The shorter side: `480p`, `720p`, `1080p`, `2k`, `4k`, `8k` or a number of pixels;
    /// the canvas's when omitted.
    pub resolution: Option<String>,
    /// Frame rate such as `24`, `25`, `29.97`, `30`, `50`, `59.94`, `60` or `120`; the
    /// timeline's when omitted.
    pub fps: Option<String>,
    /// Video bit rate such as `8M`; the encoder's choice for the size when omitted.
    pub bitrate: Option<String>,
    /// Audio bit rate such as `192k`.
    pub audio_bitrate: Option<String>,
    /// Only this part of the timeline: `{"start": {"seconds": "2"}, "end": {"seconds": "10"}}`.
    pub range: Option<RangeArgs>,
    /// Write a cover image (PNG) of the frame at this time of the export beside the file.
    pub cover: Option<fosscut_time::TimeInput>,
    /// Write the captions beside the file as `srt` or `vtt` subtitles.
    pub subtitles: Option<String>,
}

/// A part of a timeline.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RangeArgs {
    /// Where it starts.
    pub start: fosscut_time::TimeInput,
    /// Where it ends.
    pub end: fosscut_time::TimeInput,
}

/// Arguments of `job_status` and `job_cancel`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct JobArgs {
    /// Job id from `export`, for example `job_0001`.
    pub job: String,
    /// Seconds to wait for the job to finish before answering, 0 to 60 (default 0).
    pub wait_seconds: Option<u64>,
}

/// What the `record` tool does.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordAction {
    /// List the microphones, cameras and screens of this machine.
    Devices,
    /// Start recording as a background job.
    Start,
    /// Make a voiceover from text with a local English voice.
    Speak,
    /// List the local voices.
    Voices,
}

/// Arguments of the `record` tool.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RecordArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// `devices`, `start`, `speak` or `voices`.
    pub action: RecordAction,
    /// speak: the text to say.
    pub text: Option<String>,
    /// speak: the voice, such as `af_heart` (American) or `bm_george` (British); see
    /// action voices.
    pub voice: Option<String>,
    /// speak: speaking speed from 0.5 to 2 (default 1).
    pub speed: Option<f32>,
    /// speak: where to place the speech on the timeline; after the first audio track's last
    /// clip when omitted.
    pub at: Option<TimeInput>,
    /// speak: the audio track to place it on; a free one when omitted.
    pub track: Option<String>,
    /// speak: fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
    /// start: `microphone` (a voiceover), `camera` or `screen`.
    pub what: Option<String>,
    /// start: the device by name, a part of its name, or its number from 1 among its kind
    /// in `devices`; the first of its kind when omitted.
    pub device: Option<String>,
    /// start, for camera and screen: the microphone to record with it, chosen the same way,
    /// or `none`. A camera takes the first microphone and a screen none when omitted.
    pub microphone: Option<String>,
    /// start: stop after this long; until job_cancel when omitted.
    pub duration: Option<TimeInput>,
    /// start: picture size as [width, height]; 1920x1080 for a screen and 1280x720 for a
    /// camera when omitted.
    pub size: Option<[u32; 2]>,
    /// start: frame rate such as 30 or 60; 30 when omitted.
    pub fps: Option<String>,
    /// start: an FFmpeg input as FORMAT:INPUT instead of a device (for example
    /// `lavfi:sine=frequency=440`).
    pub input: Option<String>,
    /// start: `true` only when the user asked you to record. Recording uses their
    /// microphone, camera or screen.
    #[serde(default)]
    pub confirm: bool,
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
    /// Translate a caption track's pages with a local model (English and Hungarian).
    Translate,
    /// Speak each caption page with a local English voice on an audio track.
    Dub,
}

/// Arguments of `captions`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CaptionsArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// `generate` (from transcripts), `import` (an SRT file), `export` (to an SRT file),
    /// `translate` or `dub`.
    pub action: CaptionsAction,
    /// translate: the language to translate into, `hu` or `en`.
    pub to: Option<String>,
    /// translate: `bilingual` (a second line under each page, the default) or `replace`
    /// (the page's words become the translation, spread over the page's time).
    pub mode: Option<String>,
    /// dub: the voice, such as `af_heart`; see the record tool's action voices.
    pub voice: Option<String>,
    /// dub: speaking speed from 0.5 to 2 (default 1).
    pub speed: Option<f32>,
    /// dub: speak each page's translation instead of its words.
    #[serde(default)]
    pub translation: bool,
    /// dub: the audio track for the speech; a new one when omitted.
    pub audio_track: Option<String>,
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
    /// generate: leave out the words the transcript marks as fillers ("um", "uh").
    #[serde(default)]
    pub skip_fillers: bool,
    /// generate: caption a song's lyrics: the music track first, one sung line per page,
    /// the karaoke look unless a preset is given.
    #[serde(default)]
    pub lyrics: bool,
    /// export: `original` (the default), `translation` (each page's translation line) or
    /// `both` (bilingual: the words, then the translation).
    pub subtitles: Option<String>,
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

/// What the `library` tool does.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LibraryAction {
    /// List or search the library.
    List,
    /// Add an item to the project.
    Add,
}

/// Arguments of the `library` tool.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct LibraryArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// `list` (with `kind` and `search`) or `add` (with `name`).
    pub action: LibraryAction,
    /// add: the item's name.
    pub name: Option<String>,
    /// Only, or the item's, kind: shape, sticker, background, template, title,
    /// text_style, caption_style, look, particles, model (3D objects), device, drawing,
    /// composition, sound or music.
    pub kind: Option<String>,
    /// list: words to find in names, descriptions and tags.
    pub search: Option<String>,
    /// add: where it starts.
    pub start: Option<fosscut_time::TimeInput>,
    /// add: how long it lasts.
    pub duration: Option<fosscut_time::TimeInput>,
    /// add: the track.
    pub track: Option<String>,
    /// add: the timeline, such as a scene's compound timeline; the primary timeline when
    /// omitted.
    pub timeline: Option<String>,
    /// add: the clip a look applies to.
    pub clip: Option<String>,
    /// add: lines for titles, templates and compositions; the text of a text style's clip.
    #[serde(default)]
    pub lines: Vec<String>,
    /// add: assets for a template's slots.
    #[serde(default)]
    pub media: Vec<String>,
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

/// Arguments of the `audio_normalize` tool.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct NormalizeArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// The clips with sound to level (audio, video or compound clips).
    pub clips: Vec<String>,
    /// Target loudness in LUFS; -14 (short-form platforms) when omitted; podcasts use -16.
    pub target_lufs: Option<f64>,
    /// Highest sample peak in dBFS, from -12 to 0; -1 when omitted.
    pub ceiling_dbfs: Option<f64>,
    /// Add or adjust an `audio.limiter` where the peaks leave too little room for the
    /// volume alone (the default); false only changes volumes.
    pub limit: Option<bool>,
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

/// What the `auto_edit` tool does.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoEditAction {
    /// Split a video clip at its scene changes.
    SplitScenes,
    /// Steady a shaky video clip.
    Stabilize,
    /// Make a mask or another clip follow a region of a video clip's picture.
    Track,
    /// Make a multicam clip from angles recorded at the same time, lined up by their sound.
    Multicam,
    /// Only measure how far apart angles recorded at the same time started.
    MulticamSync,
    /// Make a clip follow its subject on its timeline's canvas (another aspect).
    Reframe,
    /// Keep the camera on a moving subject, zoomed in on the clip's own canvas.
    CameraTrack,
    /// Copy a timeline onto a vertical canvas with every video clip following its subject.
    Vertical,
    /// Put markers on the beats of a music clip.
    BeatMarkers,
    /// Move the cuts between a track's clips onto the beats of a music clip.
    BeatCuts,
    /// Score segments of a long clip that could stand alone as shorts.
    ShortCandidates,
    /// Build a vertical short from a range of a clip's media.
    MakeShort,
}

/// A region of a picture: its centre and size as fractions of the picture.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
pub struct RegionArg {
    /// The centre's distance from the left edge, 0 to 1.
    pub x: f64,
    /// The centre's distance from the top edge, 0 to 1.
    pub y: f64,
    /// The width, 0 to 1.
    pub width: f64,
    /// The height, 0 to 1.
    pub height: f64,
}

/// Arguments of the `auto_edit` tool.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct AutoEditArgs {
    /// Path of a `.fosscut` bundle; the open project when omitted.
    pub project: Option<String>,
    /// `split_scenes`, `stabilize`, `track`, `multicam`, `multicam_sync`, `reframe`,
    /// `camera_track`, `vertical`, `beat_markers`, `beat_cuts`, `short_candidates` or
    /// `make_short`.
    pub action: AutoEditAction,
    /// split_scenes, stabilize, track, reframe, camera_track, short_candidates, make_short:
    /// the video clip to measure.
    pub clip: Option<String>,
    /// stabilize: how much motion to remove, 0 (none) to 1 (very smooth), up to 3 for a
    /// locked-off look; 0.5 when omitted.
    pub strength: Option<f64>,
    /// track: the region to follow at `at`, in the picture as it shows (find it with
    /// frame_grab).
    pub region: Option<RegionArg>,
    /// track: the timeline time where `region` is given; the clip's start when omitted.
    pub at: Option<TimeInput>,
    /// track: a mask of the clip whose centre follows the region (mask tracking).
    pub mask: Option<String>,
    /// track: another clip, such as a title or sticker, that moves as the region does from
    /// where it is (motion tracking).
    pub follow: Option<String>,
    /// reframe, camera_track, vertical, make_short: what the picture follows: `person`
    /// (the cached pose analysis, the default) or `any` (whatever the cached matte keeps);
    /// make_short also takes `centre` (no analysis needed). reframe and camera_track
    /// follow `region` from `at` instead when it is given.
    pub subject: Option<String>,
    /// reframe, vertical, make_short: frame each shot on its own at the cached scene cuts
    /// (default true; run media_analyze with analyzers ["scenes"] first), or false for one
    /// framing over the whole clip.
    pub shots: Option<bool>,
    /// reframe, camera_track, vertical, make_short: how the crop follows (dead zone, speed,
    /// ease, headroom, padding, zoom limits); the defaults pan smoothly at zoom 1.
    pub framing: Option<fosscut_engine::reframe::FramingOptions>,
    /// camera_track: how far to zoom in, 1 to 4 (default 1.5).
    pub zoom: Option<f64>,
    /// beat_markers, beat_cuts: the music's tempo when known, in beats per minute, instead of
    /// estimating it.
    pub bpm: Option<f64>,
    /// beat_cuts: how far a cut may move; half the spacing of the beats that count when
    /// omitted.
    pub tolerance: Option<TimeInput>,
    /// beat_markers, beat_cuts: the music clip.
    pub music: Option<String>,
    /// beat_markers, beat_cuts: every how many beats (1 to 16; default 1).
    pub every: Option<u32>,
    /// short_candidates: shortest and longest candidate, in seconds (15 and 60), and how
    /// many to return (10).
    pub min_seconds: Option<f64>,
    /// short_candidates: see min_seconds.
    pub max_seconds: Option<f64>,
    /// short_candidates: see min_seconds.
    pub count: Option<u32>,
    /// make_short: where the short starts and ends in the clip's media (a candidate's span).
    pub from: Option<TimeInput>,
    /// make_short: see from.
    pub to: Option<TimeInput>,
    /// make_short: caption the spoken words (default true).
    pub captions: Option<bool>,
    /// make_short: the caption preset, such as `bold-pop`.
    pub caption_preset: Option<String>,
    /// make_short: a title over the first seconds.
    pub title: Option<String>,
    /// multicam, multicam_sync: the angles' video asset ids, at least two, in order; the
    /// first shows at the start.
    #[serde(default)]
    pub angles: Vec<String>,
    /// multicam: each angle's start in the scene, in order; measured from their sound when
    /// omitted, unless sync is false.
    #[serde(default)]
    pub offsets: Vec<TimeInput>,
    /// multicam: false lines the angles up at their starts instead of measuring offsets.
    pub sync: Option<bool>,
    /// multicam, multicam_sync: look at most this many seconds either way; any offset when
    /// omitted.
    pub max_offset_seconds: Option<f64>,
    /// multicam: which angle's sound plays, by number from 1; the first when omitted.
    pub sound: Option<usize>,
    /// multicam, vertical, make_short: the new timeline's name.
    pub name: Option<String>,
    /// multicam: where the multicam clip starts; appended when omitted.
    pub start: Option<TimeInput>,
    /// multicam: track for the multicam clip; beat_cuts: the video track whose cuts move.
    pub track: Option<String>,
    /// multicam: timeline for the multicam clip; vertical: the timeline to copy; the primary
    /// timeline when omitted.
    pub timeline: Option<String>,
    /// vertical, make_short: the canvas size in pixels; 1080 by 1920 when omitted.
    pub width: Option<u32>,
    /// vertical, make_short: see width.
    pub height: Option<u32>,
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
