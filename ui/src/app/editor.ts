// The editor's state and the actions that change it. Every change to the project is a
// command sent through the engine client (ADR-0002); what the window shows is read back
// from the engine after each commit, whoever made it, so agents' edits appear live.

import type { EngineClient, Rect, Unsubscribe } from "../engine/client";
import { defaultPreferences } from "../engine/preferences";
import {
  type ApiError,
  type AssetView,
  type BeatCutsResult,
  type CacheKind,
  type CacheUsage,
  type CaptureDevice,
  type Catalog,
  type Client,
  type CommandCall,
  type EditRequest,
  type EditResponse,
  type ExportChoice,
  type ExportPresets,
  type History,
  type JobStatus,
  type LibraryAddRequest,
  type LibraryItem,
  type LogEntry,
  type MediaListing,
  type MatteSource,
  type MediaWork,
  type ModelInfo,
  type ModelInstallState,
  type ModelListing,
  type MulticamRequest,
  type Normalization,
  type Opened,
  type Preferences,
  type PreferencesView,
  type PreviewSource,
  type ProjectSummary,
  type RecordChoice,
  type RecordState,
  type Region,
  type RemotionChoice,
  type ShortCandidates,
  type ShortRequest,
  type SubjectChoice,
  type SubjectRequest,
  type TimelineView,
  type TrashEntry,
  type UpdateAnswer,
  type VoiceInfo,
  type WebImportRequest,
  type WebPage,
  isApiError,
} from "../engine/types";
import { setLanguage, t } from "../i18n";
import { applyShortcuts } from "../shortcuts";
import { clipsAt, findClip } from "../timeline/geometry";
import { FLICKS_PER_SECOND, frameAt, frameStart, framesToFlicks, parseRate } from "../transport/time";
import { keysOf, pairAt } from "./animation";
import { Pictures } from "./pictures";
import {
  type AssistKind,
  BODY_NEEDS,
  SUBJECT_KEY,
  VOICE_NEEDS,
  backgroundNeeds,
  dubbedReport,
  isBodyEffect,
  missingModels,
  modelsFor,
  trackOf,
  translateNeeds,
  translatedReport,
} from "./assist";
import { multicamMade, splitAtScenes, stabilized, targetOf, tracked } from "./measure";
import {
  type MissingAnalysis,
  analysisNeeds,
  beatsCut,
  beatsMarked,
  clipShowing,
  matteModel,
  missingAnalysis,
  reframed,
  shortMade,
  shortsFound,
  verticalMade,
} from "./smart";
import { multicamPartAt } from "./multicam";

/** How many history entries the activity panel shows. */
const HISTORY = 50;

/** How often the `check` update policy checks while the app runs: daily. */
export const UPDATE_INTERVAL = 24 * 60 * 60 * 1000;

export interface Notice {
  kind: "error" | "info";
  text: string;
}

export interface EditorState {
  opened: Opened | null;
  timeline: TimelineView | null;
  history: LogEntry[];
  clients: Client[];
  recent: string[];
  /** The browser's summaries of the recent projects, and the trash. */
  projects: ProjectSummary[];
  trash: TrashEntry[];
  /** The playhead, in flicks. */
  playhead: number;
  playing: boolean;
  selection: string | null;
  notice: Notice | null;
  /** Effect types and looks, from `capabilities`; null until read. */
  catalog: Catalog | null;
  /** LUT assets in the media table, for `color.lut` effects. */
  luts: { id: string; name: string }[];
  /** The media bin: every asset, and its folders. */
  media: AssetView[];
  folders: string[];
  /** Drawing a path or painting a brush mask on the preview, when not null. */
  maskTool: MaskTool | null;
  /** More selected clips, shift-clicked after the first (for grouping). */
  also: string[];
  /** Edits of a grouped clip edit its whole group (ADR-0024). */
  linkage: boolean;
  /** The main track stays gapless: deleting from it closes the gap. */
  magnet: boolean;
  /** Clips copied or cut, as `clips.copy` returns them. */
  clipboard: unknown[] | null;
  /** The clip whose attributes Mod+Alt+V pastes. */
  attributesFrom: string | null;
  /** What a click on a clip does: select it, or split it there (blade). */
  tool: "select" | "blade";
  /** Dragged edges snap to clip edges, the playhead and markers. */
  snapping: boolean;
  /** Hovering over the timeline shows that frame in the preview (the preview axis). */
  skimming: boolean;
  /** The time the preview shows while skimming, or null. */
  skim: number | null;
  /** The shuttle rate (J, K, L): 0 when not shuttling, negative backwards. */
  shuttle: number;
  /** The window is in full screen. */
  fullscreen: boolean;
  /** Rulers along the preview's edges, and guides dragged from them. */
  rulers: boolean;
  /** Guides as fractions of the canvas: vertical lines at x, horizontal ones at y. */
  guides: Guide[];
  /** A zoom asked of the timeline, applied by it; seq tells repeats apart. */
  zoomRequest: { kind: "in" | "out" | "fit"; seq: number } | null;
  /** Media chosen in the bin, in the order chosen: for new timelines and templates. */
  chosen: string[];
  /** The built-in library (ADR-0028), read once; null until read. */
  library: LibraryItem[] | null;
  /** The export dialog and the export it started. */
  exporting: Exporting;
  /** The edit measured from media that runs, or ran last, for the panel that started it. */
  measurement: Measurement | null;
  /** Drawing the region to track on the preview, and the region drawn. */
  regionPick: RegionPick | null;
  /** The record panel and the recording it started. */
  recorder: Recorder;
  /** The person's preferences (ADR-0031); null until read, when the defaults apply. */
  preferences: PreferencesView | null;
  /** The Settings dialog. */
  settings: SettingsState;
  /** A newer release the update check found, until dismissed. */
  update: UpdateAnswer | null;
  /** Proxies and remuxed copies being made in the background; null until heard of. */
  mediaWork: MediaWork | null;
  /** The assets the preview decodes from a proxy or a remuxed copy, by id. */
  sources: Record<string, PreviewSource>;
  /** The models that run on this computer (ADR-0033); null until read. */
  models: ModelListing | null;
  /** Models the person is asked to install for something, and their install. */
  install: InstallPrompt | null;
  /** Work with local models that runs, or ran last: an analysis, speech, a translation or a dub. */
  assist: Assist | null;
  /** The text to speech panel. */
  speech: SpeechPanel;
  /** The analysis the last measured edit could not do without, which the window offers to run. */
  missingAnalysis: { kind: Measurement["kind"]; analyzer: string } | null;
  /** The planned edit a running analysis was started for, which shows its progress. */
  analysisFor: Measurement["kind"] | null;
  /** The moments of a long clip found worth a short, the last time they were looked for. */
  shortList: ShortCandidates | null;
  /** The dialog that imports a web composition, while it shows (ADR-0039). */
  webImport: WebImportDialog | null;
  /** The dialog that renders a composition of the user's own Remotion project. */
  remotion: RemotionDialog;
}

/** Importing a web composition: the page chosen, and the import that runs or failed. */
export interface WebImportDialog {
  page: WebPage;
  busy: boolean;
  error: ApiError | null;
}

/**
 * Rendering the user's own Remotion project: the choices, then the licence note and the
 * command to agree to (nothing runs before), then the render and what it became.
 */
export interface RemotionDialog {
  open: boolean;
  /** `choose` the project and composition, `agree` to the licence, then `render`. */
  step: "choose" | "agree" | "render";
  choice: Omit<RemotionChoice, "confirmed"> | null;
  /** The licence note and the command, as the engine wrote them. */
  licence: string | null;
  command: string[];
  job: JobStatus | null;
  busy: boolean;
  error: ApiError | null;
}

/** Work with local models, for the panel that started it. */
export interface Assist {
  kind: AssistKind;
  /** The clip it is for: a video clip, or a caption clip of the track; null for speech. */
  clip: string | null;
  /** True while it runs. */
  busy: boolean;
  /** The analysis it waits for, as last read. */
  job: JobStatus | null;
  /** What it did, in a sentence, once done. */
  report: string | null;
  /** Why it could not, once it failed. */
  error: ApiError | null;
}

/** Models to install before something can run: shown for the person to agree to first. */
export interface InstallPrompt {
  /** What they are for. */
  purpose: AssistKind;
  /** The models, the runtime first. */
  models: ModelInfo[];
  /** True once the person agreed, while they install. */
  installing: boolean;
  /** Which of `models` installs now. */
  index: number;
  /** Its install, as last read. */
  current: ModelInstallState | null;
  /** Why installing failed. */
  error: ApiError | null;
}

/** The text to speech panel: whether it shows, the voices, and why they could not be read. */
export interface SpeechPanel {
  open: boolean;
  /** The voice model's voices; null until read. */
  voices: VoiceInfo[] | null;
  error: ApiError | null;
}

/** The status line of copies being made ("Making proxies 2 of 5"), or null when none are. */
export function mediaWorkLine(work: MediaWork | null): string | null {
  if (!work) return null;
  const parts: string[] = [];
  if (work.proxies.done < work.proxies.total) {
    parts.push(t("app.work.proxies", { done: work.proxies.done + 1, total: work.proxies.total }));
  }
  if (work.remux.done < work.remux.total) {
    parts.push(t("app.work.remux", { done: work.remux.done + 1, total: work.remux.total }));
  }
  return parts.length > 0 ? parts.join(" \u00b7 ") : null;
}

/** The sections of the Settings dialog, as the preferences group them. */
export type SettingsSection =
  | "general"
  | "locations"
  | "editing"
  | "timeline"
  | "export"
  | "performance"
  | "cache"
  | "shortcuts";

export const SETTINGS_SECTIONS: SettingsSection[] = [
  "general",
  "locations",
  "editing",
  "timeline",
  "export",
  "performance",
  "cache",
  "shortcuts",
];

/** The Settings dialog: whether it shows, where, and what the engine refused. */
export interface SettingsState {
  open: boolean;
  section: SettingsSection;
  /** The engine's refusal of the last change of each preference, by key, as it wrote it. */
  errors: Record<string, string>;
  /** The caches' sizes, once read. */
  cache: CacheUsage | null;
  /** Why the caches could not be read or cleared. */
  cacheError: string | null;
  /** The library's sounds, for the export sound; null until read. */
  sounds: LibraryItem[] | null;
}

/** The preferences in `state`, or the defaults until they are read. */
export function preferencesOf(state: Pick<EditorState, "preferences">): Preferences {
  return state.preferences?.preferences ?? DEFAULTS;
}

const DEFAULTS = defaultPreferences();

/** The kinds of clip with a place on the canvas, which nudging moves. */
const NUDGED = new Set(["video", "image", "text", "shape", "timeline", "multicam"]);

/** The record panel: whether it shows, the devices, the recording and a refusal. */
export interface Recorder {
  open: boolean;
  /** The cameras, screens and microphones; null until read. */
  devices: CaptureDevice[] | null;
  /** The recording started last, as its status was last read. */
  job: RecordState | null;
  /** Why the devices could not be read, or the recording could not start or be imported. */
  error: ApiError | null;
}

/** An edit measured from media: stabilizing, splitting at scenes, tracking or multicam. */
export interface Measurement {
  kind: "stabilize" | "scenes" | "track" | "multicam" | "reframe" | "camera" | "vertical" | "beats" | "cuts" | "shorts" | "short";
  /** The clip measured; null when making a multicam clip. */
  clip: string | null;
  /** True while the engine measures. */
  busy: boolean;
  /** What it did, in a sentence, once done. */
  report: string | null;
  /** Why it could not, once it failed. */
  error: ApiError | null;
}

/** The region of a video clip to track, drawn over the preview. */
export interface RegionPick {
  clip: string;
  /** True while a drag over the preview draws the region. */
  drawing: boolean;
  /** The region drawn; null until drawn. */
  region: Region | null;
  /** The timeline time it was drawn at, in flicks. */
  at: number;
}

/** The export dialog: whether it shows, the presets, the job it started and a refusal. */
export interface Exporting {
  open: boolean;
  /** The presets this machine has; null until read. */
  presets: ExportPresets | null;
  /** The export started last, as its status was last read. */
  job: JobStatus | null;
  /** Why the presets could not be read or the export could not start. */
  error: ApiError | null;
}

