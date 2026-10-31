// An engine client for tests: answers from tables the test fills, records every call, and
// sends events when the test says so.

import type { EngineClient, EngineEvents, NewProjectFormat, Rect, ThumbnailRequest, Unsubscribe } from "./client";
import { CacheStandIn } from "./cache";
import { standInListing } from "./models";
import { PreferenceStandIn } from "./preferences";
import type {
  ApiError,
  AppInfo,
  CacheKind,
  CacheUsage,
  CaptureDevice,
  Client,
  ColorSuggestion,
  DubCaptionsRequest,
  DubCaptionsResult,
  EditRequest,
  EditResponse,
  ExportChoice,
  ExportPresets,
  BeatCutsResult,
  BeatMarkersResult,
  ExportReport,
  ImportResult,
  JobStatus,
  LibraryAddRequest,
  LibraryItem,
  LostRange,
  MediaWork,
  ModelInstallState,
  ModelListing,
  MotionResult,
  MulticamRequest,
  MulticamResult,
  Normalization,
  Opened,
  PreferencesView,
  PreviewSource,
  ProjectSummary,
  RecordChoice,
  RecordState,
  ReframeRequest,
  ReframeResult,
  SceneSplit,
  ShortCandidate,
  ShortCandidates,
  ShortRequest,
  ShortResult,
  SpeechFile,
  SpeechPreview,
  SpeechRequest,
  SpeechResult,
  SubjectRequest,
  TrackRequest,
  TranslateCaptionsRequest,
  TranslateCaptionsResult,
  ThumbnailStrip,
  TrashEntry,
  UpdateAnswer,
  VerticalRequest,
  VerticalResult,
  VoiceInfo,
  Waveform,
} from "./types";

/** `count` times spread evenly over `length` flicks, at the centre of equal parts. */
export function spread(count: number, length: number): number[] {
  return Array.from({ length: Math.max(1, count) }, (_, i) => Math.round(((i + 0.5) * length) / Math.max(1, count)));
}

/** A number from 0 to 1 that a text always gives. */
function hash(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i += 1) h = Math.imul(h ^ text.charCodeAt(i), 16777619);
  return (h >>> 0) / 4294967296;
}

/**
 * A stand-in frame: a landscape in the asset's own colours that drifts with time, as an
 * SVG data URL, so strips of them read like footage.
 */
