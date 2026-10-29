//! Command-line arguments.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use fosscut_time::TimeInput;

const AFTER_HELP: &str = "\
Times need an explicit unit: 118f (frames), 3.9333s (seconds), 00:00:03:28 (timecode) or 2775360000fl (flicks).

With --json, results and errors are printed as JSON on stdout; errors exit with status 1 and look like
{\"error\": {\"code\": \"fosscut.clip.overlap\", \"message\": ..., \"hint\": ..., \"details\": ...}}.

Agents: pass --expected-revision on every write, use --dry-run before large changes and reuse --request-id when retrying.";

/// FossCut: an open source video editor you can drive from the command line and from AI agents.
#[derive(Debug, Parser)]
#[command(name = "fosscut", version, after_help = AFTER_HELP)]
pub struct Cli {
    /// Project bundle (a `.fosscut` directory).
    #[arg(short, long, global = true, env = "FOSSCUT_PROJECT")]
    pub project: Option<PathBuf>,
    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(flatten)]
    pub write: WriteOptions,
    #[command(subcommand)]
    pub command: Command,
}

/// Envelope options for commands that change the project (ADR-0006).
#[derive(Debug, Args)]
pub struct WriteOptions {
    /// Fail unless the project is at this revision.
    #[arg(long, global = true)]
    pub expected_revision: Option<u64>,
    /// Validate and show the changes without saving them.
    #[arg(long, global = true)]
    pub dry_run: bool,
    /// Retry key: repeating a request with the same id returns the first result.
    #[arg(long, global = true)]
    pub request_id: Option<String>,
    /// Allow destructive edits such as removing clips from the primary video track.
    #[arg(long, global = true)]
    pub allow_destructive: bool,
    /// Why the change is made; recorded in the operation log.
    #[arg(long, global = true)]
    pub reason: Option<String>,
    /// Who makes the change, for example `agent:claude-code`.
    #[arg(long, global = true, env = "FOSSCUT_ACTOR", default_value = "cli")]
    pub actor: String,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create and inspect projects.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// List commands, queries, schemas and error codes.
    Capabilities {
        /// `outline` for names only; `full` for schemas.
        #[arg(long, value_enum, default_value_t = Detail::Outline)]
        detail: Detail,
    },
    /// Import and list media.
    #[command(subcommand)]
    Media(MediaCommand),
    /// Add tracks.
    #[command(subcommand)]
    Track(TrackCommand),
    /// Place, split, move and remove clips.
    #[command(subcommand)]
    Clip(ClipCommand),
    /// Add titles and captions.
    #[command(subcommand)]
    Text(TextCommand),
    /// Read timelines.
    #[command(subcommand)]
    Timeline(TimelineCommand),
    /// Show the operation log, newest first.
    History {
        /// Number of entries.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Undo the most recent change.
    Undo,
    /// Redo the most recently undone change.
    Redo,
    /// Execute a raw request: JSON text, @file, or - for stdin.
    Exec {
        /// `{"command": ..., "args": {...}}` or `{"commands": [...], "expected_revision": ...}`.
        request: String,
    },
    /// Run a raw query by name.
    Query {
        /// Query name, for example `timeline.read`.
        name: String,
        /// Arguments as JSON text, @file, or - for stdin.
        args: Option<String>,
    },
    /// Render a timeline to a video file and verify the file against the request.
    Export {
        /// Output file, for example `Promo.mp4`. Without --preset, the extension decides:
        /// .mov is ProRes, .webm is VP9, .m4a is audio only, anything else is the most
        /// compatible video preset this machine can encode.
        output: Option<PathBuf>,
        /// Preset: mp4-h264, mp4-hevc, mp4-av1, mov-prores, webm-vp9 or m4a-aac.
        #[arg(long)]
        preset: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
        /// List the presets and whether this machine can encode them, instead of exporting.
        #[arg(long)]
        presets: bool,
    },
}

/// `fosscut project ...`
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Create a project bundle.
    New {
        /// Bundle directory to create, for example `Promo.fosscut`.
        path: PathBuf,
        /// Project name; the directory name when omitted.
        #[arg(long)]
        name: Option<String>,
        /// Canvas width in pixels.
        #[arg(long, default_value_t = 1920)]
        width: u32,
        /// Canvas height in pixels.
        #[arg(long, default_value_t = 1080)]
        height: u32,
        /// Frame rate, for example 30 or 30000/1001.
        #[arg(long, default_value = "30")]
        rate: String,
    },
    /// Show project name, revision, timelines and undo state.
    Info,
}

