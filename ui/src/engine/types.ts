// The engine's data as the UI receives it: the JSON of FossCut's queries, commands and
// events (`fosscut capabilities --detail full` has the schemas).

/** A moment or length, reported in every unit. */
export interface Moment {
  flicks: number;
  frames: number;
  seconds: string;
  timecode: string;
}

export interface Format {
  width: number;
  height: number;
  /** Frames per second as a rational, for example "30" or "30000/1001". */
  rate: string;
}

export interface TimelineSummary {
  id: string;
  name: string;
  /** `compound` for the inside of compound clips; omitted for edits of their own. */
  role?: "sequence" | "compound";
  format: Format;
  duration: Moment;
  tracks: number;
  clips: number;
}

/** `project.info`. */
export interface ProjectInfo {
  id: string;
  name: string;
  revision: number;
  can_undo: boolean;
  can_redo: boolean;
  primary_timeline: string;
  timelines: TimelineSummary[];
  assets: number;
  /** Every font family text can use, sorted, the bundled Inter among them. */
  fonts?: string[];
  /** Names of the text styles saved in the project (`text.preset_save`). */
  text_presets?: string[];
}

/** A recent project in the browser. */
export interface ProjectSummary {
  path: string;
  name: string;
  duration: Moment;
  timelines: number;
  assets: number;
  /** The bundle's size on disk in bytes. */
  bytes: number;
  modified_ms: number;
  /** A PNG data URL of a frame, when one could be rendered. */
  thumbnail?: string;
}

/** A project in the trash. */
export interface TrashEntry {
  id: string;
  name: string;
  original: string;
  trashed_ms: number;
}

/** One asset of `media.list`. */
export interface AssetView {
  id: string;
  /** `vector` is an SVG drawing (ADR-0036), `web` a web composition (ADR-0039), `model` a 3D model. */
  kind: "video" | "audio" | "image" | "lut" | "font" | "vector" | "web" | "model";
  name: string;
  duration?: Moment | null;
  video?: { width: number; height: number } | null;
  audio?: unknown;
  /** The clips that show it. */
  used_by: string[];
  /** Its bin folder; the top level when absent. */
  folder?: string | null;
  /** True when its file cannot be found (the desktop marks it). */
  missing?: boolean;
  /** A font asset's families, for text styles. */
  families?: string[];
  /** A vector drawing's view box and how many paths it draws, each a shape when exploded. */
  vector?: { view_box: [number, number, number, number]; paths: number } | null;
}

/** `media.list`. */
export interface MediaListing {
  assets: AssetView[];
  folders?: string[];
}

/** The project the app has open. */
export interface Opened {
  bundle: string;
  info: ProjectInfo;
}

/** Placement on the canvas; omitted fields are at their defaults. */
export interface Transform {
  /** Offset of the clip's centre from the canvas centre, in fractions of its size. */
  position?: [number, number];
  /** Scale relative to the fitted size; 1 when omitted. */
  scale?: number;
  /** Extra width and height factors on top of the scale; [1, 1] when omitted. */
  stretch?: [number, number];
  /** Clockwise rotation in degrees. */
  rotation?: number;
  flip_horizontal?: boolean;
  flip_vertical?: boolean;
  /** Placed in the timeline's 3D scene, seen through its camera (ADR-0037). */
  three_d?: boolean;
  /** Depth as a fraction of the canvas width; positive is away from the viewer. */
  position_z?: number;
  /** Degrees it tilts around its horizontal axis; positive tips the top away. */
  rotation_x?: number;
  /** Degrees it turns around its vertical axis; positive turns the right edge away. */
  rotation_y?: number;
  /** The pivot of rotation and scale, as fractions of the picture from its centre. */
  anchor?: [number, number];
}

/** Edges removed from the picture, as fractions of its size; 0 when omitted. */
export interface Crop {
  left?: number;
  right?: number;
  top?: number;
  bottom?: number;
}

/** A clip's sound shaping; omitted fields are at their defaults. Fades are in flicks. */
export interface ClipAudio {
  fade_in?: number;
  fade_out?: number;
  pan?: number;
  channels?: "stereo" | "left" | "right" | "mono";
}

/** How a value moves from one keyframe to the next. */
export type Interpolation = "hold" | "linear" | { bezier: [number, number, number, number] };

/** A keyframe as `timeline.read` reports it. */
export interface KeyframeView {
  /** Position on the timeline. */
  at: Moment;
  /** Position from the clip's start. */
  offset: Moment;
  value: number | [number, number];
  interpolation: Interpolation;
}

