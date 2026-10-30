// The editor's state and the actions that change it. Every change to the project is a
// command sent through the engine client (ADR-0002); what the window shows is read back
// from the engine after each commit, whoever made it, so agents' edits appear live.

import type { EngineClient, Rect, Unsubscribe } from "../engine/client";
import {
  type ApiError,
  type AssetView,
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
  type MulticamRequest,
  type Normalization,
  type Opened,
  type ProjectSummary,
  type RecordChoice,
  type RecordState,
  type Region,
  type TimelineView,
  type TrashEntry,
  isApiError,
} from "../engine/types";
import { clipsAt, findClip } from "../timeline/geometry";
import { FLICKS_PER_SECOND, frameAt, frameStart, framesToFlicks, parseRate } from "../transport/time";
import { multicamMade, splitAtScenes, stabilized, targetOf, tracked } from "./measure";
import { multicamPartAt } from "./multicam";

/** How many history entries the activity panel shows. */
const HISTORY = 50;

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
}

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
  kind: "stabilize" | "scenes" | "track" | "multicam";
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
  if (measured.length === 0) return "Nothing to level: the clips are silent or shorter than 400 ms.";
  const count = (n: number, what: string) => (n === 1 ? `one ${what}` : `${n} ${what}s`);
  const parts = [`Levelled to ${normalization.target_lufs} LUFS`];
  const limited = measured.filter((c) => c.limited).length;
  if (limited > 0) parts.push(`${count(limited, "clip")} held back by the ${normalization.ceiling_dbfs} dBFS peak ceiling`);
  const silent = normalization.clips.length - measured.length;
  if (silent > 0) parts.push(`${count(silent, "silent clip")} left alone`);
  return `${parts.join("; ")}.`;
}