/** A guide line on the preview. */
export interface Guide {
  axis: "x" | "y";
  /** Where, as a fraction of the canvas's width (x) or height (y). */
  at: number;
}

/** What drawing on the preview edits. */
export interface MaskTool {
  clip: string;
  mask: string;
  mode: "pen" | "brush";
  /** Brush radius as a fraction of the picture's height. */
  radius: number;
  /** The brush erases instead of painting. */
  erase: boolean;
}

const EMPTY: EditorState = {
  opened: null,
  timeline: null,
  history: [],
  clients: [],
  recent: [],
  projects: [],
  trash: [],
  playhead: 0,
  playing: false,
  selection: null,
  also: [],
  linkage: true,
  magnet: false,
  clipboard: null,
  attributesFrom: null,
  tool: "select",
  snapping: true,
  skimming: false,
  skim: null,
  shuttle: 0,
  fullscreen: false,
  rulers: false,
  guides: [],
  zoomRequest: null,
  chosen: [],
  library: null,
  exporting: { open: false, presets: null, job: null, error: null },
  measurement: null,
  regionPick: null,
  recorder: { open: false, devices: null, job: null, error: null },
  preferences: null,
  settings: { open: false, section: "general", errors: {}, cache: null, cacheError: null, sounds: null },
  update: null,
  mediaWork: null,
  sources: {},
  models: null,
  install: null,
  assist: null,
  speech: { open: false, voices: null, error: null },
  missingAnalysis: null,
  analysisFor: null,
  shortList: null,
  webImport: null,
  remotion: { open: false, step: "choose", choice: null, licence: null, command: [], job: null, busy: false, error: null },
  notice: null,
  catalog: null,
  luts: [],
  media: [],
  folders: [],
  maskTool: null,
};

/** What a catalog says about text when `capabilities` has nothing on it. */
const NO_TEXT: Catalog["text"] = {
  fonts: ["Inter"],
  presets: [],
  templates: [],
  reveal: { units: ["letter", "word", "line"], motions: ["appear", "fade", "rise"] },
  cases: ["as_written", "upper", "lower", "title"],
};

/** What normalizing loudness did, in a sentence. */
export function levelled(normalization: Normalization): string {
  const measured = normalization.clips.filter((c) => c.measured_lufs !== null);
  if (measured.length === 0) return t("app.levelled.nothing");
  const parts = [t("app.levelled.to", { target: normalization.target_lufs })];
  const limited = measured.filter((c) => c.limited).length;
  if (limited > 0) parts.push(t("app.levelled.limited", { count: limited, ceiling: normalization.ceiling_dbfs }));
  const silent = normalization.clips.length - measured.length;
  if (silent > 0) parts.push(t("app.levelled.silent", { count: silent }));
  return t("app.sentence", { text: parts.join("; ") });
}

/** What an export that finished out of sight did, in a sentence. */
export function finished(status: JobStatus): Notice {
  const file = status.output ?? status.job;
  if (status.state === "succeeded") return { kind: "info", text: t("app.export.done", { file }) };
  if (status.state === "cancelled") return { kind: "info", text: t("app.export.cancelled", { file }) };
  return {
    kind: "error",
    text: t("app.export.failed", { file, reason: status.error?.message ?? t("app.export.noReason") }),
  };
}

/** Asks the user a yes-or-no question; the app passes `window.confirm`. */
export type Confirm = (question: string) => boolean;

/** A thrown value as a FossCut error. */
function asError(error: unknown): ApiError {
  if (isApiError(error)) return error;
  return { code: "fosscut.internal", message: String(error) };
}

export class Editor {
  private state: EditorState = EMPTY;
  private listeners = new Set<() => void>();
  private unsubscribers: Unsubscribe[] = [];
  private reading: Promise<void> | null = null;
  private stale = false;
  private updateTimer: ReturnType<typeof setInterval> | null = null;
  /** What runs once the models the person agreed to are installed. */
  private afterInstall: (() => Promise<void>) | null = null;
  /** What runs once the analysis that runs succeeds, answering what it did. */
  private afterAnalysis: (() => Promise<string | null>) | null = null;

  /** Frames and waveforms of the open project's media, for the bin and the timeline. */
  readonly pictures: Pictures;

  constructor(
    private readonly engine: EngineClient,
    private readonly confirm: Confirm = () => false,
  ) {
    this.pictures = new Pictures(engine);
  }

  // ---- Store ----

  getState = (): EditorState => this.state;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private set(patch: Partial<EditorState>): void {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }

  private notify(error: unknown): void {
    const api = asError(error);
    this.set({ notice: { kind: "error", text: api.hint ? `${api.message}. ${api.hint}` : api.message } });
  }

  dismissNotice(): void {
    this.set({ notice: null });
  }

  /** What outlives a project: the recent list, the preferences, Settings and an update. */
  private kept(): Pick<EditorState, "recent" | "preferences" | "settings" | "update" | "mediaWork" | "models"> {
    const { recent, preferences, settings, update, mediaWork, models } = this.state;
    return { recent, preferences, settings, update, mediaWork, models };
  }

  // ---- Lifecycle ----

  /** Listens to the engine and loads the project the app has open, if any. */
  async start(): Promise<void> {
    this.unsubscribers.push(
      this.engine.on("project-changed", () => void this.refresh()),
      this.engine.on("project-closed", () => this.set({ ...EMPTY, ...this.kept() })),
      this.engine.on("playhead", (playhead) =>
        this.set({
          playhead: playhead.time.flicks,
          playing: playhead.playing,
          shuttle: playhead.playing ? this.state.shuttle : 0,
        }),
      ),
      this.engine.on("preview-error", (message) =>
        this.set({ playing: false, notice: { kind: "error", text: message } }),
      ),
      this.engine.on("media-work", (work) => {
        const finishing = work.proxies.done + work.remux.done > (this.state.mediaWork?.proxies.done ?? 0) + (this.state.mediaWork?.remux.done ?? 0);
        this.set({ mediaWork: work });
        // A copy made changes what the media bin marks.
        if (finishing || !mediaWorkLine(work)) void this.readSources();
      }),
      this.engine.on("open-settings", () => this.openSettings()),
      this.engine.on("files-dropped", (paths) => void this.dropped(paths)),
    );
    void this.engine.media
      .work()
      .then((mediaWork) => this.set({ mediaWork }))
      .catch(() => undefined);
    await this.loadPreferences();
    void this.watchUpdates();
    try {
      const [recent, opened] = await Promise.all([this.engine.recentProjects(), this.engine.currentProject()]);
      this.set({ recent });
      if (opened) await this.show(opened);
      else await this.browse();
    } catch (error) {
      this.notify(error);
    }
  }

  stop(): void {
    for (const unsubscribe of this.unsubscribers.splice(0)) unsubscribe();
    if (this.updateTimer !== null) clearInterval(this.updateTimer);
    this.updateTimer = null;
  }

  // ---- Projects ----

  private async show(opened: Opened | null): Promise<void> {
    if (!opened) return;
    this.pictures.clear();
    this.set({ ...EMPTY, ...this.kept(), opened, linkage: preferencesOf(this.state).timeline.linked });
    await Promise.all([this.refresh(), this.loadCatalog()]);
    await this.seek(0);
  }

  /** Reads the effect types and looks once per project; without them effects still show. */
  private async loadCatalog(): Promise<void> {
    try {
      const caps = await this.engine.query<Catalog>("capabilities", { detail: "full" });
      this.set({
        catalog: {
          effects: caps.effects ?? [],
          looks: caps.looks ?? [],
          transitions: caps.transitions ?? [],
          animations: caps.animations ?? [],
          speed_presets: caps.speed_presets ?? [],
          voice_presets: caps.voice_presets ?? [],
          text: { ...NO_TEXT, ...caps.text },
          // What particle emitters can be, and the 3D object presets (ADR-0038, ADR-0040).
          particles: caps.particles,
          library: { particles: caps.library?.particles ?? [], models: caps.library?.models ?? [] },
        },
      });
    } catch {
      this.set({ catalog: null });
    }
  }

  async open(path: string): Promise<void> {
    try {
      await this.show(await this.engine.openProject(path));
    } catch (error) {
      this.notify(error);
    }
  }

  async pick(): Promise<void> {
    try {
      await this.show(await this.engine.pickProject());
    } catch (error) {
      this.notify(error);
    }
  }

  /**
   * Asks where to save a new project, `width` by `height` (1920 by 1080 when omitted) at the
   * preferred frame rate.
   */
  async create(size: { width: number; height: number } = { width: 1920, height: 1080 }): Promise<void> {
    try {
      const rate = preferencesOf(this.state).timeline.frame_rate;
      await this.show(await this.engine.newProject({ width: size.width, height: size.height, rate }));
    } catch (error) {
      this.notify(error);
    }
  }

  async close(): Promise<void> {
    await this.engine.closeProject();
    this.pictures.clear();
    const recent = await this.engine.recentProjects();
    this.set({ ...EMPTY, ...this.kept(), recent });
    await this.browse();
  }

  /** Reads the browser's project summaries (names first, then thumbnails) and the trash. */
  async browse(): Promise<void> {
    try {
      const [projects, trash] = await Promise.all([this.engine.projectSummaries(false), this.engine.trash.list()]);
      this.set({ projects, trash });
      const pictured = await this.engine.projectSummaries(true);
      if (!this.state.opened) this.set({ projects: pictured });
    } catch (error) {
      this.notify(error);
    }
  }

  /** Moves a project to the trash, from the browser. */
  async trashProject(path: string): Promise<void> {
    try {
      await this.engine.trash.add(path);
      this.set({ recent: await this.engine.recentProjects() });
      await this.browse();
    } catch (error) {
      this.notify(error);
    }
  }

  async restoreProject(id: string): Promise<void> {
    try {
      const path = await this.engine.trash.restore(id);
      this.set({ notice: { kind: "info", text: t("app.restored", { path }) } });
      await this.browse();
    } catch (error) {
      this.notify(error);
    }
  }

  /** Deletes a trashed project for good, after asking. */
  async deleteTrashed(id: string): Promise<void> {
    const entry = this.state.trash.find((t) => t.id === id);
    if (!entry || !this.confirm(t("app.trash.confirm", { name: entry.name }))) return;
    try {
      await this.engine.trash.remove(id);
      await this.browse();
    } catch (error) {
      this.notify(error);
    }
  }

  /**
   * Reads the timeline, the history and the project info again. Commits that arrive while
   * a read is in flight cause one more read, never several at once.
   */
  refresh(): Promise<void> {
    if (this.reading) {
      this.stale = true;
      return this.reading;
    }
    this.reading = (async () => {
      do {
        this.stale = false;
        await this.readOnce();
      } while (this.stale);
      this.reading = null;
    })();
    return this.reading;
  }

  private async readOnce(): Promise<void> {
    const opened = this.state.opened;
    if (!opened) return;
    try {
      const info = await this.engine.query<Opened["info"]>("project.info");
      const [timeline, history] = await Promise.all([
        this.engine.query<TimelineView>("timeline.read", { detail: "full", timeline: info.primary_timeline }),
        this.engine.query<History>("history.list", { limit: HISTORY }),
      ]);
      const selection =
        this.state.selection && findClip(timeline, this.state.selection) ? this.state.selection : null;
      const pick = this.state.regionPick;
      const regionPick = pick && findClip(timeline, pick.clip) ? pick : null;
      const listing = await this.engine
        .query<MediaListing>("media.list")
        .catch(() => ({ assets: this.state.media, folders: this.state.folders }));
      const luts = listing.assets.filter((a) => a.kind === "lut").map(({ id, name }) => ({ id, name }));
      const sources = await this.engine.media.sources().catch(() => this.state.sources);
      this.set({
        opened: { ...opened, info },
        timeline,
        history: history.entries,
        selection,
        regionPick,
        luts,
        media: listing.assets,
        folders: listing.folders ?? [],
        sources,
      });
    } catch (error) {
      this.notify(error);
    }
  }

  /** Reads which assets the preview decodes from a proxy or a remuxed copy. */
  async readSources(): Promise<void> {
    if (!this.state.opened) return;
    try {
      this.set({ sources: await this.engine.media.sources() });
    } catch {
      // The marks are a courtesy; the next read tries again.
    }
  }

