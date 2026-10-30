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
        /// Commands or queries to describe with their arguments, for example `clip.add
        /// text.add`.
        names: Vec<String>,
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
    /// Add titles.
    #[command(subcommand)]
    Text(TextCommand),
    /// Caption speech word by word, style captions, and read or write SRT subtitle files.
    #[command(subcommand)]
    Captions(CaptionsCommand),
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
    /// Install FossCut's editing skills for AI agents, or list them.
    #[command(subcommand)]
    Skills(SkillsCommand),
    /// List and install the models some analyzers need, such as speech recognition for
    /// `media analyze <asset> transcript`. Models are kept in FOSSCUT_MODELS_DIR, or in
    /// `models` in FossCut's cache folder.
    #[command(subcommand)]
    Models(ModelsCommand),
    /// Serve FossCut to an AI agent over MCP on stdin and stdout. Add it to your agent with,
    /// for example, `claude mcp add fosscut -- fosscut mcp`. Tools use --project (or the
    /// single bundle in the working directory) until the agent opens another project.
    Mcp,
    /// Keep the project open in this process as its live host until Ctrl-C, so edits from
    /// other terminals and agents (CLI or MCP) run in one session, in order, and undo in
    /// one place. The desktop app hosts projects the same way.
    Host {
        /// Print each change as it is committed.
        #[arg(long)]
        events: bool,
        /// Also stop when standard input closes, so a parent process can end the host.
        #[arg(long)]
        exit_on_eof: bool,
    },
    /// Render one frame of a timeline to a PNG file, exactly as an export shows it.
    Frame {
        /// The moment to show, for example 1s, 30f or 00:00:01:00.
        time: TimeInput,
        /// PNG file to write.
        #[arg(long)]
        out: PathBuf,
        /// Width in pixels; the timeline's width when omitted. The height keeps the aspect
        /// ratio.
        #[arg(long)]
        width: Option<u32>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Render frames spread evenly over a timeline into one PNG grid, to check an edit at
    /// a glance.
    ContactSheet {
        /// PNG file to write.
        #[arg(long)]
        out: PathBuf,
        /// Number of frames, 1 to 100.
        #[arg(long, default_value_t = 12)]
        count: u32,
        /// Width of each frame in pixels.
        #[arg(long, default_value_t = 320)]
        width: u32,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
}

/// `fosscut skills ...`
#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    /// List the built-in skills and what each is for.
    List,
    /// Copy skills into an agent's skill folder: Claude Code reads `.claude/skills` and
    /// Codex reads `.agents/skills`, in this folder or, with --user, in your home folder.
    /// Replacing a changed copy needs --allow-destructive.
    Install {
        /// The agent: claude or codex. Use --dir for any other folder.
        #[arg(long, value_enum)]
        target: Option<SkillTarget>,
        /// Install for all your projects (in your home folder) instead of this folder.
        #[arg(long, requires = "target")]
        user: bool,
        /// Install into this folder instead, one subfolder per skill.
        #[arg(long, conflicts_with_all = ["target", "user"])]
        dir: Option<PathBuf>,
        /// Install only this skill; every skill when omitted.
        #[arg(long)]
        skill: Option<String>,
    },
}

/// `fosscut models ...`
#[derive(Debug, Subcommand)]
pub enum ModelsCommand {
    /// List the models with their size, license and languages, and which are installed.
    List,
    /// Download a model and verify it against its published SHA-256. Without --yes this
    /// only shows the size and license, since it downloads.
    Install {
        /// Model name, for example whisper-base.en (see `fosscut models list`).
        name: String,
        /// Agree to the download.
        #[arg(long)]
        yes: bool,
        /// Install from a file already downloaded instead; it is verified the same way.
        #[arg(long)]
        from: Option<PathBuf>,
    },
}

/// Agents whose skill folders FossCut knows.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SkillTarget {
    /// Claude Code: `.claude/skills`.
    Claude,
    /// Codex: `.agents/skills`.
    Codex,
}

impl SkillTarget {
    /// The skill folder relative to a project or home folder.
    pub fn folder(self) -> &'static str {
        match self {
            SkillTarget::Claude => ".claude/skills",
            SkillTarget::Codex => ".agents/skills",
        }
    }
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
    /// Measure an asset's loudness, silences and shot cuts on this machine. Results are
    /// cached by the file's content, so asking again is instant.
    Analyze {
        /// The asset, for example `asset_0001`.
        asset: String,
        /// Analyzers to run: loudness, silences, scenes, transcript. Without names, every one
        /// the asset allows except transcript, which needs a speech model.
        analyzers: Vec<String>,
        /// Measure again even if results are cached.
        #[arg(long)]
        refresh: bool,
        /// Silences: level below which audio counts as quiet, in dBFS (default -40).
        #[arg(long, allow_hyphen_values = true)]
        silence_threshold: Option<f64>,
        /// Silences: shortest quiet run reported, in milliseconds (default 300).
        #[arg(long)]
        min_silence_ms: Option<u32>,
        /// Scenes: picture change that counts as a cut, from 0 to 1 (default 0.35).
        #[arg(long)]
        scene_threshold: Option<f64>,
        /// Scenes: shortest shot, in milliseconds (default 500).
        #[arg(long)]
        min_shot_ms: Option<u32>,
        /// Transcript: speech model, for example whisper-base.en; the most capable installed
        /// one for the language when omitted.
        #[arg(long)]
        model: Option<String>,
        /// Transcript: spoken language as a code such as en or de; detected when omitted.
        #[arg(long)]
        language: Option<String>,
        /// Transcript: shortest gap between words reported as a pause, in milliseconds
        /// (default 300).
        #[arg(long)]
        min_pause_ms: Option<u32>,
    },
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
    /// Move a clip's start or end edge: trim it shorter, or extend it into more of its
    /// media. The picture under the rest of the clip stays where it is.
    Trim {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// New timeline position of the first frame.
        #[arg(long, value_parser = parse_time)]
        start: Option<TimeInput>,
        /// New timeline position just after the last frame.
        #[arg(long, value_parser = parse_time)]
        end: Option<TimeInput>,
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
    /// Set a clip's placement, opacity or volume. Position and scale are relative to the
    /// canvas and the fitted picture, so the same values work at any output size.
    Set {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Offset of the clip's centre from the canvas centre as fractions of its width and
        /// height, for example 0.2,0 (right) or 0,-0.3 (up).
        #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
        position: Option<Vec<f64>>,
        /// Size relative to the fitted picture, for example 1.5.
        #[arg(long)]
        scale: Option<f64>,
        /// Degrees clockwise.
        #[arg(long, allow_hyphen_values = true)]
        rotation: Option<f64>,
        /// Opacity from 0 to 1.
        #[arg(long)]
        opacity: Option<f64>,
        /// Linear audio gain: 1 unchanged, 0 silent.
        #[arg(long)]
        volume: Option<f64>,
    },
}

