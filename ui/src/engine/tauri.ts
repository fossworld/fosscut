// The engine client inside the desktop app: Tauri commands and events of apps/desktop.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { EngineClient, EngineEvents, NewProjectFormat, Rect, Unsubscribe } from "./client";
import type {
  AppInfo,
  CaptureDevice,
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
  ProjectSummary,
  RecordChoice,
  RecordState,
  SceneSplit,
  TrackRequest,
  TrashEntry,
} from "./types";

/** True inside the desktop app. */
export function inDesktopApp(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export class TauriEngine implements EngineClient {
  appInfo(): Promise<AppInfo> {
    return invoke("app_info");
  }

  projectSummaries(thumbnails: boolean): Promise<ProjectSummary[]> {
    return invoke("project_summaries", { thumbnails });
  }

  trash = {
    add: (path: string): Promise<TrashEntry> => invoke("project_trash", { path }),
    list: (): Promise<TrashEntry[]> => invoke("trash_list"),
    restore: (id: string): Promise<string> => invoke("trash_restore", { id }),
    remove: (id: string): Promise<void> => invoke("trash_delete", { id }),
  };

  recentProjects(): Promise<string[]> {
    return invoke("recent_projects");
  }

  currentProject(): Promise<Opened | null> {
    return invoke("project_current");
  }

  openProject(path: string): Promise<Opened> {
    return invoke("project_open", { path });
  }

  pickProject(): Promise<Opened | null> {
    return invoke("project_pick");
  }

  newProject(format?: NewProjectFormat): Promise<Opened | null> {
    return invoke("project_new", { ...format });
  }

  closeProject(): Promise<void> {
    return invoke("project_close");
  }

  query<T>(name: string, args?: unknown): Promise<T> {
    return invoke("query", { name, args: args ?? null });
  }

  execute(request: EditRequest): Promise<EditResponse> {
    return invoke("execute", { request });
  }

  clients(): Promise<Client[]> {
    return invoke("clients");
  }

  colorSuggest(clip: string, reference?: string): Promise<ColorSuggestion> {
    return invoke("color_suggest", { clip, reference: reference ?? null });
  }

  audioNormalize(clips: string[], targetLufs?: number): Promise<Normalization> {
    // Tauri hands camelCase arguments to the command's snake_case parameters.
    return invoke("audio_normalize", { clips, targetLufs: targetLufs ?? null });
  }

  measure = {
    scenes: (clip: string, expectedRevision?: number): Promise<SceneSplit> =>
      invoke("scenes_split", { clip, expectedRevision: expectedRevision ?? null }),
    stabilize: (clip: string, strength?: number, expectedRevision?: number): Promise<MotionResult> =>
      invoke("clip_stabilize", { clip, strength: strength ?? null, expectedRevision: expectedRevision ?? null }),
    track: (request: TrackRequest): Promise<MotionResult> => invoke("clip_track", { request }),
    multicam: ({ angles, sync, sound, name, start, expected_revision }: MulticamRequest): Promise<MulticamResult> =>
      invoke("multicam_create", {
        angles,
        sync,
        sound: sound ?? null,
        name: name ?? null,
        start: start ?? null,
        expectedRevision: expected_revision ?? null,
      }),
  };

  libraryList(kind?: string, search?: string): Promise<LibraryItem[]> {
    return invoke("library_list", { kind: kind ?? null, search: search ?? null });
  }

  libraryAdd(request: LibraryAddRequest): Promise<EditResponse> {
    return invoke("library_add", { request });
  }

  exportPresets(): Promise<ExportPresets> {
    return invoke("export_presets");
  }

  exportPick(name: string, extension?: string): Promise<string | null> {
    return invoke("export_pick", { name, extension: extension ?? null });
  }

  exportStart(choice: ExportChoice): Promise<JobStatus> {
    return invoke("export_start", { choice });
  }

  exportStatus(job: string): Promise<JobStatus> {
    return invoke("export_status", { job });
  }

  exportCancel(job: string): Promise<JobStatus> {
    return invoke("export_cancel", { job });
  }

  record = {
    devices: (): Promise<CaptureDevice[]> => invoke("record_devices"),
    start: (choice: RecordChoice): Promise<RecordState> => invoke("record_start", { choice }),
    status: (job: string): Promise<RecordState> => invoke("record_status", { job }),
    stop: (job: string): Promise<RecordState> => invoke("record_stop", { job }),
  };

  media = {
    reveal: (asset: string): Promise<void> => invoke("media_reveal", { asset }),
    relink: async (asset: string): Promise<boolean> => (await invoke("media_relink", { asset })) !== null,
  };

  preview = {
    viewport: (rect: Rect): Promise<void> => invoke("preview_viewport", { ...rect }),
    show: (flicks: number): Promise<void> => invoke("preview_show", { flicks: Math.round(flicks) }),
    play: (): Promise<void> => invoke("preview_play"),
    pause: (): Promise<void> => invoke("preview_pause"),
    shuttle: (rate: number): Promise<void> => invoke("preview_shuttle", { rate }),
  };

  fullscreen(on: boolean): Promise<void> {
    return invoke("window_fullscreen", { on });
  }

  on<E extends keyof EngineEvents>(
    event: E,
    handler: (payload: EngineEvents[E]) => void,
  ): Unsubscribe {
    let stop: (() => void) | null = null;
    let stopped = false;
    void listen<EngineEvents[E]>(event, (message) => handler(message.payload)).then((unlisten) => {
      if (stopped) unlisten();
      else stop = unlisten;
    });
    return () => {
      stopped = true;
      stop?.();
    };
  }
}