  async refreshClients(): Promise<void> {
    if (!this.state.opened) return;
    try {
      this.set({ clients: await this.engine.clients() });
    } catch {
      // The next poll tries again.
    }
  }

  // ---- Playback ----

  private rate() {
    return parseRate(this.state.timeline?.format.rate ?? "30");
  }

  /** The last frame's start, where the playhead stops. */
  private end(): number {
    const duration = this.state.timeline?.duration.flicks ?? 0;
    return Math.max(0, frameStart(duration - 1, this.rate()));
  }

  /**
   * Moves the playhead to the frame containing `flicks`, within the timeline. With
   * `scrub`, a moment of its sound plays too, as the preferences allow.
   */
  async seek(flicks: number, scrub = false): Promise<void> {
    const at = Math.min(Math.max(0, frameStart(flicks, this.rate())), this.end());
    this.set({ playhead: at, playing: false });
    await this.engine.preview.show(at, scrub);
  }

  /** Moves the playhead while dragging on the ruler, with sound when the preferences say. */
  async scrub(flicks: number): Promise<void> {
    await this.seek(flicks, true);
  }

  async step(frames: number): Promise<void> {
    const rate = this.rate();
    await this.seek(framesToFlicks(frameAt(this.state.playhead, rate) + frames, rate));
  }

  /** Steps `editing.frame_step` frames back (-1) or forward (1): Shift and an arrow key. */
  async stepMany(direction: -1 | 1): Promise<void> {
    await this.step(direction * preferencesOf(this.state).editing.frame_step);
  }

  async stepSeconds(seconds: number): Promise<void> {
    await this.seek(this.state.playhead + seconds * FLICKS_PER_SECOND);
  }

  async seekEnd(): Promise<void> {
    await this.seek(this.end());
  }

  async togglePlay(): Promise<void> {
    if (this.state.playing) {
      this.set({ playing: false, shuttle: 0 });
      await this.engine.preview.pause();
    } else if (this.state.timeline) {
      this.set({ playing: true, shuttle: 1 });
      await this.engine.preview.play();
    }
  }

  /**
   * J, K and L: L plays forwards and doubles the speed each press (to 8x), J the same
   * backwards, K stops. Speeds other than 1x play without sound.
   */
  async shuttle(direction: -1 | 0 | 1): Promise<void> {
    if (!this.state.timeline) return;
    const rate = this.state.shuttle;
    const next =
      direction === 0
        ? 0
        : direction > 0
          ? rate <= 0
            ? 1
            : Math.min(rate * 2, 8)
          : rate >= 0
            ? -1
            : Math.max(rate * 2, -8);
    this.set({ shuttle: next, playing: next !== 0 });
    await this.engine.preview.shuttle(next);
  }

  setTool(tool: "select" | "blade"): void {
    this.set({ tool });
  }

  setSnapping(on: boolean): void {
    this.set({ snapping: on });
  }

  setSkimming(on: boolean): void {
    this.set({ skimming: on, skim: null });
    if (!on) void this.engine.preview.show(this.state.playhead);
  }

  /** While skimming, shows the frame at `time` without moving the playhead; null goes back. */
  async skim(time: number | null): Promise<void> {
    if (!this.state.skimming || this.state.playing) return;
    if (time === this.state.skim) return;
    this.set({ skim: time });
    await this.engine.preview.show(time ?? this.state.playhead, time !== null);
  }

  async toggleFullscreen(): Promise<void> {
    const on = !this.state.fullscreen;
    this.set({ fullscreen: on });
    await this.engine.fullscreen(on);
  }

  toggleRulers(): void {
    this.set({ rulers: !this.state.rulers });
  }

  /** Adds a guide, moves guide `index` to `at`, or removes it when `at` is null. */
  setGuide(index: number, guide: Guide | null): void {
    const guides = [...this.state.guides];
    if (guide === null) guides.splice(index, 1);
    else if (index >= guides.length) guides.push(guide);
    else guides[index] = guide;
    this.set({ guides });
  }

  /** Asks the timeline to zoom in, out or to fit the whole edit. */
  zoom(kind: "in" | "out" | "fit"): void {
    this.set({ zoomRequest: { kind, seq: (this.state.zoomRequest?.seq ?? 0) + 1 } });
  }

  /** The blade: splits a clip at a time. */
  async bladeAt(clip: string, flicks: number): Promise<void> {
    await this.edit([{ command: "clip.split", args: { clip, at: { flicks }, ...this.linked() } }]);
  }

  /** Tells the preview where it shows through the page, in CSS pixels. */
  async setViewport(rect: Rect): Promise<void> {
    await this.engine.preview.viewport(rect);
  }

  select(clip: string | null): void {
    const pick = this.state.regionPick;
    this.set({ selection: clip, also: [], regionPick: pick && pick.clip === clip ? pick : null });
  }

  /** Adds a clip to the selection, or takes it out (shift-click). */
  toggleSelected(clip: string): void {
    const { selection, also } = this.state;
    if (!selection) {
      this.set({ selection: clip, also: [] });
    } else if (clip === selection) {
      this.set({ selection: also[0] ?? null, also: also.slice(1) });
    } else if (also.includes(clip)) {
      this.set({ also: also.filter((c) => c !== clip) });
    } else {
      this.set({ also: [...also, clip] });
    }
  }

  /** Every selected clip, the first one first. */
  selected(): string[] {
    return this.state.selection ? [this.state.selection, ...this.state.also] : [];
  }

  setLinkage(on: boolean): void {
    this.set({ linkage: on });
  }

  setMagnet(on: boolean): void {
    this.set({ magnet: on });
  }

  /** The arguments that make an edit follow the linkage switch. */
  private linked(): Record<string, unknown> {
    return this.state.linkage ? {} : { linked: false };
  }

  /** True when a clip is on the main track: the lowest video track. */
  private onMainTrack(clip: string): boolean {
    const main = this.state.timeline?.tracks.find((t) => t.kind === "video");
    return Boolean(main?.clips.some((c) => c.id === clip));
  }

  // ---- Edits ----

  /**
   * Applies commands as one undoable step against the revision on screen. A stale revision
   * reloads and says so; a destructive change asks first, then applies with consent.
   */
  async edit(commands: CommandCall[], reason?: string): Promise<boolean> {
    return (await this.apply(commands, reason)) !== null;
  }

  /** `edit`, answering the engine's response, or null when nothing was applied. */
  private async apply(commands: CommandCall[], reason?: string): Promise<EditResponse | null> {
    const timeline = this.state.timeline;
    if (!timeline) return null;
    const request: EditRequest = { commands, expected_revision: timeline.revision };
    if (reason) request.reason = reason;
    let response: EditResponse;
    try {
      response = await this.engine.execute(request);
    } catch (error) {
      const api = asError(error);
      if (api.code === "fosscut.revision.conflict") {
        await this.refresh();
        this.set({ notice: { kind: "info", text: t("app.conflict") } });
        return null;
      }
      if (api.code === "fosscut.guard.destructive" && this.confirm(t("app.destructive.confirm", { message: api.message }))) {
        try {
          response = await this.engine.execute({ ...request, allow_destructive: true });
        } catch (again) {
          this.notify(again);
          return null;
        }
      } else {
        if (api.code !== "fosscut.guard.destructive") this.notify(api);
        return null;
      }
    }
    await this.refresh();
    return response;
  }

  async deleteSelection(): Promise<void> {
    const clips = this.selected();
    if (clips.length === 0) return;
    const commands = clips.map((clip) => {
      const args: Record<string, unknown> = { clip, ...this.linked() };
      if (this.state.magnet && this.onMainTrack(clip)) args.ripple = true;
      return { command: "clip.remove", args };
    });
    if (await this.edit(commands)) this.set({ selection: null, also: [] });
  }

  /** Copies the selected clips to the clipboard. */
  async copy(): Promise<boolean> {
    const clips = this.selected();
    if (clips.length === 0) return false;
    try {
      const copied = await this.engine.query<{ clips: unknown[] }>("clips.copy", { clips });
      this.set({ clipboard: copied.clips });
      return true;
    } catch (error) {
      this.notify(error);
      return false;
    }
  }

  /** Copies the selected clips, then removes them. */
  async cut(): Promise<void> {
    if (await this.copy()) await this.deleteSelection();
  }

  /** Pastes the clipboard at the playhead. */
  async paste(): Promise<void> {
    const clips = this.state.clipboard;
    if (!clips) return;
    await this.edit([{ command: "clips.paste", args: { clips, at: { flicks: this.state.playhead } } }]);
  }

  /** Remembers the selected clip as the one whose attributes to paste. */
  copyAttributes(): void {
    if (this.state.selection) this.set({ attributesFrom: this.state.selection });
  }

  /** Pastes the remembered clip's attributes onto the selected clips. */
  async pasteAttributes(): Promise<void> {
    const from = this.state.attributesFrom;
    const to = this.selected().filter((c) => c !== from);
    if (!from || to.length === 0) return;
    await this.edit([{ command: "clip.copy_attributes", args: { from, to } }]);
  }

  /**
   * Deletes the part of the selected clip before (`left`) or after the playhead, closing
   * the gap: the clip keeps its start and what follows moves (ripple).
   */
  async deleteSide(side: "left" | "right"): Promise<void> {
    const timeline = this.state.timeline;
    const id = this.state.selection;
    const clip = timeline && id ? findClip(timeline, id) : null;
    const at = this.state.playhead;
    if (!clip || at <= clip.start.flicks || at >= clip.end.flicks) {
      this.set({ notice: { kind: "info", text: t("app.playhead.inside") } });
      return;
    }
    const edge = side === "left" ? { start: { flicks: at } } : { end: { flicks: at } };
    await this.edit([{ command: "clip.trim", args: { clip: clip.id, ...edge, ripple: true, ...this.linked() } }]);
  }

  /** Splits every clip under the playhead. */
  async splitAll(): Promise<void> {
    await this.edit([{ command: "timeline.split", args: { at: { flicks: this.state.playhead } } }]);
  }

  /** Turns the selected clips into one compound clip (Mod+Alt+G). */
  async compound(): Promise<void> {
    const clips = this.selected();
    if (clips.length === 0) return;
    if (await this.edit([{ command: "clip.compound", args: { clips } }])) this.set({ selection: null, also: [] });
  }

  /** Puts a compound clip's contents back in its place. */
  async decompose(clip: string): Promise<void> {
    if (await this.edit([{ command: "clip.decompose", args: { clip } }])) this.set({ selection: null, also: [] });
  }

  /** Places a compound timeline at the playhead. */
  async addCompound(timeline: string): Promise<void> {
    await this.edit([{ command: "clip.add", args: { compound: timeline, start: { flicks: this.state.playhead } } }]);
  }

  /** Groups the selected clips. */
  async group(): Promise<void> {
    const clips = this.selected();
    if (clips.length < 2) {
      this.set({ notice: { kind: "info", text: t("app.group.select") } });
      return;
    }
    await this.edit([{ command: "clip.group", args: { clips } }]);
  }

  /** Takes the selected clips out of their groups. */
  async ungroup(): Promise<void> {
    const clips = this.selected();
    if (clips.length > 0) await this.edit([{ command: "clip.ungroup", args: { clips } }]);
  }

  /** Extracts a video clip's sound into a linked audio clip, or puts it back. */
  async toggleExtractedAudio(clip: string): Promise<void> {
    const view = this.state.timeline ? findClip(this.state.timeline, clip) : null;
    if (!view) return;
    const command = view.audio_detached ? "clip.restore_audio" : "clip.extract_audio";
    await this.edit([{ command, args: { clip } }]);
  }

  /** Lines a clip up with the canvas's edges or centre. */
  async align(clip: string, place: { horizontal?: string; vertical?: string }): Promise<void> {
    await this.edit([{ command: "clip.align", args: { clip, ...place } }]);
  }

  /** Adds a marker at the playhead, or removes the one there. */
  async toggleMarker(): Promise<void> {
    const at = this.state.playhead;
    const here = this.state.timeline?.markers?.find((m) => m.at.flicks === at);
    if (here) {
      await this.edit([{ command: "marker.remove", args: { marker: here.id } }]);
    } else {
      await this.edit([{ command: "marker.add", args: { at: { flicks: at } } }]);
    }
  }