export function standInFrame(asset: string, flicks: number, width: number, height: number): string {
  const hue = Math.round(hash(asset) * 360);
  const t = flicks / 705_600_000;
  const sun = 20 + ((t * 9) % 60);
  const hill = 58 + Math.sin(t * 0.9 + hash(asset) * 6) * 8;
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 160 90" preserveAspectRatio="none">` +
    `<defs><linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="hsl(${hue} 55% 62%)"/>` +
    `<stop offset="1" stop-color="hsl(${(hue + 40) % 360} 60% 30%)"/></linearGradient></defs>` +
    `<rect width="160" height="90" fill="url(#s)"/>` +
    `<circle cx="${sun}%" cy="34" r="9" fill="hsl(${(hue + 190) % 360} 80% 82%)" opacity="0.85"/>` +
    `<path d="M0 ${hill} Q 50 ${hill - 16} 90 ${hill - 2} T 160 ${hill - 6} V 90 H 0 Z" fill="hsl(${(hue + 160) % 360} 35% 22%)"/>` +
    `<path d="M0 ${hill + 14} Q 70 ${hill + 2} 160 ${hill + 12} V 90 H 0 Z" fill="hsl(${(hue + 150) % 360} 30% 14%)"/>` +
    `</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

/** Stand-in peaks: a sound that swells and falls, with quiet gaps, the same every time. */
export function standInPeaks(asset: string, buckets: number): [number, number][] {
  const seed = hash(asset) * 1000;
  return Array.from({ length: buckets }, (_, i) => {
    const x = i / Math.max(1, buckets);
    const envelope = 0.35 + 0.45 * Math.abs(Math.sin(x * 9 + seed)) * (0.6 + 0.4 * Math.sin(x * 2.3 + seed));
    const grain = 0.55 + 0.45 * Math.abs(Math.sin(i * 12.9898 + seed) * 43758.5453 % 1);
    const gap = Math.sin(x * 31 + seed) > 0.93 ? 0.08 : 1;
    const peak = Math.min(1, envelope * grain * gap);
    return [-Math.round(peak * 0.94 * 1000) / 1000, Math.round(peak * 1000) / 1000];
  });
}

export interface Call {
  method: string;
  args: unknown[];
}

type Handler = (payload: unknown) => void;

export class FakeEngine implements EngineClient {
  calls: Call[] = [];
  opened: Opened | null = null;
  recent: string[] = [];
  connected: Client[] = [];
  /** Answers to queries by name; a function receives the arguments. */
  answers: Record<string, unknown | ((args: unknown) => unknown)> = {};
  /** How `execute` answers; by default it commits and counts revisions up. */
  onExecute: (request: EditRequest) => EditResponse | Promise<EditResponse> = (request) => {
    this.revision += 1;
    return { committed: !request.dry_run, dry_run: request.dry_run ?? false, replayed: false, revision: this.revision, results: [] };
  };
  revision = 0;
  private handlers = new Map<string, Set<Handler>>();

  private log(method: string, ...args: unknown[]): void {
    this.calls.push({ method, args });
  }

  /** The calls of one method. */
  callsOf(method: string): unknown[][] {
    return this.calls.filter((c) => c.method === method).map((c) => c.args);
  }

  /** Sends an event as the engine would. */
  emit<E extends keyof EngineEvents>(event: E, payload: EngineEvents[E]): void {
    for (const handler of this.handlers.get(event) ?? []) handler(payload);
  }

  async appInfo(): Promise<AppInfo> {
    return { name: "FossCut", version: "test", runtime_dir: "/tmp/run" };
  }

  summaries: ProjectSummary[] = [];
  trashed: TrashEntry[] = [];

  async projectSummaries(thumbnails: boolean): Promise<ProjectSummary[]> {
    this.log("projectSummaries", thumbnails);
    return this.summaries;
  }

  trash = {
    add: async (path: string): Promise<TrashEntry> => {
      this.log("trash.add", path);
      const entry = { id: `t-${this.trashed.length}`, name: path, original: path, trashed_ms: 1 };
      this.trashed.unshift(entry);
      this.summaries = this.summaries.filter((s) => s.path !== path);
      return entry;
    },
    list: async (): Promise<TrashEntry[]> => this.trashed,
    restore: async (id: string): Promise<string> => {
      this.log("trash.restore", id);
      const entry = this.trashed.find((t) => t.id === id);
      this.trashed = this.trashed.filter((t) => t.id !== id);
      return entry?.original ?? "";
    },
    remove: async (id: string): Promise<void> => {
      this.log("trash.remove", id);
      this.trashed = this.trashed.filter((t) => t.id !== id);
    },
  };

  async recentProjects(): Promise<string[]> {
    return this.recent;
  }

  async currentProject(): Promise<Opened | null> {
    return this.opened;
  }

  async openProject(path: string): Promise<Opened> {
    this.log("openProject", path);
    if (!this.opened) throw { code: "fosscut.project.not_found", message: `${path} is not a FossCut project` };
    return this.opened;
  }

  async pickProject(): Promise<Opened | null> {
    this.log("pickProject");
    return this.opened;
  }

  async newProject(format?: NewProjectFormat): Promise<Opened | null> {
    this.log("newProject", format);
    return this.opened;
  }

  async closeProject(): Promise<void> {
    this.log("closeProject");
    this.opened = null;
  }

  async query<T>(name: string, args?: unknown): Promise<T> {
    this.log("query", name, args);
    const answer = this.answers[name];
    if (answer === undefined) throw { code: "fosscut.command.unknown_query", message: `no answer for ${name}` };
    return (typeof answer === "function" ? (answer as (a: unknown) => unknown)(args) : answer) as T;
  }

  async execute(request: EditRequest): Promise<EditResponse> {
    this.log("execute", request);
    return this.onExecute(request);
  }

  async clients(): Promise<Client[]> {
    return this.connected;
  }

  async colorSuggest(clip: string, reference?: string): Promise<ColorSuggestion> {
    this.log("colorSuggest", clip, reference);
    const params = reference ? { saturation: -0.1 } : { temperature: 0.2 };
    return { effect_add: { clip, type: "color.adjust", params, index: 0 } };
  }

  /**
   * Levels every clip by the same 3 dB to the target, -14 LUFS unless asked, from a
   * loudness of 3 dB under it: one `clip.set` per clip.
   */
  async audioNormalize(clips: string[], targetLufs?: number): Promise<Normalization> {
    this.log("audioNormalize", clips, targetLufs);
    const target = targetLufs ?? -14;
    const volume = 1.413;
    return {
      target_lufs: target,
      ceiling_dbfs: -1,
      clips: clips.map((clip) => ({
        clip,
        measured_lufs: target - 3,
        peak_dbfs: -9,
        gain_db: 3,
        volume,
        limited: false,
      })),
      commands: clips.map((clip) => ({ command: "clip.set", args: { clip, set: { volume } } })),
    };
  }

  /** Where `measure.scenes` finds shot cuts, in timeline flicks. */
  sceneCuts: number[] = [];
  /** Where `measure.track` loses the region. */
  trackLost: LostRange[] = [];
  /** What every measurement throws instead of measuring, when set. */
  measureRefusal: ApiError | null = null;

  /** A commit of one step, counting revisions up, unless `expected` is stale. */
  private commit(results: unknown[], expected?: number): EditResponse {
    if (expected !== undefined && expected !== this.revision) {
      throw {
        code: "fosscut.revision.conflict",
        message: `the project is at revision ${this.revision}, not ${expected}`,
      } satisfies ApiError;
    }
    this.revision += 1;
    return { committed: true, dry_run: false, replayed: false, revision: this.revision, results };
  }

  /**
   * Measurements as the desktop answers them: splits at `sceneCuts`, a zoom that grows
   * with the strength, tracking that loses the region in `trackLost`, and multicam angles
   * each half a second later than the one before, matched surely.
   */
  measure = {
    scenes: async (clip: string, expectedRevision?: number): Promise<SceneSplit> => {
      this.log("measure.scenes", clip, expectedRevision);
      if (this.measureRefusal) throw this.measureRefusal;
      const cuts = [...this.sceneCuts];
      const response = cuts.length > 0 ? this.commit(cuts.map(() => ({})), expectedRevision) : null;
      return { scenes: { clip, cuts }, response };
    },
    stabilize: async (clip: string, strength?: number, expectedRevision?: number): Promise<MotionResult> => {
      this.log("measure.stabilize", clip, strength, expectedRevision);
      if (this.measureRefusal) throw this.measureRefusal;
      const zoom = Math.round((1 + 0.12 * (strength ?? 0.5)) * 1000) / 1000;
      return { motion: { clip, frames: 90, zoom }, response: this.commit([{}, {}, {}], expectedRevision) };
    },
    track: async (request: TrackRequest): Promise<MotionResult> => {
      this.log("measure.track", request);
      if (this.measureRefusal) throw this.measureRefusal;
      const keyed = request.mask ? request.clip : request.follow;
      if (!keyed || (request.mask && request.follow)) {
        throw {
          code: "fosscut.request.invalid_arguments",
          message: "tracking needs either a mask of the clip or another clip to follow the region",
          hint: "Choose one of the clip's masks, or a title, sticker or clip to follow.",
        } satisfies ApiError;
      }
      const motion: MotionResult["motion"] = { clip: keyed, frames: 60 };
      if (this.trackLost.length > 0) motion.lost = [...this.trackLost];
      return { motion, response: this.commit([{}], request.expected_revision) };
    },
    multicam: async (request: MulticamRequest): Promise<MulticamResult> => {
      this.log("measure.multicam", request);
      if (this.measureRefusal) throw this.measureRefusal;
      if (request.angles.length < 2) {
        throw {
          code: "fosscut.multicam.invalid",
          message: "multicam.create: give at least two angles",
          hint: "Give two or more video assets to multicam.create; switch angles of a multicam clip by number from 1 or by track id.",
        } satisfies ApiError;
      }
      const sync = request.sync
        ? {
            offsets: request.angles.map((_, i) => i * 352_800_000),
            confidence: request.angles.map((_, i) => (i === 0 ? 1 : 0.9)),
          }
        : null;
      const n = 50 + this.revision;
      const id = (kind: string, k: number) => `${kind}_${String(n + k).padStart(4, "0")}`;
      const created = {
        timeline: id("timeline", 0),
        angles: request.angles.map((_, i) => id("track", i + 1)),
        clip: { clip: id("clip", request.angles.length + 1), track: "track_0002" },
      };
      return { sync, response: this.commit([created], request.expected_revision) };
    },
  };

  /** The library `libraryList` searches as the engine does. */
  library: LibraryItem[] = [];

  async libraryList(kind?: string, search?: string): Promise<LibraryItem[]> {
    this.log("libraryList", kind, search);
    const words = (search ?? "").toLowerCase().split(/\s+/).filter(Boolean);
    return this.library.filter((item) => {
      const text = `${item.name} ${item.title} ${item.description} ${item.tags.join(" ")}`.toLowerCase();
      return (!kind || item.kind === kind) && words.every((word) => text.includes(word));
    });
  }

  /** How `libraryAdd` answers; by default it commits a clip and counts revisions up. */
  onLibraryAdd: (request: LibraryAddRequest) => EditResponse | Promise<EditResponse> = () => {
    this.revision += 1;
    const results = [{ clip: `clip_${String(100 + this.revision).padStart(4, "0")}` }];
    return { committed: true, dry_run: false, replayed: false, revision: this.revision, results };
  };

  async libraryAdd(request: LibraryAddRequest): Promise<EditResponse> {
    this.log("libraryAdd", request);
    return this.onLibraryAdd(request);
  }

  exportListing: ExportPresets = { presets: [], resolutions: [] };
  /** What the save dialog answers; null for a cancel. */
  exportPicks: string | null = "/work/Export.mp4";
  /** What `exportStart` throws instead of starting, when set. */
  exportRefusal: ApiError | null = null;
  /** Frames every export renders, and how many more each status read finds done. */
  exportFrames = 90;
  exportStep = 30;
  /** What a status read throws, when set. */
  exportFailure: ApiError | null = null;
  jobs = new Map<string, { status: JobStatus; choice: ExportChoice }>();

  async exportPresets(): Promise<ExportPresets> {
    this.log("exportPresets");
    return this.exportListing;
  }

  async exportPick(name: string, extension?: string): Promise<string | null> {
    this.log("exportPick", name, extension);
    return this.exportPicks;
  }

  async exportStart(choice: ExportChoice): Promise<JobStatus> {
    this.log("exportStart", choice);
    if (this.exportRefusal) throw this.exportRefusal;
    const job = `job_${String(this.jobs.size + 1).padStart(4, "0")}`;
    const status: JobStatus = {
      job,
      kind: "export",
      state: "running",
      progress: { done: 0, total: 0 },
      elapsed_ms: 0,
      output: choice.output,
    };
    this.jobs.set(job, { status, choice });
    return status;
  }

  /** A running job renders `exportStep` more frames per read, then succeeds with a report. */
  async exportStatus(job: string): Promise<JobStatus> {
    this.log("exportStatus", job);
    const entry = this.jobs.get(job);
    if (!entry) throw { code: "fosscut.job.not_found", message: `there is no job ${job}` };
    const { status, choice } = entry;
    if (status.state !== "running") return status;
    if (this.exportFailure) {
      entry.status = { ...status, state: "failed", error: this.exportFailure };
      return entry.status;
    }
    const done = Math.min(this.exportFrames, status.progress.done + this.exportStep);
    entry.status = { ...status, progress: { done, total: this.exportFrames }, elapsed_ms: status.elapsed_ms + 500 };
    if (done >= this.exportFrames) {
      const report = { ...fakeReport(choice, this.exportFrames), elapsed_ms: entry.status.elapsed_ms };
      entry.status = { ...entry.status, state: "succeeded", report };
    }
    return entry.status;
  }

  async exportCancel(job: string): Promise<JobStatus> {
    this.log("exportCancel", job);
    const entry = this.jobs.get(job);
    if (!entry) throw { code: "fosscut.job.not_found", message: `there is no job ${job}` };
    if (entry.status.state === "running") {
      entry.status = {
        ...entry.status,
        state: "cancelled",
        error: { code: "fosscut.export.cancelled", message: "the export was cancelled" },
      };
    }
    return entry.status;
  }

  /** The capture devices `record.devices` lists. */
  captureDevices: CaptureDevice[] = [];
  /** What `record.start` throws instead of starting, when set. */
  recordRefusal: ApiError | null = null;
  /** Seconds each status read of a running recording finds recorded. */
  recordStep = 0.25;
  recordings = new Map<string, { state: RecordState; seconds: number; choice: RecordChoice }>();

  /**
   * Recordings as the desktop runs them: each status read records `recordStep` seconds
   * more; stopping succeeds with a report, and imports the file once as an asset (added to
   * the `media.list` answer when it has a list) placed as a clip.
   */
  record = {
    devices: async (): Promise<CaptureDevice[]> => {
      this.log("record.devices");
      return this.captureDevices;
    },
    start: async (choice: RecordChoice): Promise<RecordState> => {
      this.log("record.start", choice);
      if (this.recordRefusal) throw this.recordRefusal;
      const job = `job_${String(this.jobs.size + this.recordings.size + 1).padStart(4, "0")}`;
      const extension = choice.what === "microphone" ? "wav" : "mp4";
      const label = { microphone: "Voiceover", camera: "Camera", screen: "Screen" }[choice.what];
      const state: RecordState = {
        job,
        kind: "recording",
        state: "running",
        progress: { done: 0, total: 0 },
        elapsed_ms: 0,
        output: `${this.opened?.bundle ?? "/work/Demo.fosscut"}/recordings/${label} ${job}.${extension}`,
      };
      this.recordings.set(job, { state, seconds: 0, choice });
      return state;
    },
    status: async (job: string): Promise<RecordState> => {
      this.log("record.status", job);
      const entry = this.recordings.get(job);
      if (!entry) throw { code: "fosscut.job.not_found", message: `there is no job ${job}` };
      if (entry.state.state === "running") {
        entry.seconds += this.recordStep;
        entry.state = {
          ...entry.state,
          progress: { done: Math.floor(entry.seconds + 1e-9), total: 0 },
          elapsed_ms: Math.round(entry.seconds * 1000),
        };
      }
      return entry.state;
    },
    stop: async (job: string): Promise<RecordState> => {
      this.log("record.stop", job);
      const entry = this.recordings.get(job);
      if (!entry) throw { code: "fosscut.job.not_found", message: `there is no job ${job}` };
      if (entry.state.state !== "running") return entry.state;
      const { choice } = entry;
      const n = 60 + this.recordings.size * 2;
      const asset = `asset_${String(n).padStart(4, "0")}`;
      const clip = choice.at === undefined ? undefined : `clip_${String(n + 1).padStart(4, "0")}`;
      const output = entry.state.output ?? "";
      const duration = Math.round(entry.seconds * 705_600_000);
      this.revision += clip ? 2 : 1;
      const listing = this.answers["media.list"] as { assets?: unknown[] } | undefined;
      listing?.assets?.push({
        id: asset,
        kind: choice.what === "microphone" ? "audio" : "video",
        name: output.slice(output.lastIndexOf("/") + 1),
        duration: { flicks: duration, frames: Math.round(entry.seconds * 30), seconds: String(entry.seconds), timecode: "" },
        used_by: clip ? [clip] : [],
        folder: "recordings",
      });
      entry.state = {
        ...entry.state,
        state: "succeeded",
        recording: {
          output,
          what: choice.what,
          device: choice.device,
          preset: choice.what === "microphone" ? "wav" : "mp4-h264",
          duration,
          stopped: true,
          elapsed_ms: entry.state.elapsed_ms,
        },
        asset,
        ...(clip ? { clip } : {}),
      };
      return entry.state;
    },
  };

  /** What relinking answers: true for a file chosen, false for a cancel. */
  relinkChooses = true;

  /** The copies the preview decodes, by asset, and the copies being made. */
  sources: Record<string, PreviewSource> = {};
  mediaWork: MediaWork = { proxies: { done: 0, total: 0 }, remux: { done: 0, total: 0 }, failed: 0 };

  media = {
    reveal: async (asset: string): Promise<void> => this.log("media.reveal", asset),
    relink: async (asset: string): Promise<boolean> => {
      this.log("media.relink", asset);
      return this.relinkChooses;
    },
    sources: async (): Promise<Record<string, PreviewSource>> => this.sources,
    work: async (): Promise<MediaWork> => this.mediaWork,
    thumbnails: async (request: ThumbnailRequest): Promise<ThumbnailStrip> => {
      this.log("media.thumbnails", request);
      if (this.pictureFailure) throw this.pictureFailure;
      const height = request.height ?? 72;
      const width = Math.round((height * 16) / 9);
      const times = request.times ?? spread(request.count ?? 1, this.assetLength(request.asset));
      return {
        asset: request.asset,
        width,
        height,
        frames: times.map((flicks) => ({ flicks, image: standInFrame(request.asset, flicks, width, height) })),
      };
    },
    waveform: async (asset: string, buckets: number): Promise<Waveform> => {
      this.log("media.waveform", asset, buckets);
      if (this.pictureFailure) throw this.pictureFailure;
      return { asset, duration: this.assetLength(asset), buckets, peaks: standInPeaks(asset, buckets) };
    },
    import: async (paths?: string[]): Promise<ImportResult | null> => {
      this.log("media.import", paths);
      return this.onImport(paths);
    },
  };

  /** Why thumbnails and waveforms fail, when a test says they do. */
  pictureFailure: ApiError | null = null;
  /** What importing answers; by default the picker is cancelled. */
  onImport: (paths?: string[]) => ImportResult | null = () => null;

  /** The length of an asset as `media.list` answers it, or ten seconds. */
  private assetLength(asset: string): number {
    const listing = this.answers["media.list"];
    const assets = (typeof listing === "function" ? null : (listing as { assets?: { id: string; duration?: { flicks: number } | null }[] } | undefined))?.assets;
    return assets?.find((a) => a.id === asset)?.duration?.flicks ?? 10 * 705_600_000;
  }

  preview = {
    viewport: async (rect: Rect): Promise<void> => this.log("preview.viewport", rect),
    show: async (flicks: number, scrub?: boolean): Promise<void> =>
      scrub ? this.log("preview.show", flicks, true) : this.log("preview.show", flicks),
    play: async (): Promise<void> => this.log("preview.play"),
    pause: async (): Promise<void> => this.log("preview.pause"),
    shuttle: async (rate: number): Promise<void> => this.log("preview.shuttle", rate),
  };

  async fullscreen(on: boolean): Promise<void> {
    this.log("fullscreen", on);
  }

  /** The preferences, in memory, refused as the engine refuses them. */
  prefs = new PreferenceStandIn();

  preferences = {
    get: async (): Promise<PreferencesView> => this.prefs.view(),
    set: async (key: string, value: unknown): Promise<PreferencesView> => {
      this.log("preferences.set", key, value);
      return this.prefs.set(key, value);
    },
    reset: async (key: string): Promise<PreferencesView> => {
      this.log("preferences.reset", key);
      return this.prefs.set(key, null);
    },
  };

  /** What `updates.check` finds while the `updates` preference allows a check. */
  latestRelease: { version: string; url: string } | null = null;

  updates = {
    check: async (): Promise<UpdateAnswer> => {
      this.log("updates.check");
      const policy = this.prefs.current.updates;
      if (policy === "off") return { policy, checked: false, message: "update checks are off" };
      const latest = this.latestRelease;
      return {
        policy,
        checked: true,
        current: "0.1.0",
        latest: latest?.version,
        newer: latest !== null,
        url: latest?.url,
      };
    },
    open: async (): Promise<void> => this.log("updates.open"),
  };

  async playSound(name: string): Promise<void> {
    this.log("playSound", name);
  }

  /** The folder `pickFolder` answers; null for a cancel. */
  folderChosen: string | null = "/Users/demo/Movies/Exports";

  async pickFolder(title?: string, directory?: string): Promise<string | null> {
    this.log("pickFolder", title, directory);
    return this.folderChosen;
  }

  // ---- Local models ----

  /** The models `models.list` lists; installs that finish mark theirs installed. */
  modelListing: ModelListing = standInListing([]);
  /** How many status reads an install takes to finish. */
  installReads = 2;
  /** What every install fails with, when set. */
  installFailure: ApiError | null = null;
  installs = new Map<string, { state: ModelInstallState; reads: number }>();

  private listed(name: string) {
    return this.modelListing.models.find((m) => m.name === name);
  }

  /** True when `name` can run: installed, and the runtime is there for any but itself. */
  private ready(name: string): boolean {
    return this.listed(name)?.installed === true;
  }

  /**
   * Refuses as the engine does when a model is missing: `fosscut.model.not_installed`
   * naming them, or `fosscut.model.runtime_missing` when only the runtime is.
   */
  private needs(names: string[]): void {
    const missing = names.filter((name) => !this.ready(name));
    if (missing.length > 0) {
      const what = missing.join(" and ");
      throw {
        code: "fosscut.model.not_installed",
        message: `${what} is not installed`,
        hint: "Ask the user before installing, since it downloads: models install.",
        details: { what },
      } satisfies ApiError;
    }
    if (this.modelListing.runtime === null) {
      throw {
        code: "fosscut.model.runtime_missing",
        message: "the ONNX Runtime library is missing",
        hint: "Ask the user before installing, since it downloads: models install onnxruntime.",
      } satisfies ApiError;
    }
  }

  /** Installs as the desktop runs them: `installReads` status reads each, then installed. */
  models = {
    list: async (): Promise<ModelListing> => {
      this.log("models.list");
      return { ...this.modelListing, models: this.modelListing.models.map((m) => ({ ...m })) };
    },
    install: async (name: string, confirmed: boolean): Promise<ModelInstallState> => {
      this.log("models.install", name, confirmed);
      const model = this.listed(name);
      if (!model) throw { code: "fosscut.model.unknown", message: `there is no model ${name}` } satisfies ApiError;
      const total = model.size_bytes;
      if (model.installed) {
        return { name, state: "succeeded", done_bytes: total, total_bytes: total, elapsed_ms: 0, install: { path: `/models/${name}`, status: "unchanged" } };
      }
      if (!confirmed) {
        throw {
          code: "fosscut.model.confirmation_required",
          message: `${name} needs confirmation`,
          details: { name, size_bytes: total, license: model.license },
        } satisfies ApiError;
      }
      const running = this.installs.get(name);
      if (running?.state.state === "running") return running.state;
      const state: ModelInstallState = { name, state: "running", done_bytes: 0, total_bytes: total, elapsed_ms: 0 };
      this.installs.set(name, { state, reads: 0 });
      return state;
    },
    installStatus: async (name: string): Promise<ModelInstallState> => {
      this.log("models.installStatus", name);
      const entry = this.installs.get(name);
      if (!entry) throw { code: "fosscut.request.invalid_arguments", message: `no install of ${name} was started` } satisfies ApiError;
      if (entry.state.state !== "running") return entry.state;
      entry.reads += 1;
      if (this.installFailure) {
        entry.state = { ...entry.state, state: "failed", error: this.installFailure };
        return entry.state;
      }
      const total = entry.state.total_bytes;
      const done = Math.min(total, Math.round((total * entry.reads) / Math.max(1, this.installReads)));
      entry.state = { ...entry.state, done_bytes: done, elapsed_ms: entry.reads * 500 };
      if (entry.reads >= this.installReads) {
        entry.state = { ...entry.state, state: "succeeded", done_bytes: total, install: { path: `/models/${name}`, status: "installed" } };
        this.modelListing = {
          ...this.modelListing,
          models: this.modelListing.models.map((m) => (m.name === name ? { ...m, installed: true } : m)),
          runtime: name === "onnxruntime" ? `/models/onnxruntime/libonnxruntime.dylib` : this.modelListing.runtime,
        };
      }
      return entry.state;
    },
  };

  /** Analyses cached already, by `clip:kind:model`. */
  analyzed = new Set<string>();
  /** How many status reads an analysis takes, and the frames it analyzes. */
  analysisReads = 2;
  analysisFrames = 120;
  /** What every analysis fails with once it runs, when set. */
  analysisFailure: ApiError | null = null;
  analyses = new Map<string, { status: JobStatus; reads: number; key: string }>();

  /**
   * Analyses as the desktop runs them: refused at once without their models, answered at
   * once when cached, otherwise `analysisReads` status reads, then cached.
   */
  subject = {
    analyze: async (request: SubjectRequest): Promise<JobStatus> => {
      this.log("subject.analyze", request);
      if (request.kind === "matte" || request.kind === "pose") {
        this.needs([request.kind === "matte" && request.model === "subject" ? "birefnet-lite" : "mediapipe-pose-full"]);
      }
      const key = `${request.clip}:${request.kind}:${request.kind === "matte" ? (request.model ?? "person") : ""}`;
      const job = `job_${String(100 + this.analyses.size).padStart(4, "0")}`;
      const cached = this.analyzed.has(key);
      const total = this.analysisFrames;
      const status: JobStatus = {
        job,
        kind: "analysis",
        state: cached ? "succeeded" : "running",
        progress: { done: cached ? total : 0, total },
        elapsed_ms: 0,
        asset: "asset_0001",
      };
      this.analyses.set(job, { status, reads: 0, key });
      return status;
    },
    status: async (job: string): Promise<JobStatus> => {
      this.log("subject.status", job);
      const entry = this.analyses.get(job);
      if (!entry) throw { code: "fosscut.job.not_found", message: `there is no job ${job}` } satisfies ApiError;
      if (entry.status.state !== "running") return entry.status;
      entry.reads += 1;
      if (this.analysisFailure) {
        entry.status = { ...entry.status, state: "failed", error: this.analysisFailure };
        return entry.status;
      }
      const total = entry.status.progress.total;
      const done = Math.min(total, Math.round((total * entry.reads) / Math.max(1, this.analysisReads)));
      entry.status = { ...entry.status, progress: { done, total }, elapsed_ms: entry.reads * 500 };
      if (entry.reads >= this.analysisReads) {
        entry.status = { ...entry.status, state: "succeeded" };
        this.analyzed.add(entry.key);
      }
      return entry.status;
    },
    cancel: async (job: string): Promise<JobStatus> => {
      this.log("subject.cancel", job);
      const entry = this.analyses.get(job);
      if (!entry) throw { code: "fosscut.job.not_found", message: `there is no job ${job}` } satisfies ApiError;
      if (entry.status.state === "running") {
        entry.status = { ...entry.status, state: "cancelled", error: { code: "fosscut.analysis.cancelled", message: "the analysis was cancelled" } };
      }
      return entry.status;
    },
  };

  /** The voices `voice.list` lists. */
  voices: VoiceInfo[] = [];

  /** Refuses as the engine does when the clip's `analyzer` analysis is not cached. */
  private analysisNeeded(clip: string, analyzer: string, model = ""): void {
    if (this.analyzed.has(`${clip}:${analyzer}:${model}`)) return;
    const asset = this.assetOf(clip);
    throw {
      code: "fosscut.analysis.missing",
      message: `the ${analyzer} analysis of ${asset ?? clip} is not cached`,
      hint: "Analyze it first.",
      details: { asset, analyzer, options: model ? { [analyzer]: { model } } : {}, effect: "reframe" },
    } satisfies ApiError;
  }

  /** The asset a clip of the timeline `timeline.read` answers shows, if it shows one. */
  private assetOf(clip: string): string | null {
    const answer = this.answers["timeline.read"];
    const timeline = (typeof answer === "function" ? answer(undefined) : answer) as { tracks?: { clips: { id: string; asset?: string }[] }[] } | undefined;
    for (const track of timeline?.tracks ?? []) {
      const found = track.clips.find((c) => c.id === clip);
      if (found) return found.asset ?? null;
    }
    return null;
  }

  /** A commit as `execute` makes one, for the planners below. */
  private committed(): EditResponse {
    this.revision += 1;
    return { committed: true, dry_run: false, replayed: false, revision: this.revision, results: [] };
  }

  /** Reframing as the desktop answers it: the pose or matte analysis is needed first. */
  reframe = {
    clip: async (request: ReframeRequest): Promise<ReframeResult> => {
      this.log("reframe.clip", request);
      return this.reframed(request);
    },
    camera: async (request: ReframeRequest & { zoom: number }): Promise<ReframeResult> => {
      this.log("reframe.camera", request);
      if (request.zoom < 1 || request.zoom > 4) throw { code: "fosscut.request.invalid_arguments", message: "zoom must be from 1 to 4" } satisfies ApiError;
      return this.reframed(request);
    },
    vertical: async (request: VerticalRequest): Promise<VerticalResult> => {
      this.log("reframe.vertical", request);
      return {
        vertical: { source: request.timeline ?? "timeline_0000", timeline: "timeline_0090", clips: 2, centred: [{ clip: "clip_0012", reason: "no person in it" }] },
        response: this.committed(),
      };
    },
  };

  private reframed(request: ReframeRequest): ReframeResult {
    if (request.source === "person") this.analysisNeeded(request.clip, "pose");
    if (request.source === "subject") this.analysisNeeded(request.clip, "matte", "subject");
    const zoom = request.zoom ?? 1;
    return {
      reframe: {
        clip: request.clip,
        frames: 210,
        subject_frames: 198,
        shots: [
          { start: 8 * 705_600_000, end: 12 * 705_600_000, zoom, subject_frames: 120, inside_frames: 116 },
          { start: 12 * 705_600_000, end: 15 * 705_600_000, zoom: zoom * 1.1, subject_frames: 78, inside_frames: 70 },
        ],
      },
      response: this.committed(),
    };
  }

  /** Beats as the desktop answers them: the beats analysis is needed first. */
  beats = {
    markers: async (clip: string, every?: number, expectedRevision?: number): Promise<BeatMarkersResult> => {
      this.log("beats.markers", clip, every, expectedRevision);
      this.analysisNeeded(clip, "beats");
      const step = every ?? 1;
      return {
        beats: { timeline: "timeline_0000", tempo_bpm: 120, beats_per_bar: 4, confidence: 0.86, markers: Math.round(48 / step), existing: 0 },
        response: this.committed(),
      };
    },
    cut: async (track: string, music: string, every?: number, expectedRevision?: number): Promise<BeatCutsResult> => {
      this.log("beats.cut", track, music, every, expectedRevision);
      this.analysisNeeded(music, "beats");
      return {
        cuts: {
          track,
          music,
          tempo_bpm: 120,
          beats: 48,
          tolerance: 176_400_000,
          snapped: [
            { before: "clip_0003", after: "clip_0006", from: 8 * 705_600_000, to: 7.5 * 705_600_000 },
            { before: "clip_0006", after: "clip_0007", from: 15 * 705_600_000, to: 15 * 705_600_000 },
          ],
          unsnapped: [{ before: null, after: "clip_0003", at: 0, reason: "the first clip starts the track" }],
        },
        response: this.committed(),
      };
    },
  };

  /** Moments of a long clip worth a short, best first, as `shorts.candidates` answers. */
  shortCandidates: ShortCandidate[] = [];

  /** Shorts as the desktop answers them: the transcript is needed first. */
  shorts = {
    candidates: async (clip: string, options?: { min_seconds?: number; max_seconds?: number; count?: number }): Promise<ShortCandidates> => {
      this.log("shorts.candidates", clip, options);
      this.analysisNeeded(clip, "transcript");
      const candidates = this.shortCandidates.slice(0, options?.count ?? 5);
      return { clip, asset: "asset_0001", measured: ["transcript", "loudness"], missing: ["scenes"], sentences: 42, considered: 18, candidates };
    },
    make: async (request: ShortRequest): Promise<ShortResult> => {
      this.log("shorts.make", request);
      return {
        short: {
          timeline: "timeline_0095",
          clip: "clip_0096",
          name: request.name ?? "Short 1",
          start: 0,
          duration: request.end - request.start,
          centred: request.follow ? null : "no subject to follow",
          caption_words: request.captions === false ? 0 : 37,
        },
        response: this.committed(),
      };
    },
  };

  /** A speech file as the engine writes one: a second for every 15 characters. */
  private speechFile(text: string, voice: string, speed: number): SpeechFile {
    const seconds = Math.max(0.5, Math.round((text.trim().length / 15 / speed) * 100) / 100);
    return { relative: `speech/${voice}-${text.trim().length}.wav`, reused: false, duration: Math.round(seconds * 705_600_000), seconds, voice, speed };
  }

  /** Text to speech as the desktop answers it, with the voice model required. */
  voice = {
    list: async (): Promise<VoiceInfo[]> => {
      this.log("voice.list");
      return this.voices;
    },
    preview: async (text: string, voice: string, speed?: number): Promise<SpeechPreview> => {
      this.log("voice.preview", text, voice, speed);
      if (!text.trim()) throw { code: "fosscut.request.invalid_arguments", message: "there is no text to speak" } satisfies ApiError;
      this.needs(["kokoro-82m"]);
      return { file: this.speechFile(text, voice, speed ?? 1), played: true };
    },
    add: async (request: SpeechRequest): Promise<SpeechResult> => {
      this.log("voice.add", request);
      if (!request.text.trim()) throw { code: "fosscut.request.invalid_arguments", message: "there is no text to speak" } satisfies ApiError;
      this.needs(["kokoro-82m"]);
      const n = 70 + this.revision * 2;
      const asset = `asset_${String(n).padStart(4, "0")}`;
      const clip = `clip_${String(n + 1).padStart(4, "0")}`;
      const response = this.commit([{ asset }, { clip }], request.expected_revision);
      const file = this.speechFile(request.text, request.voice ?? "af_heart", request.speed ?? 1);
      return { speech: { revision: response.revision - 1, asset, file }, response };
    },
  };

  /** The caption pages `captions` translates and dubs, and the translations it finds. */
  captionPages: { clip: string; text: string; translation?: string }[] = [];
  translations: Record<string, string> = {};

  /** Caption translation and dubbing as the desktop answers them, with their models required. */
  captions = {
    translate: async (request: TranslateCaptionsRequest): Promise<TranslateCaptionsResult> => {
      this.log("captions.translate", request);
      const pair = request.to === "hu" ? "en>hu" : "hu>en";
      const model = request.to === "hu" ? "opus-mt-en-hu" : "opus-mt-hu-en";
      this.needs([model]);
      if (this.captionPages.length === 0) throw { code: "fosscut.captions.none", message: "the timeline has no caption track" } satisfies ApiError;
      const pages = this.captionPages.map((p) => ({ clip: p.clip, text: p.text, translation: this.translations[p.text] ?? "" }));
      const done = pages.filter((p) => p.translation);
      const skipped = pages.filter((p) => !p.translation).map((p) => p.clip);
      const response = done.length > 0 ? this.commit(done.map(() => ({})), request.expected_revision) : null;
      for (const page of this.captionPages) if (this.translations[page.text]) page.translation = this.translations[page.text];
      return {
        translation: {
          revision: this.revision,
          timeline: "timeline_0000",
          track: request.track ?? "track_0010",
          pair,
          mode: request.mode,
          model,
          attribution: this.listed(model)?.attribution,
          pages: done,
          skipped,
        },
        response,
      };
    },
    dub: async (request: DubCaptionsRequest): Promise<DubCaptionsResult> => {
      this.log("captions.dub", request);
      this.needs(["kokoro-82m"]);
      const spoken = this.captionPages
        .map((p, i) => ({ page: p, text: request.translation ? p.translation : p.text, start: i * 705_600_000 }))
        .filter((p): p is { page: (typeof p)["page"]; text: string; start: number } => Boolean(p.text?.trim()));
      const skipped = this.captionPages.filter((p) => !spoken.some((s) => s.page === p)).map((p) => p.clip);
      const response = spoken.length > 0 ? this.commit(spoken.map(() => ({})), request.expected_revision) : null;
      const voice = request.voice ?? "af_heart";
      return {
        dub: {
          revision: this.revision,
          timeline: "timeline_0000",
          track: request.track ?? "track_0010",
          voice,
          pages: spoken.map((s, i) => {
            const file = this.speechFile(s.text, voice, request.speed ?? 1);
            return { clip: s.page.clip, start: s.start, text: s.text, speech: file.duration, speed: file.speed, asset: `asset_${String(80 + i).padStart(4, "0")}` };
          }),
          skipped,
        },
        response,
      };
    },
  };

  /** The caches, with sizes so that clearing them shows. */
  caches = new CacheStandIn({ analysis: 120_000_000, proxies: 2_400_000_000, remux: 800_000_000, frames: 650_000_000 });

  cache = {
    usage: (): Promise<CacheUsage> => this.caches.usage(),
    clear: async (kind: CacheKind | "all"): Promise<CacheUsage> => {
      this.log("cache.clear", kind);
      return this.caches.clear(kind);
    },
  };

  on<E extends keyof EngineEvents>(event: E, handler: (payload: EngineEvents[E]) => void): Unsubscribe {
    const set = this.handlers.get(event) ?? new Set<Handler>();
    set.add(handler as Handler);
    this.handlers.set(event, set);
    return () => set.delete(handler as Handler);
  }
}

/** The report of a fake 1080p30 export of `frames` frames. */
function fakeReport(choice: ExportChoice, frames: number): ExportReport {
  const duration = Math.round((frames / 30) * 705_600_000);
  const stem = choice.output.replace(/\.[^./]*$/, "");
  const report: ExportReport = {
    output: choice.output,
    settings: {
      container: "mp4",
      video: { codec: "h264", width: 1920, height: 1080, rate: "30" },
      audio: { codec: "aac", sample_rate: 48_000, channels: 2, bit_rate: 192_000 },
    },
    frames,
    duration,
    encoders: { video: "h264_videotoolbox", audio: "aac" },
    verified: {
      container: "mov,mp4,m4a,3gp,3g2,mj2",
      size_bytes: frames * 41_000,
      duration,
      video: { codec: "h264", width: 1920, height: 1080, rate: "30" },
      audio: { codec: "aac", sample_rate: 48_000, channels: 2 },
    },
    elapsed_ms: 1_500,
    rights: [
      { asset: "asset_0003", name: "music.m4a", origin: "yours", note: "Brought in by you: make sure you may publish it." },
    ],
  };
  if (choice.cover_flicks !== undefined) report.cover = `${stem}.cover.png`;
  if (choice.subtitles) report.subtitles = `${stem}.${choice.subtitles}`;
  return report;
}