export interface ClipView {
  id: string;
  kind: string;
  start: Moment;
  end: Moment;
  duration: Moment;
  text?: string;
  /** A text clip's style, restyled parts and reveal (ADR-0027). */
  style?: TextStyleView;
  spans?: TextSpanView[];
  reveal?: TextRevealView;
  /** The asset a video, audio or image clip shows. */
  asset?: string;
  asset_name?: string;
  source_in?: Moment;
  transform?: Transform;
  crop?: Crop;
  opacity?: number;
  blend?: string;
  volume?: number;
  audio?: ClipAudio;
  /** False when the clip is deactivated; omitted when active. */
  enabled?: boolean;
  /** Keyframes by property path, when the clip has any. */
  keyframes?: Record<string, KeyframeView[]>;
  /** Effects on the picture, in the order they apply. */
  effects?: EffectView[];
  /** Masks on the picture, in the order they combine. */
  masks?: MaskView[];
  /** Width and height of the picture as displayed (video and image clips). */
  picture?: [number, number];
  /** A compound clip's timeline, and its name (ADR-0025). */
  timeline?: string;
  timeline_name?: string;
  /** A multicam clip's angle (ADR-0029): the track of its timeline whose picture shows. */
  angle?: string;
  /** A multicam clip's angles, numbered from 1 in this order as `multicam.switch` takes them. */
  angles?: AngleView[];
  /** The group of clips it edits together with (ADR-0024). */
  group?: string;
  /** A video clip whose sound plays from an extracted audio clip. */
  audio_detached?: boolean;
  /** How the clip plays its media (ADR-0023); absent at normal speed. */
  speed?: SpeedView;
  /** How much media a clip with speed plays. */
  source_used?: Moment;
  /** The transition into the next clip on the track. */
  transition?: { type: string; duration: Moment; params?: Record<string, unknown> };
  /** How the clip moves in, out and while it shows; durations and delays in flicks. */
  animations?: {
    in?: AnimationView;
    out?: AnimationView;
    combo?: AnimationView;
  };
  /** A shape clip's outline (ADR-0028); its label's words are in `text`. */
  shape?: ShapeKind;
  /**
   * The whole content (detail full): a shape clip's shape, a particle clip's emitter
   * (ADR-0038) or a 3D object clip's object. Read it through the panel of its kind.
   */
  content?: ShapeContentView | ParticleContentView | ModelContentView;
  /** The shape clip whose path this clip follows (ADR-0036). */
  follow?: FollowView;
  /** A camera clip's camera (ADR-0037, detail full). */
  camera?: CameraView;
  /** A particle clip's sprite kind and rate, at every detail. */
  sprite?: SpriteKind;
  rate?: number;
  /** A 3D object clip's kind of object (`box`, `sphere`, `asset` and so on). */
  object?: string;
  /** What fills the canvas around a video or image clip's picture. */
  canvas?: CanvasFill;
}

/** One of a clip's animations: a preset, how long it takes and how long it waits, in flicks. */
export interface AnimationView {
  preset: string;
  duration: number;
  delay?: number;
}

/** Where a clip rides along a shape clip's path (ADR-0036). */
export interface FollowView {
  /** The shape clip. */
  clip: string;
  /** How far along, as a fraction of the path's length. */
  progress?: number;
  /** Turns with the path's direction. */
  orient?: boolean;
  /** Added to the point on the path, as fractions of the canvas. */
  offset?: [number, number];
}

/** A camera clip's camera as the engine stores it: sparse, omitted fields at their defaults. */
export interface CameraView {
  field_of_view?: number;
  position?: [number, number];
  position_z?: number;
  point_of_interest?: [number, number];
  point_of_interest_z?: number;
  orbit?: [number, number];
  roll?: number;
  depth_of_field?: boolean;
  focus_offset?: number;
  aperture?: number;
  max_blur?: number;
}

/** The kinds of particle sprite. */
export type SpriteKind = "dot" | "spark" | "star" | "ring" | "square" | "image";

/** A particle emitter as the engine stores it (ADR-0038): sparse, omitted fields at their defaults. */
export interface ParticleContentView {
  emitter?: {
    shape?: "point" | "line" | "circle" | "rectangle";
    position?: [number, number];
    size?: [number, number];
    angle?: number;
    edge?: boolean;
  };
  rate?: number;
  bursts?: { at: number; count: number }[];
  prewarm?: number;
  offset?: number;
  life?: number;
  life_variance?: number;
  speed?: number;
  speed_variance?: number;
  direction?: number;
  spread?: number;
  aim?: "direction" | "outward" | "inward" | "converge";
  gravity?: number;
  wind?: number;
  drag?: number;
  turbulence?: number;
  turbulence_scale?: number;
  rotation?: number;
  rotation_variance?: number;
  spin?: number;
  spin_variance?: number;
  size?: number;
  size_variance?: number;
  size_over_life?: number[];
  opacity?: number;
  opacity_over_life?: number[];
  colors?: string[];
  color_by?: "life" | "random";
  blend?: "add" | "normal";
  sprite?: { kind?: SpriteKind; softness?: number; stretch?: number; points?: number; aspect?: number; flip?: number; asset?: string };
  seed?: number;
}

/** A 3D object clip's object as the engine stores it (ADR-0040); read by its panel. */
export interface ModelContentView {
  object?: { kind?: string } & Record<string, unknown>;
  material?: Record<string, unknown>;
  light?: Record<string, unknown>;
  [field: string]: unknown;
}

/** The outlines a shape clip can have. */
export type ShapeKind = "rectangle" | "ellipse" | "star" | "heart" | "path";

/** How a stroke ends and turns at corners. */
export type LineCap = "butt" | "round" | "square";
export type LineJoin = "miter" | "round" | "bevel";

/** A shape's stroke (ADR-0036): widths and dashes are fractions of the canvas height. */
export interface ShapeStrokeView {
  color?: string;
  gradient?: { to: string; angle?: number; radial?: boolean };
  width?: number;
  cap?: LineCap;
  join?: LineJoin;
  miter_limit?: number;
  dashes?: number[];
  dash_offset?: number;
}

