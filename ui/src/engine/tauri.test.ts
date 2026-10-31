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
    await engine.audioNormalize(["clip_0003", "clip_0008"]);
    await engine.audioNormalize(["clip_0008"], -16);
    await engine.measure.scenes("clip_0003");
    await engine.measure.scenes("clip_0003", 7);
    await engine.measure.stabilize("clip_0003");
    await engine.measure.stabilize("clip_0003", 0.8, 7);
    const region = { x: 0.4, y: 0.5, width: 0.2, height: 0.1 };
    await engine.measure.track({ clip: "clip_0003", region, at: 705_600_000, mask: "mask_0010", expected_revision: 7 });
    await engine.measure.multicam({ angles: ["asset_0001", "asset_0002"], sync: true, start: 0 });
    await engine.measure.multicam({ angles: ["asset_0001", "asset_0002"], sync: false, sound: 2, name: "Interview", expected_revision: 7 });
    await engine.preview.viewport({ x: 1, y: 2, width: 3, height: 4 });
    await engine.preview.show(1234.6);
    await engine.preview.play();
    await engine.preview.pause();
    await engine.preview.shuttle(-2);
    await engine.fullscreen(true);
    await engine.media.reveal("asset_0001");
    await engine.projectSummaries(true);
    await engine.trash.add("/work/Old.fosscut");
    await engine.trash.list();
    await engine.trash.restore("t1");
    await engine.trash.remove("t1");
    await engine.libraryList();
    await engine.libraryList("sound", "whoosh");
    await engine.libraryAdd({ name: "badge-sale", kind: "sticker", start_flicks: 705_600_000, expected_revision: 4 });
    await engine.exportPresets();
    await engine.exportPick("Launch.mp4", "mp4");
    await engine.exportPick("Launch");
    await engine.exportStart({ output: "/work/Launch.mp4", preset: "mp4-h264", range_flicks: [0, 705_600_000] });
    await engine.exportStatus("job_0001");
    await engine.exportCancel("job_0001");
    await engine.record.devices();
    await engine.record.start({ what: "camera", device: "FaceTime HD Camera", microphone: "none", at: 705_600_000 });
    await engine.record.status("job_0002");
    await engine.record.stop("job_0002");
    await engine.preview.show(500, true);
    await engine.preferences.get();
    await engine.preferences.set("editing.frame_step", 5);
    await engine.preferences.reset("editing.frame_step");
    await engine.updates.check();
    await engine.updates.open();
    await engine.playSound("ding");
    await engine.pickFolder("Folder for exports", "/work");
    await engine.pickFolder();
    await engine.cache.clear("all");
    await engine.cache.clear("proxies");
    await engine.media.sources();
    await engine.media.work();
    await engine.models.list();
    await engine.models.install("birefnet-lite", true);
    await engine.models.installStatus("birefnet-lite");
    await engine.subject.analyze({ clip: "clip_0003", kind: "matte", model: "subject" });
    await engine.subject.status("job_0003");
    await engine.subject.cancel("job_0003");
    await engine.voice.list();
    await engine.voice.preview("Hello", "af_heart", 1.2);
    await engine.voice.preview("Hello", "af_heart");
    await engine.voice.add({ text: "Hello", voice: "af_heart", speed: 1, at: 705_600_000, expected_revision: 7 });
    await engine.captions.translate({ track: "track_0010", to: "hu", mode: "bilingual", expected_revision: 7 });
    await engine.captions.dub({ track: "track_0010", voice: "bm_george", speed: 1.1, translation: true, expected_revision: 8 });
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
      ["audio_normalize", { clips: ["clip_0003", "clip_0008"], targetLufs: null }],
      ["audio_normalize", { clips: ["clip_0008"], targetLufs: -16 }],
      ["scenes_split", { clip: "clip_0003", expectedRevision: null }],
      ["scenes_split", { clip: "clip_0003", expectedRevision: 7 }],
      ["clip_stabilize", { clip: "clip_0003", strength: null, expectedRevision: null }],
      ["clip_stabilize", { clip: "clip_0003", strength: 0.8, expectedRevision: 7 }],
      ["clip_track", { request: { clip: "clip_0003", region, at: 705_600_000, mask: "mask_0010", expected_revision: 7 } }],
      [
        "multicam_create",
        { angles: ["asset_0001", "asset_0002"], sync: true, sound: null, name: null, start: 0, expectedRevision: null },
      ],
      [
        "multicam_create",
        { angles: ["asset_0001", "asset_0002"], sync: false, sound: 2, name: "Interview", start: null, expectedRevision: 7 },
      ],
      ["preview_viewport", { x: 1, y: 2, width: 3, height: 4 }],
      ["preview_show", { flicks: 1235 }],
      ["preview_play"],
      ["preview_pause"],
      ["preview_shuttle", { rate: -2 }],
      ["window_fullscreen", { on: true }],
      ["media_reveal", { asset: "asset_0001" }],
      ["project_summaries", { thumbnails: true }],
      ["project_trash", { path: "/work/Old.fosscut" }],
      ["trash_list"],
      ["trash_restore", { id: "t1" }],
      ["trash_delete", { id: "t1" }],
      ["library_list", { kind: null, search: null }],
      ["library_list", { kind: "sound", search: "whoosh" }],
      [
        "library_add",
        { request: { name: "badge-sale", kind: "sticker", start_flicks: 705_600_000, expected_revision: 4 } },
      ],
      ["export_presets"],
      ["export_pick", { name: "Launch.mp4", extension: "mp4" }],
      ["export_pick", { name: "Launch", extension: null }],
      ["export_start", { choice: { output: "/work/Launch.mp4", preset: "mp4-h264", range_flicks: [0, 705_600_000] } }],
      ["export_status", { job: "job_0001" }],
      ["export_cancel", { job: "job_0001" }],
      ["record_devices"],
      ["record_start", { choice: { what: "camera", device: "FaceTime HD Camera", microphone: "none", at: 705_600_000 } }],
      ["record_status", { job: "job_0002" }],
      ["record_stop", { job: "job_0002" }],
      ["preview_show", { flicks: 500, scrub: true }],
      ["preferences_get"],
      ["preferences_set", { key: "editing.frame_step", value: 5 }],
      ["preferences_reset", { key: "editing.frame_step" }],
      ["update_check"],
      ["update_open"],
      ["sound_play", { name: "ding" }],
      ["folder_pick", { title: "Folder for exports", directory: "/work" }],
      ["folder_pick", { title: null, directory: null }],
      ["cache_clear", { kinds: [] }],
      ["cache_clear", { kinds: ["proxies"] }],
      ["media_sources"],
      ["media_work"],
      ["models_list"],
      ["models_install", { name: "birefnet-lite", confirmed: true }],
      ["models_install_status", { name: "birefnet-lite" }],
      ["subject_analyze", { request: { clip: "clip_0003", kind: "matte", model: "subject" } }],
      ["subject_status", { job: "job_0003" }],
      ["subject_cancel", { job: "job_0003" }],
      ["voices_list"],
      ["speech_preview", { text: "Hello", voice: "af_heart", speed: 1.2 }],
      ["speech_preview", { text: "Hello", voice: "af_heart", speed: null }],
      ["speech_add", { request: { text: "Hello", voice: "af_heart", speed: 1, at: 705_600_000, expected_revision: 7 } }],
      ["captions_translate", { request: { track: "track_0010", to: "hu", mode: "bilingual", expected_revision: 7 } }],
      [
        "captions_dub",
        { request: { track: "track_0010", voice: "bm_george", speed: 1.1, translation: true, expected_revision: 8 } },
      ],
      ["media_relink", { asset: "asset_0001" }],
    ]);
  });

  it("reads the caches' sizes as the app lists them", async () => {
    const engine = new TauriEngine();
    const caches = [{ kind: "proxies", path: "/cache/proxies", bytes: 300, entries: 1 }];
    invoke.mockImplementationOnce((async () => caches) as unknown as () => Promise<null>);
    expect(await engine.cache.usage()).toEqual({ caches });
    expect(invoke).toHaveBeenCalledWith("cache_info");
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
