// The engine client inside the desktop app: Tauri commands and events of apps/desktop.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { EngineClient, EngineEvents, NewProjectFormat, Rect, Unsubscribe } from "./client";
import type { AppInfo, Client, ColorSuggestion, EditRequest, EditResponse, Opened } from "./types";

/** True inside the desktop app. */
export function inDesktopApp(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export class TauriEngine implements EngineClient {
  appInfo(): Promise<AppInfo> {
    return invoke("app_info");
  }

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

  preview = {
    viewport: (rect: Rect): Promise<void> => invoke("preview_viewport", { ...rect }),
    show: (flicks: number): Promise<void> => invoke("preview_show", { flicks: Math.round(flicks) }),
    play: (): Promise<void> => invoke("preview_play"),
    pause: (): Promise<void> => invoke("preview_pause"),
  };

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