/**
 * A shape clip's shape as the engine stores it: sparse, so omitted fields are at their
 * defaults. Sizes are fractions of the canvas; colours are `#RRGGBB` or `#RRGGBBAA`.
 */
export interface ShapeContentView {
  shape: ShapeKind;
  size: [number, number];
  /** Corner rounding of a rectangle, 0 to 1. */
  roundness?: number;
  points?: PathPointView[];
  /** False leaves a path open: a line (ADR-0036). */
  closed?: boolean;
  /** More subpaths of a path, in the same box. */
  contours?: { points: PathPointView[]; closed?: boolean }[];
  fill_rule?: "even_odd" | "nonzero";
  /** Null for no fill, as lines have; white when omitted. */
  fill?: {
    color?: string;
    /** A gradient to a second colour; `angle` in degrees, 0 left to right, 90 top to bottom. */
    gradient?: { to: string; angle?: number; radial?: boolean };
  } | null;
  /** A stroke centred on its edge, or along an open path. */
  stroke?: ShapeStrokeView;
  /** The visible part of the outline, 0 to 1 of its length; `trim_offset` wraps it round. */
  trim_start?: number;
  trim_end?: number;
  trim_offset?: number;
  /** An outline inside the edge; `width` is a fraction of the shorter side. */
  outline?: { enabled: boolean; color?: string; width?: number };
  /** Text on it, styled as a text clip's. */
  label?: { text: string; style?: TextStyleView; spans?: TextSpanView[] };
  /** Where the label's centre sits in the shape's box, -1 to 1 on each axis. */
  label_at?: [number, number];
}

/** What the built-in library holds (ADR-0028). */
export type LibraryKind =
  | "shape"
  | "sticker"
  | "background"
  | "template"
  | "title"
  | "text_style"
  | "caption_style"
  | "look"
  | "particles"
  | "sound"
  | "music";

/** One item of the built-in library, made by FossCut. */
export interface LibraryItem {
  /** Stable name, unique within its kind. */
  name: string;
  kind: LibraryKind;
  title: string;
  description: string;
  tags: string[];
  /** A sound's length in seconds. */
  seconds?: number;
  /** A music loop's tempo. */
  bpm?: number;
  /** Free to use in commercial work: true for everything FossCut makes. */
  commercial: boolean;
}

/** What to add from the library, and where (`library_add`). */
export interface LibraryAddRequest {
  name: string;
  /** For names that repeat across kinds. */
  kind?: LibraryKind;
  /** Where it starts on the timeline. */
  start_flicks?: number;
  track?: string;
  /** The clip a look applies to. */
  clip?: string;
  /** Lines for titles and templates; a text style's text. */
  lines?: string[];
  /** Video or image assets for a template's slots, in order. */
  media?: string[];
  expected_revision?: number;
}

/** An export preset and whether this machine can encode it. */
export interface ExportPreset {
  name: string;
  description: string;
  /** The usual file extension, without the dot. */
  extension: string;
  available: boolean;
}

/** `export_presets`: the presets, and the resolutions export offers by name. */
export interface ExportPresets {
  presets: ExportPreset[];
  /** Named resolutions as shorter sides in pixels, such as 720p: 720. */
  resolutions: { name: string; pixels: number }[];
}

/** How to export the primary timeline (`export_start`); texts as the CLI takes them. */
export interface ExportChoice {
  /** The file to write, as a whole path. */
  output: string;
  preset?: string;
  /** `720p`, `4k` or a number of pixels for the shorter side; the timeline's when omitted. */
  resolution?: string;
  /** `30`, `29.97`; the timeline's when omitted. */
  fps?: string;
  /** `8M`; the encoder's choice when omitted. */
  bitrate?: string;
  /** Only this part of the timeline, start and end in flicks. */
  range_flicks?: [number, number];
  /** A cover image of the frame this long after the export's start, in flicks. */
  cover_flicks?: number;
  subtitles?: "srt" | "vtt";
  /** Replace the output if it exists. */
  overwrite?: boolean;
}

/** Where a background job stands. */
export type JobState = "running" | "succeeded" | "failed" | "cancelled";

/** Where a file the export uses comes from, and what that means for publishing it. */
export interface RightsNote {
  asset: string;
  name: string;
  /** `library` (made by FossCut, free to use) or `yours`. */
  origin: string;
  note: string;
}

/** A verified export. Durations are in flicks. */
export interface ExportReport {
  output: string;
  settings: {
    container: string;
    video?: { codec: string; width: number; height: number; rate: string; bit_rate?: number | null } | null;
    audio?: { codec: string; sample_rate: number; channels: number; bit_rate: number } | null;
  };
  frames: number;
  duration: number;
  encoders: { video?: string | null; audio?: string | null };
  /** What the file contains, as probed after writing. */
  verified: {
    container: string;
    size_bytes: number;
    duration?: number | null;
    video?: { codec: string; width: number; height: number; rate?: string | null } | null;
    audio?: { codec: string; sample_rate?: number; channels?: number } | null;
  };
  elapsed_ms: number;
  replayed?: boolean;
  /** The cover image written beside the file. */
  cover?: string;
  /** The subtitle file written beside it. */
  subtitles?: string;
  rights?: RightsNote[];
}

