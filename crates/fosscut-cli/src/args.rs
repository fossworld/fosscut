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
    /// Animate clip properties with keyframes.
    #[command(subcommand)]
    Keyframe(KeyframeCommand),
    /// Add, change, reorder and copy effects on clips.
    #[command(subcommand)]
    Effect(EffectCommand),
    /// Add adjustment layers: effects over everything below them.
    #[command(subcommand)]
    Adjustment(AdjustmentCommand),
    /// Mask clips: shapes, pen paths, text and brush strokes.
    #[command(subcommand)]
    Mask(MaskCommand),
    /// Transitions over the cut from a clip to the next one.
    #[command(subcommand)]
    Transition(TransitionCommand),
    /// Animate clips in, out and while they show.
    #[command(subcommand)]
    Animation(AnimationCommand),
    /// Apply built-in looks (filters) as effects.
    #[command(subcommand)]
    Look(LookCommand),
    /// Correct a clip's color automatically, or match it to another clip.
    #[command(subcommand)]
    Color(ColorCommand),
    /// Add titles.
    #[command(subcommand)]
    Text(TextCommand),
    /// Caption speech word by word, style captions, and read or write SRT subtitle files.
    #[command(subcommand)]
    Captions(CaptionsCommand),
    /// Read timelines.
    #[command(subcommand)]
    Timeline(TimelineCommand),
    /// Copy and paste clips.
    #[command(subcommand)]
    Clips(ClipsCommand),
    /// Timeline markers.
    #[command(subcommand)]
    Marker(MarkerCommand),
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
    /// List imported media, marking files that cannot be found.
    List,
    /// Rename an asset in the media bin (not the file).
    Rename {
        /// The asset.
        asset: String,
        /// Its new name.
        name: String,
    },
    /// Remove an asset from the media bin (never the file).
    Remove {
        /// The asset.
        asset: String,
        /// Also remove the clips that show it.
        #[arg(long)]
        with_clips: bool,
    },
    /// Put assets in a bin folder such as `B-roll/Day 1`, or back at the top level.
    Move {
        /// The assets.
        #[arg(required = true)]
        assets: Vec<String>,
        /// The folder; the top level when omitted.
        #[arg(long)]
        folder: Option<String>,
    },
    /// Add, rename or remove a bin folder.
    Folder {
        /// add, rename or remove.
        action: String,
        /// The folder.
        folder: String,
        /// For rename: the new path.
        #[arg(long)]
        to: Option<String>,
    },
    /// Point an asset at its file's new place, after the file was moved or renamed.
    Relink {
        /// The asset.
        asset: String,
        /// Where the file is now.
        file: PathBuf,
    },
    /// Show another asset in every clip that shows this one.
    Replace {
        /// The asset to replace.
        asset: String,
        /// The asset that takes its place.
        with: String,
    },
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
        #[arg(long, required_unless_present = "compound")]
        asset: Option<String>,
        /// A compound timeline to place instead of an asset.
        #[arg(long, conflicts_with = "asset")]
        compound: Option<String>,
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
        /// Split only this clip, not the clips of its group.
        #[arg(long)]
        unlinked: bool,
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
        /// Keep the clip's start and move what follows on its track by the change in
        /// length (trim and delete left or right).
        #[arg(long)]
        ripple: bool,
        /// Trim only this clip, not the clips of its group.
        #[arg(long)]
        unlinked: bool,
    },
    /// Change a video or audio clip's speed: a rate, a curve or a preset, reverse, pitch
    /// and smooth slow motion. The clip keeps the media it plays, so faster clips get
    /// shorter.
    Speed {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// A constant rate from 0.05 to 100, for example 2 (twice as fast).
        #[arg(long)]
        rate: Option<f64>,
        /// A speed curve as AT:RATE points, for example `0:1,0.5:3,1:1`.
        #[arg(long)]
        curve: Option<String>,
        /// A built-in curve: montage, hero, bullet, jump_cut, flash_in or flash_out.
        #[arg(long)]
        preset: Option<String>,
        /// Play backwards.
        #[arg(long, conflicts_with = "forward")]
        reverse: bool,
        /// Play forwards again.
        #[arg(long)]
        forward: bool,
        /// `preserve` keeps the sound's pitch; `shift` lets it follow the speed.
        #[arg(long)]
        pitch: Option<String>,
        /// Smooth slow motion: blend neighbouring frames.
        #[arg(long, conflicts_with = "nearest")]
        blend: bool,
        /// Show the nearest frame again.
        #[arg(long)]
        nearest: bool,
        /// Move later clips on the track when the clip's length changes.
        #[arg(long)]
        ripple: bool,
        /// Back to normal speed.
        #[arg(long)]
        reset: bool,
    },
    /// Hold one frame of a video clip, moving what follows later.
    Freeze {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Timeline time of the frame to hold.
        #[arg(long, value_parser = parse_time)]
        at: TimeInput,
        /// How long to hold it (two seconds by default).
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
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
        /// Move only this clip, not the clips of its group.
        #[arg(long)]
        unlinked: bool,
    },
    /// Remove a clip.
    Remove {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Shift later clips left to close the gap.
        #[arg(long)]
        ripple: bool,
        /// Remove only this clip, not the clips of its group.
        #[arg(long)]
        unlinked: bool,
    },
    /// Turn clips into one compound clip that shows a new compound timeline.
    Compound {
        /// Clip ids.
        #[arg(long = "clip", required = true)]
        clips: Vec<String>,
        /// The compound timeline's name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Put a compound clip's contents back in its place.
    Decompose {
        /// The compound clip.
        #[arg(long)]
        clip: String,
    },
    /// Group clips so moves, splits, trims and removals edit them together.
    Group {
        /// Clip ids, two or more.
        #[arg(long = "clip", required = true)]
        clips: Vec<String>,
    },
    /// Take clips out of their groups.
    Ungroup {
        /// Clip ids.
        #[arg(long = "clip", required = true)]
        clips: Vec<String>,
    },
    /// Move a video clip's sound to an audio clip of its own, linked to it.
    ExtractAudio {
        /// Clip id.
        #[arg(long)]
        clip: String,
    },
    /// Put a video clip's extracted sound back.
    RestoreAudio {
        /// Clip id.
        #[arg(long)]
        clip: String,
    },
    /// Put other media under a clip, keeping its place and everything done to it.
    Replace {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// The new asset id.
        #[arg(long)]
        asset: String,
        /// Where in the new media the clip starts.
        #[arg(long, value_parser = parse_time)]
        source_in: Option<TimeInput>,
    },
    /// Line a clip's picture up with the canvas's edges or centre.
    Align {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// left, center or right.
        #[arg(long)]
        horizontal: Option<String>,
        /// top, center or bottom.
        #[arg(long)]
        vertical: Option<String>,
    },
    /// Copy one clip's attributes (placement, crop, opacity, blend, volume, audio, effects,
    /// masks, animations) to other clips.
    CopyAttributes {
        /// The clip to copy from.
        #[arg(long)]
        from: String,
        /// The clips that receive them.
        #[arg(long = "to", required = true)]
        to: Vec<String>,
        /// Only these attributes; all when omitted.
        #[arg(long = "attribute")]
        attributes: Vec<String>,
    },
    /// Set a clip's placement, crop, opacity, blend mode or sound. Position and scale are
    /// relative to the canvas and the fitted picture, so the same values work at any
    /// output size. `--set PATH=VALUE` reaches every property `fosscut capabilities` lists.
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
        /// Extra width and height factors for non-uniform scaling, for example 1.2,1.
        #[arg(long, value_delimiter = ',')]
        stretch: Option<Vec<f64>>,
        /// Blend mode: normal, multiply, screen, overlay, darken, lighten, color_dodge,
        /// color_burn, hard_light, soft_light, linear_burn, add, difference or exclusion.
        #[arg(long)]
        blend: Option<String>,
        /// Fade the sound in over this long, for example 0.5s.
        #[arg(long, value_parser = parse_time)]
        fade_in: Option<TimeInput>,
        /// Fade the sound out over this long.
        #[arg(long, value_parser = parse_time)]
        fade_out: Option<TimeInput>,
        /// Stereo balance from -1 (left) to 1 (right).
        #[arg(long, allow_hyphen_values = true)]
        pan: Option<f64>,
        /// false deactivates the clip; true activates it again.
        #[arg(long)]
        enabled: Option<bool>,
        /// Any property as PATH=VALUE, VALUE in JSON (a bare word is a string), for example
        /// transform.flip_horizontal=true, crop.left=0.1 or audio.channels=left. Repeatable.
        #[arg(long = "set", value_name = "PATH=VALUE")]
        properties: Vec<String>,
    },
}