/** What an export that finished out of sight did, in a sentence. */
export function finished(status: JobStatus): Notice {
  const file = status.output ?? status.job;
  if (status.state === "succeeded") return { kind: "info", text: `Exported ${file}.` };
  if (status.state === "cancelled") return { kind: "info", text: `The export to ${file} was cancelled.` };
  return { kind: "error", text: `The export to ${file} failed: ${status.error?.message ?? "no reason given"}.` };
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

  constructor(
    private readonly engine: EngineClient,
    private readonly confirm: Confirm = () => false,
  ) {}

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

  // ---- Lifecycle ----

  /** Listens to the engine and loads the project the app has open, if any. */
  async start(): Promise<void> {
    this.unsubscribers.push(
      this.engine.on("project-changed", () => void this.refresh()),
      this.engine.on("project-closed", () => this.set({ ...EMPTY, recent: this.state.recent })),
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
    );
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
  }

  // ---- Projects ----

  private async show(opened: Opened | null): Promise<void> {
    if (!opened) return;
    this.set({ ...EMPTY, recent: this.state.recent, opened });
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
          text: { ...NO_TEXT, ...caps.text },
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

  async create(): Promise<void> {
    try {
      await this.show(await this.engine.newProject());
    } catch (error) {
      this.notify(error);
    }
  }

  async close(): Promise<void> {
    await this.engine.closeProject();
    const recent = await this.engine.recentProjects();
    this.set({ ...EMPTY, recent });
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
      this.set({ notice: { kind: "info", text: `Restored to ${path}.` } });
      await this.browse();
    } catch (error) {
      this.notify(error);
    }
  }

  /** Deletes a trashed project for good, after asking. */
  async deleteTrashed(id: string): Promise<void> {
    const entry = this.state.trash.find((t) => t.id === id);
    if (!entry || !this.confirm(`Delete ${entry.name} for good? This cannot be undone.`)) return;
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
      this.set({
        opened: { ...opened, info },
        timeline,
        history: history.entries,
        selection,
        regionPick,
        luts,
        media: listing.assets,
        folders: listing.folders ?? [],
      });
    } catch (error) {
      this.notify(error);
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

  /** Moves the playhead to the frame containing `flicks`, within the timeline. */
  async seek(flicks: number): Promise<void> {
    const at = Math.min(Math.max(0, frameStart(flicks, this.rate())), this.end());
    this.set({ playhead: at, playing: false });
    await this.engine.preview.show(at);
  }

  async step(frames: number): Promise<void> {
    const rate = this.rate();
    await this.seek(framesToFlicks(frameAt(this.state.playhead, rate) + frames, rate));
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
    await this.engine.preview.show(time ?? this.state.playhead);
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
        this.set({ notice: { kind: "info", text: "The project changed while you were editing. It is up to date now; try again." } });
        return null;
      }
      if (api.code === "fosscut.guard.destructive" && this.confirm(`${api.message}. Do it anyway?`)) {
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
      this.set({ notice: { kind: "info", text: "Put the playhead inside the selected clip first." } });
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
      this.set({ notice: { kind: "info", text: "Shift-click to select two or more clips to group." } });
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
        ? "The playhead is not inside the selected clip."
        : under.length > 1
          ? "Several clips are under the playhead; select the one to split."
          : "No clip is under the playhead.";
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
        this.set({ notice: { kind: "info", text: "A path needs at least three points." } });
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

  /** Places an asset on the timeline at the playhead, on a track free there. */
  async addToTimeline(asset: string): Promise<void> {
    await this.edit([{ command: "clip.add", args: { asset, start: { flicks: this.state.playhead } } }]);
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

  /** Sets or clears one of a clip's animations (`in`, `out` or `combo`). */
  async setAnimation(clip: string, place: "in" | "out" | "combo", preset: string, seconds?: number): Promise<void> {
    const args: Record<string, unknown> = { clip };
    if (preset) {
      const animation: Record<string, unknown> = { preset };
      if (seconds !== undefined) animation.duration = { seconds: String(seconds) };
      args[place] = animation;
    } else {
      args.clear = [place];
    }
    await this.edit([{ command: "animation.set", args }]);
  }

  /** Adds an effect at its defaults to the end of a clip's effects. */
  async addEffect(clip: string, type: string): Promise<void> {
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
      await this.edit([{ command: "effect.add", args: suggestion.effect_add }], reference ? "Match color" : "Auto color");
    } catch (error) {
      this.notify(error);
    }
  }

  /**
   * Levels the loudness of clips with sound: the engine measures them, and the volumes it
   * works out apply as one undoable step, with a note of what it did.
   */
  async normalizeLoudness(clips: string[], targetLufs?: number): Promise<void> {
    if (clips.length === 0) {
      this.set({ notice: { kind: "info", text: "Select clips with sound to normalize their loudness." } });
      return;
    }
    let normalization: Normalization;
    try {
      normalization = await this.engine.audioNormalize(clips, targetLufs);
    } catch (error) {
      this.notify(error);
      return;
    }
    if (normalization.commands.length > 0 && !(await this.edit(normalization.commands, "Normalize loudness"))) return;
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
      this.set({ notice: { kind: "info", text: "Another measurement is running; try again when it is done." } });
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
        this.set({ notice: { kind: "info", text: "The project changed while you were editing. It is up to date now; try again." } });
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
    if (!this.state.measurement?.busy) this.set({ measurement: null });
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
      this.set({ notice: { kind: "info", text: "Draw the region to follow on the preview, then choose what follows it." } });
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
      this.set({ notice: { kind: "info", text: "Select a multicam clip and put the playhead inside it to cut between angles." } });
      return;
    }
    const chosen = part.angles[angle - 1];
    if (!chosen) {
      this.set({ notice: { kind: "info", text: `This multicam clip has ${part.angles.length} angles.` } });
      return;
    }
    if (part.angle === chosen.id) {
      if (part.id !== this.state.selection) this.set({ selection: part.id, also: [] });
      return;
    }
    const response = await this.apply(
      [{ command: "multicam.switch", args: { clip: part.id, at: { flicks: at }, angle } }],
      `Cut to angle ${angle}`,
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
      this.set({ notice: { kind: "info", text: "The recording is in the project, at the playhead where it started." } });
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
        this.set({ notice: { kind: "info", text: "The project changed while you were editing. It is up to date now; try again." } });
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
        if (this.confirm(`${api.message}. Replace it?`)) return this.startExport({ ...choice, overwrite: true });
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
      if (!this.confirm(`The project already has a text preset called ${preset}. Replace it?`)) return false;
      args.replace = true;
    }
    if (!(await this.edit([{ command: "text.preset_save", args }]))) return false;
    this.set({ notice: { kind: "info", text: `Saved the style as ${preset}.` } });
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
      : { command: "text.add", args: { text: "Title", start } };
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

  async undo(): Promise<void> {
    if (this.state.opened?.info.can_undo) await this.edit([{ command: "history.undo" }]);
  }

  async redo(): Promise<void> {
    if (this.state.opened?.info.can_redo) await this.edit([{ command: "history.redo" }]);
  }
}