/** An export job's state, progress and outcome. */
export interface JobStatus {
  job: string;
  kind: string;
  state: JobState;
  /** Frames (or seconds of sound) done and in total; zero until rendering starts. */
  progress: { done: number; total: number };
  elapsed_ms: number;
  output?: string;
  /** The verified export, once it succeeded. */
  report?: ExportReport;
  /** The typed error, once it failed or was cancelled. */
  error?: ApiError;
  /** The asset an analysis reads. */
  asset?: string;
  /** A Remotion render once it succeeded: the file and the asset it became (ADR-0039). */
  remotion?: { output: string; composition: string; command: string[]; asset?: string };
}

/** What a capture device records, and what a recording records from. */
export type DeviceKind = "microphone" | "camera" | "screen";

/** A capture device on this machine. */
export interface CaptureDevice {
  kind: DeviceKind;
  /** The name people know it by, such as `FaceTime HD Camera`. */
  name: string;
  /** The FFmpeg input format and what it calls the device. */
  format: string;
  input: string;
}

/** What to record (`record_start`). */
export interface RecordChoice {
  what: DeviceKind;
  /** The device by name; the first of its kind when omitted. */
  device?: string;
  /** For a camera or screen, the microphone recorded with it by name, or `none`. */
  microphone?: string;
  /** For a camera or screen, `WIDTHxHEIGHT`. */
  size?: string;
  /** For a camera or screen, the frame rate. */
  fps?: string;
  /** Where the recording goes on the timeline once it is imported, in flicks. */
  at?: number;
}

/** A finished recording: the file and what it was recorded from. Durations in flicks. */
export interface RecordingReport {
  output: string;
  what: DeviceKind;
  device?: string;
  microphone?: string;
  preset: string;
  duration: number;
  /** True when it was stopped rather than ending by itself. */
  stopped: boolean;
  elapsed_ms: number;
}

/**
 * A recording's status (`progress.done` is the whole seconds recorded), and once it
 * succeeded its report and the asset and clip it became.
 */
export interface RecordState extends JobStatus {
  recording?: RecordingReport;
  asset?: string;
  clip?: string;
  /** Why the file could not be imported or placed; it stays in the project's folder. */
  import_error?: ApiError;
}

export type MaskShape = "rectangle" | "ellipse" | "linear" | "mirror" | "star" | "heart" | "path" | "text" | "brush";

/** A path point in the mask's box, -1 to 1, with optional Bezier handles. */
export interface PathPointView {
  at: [number, number];
  in?: [number, number];
  out?: [number, number];
}

/** A painted stroke, in the picture from 0 to 1. */
export interface BrushStrokeView {
  points: [number, number][];
  radius: number;
  erase?: boolean;
}

/** A mask as `timeline.read` (detail full) reports it; omitted fields are defaults. */
export interface MaskView {
  id: string;
  shape?: MaskShape;
  enabled?: boolean;
  center?: [number, number];
  size?: [number, number];
  rotation?: number;
  feather?: number;
  expansion?: number;
  roundness?: number;
  invert?: boolean;
  mode?: "add" | "subtract" | "intersect";
  points?: PathPointView[];
  text?: string;
  strokes?: BrushStrokeView[];
  keyframes?: Record<string, KeyframeView[]>;
}

/** An effect as `timeline.read` (detail full) reports it. */
export interface EffectView {
  id: string;
  type: string;
  /** False when bypassed; omitted when on. */
  enabled?: boolean;
  version?: number;
  /** Parameters that differ from their defaults. */
  params?: Record<string, unknown>;
  /** Keyframes by parameter name, with timeline times. */
  keyframes?: Record<string, KeyframeView[]>;
}

/** One parameter of an effect type, as `capabilities` describes it. */
export interface ParamSpec {
  name: string;
  kind: "number" | "pair" | "bool" | "choice" | "curve" | "asset" | "color";
  default: unknown;
  min?: number;
  max?: number;
  unit?: string;
  choices?: string[];
  animatable: boolean;
  meaning: string;
}

/** An effect type and its parameters. */
export interface EffectSpec {
  type: string;
  version: number;
  title: string;
  category: string;
  summary: string;
  params: ParamSpec[];
}

/** A built-in look. */
export interface LookView {
  name: string;
  title: string;
  description: string;
}

/** A clip's speed as `timeline.read` reports it. */
export interface SpeedView {
  /** Constant rate; 0 is a freeze. Omitted (1) with a curve. */
  rate?: number;
  curve?: { at: number; rate: number }[];
  reverse?: boolean;
  pitch?: "preserve" | "shift";
  interpolate?: FrameBlend;
  /** For curves, the rates at the clip's start and end. */
  start_rate?: number;
  end_rate?: number;
}

/** A built-in speed curve. */
export interface SpeedPreset {
  preset: string;
  meaning: string;
}

/** A transition type. */
export interface TransitionSpec {
  type: string;
  title: string;
  summary: string;
  params: ParamSpec[];
}

/** An animation preset and where it applies ("in or out", or "combo"). */
export interface AnimationPreset {
  preset: string;
  applies: string;
  meaning: string;
}

/** What the UI needs from `capabilities` (detail full). */
/** How slowed footage makes the frames between its own: repeats, cross-fades or optical flow. */
export type FrameBlend = "nearest" | "blend" | "flow";

/** A preset of voice filters (`voice.apply`), as `capabilities` lists it. */
export interface VoicePreset {
  name: string;
  title: string;
  description: string;
}