  /** Moves the playhead to the previous or next marker. */
  async seekMarker(direction: -1 | 1): Promise<void> {
    const at = this.state.playhead;
    const markers = this.state.timeline?.markers ?? [];
    const target =
      direction > 0 ? markers.find((m) => m.at.flicks > at) : [...markers].reverse().find((m) => m.at.flicks < at);
    if (target) await this.seek(target.at.flicks);
  }

  /** Splits the selected clip at the playhead, or the only clip under it. */
  async splitAtPlayhead(): Promise<void> {
    const timeline = this.state.timeline;
    if (!timeline) return;
    const at = this.state.playhead;
    const under = clipsAt(timeline, at);
    const chosen = this.state.selection
      ? under.find((c) => c.id === this.state.selection)
      : under.length === 1
        ? under[0]
        : undefined;
    if (!chosen) {
      const text = this.state.selection
        ? t("app.split.outside")
        : under.length > 1
          ? t("app.split.several")
          : t("app.split.none");
      this.set({ notice: { kind: "info", text } });
      return;
    }
    await this.edit([{ command: "clip.split", args: { clip: chosen.id, at: { flicks: at } } }]);
  }

  /** Moves a clip to `start` (flicks), and to `track` when given. */
  async moveClip(clip: string, start: number, track?: string): Promise<void> {
    const args: Record<string, unknown> = { clip, start: { flicks: start }, ...this.linked() };
    if (track) args.track = track;
    await this.edit([{ command: "clip.move", args }]);
  }

  /** Moves a clip's start or end edge (flicks). */
  async trimClip(clip: string, edges: { start?: number; end?: number }): Promise<void> {
    const args: Record<string, unknown> = { clip, ...this.linked() };
    if (edges.start !== undefined) args.start = { flicks: edges.start };
    if (edges.end !== undefined) args.end = { flicks: edges.end };
    await this.edit([{ command: "clip.trim", args }]);
  }

  /**
   * Moves the selected clips on the canvas by `dx` and `dy` nudges of `editing.nudge_pixels`
   * canvas pixels each, as one step: a keyframe at the playhead where their position is
   * animated, their position otherwise.
   */
  async nudge(dx: number, dy: number): Promise<void> {
    const timeline = this.state.timeline;
    if (!timeline) return;
    const pixels = preferencesOf(this.state).editing.nudge_pixels;
    const { width, height } = timeline.format;
    const playhead = this.state.playhead;
    const round = (value: number) => Math.round(value * 1e6) / 1e6;
    const commands: CommandCall[] = this.selected()
      .map((id) => findClip(timeline, id))
      .filter((clip): clip is NonNullable<typeof clip> => clip !== null && NUDGED.has(clip.kind))
      .map((clip) => {
        const [x, y] = pairAt(clip, "transform.position", playhead, clip.transform?.position ?? [0, 0]);
        const value = [round(x + (dx * pixels) / width), round(y + (dy * pixels) / height)];
        return keysOf(clip, "transform.position")
          ? { command: "keyframe.set", args: { clip: clip.id, property: "transform.position", at: { flicks: playhead }, value } }
          : { command: "clip.set", args: { clip: clip.id, set: { "transform.position": value } } };
      });
    if (commands.length > 0) await this.edit(commands, t("app.nudge"));
  }

  /** Sets clip properties by path (`transform.position`, `opacity`, ...) as one step. */
  async setProperties(clip: string, set: Record<string, unknown>): Promise<void> {
    if (Object.keys(set).length === 0) return;
    await this.edit([{ command: "clip.set", args: { clip, set } }]);
  }

  /** Starts or stops drawing a mask on the preview. */
  setMaskTool(tool: MaskTool | null): void {
    this.set({ maskTool: tool });
  }

  /** Adds a mask of `shape` to a clip; path and brush masks start drawing on the preview. */
  async addMask(clip: string, shape: string): Promise<void> {
    if (shape === "path" || shape === "brush") {
      // These need points or strokes before they are valid: draw first, then add.
      this.set({ maskTool: { clip, mask: "", mode: shape === "path" ? "pen" : "brush", radius: 0.04, erase: false } });
      return;
    }
    const args: Record<string, unknown> = { clip, shape };
    if (shape === "text") args.text = "TEXT";
    await this.edit([{ command: "mask.add", args }]);
  }

  /**
   * Finishes drawing: a new path or brush mask with what was drawn, or the drawing added
   * to the mask being edited. Brush strokes add to the mask's strokes.
   */
  async finishMask(drawn: { points?: { at: [number, number] }[]; stroke?: { points: [number, number][]; radius: number; erase: boolean } }): Promise<void> {
    const tool = this.state.maskTool;
    if (!tool) return;
    const timeline = this.state.timeline;
    const clip = timeline ? findClip(timeline, tool.clip) : null;
    const existing = clip?.masks?.find((m) => m.id === tool.mask);
    if (drawn.points) {
      if (drawn.points.length < 3) {
        this.set({ notice: { kind: "info", text: t("app.mask.points") } });
        return;
      }
      const command = existing
        ? { command: "mask.set", args: { clip: tool.clip, mask: tool.mask, points: drawn.points } }
        : { command: "mask.add", args: { clip: tool.clip, shape: "path", size: [1, 1], points: drawn.points } };
      if (await this.edit([command])) this.set({ maskTool: null });
      return;
    }
    if (drawn.stroke) {
      const strokes = [...(existing?.strokes ?? []), drawn.stroke];
      const command = existing
        ? { command: "mask.set", args: { clip: tool.clip, mask: tool.mask, strokes } }
        : { command: "mask.add", args: { clip: tool.clip, shape: "brush", strokes } };
      await this.edit([command]);
      if (!existing) {
        // Keep painting into the mask just made.
        const after = this.state.timeline ? findClip(this.state.timeline, tool.clip) : null;
        const made = after?.masks?.[after.masks.length - 1];
        if (made) this.set({ maskTool: { ...tool, mask: made.id } });
      }
    }
  }

  /**
   * Changes a clip's speed with `clip.speed`: `rate`, `preset`, `reverse`, `pitch`,
   * `interpolate` or `reset`. Later clips on the track move to make room.
   */
  async setSpeed(clip: string, change: Record<string, unknown>): Promise<void> {
    await this.edit([{ command: "clip.speed", args: { clip, ripple: true, ...change } }]);
  }

  /** Holds the frame of a video clip at the playhead for `seconds`. */
  async freeze(clip: string, seconds = 2): Promise<void> {
    await this.edit([
      {
        command: "clip.freeze",
        args: { clip, at: { flicks: this.getState().playhead }, duration: { seconds: String(seconds) } },
      },
    ]);
  }

  // ---- Media bin and timelines ----

  async renameAsset(asset: string, name: string): Promise<void> {
    if (name.trim()) await this.edit([{ command: "media.rename", args: { asset, name } }]);
  }

  /** Removes an asset; when clips show it, asks before removing them too. */
  async removeAsset(asset: string): Promise<void> {
    const used = this.state.media.find((a) => a.id === asset)?.used_by.length ?? 0;
    await this.edit([{ command: "media.remove", args: used > 0 ? { asset, remove_clips: true } : { asset } }]);
  }

  /** Puts assets in a folder, or at the top level when `folder` is empty. */
  async moveAssets(assets: string[], folder: string): Promise<void> {
    if (assets.length === 0) return;
    await this.edit([{ command: "media.move", args: folder ? { assets, folder } : { assets } }]);
  }

  async folder(action: "add" | "rename" | "remove", folder: string, to?: string): Promise<void> {
    if (!folder.trim()) return;
    await this.edit([{ command: "media.folder", args: to ? { action, folder, to } : { action, folder } }]);
  }

  async revealAsset(asset: string): Promise<void> {
    try {
      await this.engine.media.reveal(asset);
    } catch (error) {
      this.notify(error);
    }
  }

  /** Asks where a missing asset's file is now. */
  async relinkAsset(asset: string): Promise<void> {
    try {
      if (await this.engine.media.relink(asset)) await this.refresh();
    } catch (error) {
      this.notify(error);
    }
  }

  /** Chooses an asset in the bin, or takes it out of the choice. */
  chooseMedia(asset: string, on: boolean): void {
    const rest = this.state.chosen.filter((id) => id !== asset);
    this.set({ chosen: on ? [...rest, asset] : rest });
  }

  /**
   * Places an asset on the timeline at the playhead, on a track free there; a still image
   * is as long as `editing.image_duration` says.
   */
  async addToTimeline(asset: string, at?: { start: number; track?: string }): Promise<void> {
    const args: Record<string, unknown> = { asset, start: { flicks: at?.start ?? this.state.playhead } };
    const kind = this.state.media.find((a) => a.id === asset)?.kind;
    // A 3D model is an object of its own, placed in the 3D scene (ADR-0040).
    if (kind === "model") {
      await this.addModel({ asset }, at?.start);
      return;
    }
    if (kind === "image" || kind === "vector") {
      args.duration = { seconds: String(preferencesOf(this.state).editing.image_duration) };
    }
    // Dropped on the timeline: on the track it was dropped on, or the first free one there.
    if (at?.track) args.track = at.track;
    else if (at) args.free_track = true;
    await this.edit([{ command: "clip.add", args }]);
  }

  /**
   * Files dropped on the window: a project folder opens; other files are imported into the
   * open project.
   */
  async dropped(paths: string[]): Promise<void> {
    const project = paths.find((path) => /\.fosscut\/?$/i.test(path));
    if (project) {
      if (this.state.opened?.bundle !== project) await this.open(project);
      return;
    }
    if (this.state.opened && paths.length > 0) await this.importMedia(paths);
  }

  /**
   * Imports files into the project, `paths` or those the user picks, as one step; files
   * that could not be imported are named in a notice.
   */
  async importMedia(paths?: string[]): Promise<void> {
    if (!this.state.opened) return;
    try {
      const result = await this.engine.media.import(paths);
      if (!result) return;
      await this.refresh();
      if (result.failed.length > 0) {
        const names = result.failed.map((f) => f.path.split(/[\\/]/).pop() ?? f.path).join(", ");
        this.set({ notice: { kind: "error", text: t("app.import.failed", { count: result.failed.length, files: names }) } });
      }
    } catch (error) {
      this.notify(error);
    }
  }

  /**
   * Makes a version of the shown timeline on a canvas of `width` by `height`, pictures
   * scaled to fill it, and shows it.
   */
  async makeVersion(width: number, height: number): Promise<void> {
    const timeline = this.state.timeline;
    if (!timeline) return;
    if (await this.edit([{ command: "timeline.duplicate", args: { timeline: timeline.timeline, width, height, fit: "fill", primary: true } }])) {
      this.set({ selection: null, also: [], playhead: 0 });
      await this.engine.preview.show(0);
    }
  }

  /** Adds a timeline (from `assets`, when given) and shows it. */
  async addTimeline(assets: string[] = []): Promise<void> {
    await this.edit([{ command: "timeline.add", args: assets.length > 0 ? { assets, primary: true } : { primary: true } }]);
  }

  /** Shows another timeline: it becomes the primary one. */
  async showTimeline(timeline: string): Promise<void> {
    if (timeline === this.state.opened?.info.primary_timeline) return;
    if (await this.edit([{ command: "timeline.set_primary", args: { timeline } }])) {
      this.set({ selection: null, also: [], playhead: 0 });
      await this.engine.preview.show(0);
    }
  }

  async renameTimeline(timeline: string, name: string): Promise<void> {
    if (name.trim()) await this.edit([{ command: "timeline.rename", args: { timeline, name } }]);
  }

  async removeTimeline(timeline: string): Promise<void> {
    await this.edit([{ command: "timeline.remove", args: { timeline } }]);
  }

  /** Sets the transition from a clip into the next, or removes it when `type` is empty. */
  async setTransition(clip: string, type: string, seconds = 0.5, params: Record<string, unknown> = {}): Promise<void> {
    if (!type) {
      await this.edit([{ command: "transition.remove", args: { clip } }]);
      return;
    }
    await this.edit([
      { command: "transition.set", args: { clip, type, duration: { seconds: String(seconds) }, params } },
    ]);
  }

