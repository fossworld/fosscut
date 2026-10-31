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
    /// Add and restyle titles, save text presets, and add title templates.
    #[command(subcommand)]
    Text(TextCommand),
    /// Level clips' loudness.
    #[command(subcommand)]
    Audio(AudioCommand),
    /// FossCut's own library: shapes, stickers, backgrounds, templates, text and caption
    /// styles, looks, sound effects and music, free to use, also commercially.
    #[command(subcommand)]
    Library(LibraryCommand),
    /// Caption speech word by word, style captions, and read or write SRT subtitle files.
    #[command(subcommand)]
    Captions(CaptionsCommand),
    /// Read timelines.
    #[command(subcommand)]
    Timeline(TimelineCommand),
    /// Copy and paste clips.
    #[command(subcommand)]
    Clips(ClipsCommand),
    /// Multicam editing: angles recorded at the same time in one clip, cut between them.
    #[command(subcommand)]
    Multicam(MulticamCommand),
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
        /// .mov is ProRes, .webm is VP9, .gif a GIF, .m4a, .wav, .flac and .mp3 audio only,
        /// anything else the most compatible video preset this machine can encode.
        output: Option<PathBuf>,
        /// Preset, for example mp4-h264, mp4-hevc, mov-prores-hq, gif, wav or mp3 (see
        /// --presets).
        #[arg(long)]
        preset: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
        /// List the presets and whether this machine can encode them, instead of exporting.
        #[arg(long)]
        presets: bool,
        /// The shorter side: 480p, 720p, 1080p, 2k, 4k, 8k or a number of pixels.
        #[arg(long)]
        resolution: Option<String>,
        /// Frame rate, such as 24, 25, 29.97, 30, 50, 59.94, 60 or 120.
        #[arg(long)]
        fps: Option<String>,
        /// Video bit rate, such as 8M.
        #[arg(long)]
        bitrate: Option<String>,
        /// Audio bit rate, such as 192k.
        #[arg(long)]
        audio_bitrate: Option<String>,
        /// Only this part, START..END, for example 2s..10s.
        #[arg(long)]
        range: Option<String>,
        /// Write a cover image (PNG) of the frame at this time of the export beside the file.
        #[arg(long, value_parser = parse_time)]
        cover: Option<TimeInput>,
        /// Write the captions beside the file as srt or vtt subtitles.
        #[arg(long)]
        subtitles: Option<String>,
    },
    /// Record a voiceover, a camera or the screen straight into the project: the file goes
    /// to the bundle's `recordings/` folder and is imported (and placed with --at). Stops
    /// after --duration, when Enter is pressed, or when standard input closes.
    Record {
        /// microphone (a voiceover), camera or screen.
        #[arg(required_unless_present = "list")]
        what: Option<String>,
        /// List the microphones, cameras and screens of this machine instead.
        #[arg(long)]
        list: bool,
        /// The device: its name, a part of it, or its number from 1 in --list; the first
        /// of its kind when omitted.
        #[arg(long)]
        device: Option<String>,
        /// For camera and screen: the microphone to record with it, chosen the same way, or
        /// `none`. A camera takes the first microphone and a screen none when omitted.
        #[arg(long)]
        microphone: Option<String>,
        /// Stop after this long, for example 30s.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// The file to write; a new file in the bundle's `recordings/` folder when omitted.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Picture size as WIDTHxHEIGHT, for example 1280x720 (1920x1080 for a screen and
        /// 1280x720 for a camera when omitted).
        #[arg(long)]
        size: Option<String>,
        /// Frame rate, 30 when omitted.
        #[arg(long)]
        fps: Option<String>,
        /// Also place the recording on the timeline from this time, on a free track.
        #[arg(long, value_parser = parse_time)]
        at: Option<TimeInput>,
        /// Only write the file; do not import it.
        #[arg(long, conflicts_with = "at")]
        no_import: bool,
        /// An FFmpeg input instead of a device, as FORMAT:INPUT (for example
        /// `lavfi:sine=frequency=440`).
        #[arg(long, hide = true)]
        input: Option<String>,
        /// A second FFmpeg input for the sound, as FORMAT:INPUT.
        #[arg(long, hide = true)]
        microphone_input: Option<String>,
    },
    /// FossCut's caches (analysis, proxies, remuxed copies, rendered frames): their sizes,
    /// clearing them, and cleaning them by the cache preferences.
    #[command(subcommand)]
    Cache(CacheCommand),
    /// Make a voiceover from text with a local English voice (Kokoro, ADR-0033): written
    /// into the bundle's `speech/` folder, imported and placed. The same text, voice and
    /// speed reuse the file.
    Speak {
        /// The text to say.
        #[arg(required_unless_present = "voices")]
        text: Option<String>,
        /// The voice, such as af_heart (American) or bm_george (British); see --voices.
        #[arg(long)]
        voice: Option<String>,
        /// Speaking speed from 0.5 to 2 (default 1).
        #[arg(long)]
        speed: Option<f32>,
        /// Where to place the speech on the timeline; after the first audio track's last
        /// clip when omitted.
        #[arg(long, value_parser = parse_time)]
        at: Option<TimeInput>,
        /// The audio track to place it on; a free one when omitted.
        #[arg(long)]
        track: Option<String>,
        /// List the voices instead.
        #[arg(long)]
        voices: bool,
    },
    /// Your preferences (ADR-0031): defaults the CLI and the app fill requests with, and
    /// performance and cache settings. Without a subcommand, every preference.
    Preferences {
        #[command(subcommand)]
        command: Option<PreferencesCommand>,
    },
    /// Check whether a newer FossCut release exists (one request to GitHub; nothing about
    /// you is sent). Installing stays yours to do.
    Update {
        /// Ask this address instead of GitHub's releases API.
        #[arg(long, hide = true)]
        from: Option<String>,
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
        /// Frame rate, for example 30, 29.97 or 30000/1001; the preferred rate of new
        /// projects (`timeline.frame_rate`, 30 by default) when omitted.
        #[arg(long)]
        rate: Option<String>,
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
    /// Make a proxy of a video asset: a smaller ProRes Proxy copy in FossCut's cache that
    /// the desktop preview plays instead (exports always read the original).
    Proxy {
        /// The asset.
        asset: String,
        /// half or quarter; the preferred size (`performance.proxy_size`) when omitted.
        #[arg(long)]
        size: Option<String>,
    },
    /// Copy a video asset in a hard-to-seek container (MPEG-TS, MPEG-PS, AVI, FLV) into
    /// QuickTime in FossCut's cache, without re-encoding, for the desktop preview.
    Remux {
        /// The asset.
        asset: String,
    },
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
        /// Analyzers to run: loudness, silences, scenes, transcript, pose, matte. Without
        /// names, every one the asset allows except those that need a model (transcript,
        /// pose, matte).
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
        /// Matte: `person` (fast, from the pose model) or `subject` (any subject, through
        /// BiRefNet, several seconds a frame).
        #[arg(long)]
        matte_model: Option<String>,
        /// Matte: measure every Nth frame and follow the subject between them (default 1).
        #[arg(long)]
        matte_every: Option<u32>,
        /// Matte: the size mattes are kept at, in pixels on the long side (default 512).
        #[arg(long)]
        matte_size: Option<u32>,
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
    /// Fill the canvas around video or image clips whose picture does not cover it, such
    /// as a 16:9 shot in a vertical video: with their own picture blurred, or a colour.
    Canvas {
        /// Clip ids (repeat the option).
        #[arg(long = "clip", required = true)]
        clips: Vec<String>,
        /// Their own picture, blurred to cover the canvas, by this much (0 to 0.2 of the
        /// picture's height; 0.05 when given without a value).
        #[arg(long, num_args = 0..=1, default_missing_value = "0.05", conflicts_with_all = ["color", "none"])]
        blur: Option<f64>,
        /// A colour as #RRGGBB.
        #[arg(long, conflicts_with = "none")]
        color: Option<String>,
        /// Remove the fill.
        #[arg(long)]
        none: bool,
    },
    /// Split a video clip at its scene changes, so each shot becomes a clip of its own.
    /// The shot cuts are measured from the media once and cached.
    SplitScenes {
        /// Clip id.
        #[arg(long)]
        clip: String,
    },
    /// Steady a shaky video clip: measures how the camera moved and keys the clip's
    /// position, rotation and scale to cancel the shake, with one zoom (up to 1.5) that
    /// hides the moving edges. Replaces those properties' keyframes.
    Stabilize {
        /// Clip id.
        #[arg(long)]
        clip: String,
        /// How much motion to remove, from 0 (none) to 1 (very smooth); up to 3 for a
        /// locked-off look.
        #[arg(long, default_value_t = fosscut_engine::motion::DEFAULT_STRENGTH)]
        strength: f64,
    },
    /// Follow a region of a video clip's picture from a time on: one of the clip's masks
    /// (mask tracking), or another clip such as a title or sticker (motion tracking), moves
    /// with it. Replaces the keyframes of the mask's centre or the other clip's position.
    Track {
        /// The clip whose picture is followed.
        #[arg(long)]
        clip: String,
        /// The region at --at as X,Y,WIDTH,HEIGHT: its centre and size as fractions of the
        /// picture, for example 0.5,0.4,0.2,0.3.
        #[arg(long, value_delimiter = ',', num_args = 1, required = true)]
        region: Vec<f64>,
        /// Where the region is given, on the timeline; the clip's start when omitted.
        #[arg(long, value_parser = parse_time)]
        at: Option<TimeInput>,
        /// A mask of the clip whose centre follows the region.
        #[arg(long, required_unless_present = "follow", conflicts_with = "follow")]
        mask: Option<String>,
        /// Another clip that moves as the region does, from where it is.
        #[arg(long)]
        follow: Option<String>,
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
        /// Effect type: for pictures color.adjust, color.curves, color.hsl, color.wheels,
        /// color.lut, color.vignette, blur.gaussian, detail.sharpen, stylize.grain and
        /// others; for sound audio.denoise, audio.enhance_voice or audio.vocals.
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
    /// Translate a caption track's pages with a local model (English and Hungarian), as a
    /// second line under each page (bilingual) or in place of its words (replace).
    Translate {
        /// The language to translate into: hu or en.
        #[arg(long)]
        to: String,
        /// bilingual (default) or replace.
        #[arg(long, default_value = "bilingual")]
        mode: String,
        /// The caption track; the first one when omitted.
        #[arg(long)]
        track: Option<String>,
        /// The timeline; the primary one when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Speak each caption page with a local English voice, one clip per page at its start
    /// on an audio track: the page's words, or with --translation its translation.
    Dub {
        /// The voice, such as af_heart.
        #[arg(long)]
        voice: Option<String>,
        /// Speaking speed from 0.5 to 2 (default 1); pages that run long are sped up a little.
        #[arg(long)]
        speed: Option<f32>,
        /// Speak each page's translation instead of its words.
        #[arg(long)]
        translation: bool,
        /// The caption track; the first one when omitted.
        #[arg(long)]
        track: Option<String>,
        /// The audio track for the speech; a new one when omitted.
        #[arg(long)]
        audio_track: Option<String>,
        /// The timeline; the primary one when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
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
        /// Leave out words the transcript marks as fillers ("um", "uh").
        #[arg(long)]
        skip_fillers: bool,
        /// Caption a song's lyrics: the music track first, one sung line per page, the
        /// karaoke look unless --preset says otherwise.
        #[arg(long)]
        lyrics: bool,
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
        /// What the cues hold: original, translation, or both (bilingual).
        #[arg(long, default_value = "original")]
        subtitles: String,
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
        /// The page in a second language, shown under it; "" removes it.
        #[arg(long)]
        translation: Option<String>,
    },
}

/// How a text appears, as `text add` and `text set` take it.
#[derive(Debug, Clone, clap::Args)]
pub struct RevealArgs {
    /// Reveal the text one unit at a time: letter, word or line.
    #[arg(long)]
    pub reveal: Option<String>,
    /// How each unit appears: appear (at once), fade or rise.
    #[arg(long, requires = "reveal")]
    pub reveal_motion: Option<String>,
    /// Time until the whole text shows; 1 second when omitted.
    #[arg(long, value_parser = parse_time, requires = "reveal")]
    pub reveal_duration: Option<TimeInput>,
    /// Also leave the same way before the clip ends.
    #[arg(long, requires = "reveal")]
    pub reveal_exit: bool,
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
        /// Font family: Inter, or a family of an imported font file.
        #[arg(long)]
        font: Option<String>,
        /// A text preset: one saved in the project or a built-in one (outline, shadow,
        /// boxed, highlight, headline, spaced, neon, glow, retro, comic).
        #[arg(long)]
        preset: Option<String>,
        /// Style changes as a JSON merge patch (text, @file or - for stdin), for example
        /// '{"stroke": {"enabled": true}, "letter_spacing": 0.05}'.
        #[arg(long)]
        style: Option<String>,
        /// Parts styled differently as JSON, for example
        /// '[{"start": 0, "end": 4, "color": "#FFCC00"}]'.
        #[arg(long)]
        spans: Option<String>,
        #[command(flatten)]
        reveal: RevealArgs,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Change a text clip's text, style, spans or reveal.
    Set {
        /// The text clip, for example clip_0004.
        clip: String,
        /// New text; use \n for line breaks.
        #[arg(long)]
        text: Option<String>,
        /// A preset whose style replaces the current one before --style applies.
        #[arg(long)]
        preset: Option<String>,
        /// Style changes as a JSON merge patch; null resets a field.
        #[arg(long)]
        style: Option<String>,
        /// New spans as JSON; [] removes them.
        #[arg(long)]
        spans: Option<String>,
        #[command(flatten)]
        reveal: RevealArgs,
        /// Show the text at once, without a reveal.
        #[arg(long, conflicts_with = "reveal")]
        no_reveal: bool,
    },
    /// Save a text clip's style, or a given style, as a preset of the project.
    SavePreset {
        /// The preset's name.
        name: String,
        /// The text clip whose style to save.
        #[arg(long, conflicts_with = "style")]
        clip: Option<String>,
        /// Or the style as a JSON merge patch over the default style.
        #[arg(long)]
        style: Option<String>,
        /// Overwrite a saved preset of this name.
        #[arg(long)]
        replace: bool,
    },
    /// Remove a text preset saved in the project.
    RemovePreset {
        /// The preset's name.
        name: String,
    },
    /// Add a title template (lower-third, headline, chapter, quote, callout, end-card).
    Template {
        /// The template.
        template: String,
        /// The text of each line, in order (repeat the option); placeholders fill the rest.
        #[arg(long = "line")]
        lines: Vec<String>,
        /// Text track id; the first text track when omitted.
        #[arg(long)]
        track: Option<String>,
        /// Timeline position; appended when omitted.
        #[arg(long, value_parser = parse_time)]
        start: Option<TimeInput>,
        /// Length; 4 seconds when omitted.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// Style changes as a JSON merge patch.
        #[arg(long)]
        style: Option<String>,
        /// Timeline id; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
}

/// `fosscut library ...`
#[derive(Debug, Subcommand)]
pub enum LibraryCommand {
    /// List the library, or search it.
    List {
        /// Only one kind: shape, sticker, background, template, title, text_style,
        /// caption_style, look, sound or music.
        #[arg(long)]
        kind: Option<String>,
        /// Words to find in names, descriptions and tags.
        #[arg(long)]
        search: Option<String>,
    },
    /// Add an item to the project: a sound or music loop is imported and placed, a shape,
    /// sticker or background drawn, a title or template added, a style or look applied.
    Add {
        /// The item's name.
        name: String,
        /// Its kind, when the name is used by several.
        #[arg(long)]
        kind: Option<String>,
        /// Where it starts.
        #[arg(long, value_parser = parse_time)]
        at: Option<TimeInput>,
        /// How long it lasts.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// The track to put it on.
        #[arg(long)]
        track: Option<String>,
        /// The clip a look applies to.
        #[arg(long)]
        clip: Option<String>,
        /// Lines for titles and templates (repeat the option).
        #[arg(long = "line")]
        lines: Vec<String>,
        /// Assets for a template's slots (repeat the option).
        #[arg(long = "media")]
        media: Vec<String>,
    },
}

/// `fosscut audio ...`
#[derive(Debug, Subcommand)]
pub enum AudioCommand {
    /// Measure clips' loudness as they play and set their volumes to reach a target
    /// (ITU-R BS.1770), keeping peaks under a ceiling; where the peaks leave too little
    /// room, or the clip needs more than the volume's +20 dB, an audio.limiter makes up the
    /// rest.
    Normalize {
        /// Clip ids (repeat the option).
        #[arg(long = "clip", required = true)]
        clips: Vec<String>,
        /// Target loudness in LUFS; -14 when omitted.
        #[arg(long, allow_hyphen_values = true)]
        target: Option<f64>,
        /// Highest sample peak in dBFS, from -12 to 0; -1 when omitted.
        #[arg(long, allow_hyphen_values = true)]
        ceiling: Option<f64>,
        /// Only change volumes, never add a limiter: clips with loud peaks may stay
        /// quieter than the target.
        #[arg(long)]
        no_limit: bool,
    },
}

/// `fosscut cache ...`
#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Each cache's folder, size and number of entries.
    Info,
    /// Empty caches: analysis, proxies, remux or frames (repeat it); all when omitted.
    /// Everything in them is made again when needed.
    Clear {
        /// The caches to empty.
        kinds: Vec<String>,
    },
    /// Remove entries unused for longer than `cache.max_age_days`, then the oldest beyond
    /// `cache.limit_gb`, as hosts do when they start.
    Clean,
}

/// `fosscut preferences ...`
#[derive(Debug, Subcommand)]
pub enum PreferencesCommand {
    /// One preference, or a group, by dotted name such as `editing.frame_step`.
    Get {
        /// The preference.
        key: String,
    },
    /// Change a preference: `fosscut preferences set performance.proxies auto`.
    Set {
        /// The preference.
        key: String,
        /// The value: JSON, or a word taken as text.
        value: String,
    },
    /// Return a preference to its default.
    Reset {
        /// The preference.
        key: String,
    },
    /// Where the preferences file is.
    Path,
}

/// `fosscut multicam ...`
#[derive(Debug, Subcommand)]
pub enum MulticamCommand {
    /// Make a multicam clip from angles recorded at the same time: each angle on a track of
    /// its own in a new compound timeline, lined up by their offsets (measured from their
    /// sound unless --offset is given), with one angle's sound.
    Create {
        /// The angles' asset ids, at least two, in order; the first shows at the start.
        #[arg(required = true, num_args = 2..)]
        angles: Vec<String>,
        /// Each angle's start in the scene, in order (repeat the option): an angle that
        /// started later has a larger offset. Measured from the sound when omitted.
        #[arg(long = "offset", value_parser = parse_time)]
        offsets: Vec<TimeInput>,
        /// Look at most this many seconds either way when measuring offsets; any offset
        /// when omitted.
        #[arg(long)]
        max_offset: Option<f64>,
        /// Do not measure offsets: line the angles up at their starts.
        #[arg(long, conflicts_with = "offsets")]
        no_sync: bool,
        /// Which angle's sound plays, by number from 1; the first when omitted.
        #[arg(long)]
        sound: Option<usize>,
        /// The multicam timeline's name.
        #[arg(long)]
        name: Option<String>,
        /// Where the multicam clip starts; appended when omitted.
        #[arg(long, value_parser = parse_time)]
        start: Option<TimeInput>,
        /// Track for the multicam clip.
        #[arg(long)]
        track: Option<String>,
        /// Timeline for the multicam clip; the primary timeline when omitted.
        #[arg(long)]
        timeline: Option<String>,
    },
    /// Cut a multicam clip to another angle from a time on.
    Switch {
        /// The multicam clip, or a part of it.
        #[arg(long)]
        clip: String,
        /// Timeline position of the cut; the part's start switches the whole part.
        #[arg(long, value_parser = parse_time)]
        at: TimeInput,
        /// The angle: its number from 1, or its track id.
        #[arg(long)]
        angle: String,
    },
    /// Measure how far apart angles recorded at the same time started, from their sound,
    /// without changing the project.
    Sync {
        /// The angles' asset ids, at least two; offsets are relative to the first.
        #[arg(required = true, num_args = 2..)]
        angles: Vec<String>,
        /// Look at most this many seconds either way; any offset when omitted.
        #[arg(long)]
        max_offset: Option<f64>,
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