export interface Catalog {
  /** Voice filter presets; empty when the engine lists none. */
  voice_presets?: VoicePreset[];
  /** What particle emitters can be (ADR-0038), when the engine says. */
  particles?: { emitters?: string[]; aims?: string[]; sprites?: string[]; blends?: string[]; color_by?: string[] };
  /** Particle presets, from the library's listing in `capabilities`. */
  library?: {
    particles?: { name: string; title: string; description?: string; tags?: string[] }[];
    /** 3D object presets (ADR-0040), added with `model.add`. */
    models?: { name: string; title: string; description?: string; tags?: string[] }[];
  };
  effects: EffectSpec[];
  looks: LookView[];
  transitions: TransitionSpec[];
  animations: AnimationPreset[];
  speed_presets: SpeedPreset[];
  text: TextCatalog;
}

export type TextCase = "as_written" | "upper" | "lower" | "title";
export type TextAlign = "left" | "center" | "right";

/**
 * A text style as the engine stores it: sparse, so omitted fields are at their defaults.
 * Lengths are in em (fractions of the font size); `size` is a fraction of the canvas
 * height; colours are `#RRGGBB` or `#RRGGBBAA`.
 */
export interface TextStyleView {
  font?: string;
  size?: number;
  color?: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  case?: TextCase;
  align?: TextAlign;
  letter_spacing?: number;
  line_height?: number;
  stroke?: { enabled: boolean; color?: string; width?: number };
  box?: { enabled: boolean; color?: string; padding?: number; radius?: number };
  shadow?: { enabled: boolean; color?: string; offset?: [number, number]; blur?: number };
  glow?: { enabled: boolean; color?: string; radius?: number };
}

/** Part of a text styled differently, by character positions (`end` excluded). */
export interface TextSpanView {
  start: number;
  end: number;
  color?: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  font?: string;
  size?: number;
}

export type RevealUnit = "letter" | "word" | "line";
export type RevealMotion =
  | "appear"
  | "fade"
  | "rise"
  | "blur"
  | "pop"
  | "zoom"
  | "slide_left"
  | "slide_right"
  | "drop"
  | "collapse";
/** How each part of a reveal eases; a Bezier curve as `{ bezier: [x1, y1, x2, y2] }`. */
export type RevealEasing = "linear" | "ease_in" | "ease_out" | "ease_in_out" | "back" | "bounce" | "elastic" | { bezier: [number, number, number, number] };
export type RevealOrder = "forward" | "backward" | "center" | "edges" | "random";

/** How a text appears, as `timeline.read` reports it. */
export interface TextRevealView {
  unit: RevealUnit;
  motion?: RevealMotion;
  duration: Moment;
  exit?: boolean;
  /** How much each part's motion overlaps the next one's, 0 to 1 (0.5 when omitted). */
  overlap?: number;
  /** The easing; each motion's own when omitted. */
  easing?: RevealEasing;
  order?: RevealOrder;
  /** How it leaves, when it exits; its entrance motion when omitted. */
  exit_motion?: RevealMotion;
}

/** A built-in text style. */
export interface TextPresetView {
  name: string;
  description: string;
  style?: TextStyleView;
}

/** A built-in title template and the placeholders of its lines. */
export interface TitleTemplateView {
  name: string;
  description: string;
  lines: string[];
}

/** What `capabilities` says about text. */
export interface TextCatalog {
  fonts: string[];
  presets: TextPresetView[];
  templates: TitleTemplateView[];
  reveal: { units: RevealUnit[]; motions: RevealMotion[] };
  cases: TextCase[];
}

/** A color correction measured by the engine, ready for `effect.add`. */
export interface ColorSuggestion {
  effect_add: Record<string, unknown>;
}

/** One clip's loudness and the volume that levels it. */
export interface Leveled {
  clip: string;
  /** Integrated loudness at volume 1, in LUFS; null when silent or under 400 ms. */
  measured_lufs: number | null;
  /** Sample peak at volume 1, in dBFS. */
  peak_dbfs: number | null;
  gain_db: number;
  /** The clip's new volume. */
  volume: number;
  /** True when the peak ceiling or the gain range held the gain back. */
  limited: boolean;
}

/** Loudness measured by the engine, with the `clip.set` calls that level it. */
export interface Normalization {
  target_lufs: number;
  ceiling_dbfs: number;
  clips: Leveled[];
  /** One `clip.set` per measured clip, to apply as one edit. */
  commands: CommandCall[];
}

/** An angle of a multicam clip: a video track of its timeline. */
export interface AngleView {
  id: string;
  name?: string | null;
}