/// `fosscut captions ...`
#[derive(Debug, Subcommand)]
pub enum CaptionsCommand {
    /// Caption the clips of a track from their cached transcripts: run `fosscut media
    /// analyze <asset> transcript` first. Words become pages of a few words on a caption
    /// track.
    Generate {
        /// Built-in look: bold-pop, highlight, karaoke or boxed (see `fosscut capabilities`).
        #[arg(long)]
        preset: Option<String>,
        /// Track whose clips are captioned; the first video or audio track with sound when
        /// omitted.
        #[arg(long)]
        source_track: Option<String>,
        /// Caption track to add to; the first caption track (created if needed) when
        /// omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
        /// Most words on a page (default 4).
        #[arg(long)]
        max_words: Option<u32>,
        /// Most lines on a page (default 2).
        #[arg(long)]
        max_lines: Option<u32>,
        /// The speech model the media was transcribed with, if not the default.
        #[arg(long)]
        model: Option<String>,
        /// The language the media was transcribed with, if given then.
        #[arg(long)]
        language: Option<String>,
        /// The pause length the media was transcribed with, if not 300.
        #[arg(long)]
        min_pause_ms: Option<u32>,
        /// Replace the track's captions; needs --allow-destructive.
        #[arg(long)]
        replace: bool,
    },
    /// Import an SRT subtitle file: one caption page per cue, lines kept.
    Import {
        /// The .srt file.
        file: PathBuf,
        /// Built-in look for the caption track.
        #[arg(long)]
        preset: Option<String>,
        /// Caption track to add to; the first caption track (created if needed) when
        /// omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
        /// Replace the track's captions; needs --allow-destructive.
        #[arg(long)]
        replace: bool,
    },
    /// Write a caption track as an SRT file. Overwriting needs --allow-destructive.
    Export {
        /// The .srt file to write.
        file: PathBuf,
        /// Caption track; the timeline's first caption track when omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Set how a caption track looks and moves: a preset, a style patch, or both.
    Style {
        /// Built-in look: bold-pop, highlight, karaoke or boxed.
        #[arg(long)]
        preset: Option<String>,
        /// Changes as a JSON merge patch (text, @file or - for stdin), for example
        /// '{"size": 0.07, "placement": "middle"}'.
        #[arg(long)]
        style: Option<String>,
        /// Caption track; the timeline's first caption track when omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Change one caption page: new wording that keeps its timing, or which words stand out.
    Edit {
        /// The caption clip, for example clip_0012.
        clip: String,
        /// New wording; `\n` between lines.
        #[arg(long)]
        text: Option<String>,
        /// Words to emphasise, counted from 0 over the page, for example 1,3.
        #[arg(long, value_delimiter = ',')]
        emphasis: Option<Vec<usize>>,
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
    /// Copy a timeline, for example onto a 1080x1920 canvas for a vertical version.
    Duplicate {
        /// Canvas width of the copy; the original's when omitted.
        #[arg(long)]
        width: Option<u32>,
        /// Canvas height of the copy; the original's when omitted.
        #[arg(long)]
        height: Option<u32>,
        /// Scale video and image clips to cover the new canvas instead of fitting inside it.
        #[arg(long)]
        fill: bool,
        /// Name of the copy.
        #[arg(long)]
        name: Option<String>,
        /// Make the copy the primary timeline.
        #[arg(long)]
        primary: bool,
        /// Timeline to copy; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Cut time ranges out of every track and close the gaps, keeping picture, sound and
    /// captions in sync. Cutting the primary video track needs --allow-destructive.
    RemoveRanges {
        /// Ranges as START..END, for example 3.4s..4.1s or 00:00:10:00..00:00:12:15.
        #[arg(required = true)]
        ranges: Vec<String>,
        /// Cut only these tracks, for example track_0002,track_0004; every track when
        /// omitted.
        #[arg(long, value_delimiter = ',')]
        tracks: Option<Vec<String>>,
        /// Leave the gaps instead of closing them.
        #[arg(long)]
        no_ripple: bool,
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
