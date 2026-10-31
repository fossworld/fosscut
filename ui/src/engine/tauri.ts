// The engine client inside the desktop app: Tauri commands and events of apps/desktop.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { EngineClient, EngineEvents, NewProjectFormat, Rect, ThumbnailRequest, Unsubscribe } from "./client";
import type {
  AppInfo,
  CacheKind,
  CacheUsage,
  CaptureDevice,
  Client,
  DubCaptionsRequest,
  DubCaptionsResult,
  ColorSuggestion,
  EditRequest,
  EditResponse,
  ExportChoice,
  BeatCutsResult,
  BeatMarkersResult,
  ExportPresets,
  ImportResult,
  JobStatus,
  LibraryAddRequest,
  LibraryItem,
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
  ShortCandidates,
  ShortRequest,
  ShortResult,
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

/** A reframe request as the desktop's commands take it: flat, with nulls for what is left out. */
function reframeArgs(request: ReframeRequest): Record<string, unknown> {
  return {
    clip: request.clip,
    source: request.source,
    region: request.region ?? null,
    at: request.at === undefined ? null : Math.round(request.at),
    shots: request.shots ?? null,
    expectedRevision: request.expected_revision ?? null,
  };
}

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
    sources: (): Promise<Record<string, PreviewSource>> => invoke("media_sources"),
    work: (): Promise<MediaWork> => invoke("media_work"),
    thumbnails: (request: ThumbnailRequest): Promise<ThumbnailStrip> =>
      invoke("media_thumbnails", {
        asset: request.asset,
        times: request.times?.map(Math.round) ?? null,
        count: request.count ?? null,
        height: request.height ?? null,
      }),
    waveform: (asset: string, buckets: number): Promise<Waveform> => invoke("media_waveform", { asset, buckets }),
    import: (paths?: string[]): Promise<ImportResult | null> => invoke("media_import", { paths: paths ?? null }),
  };

  preview = {
    viewport: (rect: Rect): Promise<void> => invoke("preview_viewport", { ...rect }),
    show: (flicks: number, scrub?: boolean): Promise<void> =>
      invoke("preview_show", scrub ? { flicks: Math.round(flicks), scrub: true } : { flicks: Math.round(flicks) }),
    play: (): Promise<void> => invoke("preview_play"),
    pause: (): Promise<void> => invoke("preview_pause"),
    shuttle: (rate: number): Promise<void> => invoke("preview_shuttle", { rate }),
  };

  fullscreen(on: boolean): Promise<void> {
    return invoke("window_fullscreen", { on });
  }

  preferences = {
    get: (): Promise<PreferencesView> => invoke("preferences_get"),
    set: (key: string, value: unknown): Promise<PreferencesView> => invoke("preferences_set", { key, value }),
    reset: (key: string): Promise<PreferencesView> => invoke("preferences_reset", { key }),
  };

  updates = {
    check: (): Promise<UpdateAnswer> => invoke("update_check"),
    open: (): Promise<void> => invoke("update_open"),
  };

  playSound(name: string): Promise<void> {
    return invoke("sound_play", { name });
  }

  pickFolder(title?: string, directory?: string): Promise<string | null> {
    return invoke("folder_pick", { title: title ?? null, directory: directory ?? null });
  }

  models = {
    list: (): Promise<ModelListing> => invoke("models_list"),
    install: (name: string, confirmed: boolean): Promise<ModelInstallState> => invoke("models_install", { name, confirmed }),
    installStatus: (name: string): Promise<ModelInstallState> => invoke("models_install_status", { name }),
  };

  subject = {
    analyze: (request: SubjectRequest): Promise<JobStatus> => invoke("subject_analyze", { request }),
    status: (job: string): Promise<JobStatus> => invoke("subject_status", { job }),
    cancel: (job: string): Promise<JobStatus> => invoke("subject_cancel", { job }),
  };

  voice = {
    list: (): Promise<VoiceInfo[]> => invoke("voices_list"),
    preview: (text: string, voice: string, speed?: number): Promise<SpeechPreview> =>
      invoke("speech_preview", { text, voice, speed: speed ?? null }),
    add: (request: SpeechRequest): Promise<SpeechResult> => invoke("speech_add", { request }),
  };

  captions = {
    translate: (request: TranslateCaptionsRequest): Promise<TranslateCaptionsResult> =>
      invoke("captions_translate", { request }),
    dub: (request: DubCaptionsRequest): Promise<DubCaptionsResult> => invoke("captions_dub", { request }),
  };

  reframe = {
    clip: (request: ReframeRequest): Promise<ReframeResult> => invoke("clip_reframe", reframeArgs(request)),
    camera: (request: ReframeRequest & { zoom: number }): Promise<ReframeResult> => {
      // Camera tracking frames the clip as one move: it takes no shots.
      const args = reframeArgs(request);
      delete args.shots;
      return invoke("clip_camera_track", { ...args, zoom: request.zoom });
    },
    vertical: (request: VerticalRequest): Promise<VerticalResult> =>
      invoke("timeline_vertical", {
        timeline: request.timeline ?? null,
        source: request.source,
        width: request.width ?? null,
        height: request.height ?? null,
        name: request.name ?? null,
        shots: request.shots ?? null,
        expectedRevision: request.expected_revision ?? null,
      }),
  };

  beats = {
    markers: (clip: string, every?: number, expectedRevision?: number): Promise<BeatMarkersResult> =>
      invoke("beats_markers", { clip, every: every ?? null, expectedRevision: expectedRevision ?? null }),
    cut: (track: string, music: string, every?: number, expectedRevision?: number): Promise<BeatCutsResult> =>
      invoke("beats_cut", { track, music, every: every ?? null, expectedRevision: expectedRevision ?? null }),
  };

  shorts = {
    candidates: (clip: string, options: { min_seconds?: number; max_seconds?: number; count?: number } = {}): Promise<ShortCandidates> =>
      invoke("shorts_candidates", {
        clip,
        minSeconds: options.min_seconds ?? null,
        maxSeconds: options.max_seconds ?? null,
        count: options.count ?? null,
      }),
    make: (request: ShortRequest): Promise<ShortResult> =>
      invoke("shorts_make", {
        clip: request.clip,
        start: Math.round(request.start),
        end: Math.round(request.end),
        name: request.name ?? null,
        width: request.width ?? null,
        height: request.height ?? null,
        follow: request.follow ?? null,
        captions: request.captions ?? null,
        captionPreset: request.caption_preset ?? null,
        title: request.title ?? null,
        expectedRevision: request.expected_revision ?? null,
      }),
  };

  cache = {
    usage: async (): Promise<CacheUsage> => ({ caches: await invoke<CacheUsage["caches"]>("cache_info") }),
    clear: (kind: CacheKind | "all"): Promise<CacheUsage> => invoke("cache_clear", { kinds: kind === "all" ? [] : [kind] }),
  };

  on<E extends keyof EngineEvents>(
    event: E,
    handler: (payload: EngineEvents[E]) => void,
  ): Unsubscribe {
    let stop: (() => void) | null = null;
    let stopped = false;
    // Files dropped from the system arrive as the window's own drag and drop event.
    const listening =
      event === "files-dropped"
        ? listen<{ paths?: string[] }>("tauri://drag-drop", (message) => handler((message.payload.paths ?? []) as EngineEvents[E]))
        : listen<EngineEvents[E]>(event, (message) => handler(message.payload));
    void listening.then((unlisten) => {
      if (stopped) unlisten();
      else stop = unlisten;
    });
    return () => {
      stopped = true;
      stop?.();
    };
  }
}