/** A region of a clip's picture: centre and size as fractions of the picture as displayed. */
export interface Region {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Shot cuts found in a clip, in timeline flicks, and the response of splitting there. */
export interface SceneSplit {
  scenes: { clip: string; cuts: number[] };
  /** Null when the clip has no cuts, so nothing was split. */
  response: EditResponse | null;
}

/** A timeline range where tracking lost the region and held its last place, in flicks. */
export interface LostRange {
  start: number;
  end: number;
}

/** Motion measured in a clip and applied as keyframes: by stabilizing or tracking. */
export interface MotionResult {
  motion: {
    /** The clip keyed: the one stabilized, or the mask's clip or follower when tracking. */
    clip: string;
    frames: number;
    /** Stabilizing: the zoom that hides the moving edges, from 1 to 1.5. */
    zoom?: number;
    /** Tracking: where the region was lost. */
    lost?: LostRange[];
  };
  response: EditResponse | null;
}

/** What to track and what follows it (`clip_track`): exactly one of `mask` and `follow`. */
export interface TrackRequest {
  clip: string;
  region: Region;
  /** The timeline time the region was drawn at, inside the clip, in flicks. */
  at: number;
  /** A mask of the clip, whose centre then follows the region. */
  mask?: string;
  /** Another clip, such as a title or sticker, which then moves as the region does. */
  follow?: string;
  expected_revision?: number;
}

/** How to make a multicam clip (`multicam_create`). */
export interface MulticamRequest {
  /** Video assets recorded at the same time, at least two. */
  angles: string[];
  /** Line the angles up by their sound; they all start together when false. */
  sync: boolean;
  /** Whose sound plays, by angle number from 1; the first when omitted. */
  sound?: number;
  name?: string;
  /** Where the clip starts, in flicks. */
  start?: number;
  expected_revision?: number;
}

/** Offsets measured from the angles' sound, in flicks, and how sure each match is (0 to 1). */
export interface SyncOffsets {
  offsets: number[];
  confidence: number[];
}

/** What making a multicam clip measured and did. */
export interface MulticamResult {
  /** Null when the angles were not synced by sound. */
  sync: SyncOffsets | null;
  response: EditResponse;
}

export interface TrackView {
  id: string;
  kind: string;
  index: number;
  name?: string;
  clips: ClipView[];
}

/** `timeline.read` with `detail: "full"`. */
export interface TimelineView {
  timeline: string;
  name: string;
  revision: number;
  format: Format;
  duration: Moment;
  tracks: TrackView[];
  /** Points to come back to, in time order; omitted when there are none. */
  markers?: MarkerView[];
}

/** A timeline marker. */
export interface MarkerView {
  id: string;
  at: Moment;
  name?: string;
  /** `#RRGGBB`; the default marker color when omitted. */
  color?: string;
}

/** One entry of `history.list`. */
export interface LogEntry {
  seq: number;
  kind: "commit" | "undo" | "redo";
  actor: string;
  at_ms?: number;
  commands: string[];
  reason?: string | null;
  revision_after: number;
  target_seq?: number;
  undone?: boolean;
}

export interface History {
  entries: LogEntry[];
}

/** A commit, as the app hears of it, whoever made it. */
export interface Change {
  revision: number;
  seq: number;
  kind: "commit" | "undo" | "redo";
  actor: string;
  commands: string[];
}

/** An agent or other front end connected to the open project. */
export interface Client {
  id: number;
  name: string;
  since_ms: number;
}

/** Where the preview's playhead is. */
export interface Playhead {
  time: Moment;
  playing: boolean;
}

/** A FossCut error: a stable code, a message, and a hint for recovery. */
export interface ApiError {
  code: string;
  message: string;
  hint?: string;
  details?: unknown;
}

export interface CommandCall {
  command: string;
  args?: unknown;
}

/** A write: commands applied as one undoable step (ADR-0006). */
export interface EditRequest {
  commands: CommandCall[];
  expected_revision?: number;
  dry_run?: boolean;
  request_id?: string;
  allow_destructive?: boolean;
  reason?: string;
}

export interface EditResponse {
  committed: boolean;
  dry_run: boolean;
  replayed: boolean;
  revision: number;
  results: unknown[];
}

export interface AppInfo {
  name: string;
  version: string;
  runtime_dir: string;
}

/** True for a FossCut error object. */
export function isApiError(value: unknown): value is ApiError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as ApiError).code === "string" &&
    typeof (value as ApiError).message === "string"
  );
}

/** How the timeline, the transport and the inspector show times. */
export type TimeDisplay = "timecode" | "frames" | "seconds";

/** The person's preferences (ADR-0031), every one with its default where none is set. */
export interface Preferences {
  version: number;
  /** Folders for new projects, recordings and exports; null uses the default place. */
  locations: { projects: string | null; recordings: string | null; exports: string | null };
  cache: { max_age_days: number; limit_gb: number };
  editing: { frame_step: number; nudge_pixels: number; image_duration: number; loudness_target: number };
  timeline: { scrub_audio: boolean; linked: boolean; frame_rate: string; time_display: TimeDisplay };
  export: { sound: boolean; sound_name: string };
  performance: {
    hardware_decoding: "auto" | "off";
    hardware_encoding: boolean;
    proxies: "off" | "auto" | "on";
    proxy_size: "half" | "quarter";
    render_cache_gb: number;
    auto_render: boolean;
    auto_remux: boolean;
  };
  language: string;
  updates: "off" | "notify" | "check";
  notifications: boolean;
  shortcuts: { set: string; overrides: Record<string, string> };
}

/** What `preferences_get` and the calls that change preferences answer. */
export interface PreferencesView {
  /** The preferences file. */
  path: string;
  preferences: Preferences;
  defaults: Preferences;
  languages: string[];
  shortcut_sets: string[];
  /** Why the file could not be read, while the defaults stand in for it. */
  problem?: string;
}

/** What `update_check` answers: whether it asked the release server, and what it found. */
export interface UpdateAnswer {
  policy: Preferences["updates"];
  checked: boolean;
  current?: string;
  latest?: string;
  newer?: boolean;
  /** The release's page. */
  url?: string;
  published?: string;
  /** Why nothing was asked. */
  message?: string;
}

