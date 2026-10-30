// An engine client for tests: answers from tables the test fills, records every call, and
// sends events when the test says so.

import type { EngineClient, EngineEvents, NewProjectFormat, Rect, Unsubscribe } from "./client";
import type {
  ApiError,
  AppInfo,
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
  Normalization,
  Opened,
  ProjectSummary,
  TrashEntry,
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

  private record(method: string, ...args: unknown[]): void {
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
    this.record("projectSummaries", thumbnails);
    return this.summaries;
  }

  trash = {
    add: async (path: string): Promise<TrashEntry> => {
      this.record("trash.add", path);
      const entry = { id: `t-${this.trashed.length}`, name: path, original: path, trashed_ms: 1 };
      this.trashed.unshift(entry);
      this.summaries = this.summaries.filter((s) => s.path !== path);
      return entry;
    },
    list: async (): Promise<TrashEntry[]> => this.trashed,
    restore: async (id: string): Promise<string> => {
      this.record("trash.restore", id);
      const entry = this.trashed.find((t) => t.id === id);
      this.trashed = this.trashed.filter((t) => t.id !== id);
      return entry?.original ?? "";
    },
    remove: async (id: string): Promise<void> => {
      this.record("trash.remove", id);
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
    this.record("openProject", path);
    if (!this.opened) throw { code: "fosscut.project.not_found", message: `${path} is not a FossCut project` };
    return this.opened;
  }

  async pickProject(): Promise<Opened | null> {
    this.record("pickProject");
    return this.opened;
  }

  async newProject(format?: NewProjectFormat): Promise<Opened | null> {
    this.record("newProject", format);
    return this.opened;
  }

  async closeProject(): Promise<void> {
    this.record("closeProject");
    this.opened = null;
  }

  async query<T>(name: string, args?: unknown): Promise<T> {
    this.record("query", name, args);
    const answer = this.answers[name];
    if (answer === undefined) throw { code: "fosscut.command.unknown_query", message: `no answer for ${name}` };
    return (typeof answer === "function" ? (answer as (a: unknown) => unknown)(args) : answer) as T;
  }

  async execute(request: EditRequest): Promise<EditResponse> {
    this.record("execute", request);
    return this.onExecute(request);
  }

  async clients(): Promise<Client[]> {
    return this.connected;
  }

  async colorSuggest(clip: string, reference?: string): Promise<ColorSuggestion> {
    this.record("colorSuggest", clip, reference);
    const params = reference ? { saturation: -0.1 } : { temperature: 0.2 };
    return { effect_add: { clip, type: "color.adjust", params, index: 0 } };
  }

  /**
   * Levels every clip by the same 3 dB to the target, -14 LUFS unless asked, from a
   * loudness of 3 dB under it: one `clip.set` per clip.
   */
  async audioNormalize(clips: string[], targetLufs?: number): Promise<Normalization> {
    this.record("audioNormalize", clips, targetLufs);
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

  /** The library `libraryList` searches as the engine does. */
  library: LibraryItem[] = [];

  async libraryList(kind?: string, search?: string): Promise<LibraryItem[]> {
    this.record("libraryList", kind, search);
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
    this.record("libraryAdd", request);
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
    this.record("exportPresets");
    return this.exportListing;
  }

  async exportPick(name: string, extension?: string): Promise<string | null> {
    this.record("exportPick", name, extension);
    return this.exportPicks;
  }

  async exportStart(choice: ExportChoice): Promise<JobStatus> {
    this.record("exportStart", choice);
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
    this.record("exportStatus", job);
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
    this.record("exportCancel", job);
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

  /** What relinking answers: true for a file chosen, false for a cancel. */
  relinkChooses = true;

  media = {
    reveal: async (asset: string): Promise<void> => this.record("media.reveal", asset),
    relink: async (asset: string): Promise<boolean> => {
      this.record("media.relink", asset);
      return this.relinkChooses;
    },
  };

  preview = {
    viewport: async (rect: Rect): Promise<void> => this.record("preview.viewport", rect),
    show: async (flicks: number): Promise<void> => this.record("preview.show", flicks),
    play: async (): Promise<void> => this.record("preview.play"),
    pause: async (): Promise<void> => this.record("preview.pause"),
    shuttle: async (rate: number): Promise<void> => this.record("preview.shuttle", rate),
  };

  async fullscreen(on: boolean): Promise<void> {
    this.record("fullscreen", on);
  }

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