  /**
   * Sets or clears one of a clip's animations (`in`, `out` or `combo`), lasting `seconds`
   * and waiting `delay` seconds first when given.
   */
  async setAnimation(clip: string, place: "in" | "out" | "combo", preset: string, seconds?: number, delay?: number): Promise<void> {
    const args: Record<string, unknown> = { clip };
    if (preset) {
      const animation: Record<string, unknown> = { preset };
      if (seconds !== undefined) animation.duration = { seconds: String(seconds) };
      if (delay !== undefined && delay > 0) animation.delay = { seconds: String(delay) };
      args[place] = animation;
    } else {
      args.clear = [place];
    }
    await this.edit([{ command: "animation.set", args }]);
  }

  /**
   * Animates the selected clips with one preset, each `step` seconds after the one before,
   * in `order` (`clips.stagger`), as one step.
   */
  async stagger(request: { preset: string; place: "in" | "out" | "combo"; step: number; duration?: number; order: string }): Promise<void> {
    const clips = this.selected();
    if (clips.length === 0 || !request.preset) return;
    const args: Record<string, unknown> = {
      clips,
      preset: request.preset,
      place: request.place,
      step: { seconds: String(request.step) },
      order: request.order,
    };
    if (request.duration !== undefined) args.duration = { seconds: String(request.duration) };
    await this.edit([{ command: "clips.stagger", args }]);
  }

  /**
   * Adds an effect at its defaults to the end of a clip's effects. Removing the background
   * and body effects analyze the clip first, installing their models once the person agrees.
   */
  async addEffect(clip: string, type: string): Promise<void> {
    if (type === SUBJECT_KEY) return this.removeBackground(clip);
    if (isBodyEffect(type)) return this.addBodyEffect(clip, type);
    await this.edit([{ command: "effect.add", args: { clip, type } }]);
  }

  /** Adds a built-in look's effects to a clip. */
  async applyLook(clip: string, look: string): Promise<void> {
    await this.edit([{ command: "look.apply", args: { clip, look } }]);
  }

  /** Adds an adjustment layer from the playhead, three seconds long. */
  async addAdjustment(): Promise<void> {
    if (!this.state.timeline) return;
    await this.edit([
      {
        command: "adjustment.add",
        args: { start: { flicks: this.state.playhead }, duration: { seconds: "3" } },
      },
    ]);
  }

  /**
   * Measures a clip and adds the engine's color correction as one step: automatic, or
   * matching `reference`.
   */
  async autoColor(clip: string, reference?: string): Promise<void> {
    try {
      const suggestion = await this.engine.colorSuggest(clip, reference);
      await this.edit([{ command: "effect.add", args: suggestion.effect_add }], reference ? t("app.color.match") : t("app.color.auto"));
    } catch (error) {
      this.notify(error);
    }
  }

  /**
   * Levels the loudness of clips with sound to `targetLufs`, the preferred
   * `editing.loudness_target` unless given: the engine measures them, and the volumes it
   * works out apply as one undoable step, with a note of what it did.
   */
  async normalizeLoudness(clips: string[], targetLufs = preferencesOf(this.state).editing.loudness_target): Promise<void> {
    if (clips.length === 0) {
      this.set({ notice: { kind: "info", text: t("app.normalize.select") } });
      return;
    }
    let normalization: Normalization;
    try {
      normalization = await this.engine.audioNormalize(clips, targetLufs);
    } catch (error) {
      this.notify(error);
      return;
    }
    if (normalization.commands.length > 0 && !(await this.edit(normalization.commands, t("app.normalize.reason")))) return;
    this.set({ notice: { kind: "info", text: levelled(normalization) } });
  }

  // ---- Measured edits ----

  /**
   * Runs an edit the engine measures from media and applies as one undoable step against
   * the revision on screen: busy while it measures, then what it did, or its error with
   * the hint. One runs at a time. A stale revision reloads and says so, as any edit.
   */
  private async measure<T>(
    kind: Measurement["kind"],
    clip: string | null,
    run: (revision: number) => Promise<T>,
    report: (result: T) => string,
  ): Promise<T | null> {
    const timeline = this.state.timeline;
    if (!timeline) return null;
    if (this.state.measurement?.busy) {
      this.set({ notice: { kind: "info", text: t("app.measure.busy") } });
      return null;
    }
    this.set({ measurement: { kind, clip, busy: true, report: null, error: null } });
    let result: T;
    try {
      result = await run(timeline.revision);
    } catch (error) {
      const api = asError(error);
      if (api.code === "fosscut.revision.conflict") {
        this.set({ measurement: null });
        await this.refresh();
        this.set({ notice: { kind: "info", text: t("app.conflict") } });
      } else {
        this.set({ measurement: { kind, clip, busy: false, report: null, error: api } });
      }
      return null;
    }
    await this.refresh();
    this.set({ measurement: { kind, clip, busy: false, report: report(result), error: null } });
    return result;
  }

  /** Puts a finished measurement's report or error away. */
  dismissMeasurement(): void {
    if (!this.state.measurement?.busy) this.set({ measurement: null, missingAnalysis: null });
    this.missing = null;
  }

  /** What runs again once the analysis the last measured edit missed has run. */
  private missing: { clip: string; analysis: MissingAnalysis; retry: () => Promise<void> } | null = null;

  /**
   * A measured edit that plans from analyses: when one is missing, the refusal shows with
   * an offer to run it (`runMissingAnalysis`), and the edit is tried again once it has.
   */
  private async planned<T>(
    kind: Measurement["kind"],
    clip: string | null,
    run: (revision: number) => Promise<T>,
    report: (result: T) => string,
    retry: () => Promise<void>,
  ): Promise<T | null> {
    this.missing = null;
    this.set({ missingAnalysis: null });
    const result = await this.measure(kind, clip, run, report);
    const analysis = missingAnalysis(this.state.measurement?.error);
    const target = clipShowing(this.state.timeline, analysis?.asset ?? null) ?? clip;
    if (!result && analysis && target && this.state.measurement?.kind === kind) {
      this.missing = { clip: target, analysis, retry };
      this.set({ missingAnalysis: { kind, analyzer: analysis.analyzer } });
    }
    return result;
  }

  /**
   * Runs the analysis the last measured edit missed (after the person agreed to install
   * the models it needs, if any), then tries the edit again.
   */
  async runMissingAnalysis(): Promise<void> {
    const missing = this.missing;
    if (!missing) return;
    this.missing = null;
    this.set({ measurement: null, missingAnalysis: null, analysisFor: this.state.missingAnalysis?.kind ?? null });
    const { analysis } = missing;
    const needs = analysisNeeds(analysis);
    const request: SubjectRequest =
      analysis.analyzer === "matte"
        ? { clip: missing.clip, kind: "matte", model: matteModel(analysis) }
        : { clip: missing.clip, kind: analysis.analyzer, ...(analysis.options ? { options: analysis.options } : {}) };
    await this.withModels("analysis", needs, () =>
      this.analyzeThen("analysis", request, needs, async () => {
        await missing.retry();
        return null;
      }),
    );
  }

  /** Keys a video clip so its crop follows a person or the subject, shot by shot when `shots`. */
  async reframe(clip: string, source: SubjectChoice, shots = false): Promise<void> {
    await this.planned(
      "reframe",
      clip,
      (revision) => this.engine.reframe.clip({ clip, source, shots, expected_revision: revision }),
      reframed,
      () => this.reframe(clip, source, shots),
    );
  }

  /** Keys a video clip as a camera following the subject would move, zoomed in by `zoom` (1 to 4). */
  async cameraTrack(clip: string, source: SubjectChoice, zoom: number): Promise<void> {
    await this.planned(
      "camera",
      clip,
      (revision) => this.engine.reframe.camera({ clip, source, zoom, expected_revision: revision }),
      reframed,
      () => this.cameraTrack(clip, source, zoom),
    );
  }

  /** Makes a vertical version of the shown timeline that follows its subject, and shows it. */
  async verticalVersion(source: SubjectChoice): Promise<void> {
    const timeline = this.state.timeline;
    if (!timeline) return;
    const result = await this.planned(
      "vertical",
      null,
      (revision) => this.engine.reframe.vertical({ timeline: timeline.timeline, source, expected_revision: revision }),
      verticalMade,
      () => this.verticalVersion(source),
    );
    if (result) await this.showMade(result.vertical.timeline);
  }

  /** Puts a marker on every `every`-th beat of a music clip. */
  async beatMarkers(clip: string, every = 1): Promise<void> {
    await this.planned(
      "beats",
      clip,
      (revision) => this.engine.beats.markers(clip, every, revision),
      beatsMarked,
      () => this.beatMarkers(clip, every),
    );
  }

  /** Moves the cuts of `track` onto every `every`-th beat of the music clip `music`. */
  async cutToBeats(music: string, track: string, every = 1): Promise<BeatCutsResult | null> {
    return this.planned(
      "cuts",
      music,
      (revision) => this.engine.beats.cut(track, music, every, revision),
      beatsCut,
      async () => void (await this.cutToBeats(music, track, every)),
    );
  }

  /** Looks for the moments of a long clip that would stand alone as shorts. */
  async findShorts(clip: string, options: { min_seconds?: number; max_seconds?: number; count?: number } = {}): Promise<void> {
    this.set({ shortList: null });
    const result = await this.planned(
      "shorts",
      clip,
      () => this.engine.shorts.candidates(clip, options),
      shortsFound,
      () => this.findShorts(clip, options),
    );
    if (result) this.set({ shortList: result });
  }

  /** Makes a short of part of a clip, as a new vertical timeline, and shows it. */
  async makeShort(request: Omit<ShortRequest, "expected_revision">): Promise<void> {
    const result = await this.planned(
      "short",
      request.clip,
      (revision) => this.engine.shorts.make({ ...request, expected_revision: revision }),
      shortMade,
      () => this.makeShort(request),
    );
    if (result) await this.showMade(result.short.timeline);
  }

  /** Shows a timeline a planned edit just made, from its start, keeping the report. */
  private async showMade(timeline: string): Promise<void> {
    const measurement = this.state.measurement;
    await this.showTimeline(timeline);
    if (measurement) this.set({ measurement });
  }

  /** Applies a voice preset (`voice.apply`) to a clip's sound, replacing its voice filters when `replace`. */
  async applyVoice(clip: string, preset: string, replace = true): Promise<void> {
    await this.edit([{ command: "voice.apply", args: { clip, preset, replace } }]);
  }

  /** Steadies a video clip's camera shake: `strength` from 0 (none) to 1 (very smooth). */
  async stabilize(clip: string, strength = 0.5): Promise<void> {
    await this.measure("stabilize", clip, (revision) => this.engine.measure.stabilize(clip, strength, revision), stabilized);
  }

  /** Splits a video clip at its shot cuts, so each shot is a clip of its own. */
  async splitAtScenes(clip: string): Promise<void> {
    const rate = this.rate();
    await this.measure("scenes", clip, (revision) => this.engine.measure.scenes(clip, revision), (r) => splitAtScenes(r, rate));
  }

  /**
   * Starts drawing the region of a video clip to track, on the frame at the playhead; the
   * playhead moves to the clip's start when it is outside the clip.
   */
  async pickRegion(clip: string): Promise<void> {
    const view = this.state.timeline ? findClip(this.state.timeline, clip) : null;
    if (!view) return;
    const playhead = this.state.playhead;
    if (playhead < view.start.flicks || playhead >= view.end.flicks) await this.seek(view.start.flicks);
    this.set({ regionPick: { clip, drawing: true, region: null, at: this.state.playhead }, maskTool: null });
  }

  /** The region drawn over the preview, on the frame at the playhead. */
  setRegion(region: Region): void {
    const pick = this.state.regionPick;
    if (pick) this.set({ regionPick: { ...pick, drawing: false, region, at: this.state.playhead } });
  }

  /** Stops drawing the region, or forgets the one drawn. */
  cancelRegion(): void {
    this.set({ regionPick: null });
  }

