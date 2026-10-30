import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<(...args: unknown[]) => Promise<null>>(async () => null);
const unlisten = vi.fn();
const handlers = new Map<string, (message: { payload: unknown }) => void>();
const listen = vi.fn(async (event: string, handler: (message: { payload: unknown }) => void) => {
  handlers.set(event, handler);
  return unlisten;
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (message: { payload: unknown }) => void) => listen(event, handler),
}));

const { TauriEngine, inDesktopApp } = await import("./tauri");

describe("the Tauri engine client", () => {
  beforeEach(() => {
    invoke.mockClear();
    unlisten.mockClear();
    handlers.clear();
  });

  it("calls the app's commands by their names with their arguments", async () => {
    const engine = new TauriEngine();
    await engine.appInfo();
    await engine.recentProjects();
    await engine.currentProject();
    await engine.openProject("/work/A.fosscut");
    await engine.pickProject();
    await engine.newProject({ width: 1080, height: 1920, rate: "30" });
    await engine.newProject();
    await engine.closeProject();
    await engine.query("timeline.read", { detail: "full" });
    await engine.query("project.info");
    await engine.execute({ commands: [{ command: "history.undo" }], expected_revision: 3 });
    await engine.clients();
    await engine.colorSuggest("clip_0003");
    await engine.colorSuggest("clip_0003", "clip_0007");
    await engine.preview.viewport({ x: 1, y: 2, width: 3, height: 4 });
    await engine.preview.show(1234.6);
    await engine.preview.play();
    await engine.preview.pause();
    await engine.preview.shuttle(-2);
    await engine.fullscreen(true);
    await engine.media.reveal("asset_0001");
    invoke.mockResolvedValueOnce(null);
    expect(await engine.media.relink("asset_0001"), "a cancelled dialog").toBe(false);
    expect(invoke.mock.calls).toEqual([
      ["app_info"],
      ["recent_projects"],
      ["project_current"],
      ["project_open", { path: "/work/A.fosscut" }],
      ["project_pick"],
      ["project_new", { width: 1080, height: 1920, rate: "30" }],
      ["project_new", {}],
      ["project_close"],
      ["query", { name: "timeline.read", args: { detail: "full" } }],
      ["query", { name: "project.info", args: null }],
      ["execute", { request: { commands: [{ command: "history.undo" }], expected_revision: 3 } }],
      ["clients"],
      ["color_suggest", { clip: "clip_0003", reference: null }],
      ["color_suggest", { clip: "clip_0003", reference: "clip_0007" }],
      ["preview_viewport", { x: 1, y: 2, width: 3, height: 4 }],
      ["preview_show", { flicks: 1235 }],
      ["preview_play"],
      ["preview_pause"],
      ["preview_shuttle", { rate: -2 }],
      ["window_fullscreen", { on: true }],
      ["media_reveal", { asset: "asset_0001" }],
      ["media_relink", { asset: "asset_0001" }],
    ]);
  });

  it("delivers event payloads and stops listening when asked, even before listening began", async () => {
    const engine = new TauriEngine();
    const changes: unknown[] = [];
    const stop = engine.on("project-changed", (change) => changes.push(change));
    await Promise.resolve();
    await Promise.resolve();
    handlers.get("project-changed")?.({ payload: { revision: 2 } });
    expect(changes).toEqual([{ revision: 2 }]);
    stop();
    expect(unlisten).toHaveBeenCalledTimes(1);

    const early = engine.on("playhead", () => undefined);
    early();
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten, "a listener stopped before it started is removed once it starts").toHaveBeenCalledTimes(2);
  });

  it("knows whether it runs inside the desktop app", () => {
    expect(inDesktopApp()).toBe(false);
  });
});
