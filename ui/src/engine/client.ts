// The one way the UI reaches the engine (ADR-0010, item 4). The desktop app implements it
// with Tauri (./tauri.ts); a web build will implement it over WebAssembly or a WebSocket.
// Components never call Tauri themselves.

import type {
  AppInfo,
  Change,
  Client,
  ColorSuggestion,
  EditRequest,
  EditResponse,
  Opened,
  Playhead,
} from "./types";

export type Unsubscribe = () => void;

/** Events the engine sends without being asked. */
export interface EngineEvents {
  /** A commit to the open project, by the user or anyone connected. */
  "project-changed": Change;
  /** The open project was closed. */
  "project-closed": null;
  /** The preview showed a frame. */
  playhead: Playhead;
  /** The preview could not show a frame, for example when media is missing. */
  "preview-error": string;
}

/** Where the preview shows through the page, in CSS pixels. */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface NewProjectFormat {
  width: number;
  height: number;
  rate: string;
}

export interface EngineClient {
  appInfo(): Promise<AppInfo>;
  recentProjects(): Promise<string[]>;
  currentProject(): Promise<Opened | null>;
  openProject(path: string): Promise<Opened>;
  /** Asks the user for a project folder; null when they cancel. */
  pickProject(): Promise<Opened | null>;
  /** Asks the user where to save a new project; null when they cancel. */
  newProject(format?: NewProjectFormat): Promise<Opened | null>;
  closeProject(): Promise<void>;
  query<T>(name: string, args?: unknown): Promise<T>;
  execute(request: EditRequest): Promise<EditResponse>;
  clients(): Promise<Client[]>;
  /**
   * Measures a clip and suggests a color correction (matching `reference` when given),
   * to apply with `execute` like any edit.
   */
  colorSuggest(clip: string, reference?: string): Promise<ColorSuggestion>;
  preview: {
    viewport(rect: Rect): Promise<void>;
    show(flicks: number): Promise<void>;
    play(): Promise<void>;
    pause(): Promise<void>;
  };
  on<E extends keyof EngineEvents>(
    event: E,
    handler: (payload: EngineEvents[E]) => void,
  ): Unsubscribe;
}