/// `fosscut effect ...`
#[derive(Debug, Subcommand)]
pub enum EffectCommand {
    /// Add an effect to a clip, for example `--type color.adjust --param exposure=0.5`.
    Add {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Effect type: color.adjust, color.curves, color.hsl, color.wheels, color.lut,
        /// color.vignette, blur.gaussian, detail.sharpen or stylize.grain.
        #[arg(long = "type")]
        effect_type: String,
        /// A parameter as NAME=VALUE, VALUE in JSON (a bare word is a string). Repeatable.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
        /// Position in the clip's effects; last when omitted.
        #[arg(long)]
        index: Option<usize>,
    },
    /// Change an effect's parameters, reset some, or bypass it.
    Set {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Effect id.
        #[arg(long)]
        effect: String,
        /// A parameter as NAME=VALUE. Repeatable.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
        /// A parameter to return to its default. Repeatable.
        #[arg(long)]
        reset: Vec<String>,
        /// false bypasses the effect; true applies it again.
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Remove an effect.
    Remove {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Effect id.
        #[arg(long)]
        effect: String,
    },
    /// Move an effect to another position (0 applies first).
    Move {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Effect id.
        #[arg(long)]
        effect: String,
        /// New position.
        #[arg(long)]
        index: usize,
    },
    /// Copy one clip's effects to other clips.
    Copy {
        /// The clip whose effects are copied.
        #[arg(long)]
        from: String,
        /// A clip that receives them. Repeatable.
        #[arg(long, required = true)]
        to: Vec<String>,
        /// Add after the receivers' effects instead of replacing them.
        #[arg(long)]
        append: bool,
    },
}

/// `fosscut mask ...`
#[derive(Debug, Subcommand)]
pub enum MaskCommand {
    /// Add a mask to a clip.
    Add {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// rectangle, ellipse, linear, mirror, star, heart, path, text or brush.
        #[arg(long)]
        shape: String,
        /// Any setting as NAME=VALUE, VALUE in JSON, for example center=[0.3,0.5],
        /// feather=0.4, invert=true, text=\"SALE\" or points=[...]. Repeatable.
        #[arg(long = "set", value_name = "NAME=VALUE")]
        settings: Vec<String>,
    },
    /// Change a mask.
    Set {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Mask id.
        #[arg(long)]
        mask: String,
        /// A setting as NAME=VALUE, VALUE in JSON. Repeatable.
        #[arg(long = "set", value_name = "NAME=VALUE")]
        settings: Vec<String>,
    },
    /// Remove a mask.
    Remove {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Mask id.
        #[arg(long)]
        mask: String,
    },
}

/// `fosscut transition ...`
#[derive(Debug, Subcommand)]
pub enum TransitionCommand {
    /// Add or replace the transition from a clip into the next one on its track.
    Set {
        /// The clip the transition leaves.
        #[arg(long)]
        clip: String,
        /// dissolve, dip, wipe, slide, push, zoom, blur, iris, clock or spin.
        #[arg(long = "type")]
        transition_type: String,
        /// How long it lasts, centred on the cut; 0.5s when omitted.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// A parameter as NAME=VALUE, for example direction=up or color=#FFFFFF.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
    },
    /// Remove a clip's transition: a plain cut again.
    Remove {
        /// The clip the transition leaves.
        #[arg(long)]
        clip: String,
    },
}

/// `fosscut animation ...`
#[derive(Debug, Subcommand)]
pub enum AnimationCommand {
    /// Set a clip's animations; each as PRESET or PRESET:DURATION, for example fade:1s.
    Set {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Entrance: fade, slide_left, slide_right, slide_up, slide_down, zoom_in,
        /// zoom_out, spin, pop or drop.
        #[arg(long = "in")]
        entrance: Option<String>,
        /// Exit: the same presets.
        #[arg(long = "out")]
        exit: Option<String>,
        /// Repeating: pulse, swing, float or shake.
        #[arg(long)]
        combo: Option<String>,
        /// Remove in, out or combo. Repeatable.
        #[arg(long)]
        clear: Vec<String>,
    },
}

/// `fosscut adjustment ...`
#[derive(Debug, Subcommand)]
pub enum AdjustmentCommand {
    /// Add an adjustment layer above the video tracks.
    Add {
        /// Where it starts.
        #[arg(long, value_parser = parse_time)]
        start: TimeInput,
        /// How long it lasts.
        #[arg(long, value_parser = parse_time)]
        duration: TimeInput,
        /// An effect type to start with, at its defaults. Repeatable.
        #[arg(long = "type")]
        effects: Vec<String>,
        /// Video track id; an adjustment track above the video when omitted.
        #[arg(long)]
        track: Option<String>,
    },
}

/// `fosscut look ...`
#[derive(Debug, Subcommand)]
pub enum LookCommand {
    /// Apply a look, such as warm, teal_orange, vintage or noir (`fosscut capabilities`
    /// lists them).
    Apply {
        /// The look.
        look: String,
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Replace the clip's effects instead of adding after them.
        #[arg(long)]
        replace: bool,
    },
}

/// `fosscut color ...`
#[derive(Debug, Subcommand)]
pub enum ColorCommand {
    /// Measure a clip and add a color.adjust that corrects its white balance, brightness
    /// and levels.
    Auto {
        /// Clip id.
        #[arg(long)]
        clip: String,
    },
    /// Measure two clips and add a color.adjust to the first that matches the second.
    Match {
        /// The clip to change.
        #[arg(long)]
        clip: String,
        /// The clip to match.
        #[arg(long)]
        reference: String,
    },
}

/// `fosscut keyframe ...`
#[derive(Debug, Subcommand)]
pub enum KeyframeCommand {
    /// Add a keyframe to a clip property at a time, or replace the one there.
    Set {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// Animatable property, for example opacity, transform.position or volume.
        #[arg(long)]
        property: String,
        /// Timeline position of the keyframe.
        #[arg(long, value_parser = parse_time)]
        at: Option<TimeInput>,
        /// Position from the clip's start, instead of --at.
        #[arg(long, value_parser = parse_time)]
        offset: Option<TimeInput>,
        /// The value: a number, or x,y for position and stretch. The current value when
        /// omitted.
        #[arg(long, allow_hyphen_values = true)]
        value: Option<String>,
        /// Easing to the next keyframe: linear, hold, ease_in, ease_out or ease_in_out.
        #[arg(long)]
        easing: Option<String>,
    },
    /// Remove a keyframe at a time, or all keyframes of a property.
    Remove {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// The property.
        #[arg(long)]
        property: String,
        /// Timeline position of the keyframe.
        #[arg(long, value_parser = parse_time)]
        at: Option<TimeInput>,
        /// Position from the clip's start, instead of --at.
        #[arg(long, value_parser = parse_time)]
        offset: Option<TimeInput>,
        /// Remove every keyframe of the property.
        #[arg(long)]
        all: bool,
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

/// `fosscut clips ...`
#[derive(Debug, Subcommand)]
pub enum ClipsCommand {
    /// Copy clips as JSON documents, to paste with `clips paste` here or in another
    /// project.
    Copy {
        /// Clip ids.
        #[arg(long = "clip", required = true)]
        clips: Vec<String>,
    },
    /// Paste clips copied with `clips copy`, keeping their arrangement.
    Paste {
        /// Where the earliest of them starts.
        #[arg(long, value_parser = parse_time)]
        at: TimeInput,
        /// The output of `clips copy --json`: `@file`, `-` for standard input, or the JSON
        /// itself.
        #[arg(long, default_value = "-")]
        from: String,
        /// Put them all on this track.
        #[arg(long)]
        track: Option<String>,
    },
}

/// `fosscut marker ...`
#[derive(Debug, Subcommand)]
pub enum MarkerCommand {
    /// Add a marker.
    Add {
        /// Where.
        #[arg(long, value_parser = parse_time)]
        at: TimeInput,
        /// A label.
        #[arg(long)]
        name: Option<String>,
        /// A color as #RRGGBB.
        #[arg(long)]
        color: Option<String>,
    },
    /// Move, rename or recolor a marker.
    Set {
        /// Marker id.
        #[arg(long)]
        marker: String,
        /// A new time.
        #[arg(long, value_parser = parse_time)]
        at: Option<TimeInput>,
        /// A new label; empty removes it.
        #[arg(long)]
        name: Option<String>,
        /// A new color; empty returns to the default.
        #[arg(long)]
        color: Option<String>,
    },
    /// Remove a marker.
    Remove {
        /// Marker id.
        #[arg(long)]
        marker: String,
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
    /// Add a timeline: empty, or with media placed one after another.
    Add {
        /// Its name.
        #[arg(long)]
        name: Option<String>,
        /// Assets to place, in order.
        #[arg(long = "asset")]
        assets: Vec<String>,
        /// Canvas width.
        #[arg(long)]
        width: Option<u32>,
        /// Canvas height.
        #[arg(long)]
        height: Option<u32>,
        /// Frame rate, such as 30 or 30000/1001.
        #[arg(long)]
        rate: Option<String>,
        /// Make it the primary timeline.
        #[arg(long)]
        primary: bool,
    },
    /// Rename a timeline (the primary one unless --timeline names another).
    Rename {
        /// Its new name.
        name: String,
        /// The timeline.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Make a timeline the primary one.
    Primary {
        /// The timeline.
        timeline: String,
    },
    /// Remove a timeline.
    Remove {
        /// The timeline.
        timeline: String,
    },
    /// Split every clip under a time, on every unlocked track or the ones named.
    Split {
        /// Where to cut.
        #[arg(long, value_parser = parse_time)]
        at: TimeInput,
        /// Only these tracks.
        #[arg(long = "track")]
        tracks: Vec<String>,
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
