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
  kind: "video" | "audio" | "image" | "lut" | "font";
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
  /** How the clip moves in, out and while it shows; durations in flicks. */
  animations?: {
    in?: { preset: string; duration: number };
    out?: { preset: string; duration: number };
    combo?: { preset: string; duration: number };
  };
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
  kind: "number" | "pair" | "bool" | "choice" | "curve" | "asset";
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
  interpolate?: "nearest" | "blend";
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
export interface Catalog {
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
export type RevealMotion = "appear" | "fade" | "rise";

/** How a text appears, as `timeline.read` reports it. */
export interface TextRevealView {
  unit: RevealUnit;
  motion?: RevealMotion;
  duration: Moment;
  exit?: boolean;
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