/** Frames of a video or image asset (`media_thumbnails`), as PNG data URLs. */
export interface ThumbnailStrip {
  asset: string;
  width: number;
  height: number;
  /** One frame per time asked for, at its source time in flicks. */
  frames: { flicks: number; image: string }[];
}

/** The peaks of an asset's sound (`media_waveform`): the lowest and highest sample of each of `buckets` equal spans. */
export interface Waveform {
  asset: string;
  /** The sound's length in flicks. */
  duration: number;
  buckets: number;
  /** `[min, max]` per span, from -1 to 1, over every channel. */
  peaks: [number, number][];
}

/** What importing files did (`media_import`). */
export interface ImportResult {
  /** The assets added, in the order of the files. */
  assets: string[];
  /** Files that could not be imported, and why, as the engine wrote it. */
  failed: { path: string; error: string; code: string }[];
  /** The project's revision after the import. */
  revision: number;
}

/** The caches FossCut keeps outside projects (ADR-0019, ADR-0032). */
export type CacheKind = "analysis" | "proxies" | "remux" | "frames";

/** How large each cache is (`cache_info`); bytes are null while unknown. */
export interface CacheUsage {
  caches: { kind: CacheKind; bytes: number | null; entries?: number; path?: string }[];
  /** What the last clearing freed. */
  freed_bytes?: number;
}

/** Which copy of an asset's media the preview decodes instead of the original. */
export type PreviewSource = "original" | "remux" | { proxy: "half" | "quarter" };

/** Proxies and remuxed copies being made in the background (`media_work`). */
export interface MediaWork {
  proxies: { done: number; total: number };
  remux: { done: number; total: number };
  /** Copies that could not be made since the app started. */
  failed: number;
}

/** What fills the canvas around a video or image clip (`clip.canvas`). */
export type CanvasFill = { kind: "blur"; amount: number } | { kind: "color"; color: string };

// ---- Local models (ADR-0033) ----

/** A model FossCut can install and run on this computer, as `models_list` lists it. */
export interface ModelInfo {
  name: string;
  /** `runtime`, `matte`, `pose`, `voice`, `translate` or `speech`. */
  task: string;
  /** The download's size on this system. */
  size_bytes: number;
  /** SPDX license. */
  license: string;
  /** The credit its license asks for, shown wherever it is listed. */
  attribution?: string;
  /** `en`, `multilingual`, a direction such as `en>hu`, or `none`. */
  languages: string;
  /** Where it downloads from; empty when it has no build for this system. */
  url: string;
  installed: boolean;
  files: { name: string; url: string; size_bytes: number; sha256: string }[];
  /** What to know before installing it. */
  note?: string;
}

/** The models and where they are (`models_list`). */
export interface ModelListing {
  dir: string;
  models: ModelInfo[];
  /** The ONNX Runtime library the models will load; null until it is installed. */
  runtime: string | null;
}

/** A model being installed in the background, or installed (`models_install`). */
export interface ModelInstallState {
  name: string;
  state: JobState;
  done_bytes: number;
  total_bytes: number;
  elapsed_ms: number;
  /** Once it succeeded: `installed`, or `unchanged` when it already was. */
  install?: { path: string; status: "installed" | "unchanged" | "dry_run" };
  error?: ApiError;
}

/** What a clip's subject is analyzed for: a matte (background removal) or a pose (body effects). */
export type SubjectKind = "matte" | "pose";

/** Whose matte: a person (fast) or any subject (slow). */
export type MatteSource = "person" | "subject";

/** What `subject_analyze` analyzes. */
export interface SubjectRequest {
  clip: string;
  /** `matte` or `pose`, or another analyzer a planner asks for (`scenes`, `beats`, `transcript`). */
  kind: SubjectKind | string;
  model?: MatteSource;
  /** The analyzer's options, as a missing-analysis refusal's details give them. */
  options?: Record<string, unknown>;
}

/** A voice of the voice model (`voices_list`). */
export interface VoiceInfo {
  /** For example `af_heart`. */
  name: string;
  accent: "american" | "british";
  gender: "female" | "male";
  installed: boolean;
}

/** A speech file the engine wrote. */
export interface SpeechFile {
  relative: string;
  reused: boolean;
  duration: number;
  seconds: number;
  voice: string;
  speed: number;
}

/** A preview of speech, played and not added (`speech_preview`). */
export interface SpeechPreview {
  file: SpeechFile;
  /** False when there is no sound output to play it on. */
  played: boolean;
}

/** What `speech_add` says and where. */
export interface SpeechRequest {
  text: string;
  voice?: string;
  /** 0.5 to 2. */
  speed?: number;
  /** Where it starts on the timeline, in flicks. */
  at?: number;
  expected_revision?: number;
}

/** Speech added to the timeline. */
export interface SpeechResult {
  speech: { revision: number; asset: string; file: SpeechFile };
  response: EditResponse | null;
}

/** What `captions_translate` translates. */
export interface TranslateCaptionsRequest {
  track?: string;
  /** The language translated into. */
  to: "hu" | "en";
  mode: "bilingual" | "replace";
  expected_revision?: number;
}

/** A translated caption track. */
export interface TranslateCaptionsResult {
  translation: {
    revision: number;
    timeline: string;
    track: string;
    pair: string;
    mode: "bilingual" | "replace";
    model: string;
    attribution?: string;
    pages: { clip: string; text: string; translation: string }[];
    skipped: string[];
  };
  response: EditResponse | null;
}

