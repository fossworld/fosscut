// The editor's state and the actions that change it. Every change to the project is a
// command sent through the engine client (ADR-0002); what the window shows is read back
// from the engine after each commit, whoever made it, so agents' edits appear live.

import type { EngineClient, Rect, Unsubscribe } from "../engine/client";
import {
  type ApiError,
  type Catalog,
  type Client,
  type CommandCall,
  type EditRequest,
  type History,
  type LogEntry,
  type Opened,
  type TimelineView,
  isApiError,
} from "../engine/types";
import { clipsAt, findClip } from "../timeline/geometry";
import { FLICKS_PER_SECOND, frameAt, frameStart, framesToFlicks, parseRate } from "../transport/time";

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
  /** The playhead, in flicks. */
  playhead: number;
  playing: boolean;
  selection: string | null;
  notice: Notice | null;
  /** Effect types and looks, from `capabilities`; null until read. */
  catalog: Catalog | null;
  /** LUT assets in the media table, for `color.lut` effects. */
  luts: { id: string; name: string }[];
  /** Drawing a path or painting a brush mask on the preview, when not null. */
  maskTool: MaskTool | null;
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
  playhead: 0,
  playing: false,
  selection: null,
  notice: null,
  catalog: null,
  luts: [],
  maskTool: null,
};

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
        this.set({ playhead: playhead.time.flicks, playing: playhead.playing }),
      ),
      this.engine.on("preview-error", (message) =>
        this.set({ playing: false, notice: { kind: "error", text: message } }),
      ),
    );
    try {
      const [recent, opened] = await Promise.all([this.engine.recentProjects(), this.engine.currentProject()]);
      this.set({ recent });
      if (opened) await this.show(opened);
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
      const luts = await this.engine
        .query<{ assets: { id: string; name: string; kind: string }[] }>("media.list")
        .then((media) => media.assets.filter((a) => a.kind === "lut").map(({ id, name }) => ({ id, name })))
        .catch(() => this.state.luts);
      this.set({ opened: { ...opened, info }, timeline, history: history.entries, selection, luts });
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
      this.set({ playing: false });
      await this.engine.preview.pause();
    } else if (this.state.timeline) {
      this.set({ playing: true });
      await this.engine.preview.play();
    }
  }

  /** Tells the preview where it shows through the page, in CSS pixels. */
  async setViewport(rect: Rect): Promise<void> {
    await this.engine.preview.viewport(rect);
  }

  select(clip: string | null): void {
    this.set({ selection: clip });
  }

  // ---- Edits ----

  /**
   * Applies commands as one undoable step against the revision on screen. A stale revision
   * reloads and says so; a destructive change asks first, then applies with consent.
   */
  async edit(commands: CommandCall[], reason?: string): Promise<boolean> {
    const timeline = this.state.timeline;
    if (!timeline) return false;
    const request: EditRequest = { commands, expected_revision: timeline.revision };
    if (reason) request.reason = reason;
    try {
      await this.engine.execute(request);
    } catch (error) {
      const api = asError(error);
      if (api.code === "fosscut.revision.conflict") {
        await this.refresh();
        this.set({ notice: { kind: "info", text: "The project changed while you were editing. It is up to date now; try again." } });
        return false;
      }
      if (api.code === "fosscut.guard.destructive" && this.confirm(`${api.message}. Do it anyway?`)) {
        try {
          await this.engine.execute({ ...request, allow_destructive: true });
        } catch (again) {
          this.notify(again);
          return false;
        }
      } else {
        if (api.code !== "fosscut.guard.destructive") this.notify(api);
        return false;
      }
    }
    await this.refresh();
    return true;
  }

  async deleteSelection(): Promise<void> {
    const clip = this.state.selection;
    if (!clip) return;
    if (await this.edit([{ command: "clip.remove", args: { clip } }])) this.set({ selection: null });
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
    const args: Record<string, unknown> = { clip, start: { flicks: start } };
    if (track) args.track = track;
    await this.edit([{ command: "clip.move", args }]);
  }

  /** Moves a clip's start or end edge (flicks). */
  async trimClip(clip: string, edges: { start?: number; end?: number }): Promise<void> {
    const args: Record<string, unknown> = { clip };
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