  /**
   * Follows the region drawn from the time it was drawn at to the clip's end, and keys
   * `target` to it: `mask:<id>` (a mask of the clip) or `clip:<id>` (another clip).
   */
  async track(target: string): Promise<void> {
    const pick = this.state.regionPick;
    const which = targetOf(target);
    const region = pick?.region;
    if (!pick || !region || !which) {
      this.set({ notice: { kind: "info", text: t("app.track.draw") } });
      return;
    }
    const rate = this.rate();
    const request = { clip: pick.clip, region, at: pick.at, ...which };
    const done = await this.measure(
      "track",
      pick.clip,
      (revision) => this.engine.measure.track({ ...request, expected_revision: revision }),
      (r) => tracked(r, rate),
    );
    if (done) this.set({ regionPick: null });
  }

  /** Makes a multicam clip of video assets and selects it; true when it was made. */
  async createMulticam(request: MulticamRequest): Promise<boolean> {
    const result = await this.measure(
      "multicam",
      null,
      (revision) => this.engine.measure.multicam({ ...request, expected_revision: revision }),
      (r) => multicamMade(r, request.angles.length),
    );
    const made = result?.response.results[0] as { clip?: { clip?: string } } | undefined;
    if (made?.clip?.clip) this.set({ selection: made.clip.clip, also: [] });
    return result !== null;
  }

  /**
   * Cuts the selected multicam clip to angle `angle` (from 1) at the playhead: the part
   * under the playhead splits there and shows the angle from then on, and becomes the
   * selection, so the next cut follows on. A part already showing it is left alone.
   */
  async switchAngle(angle: number): Promise<void> {
    const timeline = this.state.timeline;
    if (!timeline) return;
    const at = this.state.playhead;
    const part = multicamPartAt(timeline, this.state.selection, at);
    if (!part?.angles) {
      this.set({ notice: { kind: "info", text: t("app.multicam.select") } });
      return;
    }
    const chosen = part.angles[angle - 1];
    if (!chosen) {
      this.set({ notice: { kind: "info", text: t("app.multicam.angles", { count: part.angles.length }) } });
      return;
    }
    if (part.angle === chosen.id) {
      if (part.id !== this.state.selection) this.set({ selection: part.id, also: [] });
      return;
    }
    const response = await this.apply(
      [{ command: "multicam.switch", args: { clip: part.id, at: { flicks: at }, angle } }],
      t("app.multicam.cut", { angle }),
    );
    const switched = response?.results[0] as { clip?: { clip?: string } } | undefined;
    if (switched?.clip?.clip) this.set({ selection: switched.clip.clip, also: [] });
  }

  // ---- Recording ----

  private setRecorder(patch: Partial<Recorder>): void {
    this.set({ recorder: { ...this.state.recorder, ...patch } });
  }

  /** Opens the record panel and reads the devices again, as they come and go. */
  async openRecorder(): Promise<void> {
    // The two panels share the preview's corner.
    if (this.state.speech.open) this.closeSpeech();
    this.setRecorder({ open: true, error: null });
    try {
      this.setRecorder({ devices: await this.engine.record.devices() });
    } catch (error) {
      this.setRecorder({ devices: this.state.recorder.devices ?? [], error: asError(error) });
    }
  }

  /** Closes the record panel; a recording that is running goes on. */
  closeRecorder(): void {
    this.setRecorder({ open: false });
  }

  /** Starts recording; it is placed where the playhead is now once it is stopped. */
  async startRecording(choice: Omit<RecordChoice, "at">): Promise<boolean> {
    if (!this.state.opened || this.state.recorder.job?.state === "running") return false;
    this.setRecorder({ error: null });
    try {
      this.setRecorder({ job: await this.engine.record.start({ ...choice, at: this.state.playhead }) });
      return true;
    } catch (error) {
      this.setRecorder({ error: asError(error) });
      return false;
    }
  }

  /** Reads the running recording's time again; the panel asks four times a second. */
  async pollRecording(): Promise<void> {
    const job = this.state.recorder.job;
    if (job?.state !== "running") return;
    try {
      await this.recorded(await this.engine.record.status(job.job));
    } catch (error) {
      this.setRecorder({ error: asError(error) });
    }
  }

  /** Stops the recording, keeping what it recorded. */
  async stopRecording(): Promise<void> {
    const job = this.state.recorder.job;
    if (job?.state !== "running") return;
    try {
      await this.recorded(await this.engine.record.stop(job.job));
    } catch (error) {
      this.setRecorder({ error: asError(error) });
    }
  }

  /**
   * A recording's new status. The first time it shows finished, the project is read again
   * and the clip it became is selected (the engine imports and places it once); a failure,
   * or a file that could not be imported, shows in the panel.
   */
  private async recorded(status: RecordState): Promise<void> {
    const before = this.state.recorder.job;
    // An answer for a recording the panel has left behind changes nothing.
    if (before?.job !== status.job) return;
    this.setRecorder({ job: status });
    if (before.state !== "running" || status.state === "running") return;
    const error = status.state === "failed" ? status.error : status.import_error;
    if (error) this.setRecorder({ error });
    if (status.state !== "succeeded") return;
    await this.refresh();
    if (status.clip) this.set({ selection: status.clip, also: [] });
    if (!this.state.recorder.open && !error) {
      this.set({ notice: { kind: "info", text: t("app.recording.placed") } });
    }
  }

  // ---- Local models (ADR-0033) ----

  /** Reads which models are installed; null when they cannot be read. */
  async loadModels(): Promise<ModelListing | null> {
    try {
      const models = await this.engine.models.list();
      this.set({ models });
      return models;
    } catch {
      return null;
    }
  }

  private setInstall(patch: Partial<InstallPrompt>): void {
    const install = this.state.install;
    if (install) this.set({ install: { ...install, ...patch } });
  }

  private setAssist(patch: Partial<Assist>): void {
    const assist = this.state.assist;
    if (assist) this.set({ assist: { ...assist, ...patch } });
  }

  /** Asks the person to install `models` for `purpose`; `then` runs once they are installed. */
  private askToInstall(purpose: AssistKind, models: ModelInfo[], then: () => Promise<void>): void {
    this.afterInstall = then;
    this.set({ install: { purpose, models, installing: false, index: 0, current: null, error: null } });
  }

  /**
   * Runs `then` once the models of `needs` are installed: at once when they are (or when
   * the listing cannot be read, so the engine says what is missing), otherwise after the
   * person agreed to install the missing ones.
   */
  private async withModels(purpose: AssistKind, needs: string[], then: () => Promise<void>): Promise<void> {
    if (this.state.assist?.busy) {
      this.set({ notice: { kind: "info", text: t("app.assist.busy") } });
      return;
    }
    const listing = await this.loadModels();
    const missing = listing ? missingModels(listing, needs) : [];
    if (missing.length === 0) await then();
    else this.askToInstall(purpose, missing, then);
  }

  /**
   * Offers to install what the engine said is missing, retrying with `retry` once it is;
   * false when the refusal is about something else.
   */
  private async offerInstall(purpose: AssistKind, error: ApiError, needs: string[], retry: () => Promise<void>): Promise<boolean> {
    const listing = await this.loadModels();
    const models = listing ? modelsFor(error, listing, needs) : [];
    if (models.length === 0) return false;
    this.askToInstall(purpose, models, retry);
    return true;
  }

  /** The person agreed to the models shown: they install one after the other. */
  async confirmInstall(): Promise<void> {
    const prompt = this.state.install;
    if (!prompt || prompt.installing) return;
    this.setInstall({ installing: true, error: null, index: 0, current: null });
    await this.installFrom(0);
  }

  /** Installs the prompt's models from `index` on, then runs what waited for them. */
  private async installFrom(index: number): Promise<void> {
    const prompt = this.state.install;
    if (!prompt?.installing) return;
    const model = prompt.models[index];
    if (!model) {
      const then = this.afterInstall;
      this.afterInstall = null;
      this.set({ install: null });
      await this.loadModels();
      await then?.();
      return;
    }
    let state: ModelInstallState;
    try {
      state = await this.engine.models.install(model.name, true);
    } catch (error) {
      this.setInstall({ installing: false, error: asError(error) });
      return;
    }
    await this.installRead(index, state);
  }

  /** An install's new state: the next model once it succeeded, the error once it failed. */
  private async installRead(index: number, state: ModelInstallState): Promise<void> {
    // An answer after the dialog was put away changes nothing.
    if (!this.state.install?.installing) return;
    this.setInstall({ index, current: state });
    if (state.state === "succeeded") await this.installFrom(index + 1);
    else if (state.state !== "running") {
      this.setInstall({ installing: false, error: state.error ?? { code: "fosscut.model.download_failed", message: t("models.install.failed", { name: state.name }) } });
    }
  }

  /** Reads the running install again; the install dialog asks twice a second. */
  async pollInstall(): Promise<void> {
    const prompt = this.state.install;
    const current = prompt?.current;
    if (!prompt?.installing || current?.state !== "running") return;
    let state: ModelInstallState;
    try {
      state = await this.engine.models.installStatus(current.name);
    } catch (error) {
      this.setInstall({ installing: false, error: asError(error) });
      return;
    }
    const now = this.state.install;
    if (now?.current?.name !== state.name || now.current.state !== "running") return;
    await this.installRead(now.index, state);
  }

  /** Puts the install dialog away without installing; an install that runs goes on. */
  cancelInstall(): void {
    if (this.state.install?.installing) return;
    this.afterInstall = null;
    this.set({ install: null });
  }

  /**
   * Runs work with local models for the panel of `kind`: busy while it runs, then what it
   * did, or its error with the hint. A model that is missing turns into the install dialog,
   * and the work runs again once it is installed; a stale revision reloads and says so.
   */
  private async assisted(kind: AssistKind, clip: string | null, needs: string[], run: () => Promise<string | null>): Promise<boolean> {
    if (!this.state.timeline) return false;
    if (this.state.assist?.busy) {
      this.set({ notice: { kind: "info", text: t("app.assist.busy") } });
      return false;
    }
    this.set({ assist: { kind, clip, busy: true, job: null, report: null, error: null } });
    try {
      const report = await run();
      this.set({ assist: { kind, clip, busy: false, job: null, report, error: null } });
      return true;
    } catch (error) {
      const api = asError(error);
      this.set({ assist: null });
      if (api.code === "fosscut.revision.conflict") {
        await this.refresh();
        this.set({ notice: { kind: "info", text: t("app.conflict") } });
        return false;
      }
      const retry = async () => void (await this.assisted(kind, clip, needs, run));
      if (await this.offerInstall(kind, api, needs, retry)) return false;
      this.set({ assist: { kind, clip, busy: false, job: null, report: null, error: api } });
      return false;
    }
  }

  /**
   * Analyzes a video clip's subject as a job, then runs `then` (which adds or changes the
   * effect) once it succeeded. Cached analyses answer at once.
   */
  private async analyzeThen(kind: AssistKind, request: SubjectRequest, needs: string[], then: () => Promise<string | null>): Promise<void> {
    if (!this.state.timeline) return;
    if (this.state.assist?.busy) {
      this.set({ notice: { kind: "info", text: t("app.assist.busy") } });
      return;
    }
    this.set({ assist: { kind, clip: request.clip, busy: true, job: null, report: null, error: null } });
    let status: JobStatus;
    try {
      status = await this.engine.subject.analyze(request);
    } catch (error) {
      const api = asError(error);
      this.set({ assist: null });
      const retry = () => this.analyzeThen(kind, request, needs, then);
      if (await this.offerInstall(kind, api, needs, retry)) return;
      this.set({ assist: { kind, clip: request.clip, busy: false, job: null, report: null, error: api } });
      return;
    }
    this.afterAnalysis = then;
    await this.analysisRead(status);
  }

  /** Reads the running analysis again; the panel that started it asks twice a second. */
  async pollAnalysis(): Promise<void> {
    const job = this.state.assist?.job;
    if (!this.state.assist?.busy || job?.state !== "running") return;
    try {
      await this.analysisRead(await this.engine.subject.status(job.job));
    } catch (error) {
      this.afterAnalysis = null;
      this.setAssist({ busy: false, error: asError(error) });
    }
  }

  /** Stops the running analysis; nothing is added. */
  async cancelAnalysis(): Promise<void> {
    const job = this.state.assist?.job;
    if (!this.state.assist?.busy || job?.state !== "running") return;
    try {
      await this.analysisRead(await this.engine.subject.cancel(job.job));
    } catch (error) {
      this.afterAnalysis = null;
      this.setAssist({ busy: false, error: asError(error) });
    }
  }

