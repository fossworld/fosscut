// The one way the UI reaches the engine (ADR-0010, item 4). The desktop app implements it
// with Tauri (./tauri.ts); a web build will implement it over WebAssembly or a WebSocket.
// Components never call Tauri themselves.

import type {
  AppInfo,
  CaptureDevice,
  Change,
  Client,
  ColorSuggestion,
  EditRequest,
  EditResponse,
  ExportChoice,
  ExportPresets,
  JobStatus,
  LibraryAddRequest,
  LibraryItem,
  MotionResult,
  MulticamRequest,
  MulticamResult,
  Normalization,
  Opened,
  Playhead,
  ProjectSummary,
  RecordChoice,
  RecordState,
  SceneSplit,
  TrackRequest,
  TrashEntry,
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
  /** The recent projects for the browser, with thumbnails when `thumbnails`. */
  projectSummaries(thumbnails: boolean): Promise<ProjectSummary[]>;
  trash: {
    /** Moves a project to the trash (never the open one). */
    add(path: string): Promise<TrashEntry>;
    list(): Promise<TrashEntry[]>;
    /** Moves a project back; returns where it is now. */
    restore(id: string): Promise<string>;
    /** Deletes a trashed project for good. */
    remove(id: string): Promise<void>;
  };
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
  /**
   * Measures the loudness of clips and works out the volumes that bring each to
   * `targetLufs` (-14 LUFS when omitted), to apply with `execute` like any edit.
   */
  audioNormalize(clips: string[], targetLufs?: number): Promise<Normalization>;
  /**
   * Edits measured from media, each applied by the engine as the user's one undoable step
   * against `expectedRevision` when given, answering what was measured with the response.
   */
  measure: {
    /** Splits a video clip at its shot cuts, so each shot is a clip of its own. */
    scenes(clip: string, expectedRevision?: number): Promise<SceneSplit>;
    /** Keys a video clip's placement to cancel camera shake; `strength` 0 to 1, 0.5 by default. */
    stabilize(clip: string, strength?: number, expectedRevision?: number): Promise<MotionResult>;
    /** Follows a region of a video clip and keys one of its masks, or another clip, to it. */
    track(request: TrackRequest): Promise<MotionResult>;
    /** Makes a multicam clip of video assets, lined up by their sound unless told not to. */
    multicam(request: MulticamRequest): Promise<MulticamResult>;
  };
  /**
   * The built-in library's items, of `kind` when given, with every word of `search` in
   * their name, title, description or tags.
   */
  libraryList(kind?: string, search?: string): Promise<LibraryItem[]>;
  /**
   * Adds a library item to the open project as the user's edit: sounds and music are
   * imported, then placed; templates build a new timeline that becomes the one shown.
   */
  libraryAdd(request: LibraryAddRequest): Promise<EditResponse>;
  /** The export presets this machine has, and the resolutions export offers by name. */
  exportPresets(): Promise<ExportPresets>;
  /** Asks the user where to save an export; null when they cancel. */
  exportPick(name: string, extension?: string): Promise<string | null>;
  /** Starts exporting the primary timeline in the background, after checking it. */
  exportStart(choice: ExportChoice): Promise<JobStatus>;
  exportStatus(job: string): Promise<JobStatus>;
  /** Stops an export; nothing is written. */
  exportCancel(job: string): Promise<JobStatus>;
  /** Recording a microphone, camera or screen into the open project. */
  record: {
    /** The cameras, screens and microphones on this machine. */
    devices(): Promise<CaptureDevice[]>;
    /**
     * Starts recording in the background; once it succeeds, the file is imported and placed
     * at `choice.at` once, and the status says as which asset and clip.
     */
    start(choice: RecordChoice): Promise<RecordState>;
    status(job: string): Promise<RecordState>;
    /** Stops recording and keeps what was recorded. */
    stop(job: string): Promise<RecordState>;
  };
  media: {
    /** Shows an asset's file in the system's file manager. */
    reveal(asset: string): Promise<void>;
    /** Asks where an asset's file is now and relinks it; false when the user cancels. */
    relink(asset: string): Promise<boolean>;
  };
  preview: {
    viewport(rect: Rect): Promise<void>;
    show(flicks: number): Promise<void>;
    play(): Promise<void>;
    pause(): Promise<void>;
    /** Plays at `rate` without sound unless it is 1: 2 is twice as fast, -1 backwards. */
    shuttle(rate: number): Promise<void>;
  };
  /** Puts the window in full screen, or back. */
  fullscreen(on: boolean): Promise<void>;
  on<E extends keyof EngineEvents>(
    event: E,
    handler: (payload: EngineEvents[E]) => void,
  ): Unsubscribe;
}
