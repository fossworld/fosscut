// An engine client for tests: answers from tables the test fills, records every call, and
// sends events when the test says so.

import type { EngineClient, EngineEvents, NewProjectFormat, Rect, Unsubscribe } from "./client";
import { CacheStandIn } from "./cache";
import { PreferenceStandIn } from "./preferences";
import type {
  ApiError,
  AppInfo,
  CacheKind,
  CacheUsage,
  CaptureDevice,
  Client,
  ColorSuggestion,
  EditRequest,
  EditResponse,
  ExportChoice,
  ExportPresets,
  ExportReport,
  JobStatus,
  LibraryAddRequest,
  LibraryItem,
  LostRange,
  MediaWork,
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
  SceneSplit,
  TrackRequest,
  TrashEntry,
  UpdateAnswer,
} from "./types";

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
  };

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