  /** An analysis's new status: once it succeeded, what waited for it runs, once. */
  private async analysisRead(status: JobStatus): Promise<void> {
    const assist = this.state.assist;
    if (!assist?.busy) return;
    // An answer for another job, or for one already finished, changes nothing.
    if (assist.job && (assist.job.job !== status.job || assist.job.state !== "running")) return;
    this.setAssist({ job: status });
    if (status.state === "running") return;
    const then = this.afterAnalysis;
    this.afterAnalysis = null;
    if (status.state === "succeeded") {
      const report = then ? await then() : null;
      this.setAssist({ busy: false, report });
    } else if (status.state === "cancelled") {
      this.setAssist({ busy: false, report: t("models.analysis.cancelled") });
    } else {
      this.setAssist({ busy: false, error: status.error ?? { code: "fosscut.internal", message: t("models.analysis.failed") } });
    }
  }

  /** Puts a finished piece of work's report or error away. */
  dismissAssist(): void {
    if (!this.state.assist?.busy) this.set({ assist: null });
  }

  /**
   * Removes a video clip's background: analyzes its matte (a person by default, or any
   * subject, slowly), then adds the effect, or points the effect there is at that matte.
   */
  async removeBackground(clip: string, source: MatteSource = "person"): Promise<void> {
    const needs = backgroundNeeds(source);
    await this.withModels("background", needs, () =>
      this.analyzeThen("background", { clip, kind: "matte", model: source }, needs, async () => {
        const view = this.state.timeline ? findClip(this.state.timeline, clip) : null;
        const effect = view?.effects?.find((e) => e.type === SUBJECT_KEY);
        if (effect && (effect.params?.source ?? "person") === source) return t("models.background.done");
        const command = effect
          ? { command: "effect.set", args: { clip, effect: effect.id, params: { source } } }
          : { command: "effect.add", args: { clip, type: SUBJECT_KEY, params: { source } } };
        return (await this.edit([command], t("models.background.reason"))) ? t("models.background.done") : null;
      }),
    );
  }

  /** Adds a body effect to a video clip once its pose is analyzed. */
  async addBodyEffect(clip: string, type: string): Promise<void> {
    await this.withModels("body", BODY_NEEDS, () =>
      this.analyzeThen("body", { clip, kind: "pose" }, BODY_NEEDS, async () => {
        const title = this.state.catalog?.effects.find((e) => e.type === type)?.title ?? type;
        const added = await this.edit([{ command: "effect.add", args: { clip, type } }]);
        return added ? t("models.body.done", { effect: title }) : null;
      }),
    );
  }

  /** Opens the text to speech panel and reads the voices. */
  async openSpeech(): Promise<void> {
    // The two panels share the preview's corner; a recording that runs goes on.
    if (this.state.recorder.open) this.closeRecorder();
    this.set({ speech: { ...this.state.speech, open: true, error: null } });
    try {
      const voices = await this.engine.voice.list();
      this.set({ speech: { ...this.state.speech, voices } });
    } catch (error) {
      this.set({ speech: { ...this.state.speech, voices: this.state.speech.voices ?? [], error: asError(error) } });
    }
  }

  closeSpeech(): void {
    this.set({ speech: { ...this.state.speech, open: false } });
  }

  /** Speaks the text and plays it without adding it. */
  async previewSpeech(text: string, voice: string, speed: number): Promise<void> {
    if (!text.trim()) return;
    await this.withModels("preview", VOICE_NEEDS, async () =>
      void (await this.assisted("preview", null, VOICE_NEEDS, async () => {
        const preview = await this.engine.voice.preview(text, voice, speed);
        return preview.played ? null : t("voice.preview.silent");
      })),
    );
  }

  /** Speaks the text and places it at the playhead as one step, selecting the clip it became. */
  async insertSpeech(text: string, voice: string, speed: number): Promise<void> {
    if (!text.trim()) return;
    await this.withModels("speech", VOICE_NEEDS, async () =>
      void (await this.assisted("speech", null, VOICE_NEEDS, async () => {
        const timeline = this.state.timeline;
        const result = await this.engine.voice.add({
          text,
          voice,
          speed,
          at: this.state.playhead,
          expected_revision: timeline?.revision,
        });
        await this.refresh();
        const placed = (result.response?.results ?? [])
          .map((r) => (r as { clip?: string } | null)?.clip)
          .find((c): c is string => typeof c === "string");
        if (placed) this.set({ selection: placed, also: [] });
        return t("voice.speech.added", { seconds: Math.round(result.speech.file.seconds * 10) / 10 });
      })),
    );
  }

  /** Translates the caption track of `clip` into `to`, under each page or instead of its words. */
  async translateCaptions(clip: string, to: "hu" | "en", mode: "bilingual" | "replace"): Promise<void> {
    const needs = translateNeeds(to);
    await this.withModels("translate", needs, async () =>
      void (await this.assisted("translate", clip, needs, async () => {
        const timeline = this.state.timeline;
        const track = timeline ? trackOf(timeline, clip) : undefined;
        const result = await this.engine.captions.translate({ track, to, mode, expected_revision: timeline?.revision });
        await this.refresh();
        return translatedReport(result);
      })),
    );
  }

  /** Dubs the caption track of `clip` with a voice, speaking its words or its translations. */
  async dubCaptions(clip: string, voice: string, speed: number, translation: boolean): Promise<void> {
    await this.withModels("dub", VOICE_NEEDS, async () =>
      void (await this.assisted("dub", clip, VOICE_NEEDS, async () => {
        const timeline = this.state.timeline;
        const track = timeline ? trackOf(timeline, clip) : undefined;
        const result = await this.engine.captions.dub({ track, voice, speed, translation, expected_revision: timeline?.revision });
        await this.refresh();
        return dubbedReport(result);
      })),
    );
  }

  /** Reads the voices once, for the caption track's dubbing. */
  async loadVoices(): Promise<void> {
    if (this.state.speech.voices) return;
    try {
      this.set({ speech: { ...this.state.speech, voices: await this.engine.voice.list() } });
    } catch (error) {
      this.set({ speech: { ...this.state.speech, voices: [], error: asError(error) } });
    }
  }

  // ---- Library ----

  /** Reads the built-in library once; it is the same for every project. */
  async loadLibrary(): Promise<void> {
    if (this.state.library) return;
    try {
      this.set({ library: await this.engine.libraryList() });
    } catch (error) {
      this.notify(error);
    }
  }

  /**
   * Adds a library item against the revision on screen and selects the clip it placed; a
   * template's new timeline shows from its start. A stale revision reloads and says so.
   */
  async addFromLibrary(request: Omit<LibraryAddRequest, "expected_revision">): Promise<boolean> {
    const timeline = this.state.timeline;
    if (!timeline) return false;
    let response: EditResponse;
    try {
      response = await this.engine.libraryAdd({ ...request, expected_revision: timeline.revision });
    } catch (error) {
      // A sound may have been imported before its placement failed.
      await this.refresh();
      const api = asError(error);
      if (api.code === "fosscut.revision.conflict") {
        this.set({ notice: { kind: "info", text: t("app.conflict") } });
      } else {
        this.notify(api);
      }
      return false;
    }
    await this.refresh();
    const result = response.results[0] as { clip?: string; timeline?: string } | undefined;
    if (result?.timeline) {
      this.set({ selection: null, also: [], playhead: 0 });
      await this.engine.preview.show(0);
    } else if (result?.clip && request.kind !== "look") {
      this.set({ selection: result.clip, also: [] });
    }
    return true;
  }

  // ---- Shapes ----

  /** Changes a shape clip with one `shape.set` merge patch. */
  async setShape(clip: string, patch: Record<string, unknown> | null): Promise<void> {
    if (!patch || Object.keys(patch).length === 0) return;
    await this.edit([{ command: "shape.set", args: { clip, set: patch } }]);
  }

  /**
   * Adds a camera from the playhead to the timeline's end (ADR-0037), on a camera track
   * above the video, and selects it.
   */
  async addCamera(): Promise<void> {
    if (!this.state.timeline) return;
    const response = await this.apply([{ command: "camera.add", args: { start: { flicks: this.state.playhead } } }]);
    const clip = (response?.results[0] as { clip?: string } | undefined)?.clip;
    if (clip) this.set({ selection: clip, also: [] });
  }

  /** Adds a 3D object from a preset or a model asset at the playhead (`model.add`), and selects it. */
  async addModel(source: { preset: string } | { asset: string }, start = this.state.playhead): Promise<void> {
    if (!this.state.timeline) return;
    const response = await this.apply([{ command: "model.add", args: { ...source, start: { flicks: start } } }]);
    const clip = (response?.results[0] as { clip?: string } | undefined)?.clip;
    if (clip) this.set({ selection: clip, also: [] });
  }

  /**
   * Turns a vector drawing into a compound clip of editable shapes at the playhead
   * (`vector.explode`, ADR-0036); the app reads the drawing's paths. Selects the clip.
   */
  async explodeDrawing(asset: string): Promise<void> {
    if (!this.state.timeline) return;
    const response = await this.apply([{ command: "vector.explode", args: { asset, start: { flicks: this.state.playhead } } }]);
    const placed = (response?.results[0] as { compound?: { clip?: string } } | undefined)?.compound?.clip;
    if (placed) this.set({ selection: placed, also: [] });
  }

  // ---- Web compositions and Remotion (ADR-0039) ----

  /** Asks for a page (or a folder) to import as a web composition, and opens the dialog with it. */
  async pickComposition(folder: boolean): Promise<void> {
    if (!this.state.opened) return;
    try {
      const page = await this.engine.web.pick(folder);
      if (page) this.set({ webImport: { page, busy: false, error: null } });
    } catch (error) {
      this.notify(error);
    }
  }

  closeWebImport(): void {
    if (!this.state.webImport?.busy) this.set({ webImport: null });
  }

  /** Imports the composition chosen with the values the dialog gives, then adds it at the playhead when `place`. */
  async importComposition(request: WebImportRequest, place: boolean): Promise<boolean> {
    const dialog = this.state.webImport;
    if (!dialog) return false;
    this.set({ webImport: { ...dialog, busy: true, error: null } });
    try {
      const imported = await this.engine.web.import(request);
      this.set({ webImport: null });
      await this.refresh();
      if (place) await this.addToTimeline(imported.asset);
      return true;
    } catch (error) {
      this.set({ webImport: { ...dialog, busy: false, error: asError(error) } });
      return false;
    }
  }

  private setRemotion(patch: Partial<RemotionDialog>): void {
    this.set({ remotion: { ...this.state.remotion, ...patch } });
  }

  /** Opens the Remotion dialog at its first step, keeping a render that runs. */
  openRemotion(): void {
    const running = this.state.remotion.job?.state === "running";
    this.setRemotion({ open: true, ...(running ? {} : { step: "choose", licence: null, command: [], job: null, error: null }) });
  }

  closeRemotion(): void {
    this.setRemotion({ open: false });
  }

  /** Asks the user for the Remotion project's folder; null when they cancel. */
  async pickRemotionProject(title: string): Promise<string | null> {
    try {
      return await this.engine.pickFolder(title);
    } catch (error) {
      this.setRemotion({ error: asError(error) });
      return null;
    }
  }

  /**
   * Checks a render without running it: the engine refuses it until the user agrees, with
   * the licence note and the command, which the dialog shows next. Other refusals (not a
   * Remotion project, Remotion or Node.js missing) show as they are.
   */
  async checkRemotion(choice: Omit<RemotionChoice, "confirmed">): Promise<void> {
    this.setRemotion({ busy: true, error: null, choice });
    try {
      const job = await this.engine.remotion.start({ ...choice, confirmed: false });
      // An engine that runs without asking still renders what was chosen.
      this.setRemotion({ busy: false, step: "render", job });
    } catch (error) {
      const api = asError(error);
      if (api.code === "fosscut.web.confirmation_required") {
        const details = (api.details ?? {}) as { licence?: string; command?: string[] };
        this.setRemotion({ busy: false, step: "agree", licence: details.licence ?? api.message, command: details.command ?? [] });
      } else {
        this.setRemotion({ busy: false, error: api });
      }
    }
  }

