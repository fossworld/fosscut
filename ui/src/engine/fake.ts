// An engine client for tests: answers from tables the test fills, records every call, and
// sends events when the test says so.

import type { EngineClient, EngineEvents, NewProjectFormat, Rect, Unsubscribe } from "./client";
import type { AppInfo, Client, ColorSuggestion, EditRequest, EditResponse, Opened } from "./types";

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

  preview = {
    viewport: async (rect: Rect): Promise<void> => this.record("preview.viewport", rect),
    show: async (flicks: number): Promise<void> => this.record("preview.show", flicks),
    play: async (): Promise<void> => this.record("preview.play"),
    pause: async (): Promise<void> => this.record("preview.pause"),
  };

  on<E extends keyof EngineEvents>(event: E, handler: (payload: EngineEvents[E]) => void): Unsubscribe {
    const set = this.handlers.get(event) ?? new Set<Handler>();
    set.add(handler as Handler);
    this.handlers.set(event, set);
    return () => set.delete(handler as Handler);
  }
}