/** What `captions_dub` speaks. */
export interface DubCaptionsRequest {
  track?: string;
  voice?: string;
  speed?: number;
  /** Speak each page's translation instead of its words. */
  translation: boolean;
  expected_revision?: number;
}

/** A dubbed caption track. */
export interface DubCaptionsResult {
  dub: {
    revision: number;
    timeline: string;
    track: string;
    voice: string;
    pages: { clip: string; start: number; text: string; speech: number; speed: number; asset: string; warning?: string }[];
    skipped: string[];
  };
  response: EditResponse | null;
}

// ---- Reframing, beats and shorts (P2) ----

/** What a reframe follows: a person (the pose analysis) or the subject (the matte analysis). */
export type SubjectChoice = "person" | "subject";

/** How a clip is reframed (`clip_reframe`), or tracked by a moving camera (`clip_camera_track`, with `zoom`). */
export interface ReframeRequest {
  clip: string;
  source: SubjectChoice | "region";
  /** With `region`: where it is at `at`, as fractions of the displayed picture. */
  region?: Region;
  at?: number;
  /** Frame each shot on its own, at the cuts of the scenes analysis. */
  shots?: boolean;
  /** Camera tracking: how far the camera zooms in, 1 to 4. */
  zoom?: number;
  expected_revision?: number;
}

/** How one shot of a reframed clip is framed. */
export interface ReframeShot {
  start: number;
  end: number;
  zoom: number;
  subject_frames: number;
  inside_frames: number;
}

/** What reframing or camera tracking did. */
export interface ReframeResult {
  reframe: { clip: string; frames: number; subject_frames: number; shots: ReframeShot[] };
  response: EditResponse | null;
}

/** A vertical version of a timeline that follows its subject (`timeline_vertical`). */
export interface VerticalRequest {
  timeline?: string;
  source: SubjectChoice;
  width?: number;
  height?: number;
  name?: string;
  shots?: boolean;
  expected_revision?: number;
}

export interface VerticalResult {
  vertical: { source: string; timeline: string; clips: number; centred: { clip: string; reason: string }[] };
  response: EditResponse | null;
}

/** Markers put on a music clip's beats (`beats_markers`). */
export interface BeatMarkersResult {
  beats: { timeline: string; tempo_bpm: number | null; beats_per_bar: number | null; confidence: number; markers: number; existing: number };
  response: EditResponse | null;
}

/** A track's cuts moved onto the music's beats (`beats_cut`), and those left where they were. */
export interface BeatCutsResult {
  cuts: {
    track: string;
    music: string;
    tempo_bpm: number | null;
    beats: number;
    tolerance: number;
    snapped: { before: string; after: string; from: number; to: number }[];
    unsnapped: { before: string | null; after: string; at: number; reason: string }[];
  };
  response: EditResponse | null;
}

/** A moment of a long clip that could stand alone as a short (`shorts_candidates`). */
export interface ShortCandidate {
  /** Where it is in the media's own time: what `shorts_make` takes as start and end. */
  span: { start: Moment | { flicks: number; seconds: string }; end: Moment | { flicks: number; seconds: string } };
  text: string;
  /** From 0 to 1, best highest. */
  score: number;
  seconds: number;
  /** The measurements behind the score, by name. */
  reasons?: Record<string, unknown>;
  /** Where it plays on the clip's timeline, in flicks. */
  timeline_start: number;
  timeline_end: number;
}

export interface ShortCandidates {
  clip: string;
  asset: string;
  /** The analyses the scores come from, and those missing that would make them better. */
  measured: string[];
  missing: string[];
  sentences: number;
  considered: number;
  candidates: ShortCandidate[];
}

/** How a short is made from part of a clip (`shorts_make`). */
export interface ShortRequest {
  clip: string;
  /** The part of the media, in its own time (a candidate's span), in flicks. */
  start: number;
  end: number;
  name?: string;
  width?: number;
  height?: number;
  /** What the vertical crop follows; centred when null. */
  follow?: SubjectChoice | null;
  captions?: boolean;
  caption_preset?: string;
  title?: string;
  expected_revision?: number;
}

export interface ShortResult {
  short: { timeline: string; clip: string; name: string; start: number; duration: number; centred: string | null; caption_words: number };
  response: EditResponse | null;
}

// ---- Web compositions and Remotion (ADR-0039) ----

/** A page or folder chosen to import as a web composition, with what its meta tag says. */
export interface WebPage {
  path: string;
  name: string;
  width: number | null;
  height: number | null;
  fps: string | null;
  duration_seconds: number | null;
  /** Why the meta tag could not be read. */
  problem: string | null;
  /** Network addresses the page loads, which keep it from importing. */
  remote: string[];
  /** The browser that will render it; null when this computer has none. */
  browser: string | null;
}

/** What to import as a web composition: the values override its meta tag. */
export interface WebImportRequest {
  path: string;
  name?: string;
  width?: number;
  height?: number;
  fps?: string;
  duration_seconds?: number;
}

export interface WebImported {
  asset: string;
  revision: number;
}

/** A render of the user's own Remotion project; nothing runs until `confirmed`. */
export interface RemotionChoice {
  project: string;
  composition: string;
  props?: unknown;
  entry?: string;
  confirmed: boolean;
}