  /** Starts the render the user agreed to. */
  async confirmRemotion(): Promise<void> {
    const choice = this.state.remotion.choice;
    if (!choice) return;
    this.setRemotion({ busy: true, error: null });
    try {
      const job = await this.engine.remotion.start({ ...choice, confirmed: true });
      this.setRemotion({ busy: false, step: "render", job });
    } catch (error) {
      this.setRemotion({ busy: false, error: asError(error) });
    }
  }

  /** Reads the render's progress; once it succeeded, the media bin shows what it became. */
  async pollRemotion(): Promise<void> {
    const job = this.state.remotion.job;
    if (job?.state !== "running") return;
    try {
      const status = await this.engine.remotion.status(job.job);
      if (this.state.remotion.job?.job !== status.job) return;
      this.setRemotion({ job: status });
      if (status.state === "succeeded") await this.refresh();
      if (status.state !== "running" && !this.state.remotion.open) {
        const file = (status.output ?? status.job).split(/[\\/]/).pop() ?? status.job;
        this.set({
          notice:
            status.state === "succeeded"
              ? { kind: "info", text: t("web.remotion.done", { file }) }
              : { kind: "error", text: t("web.remotion.failed", { reason: status.error?.message ?? file }) },
        });
      }
    } catch (error) {
      this.setRemotion({ error: asError(error) });
    }
  }

  /** Stops the render; Remotion stops and no file is kept. */
  async cancelRemotion(): Promise<void> {
    const job = this.state.remotion.job;
    if (job?.state !== "running") return;
    try {
      this.setRemotion({ job: await this.engine.remotion.cancel(job.job) });
    } catch (error) {
      this.setRemotion({ error: asError(error) });
    }
  }

  /** Back to the first step, to render another composition. */
  resetRemotion(): void {
    this.setRemotion({ step: "choose", licence: null, command: [], job: null, error: null });
  }

  // ---- Export ----

  private setExporting(patch: Partial<Exporting>): void {
    this.set({ exporting: { ...this.state.exporting, ...patch } });
  }

  /** Opens the export dialog, reading the presets the first time. */
  async openExport(): Promise<void> {
    this.setExporting({ open: true, error: null });
    if (this.state.exporting.presets) return;
    try {
      this.setExporting({ presets: await this.engine.exportPresets() });
    } catch (error) {
      this.setExporting({ error: asError(error) });
    }
  }

  /** Closes the dialog; an export that is running goes on. */
  closeExport(): void {
    this.setExporting({ open: false });
  }

  /** Asks where to save the export; null when the user cancels. */
  async pickExportPath(name: string, extension?: string): Promise<string | null> {
    try {
      return await this.engine.exportPick(name, extension);
    } catch (error) {
      this.setExporting({ error: asError(error) });
      return null;
    }
  }

  /** Starts an export; a file already there is replaced only after asking. */
  async startExport(choice: ExportChoice): Promise<boolean> {
    this.setExporting({ error: null });
    try {
      this.setExporting({ job: await this.engine.exportStart(choice) });
      return true;
    } catch (error) {
      const api = asError(error);
      if (api.code === "fosscut.guard.destructive" && !choice.overwrite) {
        if (this.confirm(t("app.export.replace", { message: api.message }))) return this.startExport({ ...choice, overwrite: true });
        return false;
      }
      this.setExporting({ error: api });
      return false;
    }
  }

  /** Reads the running export's progress again; the dialog asks twice a second. */
  async pollExport(): Promise<void> {
    const job = this.state.exporting.job;
    if (job?.state !== "running") return;
    try {
      const status = await this.engine.exportStatus(job.job);
      // An answer for an export the dialog has left behind changes nothing.
      if (this.state.exporting.job?.job !== status.job) return;
      this.setExporting({ job: status });
      if (status.state !== "running" && !this.state.exporting.open) this.set({ notice: finished(status) });
    } catch (error) {
      this.setExporting({ error: asError(error) });
    }
  }

  /** Stops the running export; nothing is written. */
  async cancelExport(): Promise<void> {
    const job = this.state.exporting.job;
    if (job?.state !== "running") return;
    try {
      this.setExporting({ job: await this.engine.exportCancel(job.job) });
    } catch (error) {
      this.setExporting({ error: asError(error) });
    }
  }

  /** Puts a finished export aside, to make another. */
  clearExport(): void {
    this.setExporting({ job: null, error: null });
  }

  // ---- Text ----

  /** Changes a text clip with `text.set`: `text`, `preset`, a `style` patch, `spans` or `reveal`. */
  async setText(clip: string, change: Record<string, unknown> | null): Promise<void> {
    if (!change || Object.keys(change).length === 0) return;
    await this.edit([{ command: "text.set", args: { clip, ...change } }]);
  }

  /** Saves a text clip's style in the project as `name`, asking before replacing one. */
  async saveTextPreset(clip: string, name: string): Promise<boolean> {
    const preset = name.trim();
    if (!preset) return false;
    const args: Record<string, unknown> = { name: preset, clip };
    if (this.state.opened?.info.text_presets?.includes(preset)) {
      if (!this.confirm(t("app.preset.replace", { name: preset }))) return false;
      args.replace = true;
    }
    if (!(await this.edit([{ command: "text.preset_save", args }]))) return false;
    this.set({ notice: { kind: "info", text: t("app.preset.saved", { name: preset }) } });
    return true;
  }

  /**
   * Adds a title at the playhead and selects it: a built-in template, or plain text when
   * `template` is empty.
   */
  async addTitle(template: string): Promise<void> {
    if (!this.state.timeline) return;
    const start = { flicks: this.state.playhead };
    const command = template
      ? { command: "text.template", args: { template, start } }
      : { command: "text.add", args: { text: t("app.title.text"), start } };
    const response = await this.apply([command]);
    const placed = response?.results[0] as { clip?: string } | undefined;
    if (placed?.clip) this.set({ selection: placed.clip, also: [] });
  }

  /** Deactivates the selected clip, or activates it again. */
  async toggleEnabled(): Promise<void> {
    const timeline = this.state.timeline;
    const id = this.state.selection;
    const clip = timeline && id ? findClip(timeline, id) : null;
    if (!clip) return;
    await this.setProperties(clip.id, { enabled: clip.enabled === false });
  }

  // ---- Preferences and Settings ----

  /** Reads the preferences and applies the language and shortcuts they choose. */
  async loadPreferences(): Promise<void> {
    try {
      this.applyPreferences(await this.engine.preferences.get());
    } catch (error) {
      this.notify(error);
    }
  }

  private applyPreferences(view: PreferencesView): void {
    const { preferences } = view;
    setLanguage(preferences.language);
    applyShortcuts(preferences.shortcuts.set, preferences.shortcuts.overrides);
    const linked = preferencesOf(this.state).timeline.linked;
    // A changed default for linked editing applies at once; a session's own switch stays.
    const linkage = preferences.timeline.linked !== linked ? preferences.timeline.linked : this.state.linkage;
    this.set({ preferences: view, linkage });
  }

  private setSettings(patch: Partial<SettingsState>): void {
    this.set({ settings: { ...this.state.settings, ...patch } });
  }

  /**
   * Sets the dotted preference `key` to `value` and saves it. A refusal changes nothing
   * and shows beside the control, as the engine worded it; true when it was saved.
   */
  async setPreference(key: string, value: unknown): Promise<boolean> {
    return this.changePreference(key, () => this.engine.preferences.set(key, value));
  }

  /** Returns the dotted preference `key` to its default. */
  async resetPreference(key: string): Promise<boolean> {
    return this.changePreference(key, () => this.engine.preferences.reset(key));
  }

  private async changePreference(key: string, change: () => Promise<PreferencesView>): Promise<boolean> {
    try {
      const view = await change();
      const errors = { ...this.state.settings.errors };
      delete errors[key];
      this.setSettings({ errors });
      this.applyPreferences(view);
      if (key === "updates") this.scheduleUpdates();
      return true;
    } catch (error) {
      this.setSettings({ errors: { ...this.state.settings.errors, [key]: asError(error).message } });
      return false;
    }
  }

  /** Saves a shortcut set and the person's changes to it, as one preference. */
  async saveShortcuts(set: string, overrides: Record<string, string>): Promise<boolean> {
    return this.setPreference("shortcuts", { set, overrides });
  }

  /** Asks for the folder of `locations.<which>` and saves it; nothing when cancelled. */
  async pickLocation(which: "projects" | "recordings" | "exports"): Promise<void> {
    const key = `locations.${which}`;
    try {
      const current = preferencesOf(this.state).locations[which] ?? undefined;
      const folder = await this.engine.pickFolder(t(`settings.locations.${which}.pick`), current);
      if (folder) await this.setPreference(key, folder);
    } catch (error) {
      this.setSettings({ errors: { ...this.state.settings.errors, [key]: asError(error).message } });
    }
  }

  /** Opens Settings, at `section` when given, and reads what its sections show. */
  openSettings(section?: SettingsSection): void {
    this.setSettings({ open: true, section: section ?? this.state.settings.section });
    void this.loadSounds();
    void this.loadCacheUsage();
  }

  closeSettings(): void {
    this.setSettings({ open: false, errors: {} });
  }

  showSettings(section: SettingsSection): void {
    this.setSettings({ section });
  }

  /** Reads the library's sounds once, for the export sound. */
  private async loadSounds(): Promise<void> {
    if (this.state.settings.sounds) return;
    try {
      this.setSettings({ sounds: await this.engine.libraryList("sound") });
    } catch {
      this.setSettings({ sounds: [] });
    }
  }

  /** Plays a library sound, as a finished export would. */
  async playSound(name: string): Promise<void> {
    try {
      await this.engine.playSound(name);
    } catch (error) {
      this.setSettings({ errors: { ...this.state.settings.errors, "export.sound_name": asError(error).message } });
    }
  }

  /** Reads the caches' sizes. */
  async loadCacheUsage(): Promise<void> {
    try {
      this.setSettings({ cache: await this.engine.cache.usage(), cacheError: null });
    } catch (error) {
      this.setSettings({ cacheError: asError(error).message });
    }
  }

  /** Empties a cache, or all of them, after asking. */
  async clearCache(kind: CacheKind | "all"): Promise<void> {
    if (!this.confirm(t(kind === "all" ? "settings.cache.confirmAll" : "settings.cache.confirm", { cache: t(`settings.cache.kind.${kind === "all" ? "analysis" : kind}`) }))) {
      return;
    }
    try {
      this.setSettings({ cache: await this.engine.cache.clear(kind), cacheError: null });
    } catch (error) {
      this.setSettings({ cacheError: asError(error).message });
    }
  }

  /** Checks for a newer release now; it shows until dismissed. */
  async checkUpdates(): Promise<void> {
    try {
      const answer = await this.engine.updates.check();
      if (answer.checked && answer.newer) this.set({ update: answer });
    } catch {
      // Checking is a courtesy: offline or refused, the app carries on.
    }
  }

  /**
   * Checks for updates when the app starts unless `updates` is off, and daily while it
   * runs when it is `check`.
   */
  private async watchUpdates(): Promise<void> {
    this.scheduleUpdates();
    if (preferencesOf(this.state).updates !== "off") await this.checkUpdates();
  }

  /** Checks daily while `updates` is `check`, and stops when it changes. */
  private scheduleUpdates(): void {
    const daily = preferencesOf(this.state).updates === "check";
    if (daily && this.updateTimer === null) {
      this.updateTimer = setInterval(() => void this.checkUpdates(), UPDATE_INTERVAL);
    } else if (!daily && this.updateTimer !== null) {
      clearInterval(this.updateTimer);
      this.updateTimer = null;
    }
  }

  dismissUpdate(): void {
    this.set({ update: null });
  }

  /** Opens the newer release's page in the browser. */
  async openRelease(): Promise<void> {
    try {
      await this.engine.updates.open();
    } catch (error) {
      this.notify(error);
    }
  }

  async undo(): Promise<void> {
    if (this.state.opened?.info.can_undo) await this.edit([{ command: "history.undo" }]);
  }

  async redo(): Promise<void> {
    if (this.state.opened?.info.can_redo) await this.edit([{ command: "history.redo" }]);
  }
}