/// `fosscut media ...`
#[derive(Debug, Subcommand)]
pub enum MediaCommand {
    /// Probe a file with ffprobe and add it to the media table.
    Import {
        /// Audio, video or image file.
        file: PathBuf,
        /// Display name; the file name when omitted.
        #[arg(long)]
        name: Option<String>,
    },
    /// List imported media.
    List,
}

/// `fosscut track ...`
#[derive(Debug, Subcommand)]
pub enum TrackCommand {
    /// Add an empty track.
    Add {
        /// Track kind.
        #[arg(long, value_enum)]
        kind: TrackKindArg,
        /// Display name.
        #[arg(long)]
        name: Option<String>,
        /// Position in the track list (0 is the bottom); on top when omitted.
        #[arg(long)]
        index: Option<usize>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
}

/// `fosscut clip ...`
#[derive(Debug, Subcommand)]
pub enum ClipCommand {
    /// Place a media asset on a track.
    Add {
        /// Asset id.
        #[arg(long)]
        asset: String,
        /// Track id; the first matching track when omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline position; appended when omitted.
        #[arg(long, value_parser = parse_time)]
        start: Option<TimeInput>,
        /// Source in-point inside the asset.
        #[arg(long, value_parser = parse_time)]
        source_in: Option<TimeInput>,
        /// Length; the rest of the asset when omitted.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Split a clip at a timeline position.
    Split {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Timeline position strictly inside the clip.
        #[arg(long, value_parser = parse_time)]
        at: TimeInput,
    },
    /// Move a clip in time and/or to another track.
    Move {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// New timeline position.
        #[arg(long, value_parser = parse_time)]
        start: Option<TimeInput>,
        /// Target track id.
        #[arg(long)]
        track: Option<String>,
    },
    /// Remove a clip.
    Remove {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Shift later clips left to close the gap.
        #[arg(long)]
        ripple: bool,
    },
}

/// `fosscut text ...`
#[derive(Debug, Subcommand)]
pub enum TextCommand {
    /// Add a text clip.
    Add {
        /// The text; use \n for line breaks.
        text: String,
        /// Text track id; the first text track when omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline position; appended when omitted.
        #[arg(long, value_parser = parse_time)]
        start: Option<TimeInput>,
        /// Length; 3 seconds when omitted.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// Fill colour as #RRGGBB or #RRGGBBAA.
        #[arg(long)]
        color: Option<String>,
        /// Font size as a fraction of the canvas height, for example 0.08.
        #[arg(long)]
        size: Option<f64>,
        /// Font family.
        #[arg(long)]
        font: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
}

/// `fosscut timeline ...`
#[derive(Debug, Subcommand)]
pub enum TimelineCommand {
    /// Show tracks and clips.
    Read {
        /// Amount of detail.
        #[arg(long, value_enum, default_value_t = Detail::Normal)]
        detail: Detail,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
}

/// Amount of detail in reads.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Detail {
    /// Names and counts.
    Outline,
    /// Working view.
    Normal,
    /// Everything.
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

/// Track kinds.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum TrackKindArg {
    /// Video and images.
    Video,
    /// Audio.
    Audio,
    /// Titles and captions.
    Text,
}

impl TrackKindArg {
    /// The command argument value.
    pub fn as_str(self) -> &'static str {
        match self {
            TrackKindArg::Video => "video",
            TrackKindArg::Audio => "audio",
            TrackKindArg::Text => "text",
        }
    }
}

fn parse_time(text: &str) -> Result<TimeInput, String> {
    text.parse()
        .map_err(|e: fosscut_time::TimeError| e.to_string())
}
