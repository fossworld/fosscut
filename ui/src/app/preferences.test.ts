import { type ReactElement, createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";

import { CacheStandIn } from "../engine/cache";
import { FakeEngine } from "../engine/fake";
import type { EditRequest, Opened } from "../engine/types";
import { language, setLanguage, t } from "../i18n";
import { activeKeymap, applyShortcuts } from "../shortcuts";
import { entry, timeline } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import { Editor, type EditorState, UPDATE_INTERVAL, mediaWorkLine, preferencesOf } from "./editor";
import {
  CacheSection,
  EditingSection,
  ExportSection,
  GeneralSection,
  LocationsSection,
  PerformanceSection,
  Settings,
  ShortcutsSection,
  TimelineSection,
} from "./Settings";

function opened(): Opened {
  return {
    bundle: "/work/Demo.fosscut",
    info: {
      id: "project_1",
      name: "Demo",
      revision: 4,
      can_undo: true,
      can_redo: false,
      primary_timeline: "timeline_0000",
      timelines: [],
      assets: 2,
    },
  };
}

function engine(): FakeEngine {
  const fake = new FakeEngine();
  fake.revision = 4;
  fake.opened = opened();
  fake.answers["project.info"] = () => fake.opened?.info;
  fake.answers["timeline.read"] = () => timeline(fake.revision);
  fake.answers["history.list"] = { entries: [entry(4, "human", ["clip.add"])] };
  fake.answers["media.list"] = {
    assets: [
      { id: "asset_0001", kind: "video", name: "clip.mp4", used_by: ["clip_0003"] },
      { id: "asset_0009", kind: "image", name: "logo.png", used_by: [] },
    ],
  };
  return fake;
}

async function started(fake = engine(), confirm: (question: string) => boolean = () => true) {
  const editor = new Editor(fake, confirm);
  await editor.start();
  return { editor, fake };
}

/** A section of Settings as the window shows it. */
function shown(component: (props: { editor: Editor; state: EditorState }) => ReactElement | null, editor: Editor): string {
  return renderToStaticMarkup(createElement(component, { editor, state: editor.getState() }));
}

afterEach(() => {
  setLanguage("en");
  applyShortcuts("fosscut", {});
});

describe("preferences in the editor", () => {
  it("reads the preferences at start and applies the language and shortcuts they choose", async () => {
    const fake = engine();
    fake.prefs.set("language", "hu");
    fake.prefs.set("shortcuts", { set: "classic", overrides: { "player.play": "P" } });
    const { editor } = await started(fake);
    expect(language()).toBe("hu");
    expect(activeKeymap()["timeline.split"]).toEqual(["Mod+K"]);
    expect(activeKeymap()["player.play"]).toEqual(["P"]);
    expect(editor.getState().preferences?.path).toBe("/stand-in/preferences.json");
    await editor.setPreference("language", "en");
    expect(language(), "a change applies live").toBe("en");
  });

  it("shows the engine's refusal beside the control, and clears it once a value is taken", async () => {
    const { editor, fake } = await started();
    expect(await editor.setPreference("editing.frame_step", 0)).toBe(false);
    const refusal = "editing.frame_step must be from 1 to 1000, not 0";
    expect(editor.getState().settings.errors["editing.frame_step"]).toBe(refusal);
    expect(preferencesOf(editor.getState()).editing.frame_step, "a refusal changes nothing").toBe(10);
    editor.openSettings("editing");
    expect(shown(EditingSection, editor)).toContain(refusal);
    expect(shown(Settings, editor)).toContain(t("settings.section.editing"));
    expect(await editor.setPreference("editing.frame_step", 5)).toBe(true);
    expect(editor.getState().settings.errors).toEqual({});
    expect(shown(EditingSection, editor)).not.toContain(refusal);
    expect(fake.callsOf("preferences.set")).toEqual([
      ["editing.frame_step", 0],
      ["editing.frame_step", 5],
    ]);
    await editor.setPreference("timeline.frame_rate", "7");
    expect(shown(TimelineSection, editor)).toContain("timeline.frame_rate must be from 12 to 120, not 7");
    await editor.resetPreference("editing.frame_step");
    expect(preferencesOf(editor.getState()).editing.frame_step).toBe(10);
  });

  it("shows every section, in Hungarian too", async () => {
    const { editor } = await started();
    editor.openSettings();
    for (const section of [GeneralSection, LocationsSection, EditingSection, TimelineSection, ExportSection, PerformanceSection, CacheSection, ShortcutsSection]) {
      expect(shown(section, editor).length).toBeGreaterThan(100);
    }
    expect(shown(GeneralSection, editor)).toContain("Magyar");
    expect(shown(ShortcutsSection, editor)).toContain(t("settings.action.timeline.split"));
    setLanguage("hu");
    expect(shown(PerformanceSection, editor)).toContain("Hardveres dekódolás");
    expect(shown(ShortcutsSection, editor)).toContain("Vágás a lejátszófejnél");
    expect(shown(Settings, editor)).toContain("Beállítások");
  });

  it("fills requests with the preferred values, as the CLI does", async () => {
    const fake = engine();
    fake.prefs.set("timeline.frame_rate", "25");
    fake.prefs.set("editing.image_duration", 3.5);
    fake.prefs.set("editing.loudness_target", -18);
    const { editor } = await started(fake);
    await editor.create();
    expect(fake.callsOf("newProject")).toEqual([[{ width: 1920, height: 1080, rate: "25" }]]);
    await editor.addToTimeline("asset_0009");
    await editor.addToTimeline("asset_0001");
    const adds = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]?.args);
    expect(adds, "a still image takes the preferred length; a video its own").toEqual([
      { asset: "asset_0009", start: { flicks: 0 }, duration: { seconds: "3.5" } },
      { asset: "asset_0001", start: { flicks: 0 } },
    ]);
    await editor.normalizeLoudness(["clip_0003"]);
    expect(fake.callsOf("audioNormalize")).toEqual([[["clip_0003"], -18]]);
  });

  it("steps by the frame step and nudges by the nudge pixels", async () => {
    const fake = engine();
    fake.prefs.set("editing.frame_step", 4);
    fake.prefs.set("editing.nudge_pixels", 96);
    const { editor } = await started(fake);
    await editor.stepMany(1);
    expect(editor.getState().playhead).toBe((4 / 30) * FLICKS_PER_SECOND);
    await editor.stepMany(-1);
    expect(editor.getState().playhead).toBe(0);
    editor.select("clip_0003");
    await editor.nudge(1, -1);
    const request = fake.callsOf("execute").at(-1)?.[0] as EditRequest;
    expect(request.commands).toEqual([
      { command: "clip.set", args: { clip: "clip_0003", set: { "transform.position": [0.05, -0.088889] } } },
    ]);
    expect(request.reason).toBe(t("app.nudge"));
    editor.select(null);
    const before = fake.callsOf("execute").length;
    await editor.nudge(1, 0);
    expect(fake.callsOf("execute"), "nothing selected, nothing nudged").toHaveLength(before);
  });

  it("checks for updates at start unless they are off, and daily when asked to", async () => {
    const fake = engine();
    fake.latestRelease = { version: "0.2.0", url: "https://example.org/r" };
    const { editor } = await started(fake);
    expect(fake.callsOf("updates.check")).toHaveLength(1);
    expect(editor.getState().update).toMatchObject({ latest: "0.2.0", newer: true });
    editor.dismissUpdate();
    expect(editor.getState().update).toBeNull();
    await editor.openRelease();
    expect(fake.callsOf("updates.open")).toHaveLength(1);
    editor.stop();

    const off = engine();
    off.prefs.set("updates", "off");
    const quiet = await started(off);
    expect(off.callsOf("updates.check"), "nothing is asked").toHaveLength(0);
    quiet.editor.stop();
    expect(UPDATE_INTERVAL).toBe(86_400_000);
  });

  it("picks the folders of locations and resets them to their defaults", async () => {
    const { editor, fake } = await started();
    await editor.pickLocation("exports");
    expect(fake.callsOf("pickFolder")).toEqual([[t("settings.locations.exports.pick"), undefined]]);
    expect(preferencesOf(editor.getState()).locations.exports).toBe("/Users/demo/Movies/Exports");
    expect(shown(LocationsSection, editor)).toContain("/Users/demo/Movies/Exports");
    fake.folderChosen = null;
    await editor.pickLocation("projects");
    expect(preferencesOf(editor.getState()).locations.projects, "a cancel changes nothing").toBeNull();
    await editor.resetPreference("locations.exports");
    expect(preferencesOf(editor.getState()).locations.exports).toBeNull();
    expect(shown(LocationsSection, editor)).toContain(t("settings.locations.exports.default"));
  });

  it("plays the export sound and lists the library's sounds", async () => {
    const fake = engine();
    fake.library = [
      { name: "ding", kind: "sound", title: "Ding", description: "A bell.", tags: [], commercial: true },
      { name: "pop", kind: "sound", title: "Pop", description: "A pop.", tags: [], commercial: true },
    ];
    const { editor } = await started(fake);
    editor.openSettings("export");
    await Promise.resolve();
    await Promise.resolve();
    expect(editor.getState().settings.sounds?.map((s) => s.name)).toEqual(["ding", "pop"]);
    expect(shown(ExportSection, editor)).toContain("Pop");
    await editor.playSound("pop");
    expect(fake.callsOf("playSound")).toEqual([["pop"]]);
  });
});

describe("the cache section", () => {
  it("shows each cache's size and clears one or all after asking", async () => {
    const asked: string[] = [];
    const { editor, fake } = await started(engine(), (question) => {
      asked.push(question);
      return true;
    });
    await editor.loadCacheUsage();
    const markup = shown(CacheSection, editor);
    expect(markup).toContain(t("settings.cache.kind.proxies"));
    expect(markup).toContain("2.4 GB");
    await editor.clearCache("proxies");
    expect(asked).toEqual([t("settings.cache.confirm", { cache: t("settings.cache.kind.proxies") })]);
    expect(fake.callsOf("cache.clear")).toEqual([["proxies"]]);
    const after = editor.getState().settings.cache;
    expect(after?.caches.find((c) => c.kind === "proxies")?.bytes).toBe(0);
    expect(after?.freed_bytes).toBe(2_400_000_000);
    expect(shown(CacheSection, editor)).toContain(t("settings.cache.freed", { size: "2.4 GB" }));
    await editor.clearCache("all");
    expect(editor.getState().settings.cache?.caches.every((c) => c.bytes === 0)).toBe(true);
  });

  it("asks before clearing, and shows why clearing failed", async () => {
    const fake = engine();
    fake.caches = new CacheStandIn(null);
    const { editor } = await started(fake, () => false);
    await editor.clearCache("frames");
    expect(fake.callsOf("cache.clear"), "not without consent").toHaveLength(0);
    const sure = new Editor(fake, () => true);
    await sure.start();
    await sure.loadCacheUsage();
    expect(shown(CacheSection, sure)).toContain(t("settings.cache.unknown"));
    await sure.clearCache("frames");
    expect(sure.getState().settings.cacheError).toBe("clearing caches from the app is not available yet");
    expect(shown(CacheSection, sure)).toContain("clearing caches from the app is not available yet");
  });
});

describe("the media work status line", () => {
  it("says which copies are being made, and nothing once they are done", async () => {
    expect(mediaWorkLine(null)).toBeNull();
    const idle = { proxies: { done: 0, total: 0 }, remux: { done: 0, total: 0 }, failed: 0 };
    expect(mediaWorkLine(idle)).toBeNull();
    expect(mediaWorkLine({ ...idle, proxies: { done: 1, total: 5 } })).toBe("Making proxies 2 of 5");
    expect(mediaWorkLine({ ...idle, proxies: { done: 1, total: 2 }, remux: { done: 0, total: 1 } })).toBe(
      "Making proxies 2 of 2 · Remuxing media 1 of 1",
    );
    setLanguage("hu");
    expect(mediaWorkLine({ ...idle, proxies: { done: 1, total: 5 } })).toBe("Proxyk készítése: 2/5");
  });

  it("follows the engine's reports and reads the media marks when copies are made", async () => {
    const fake = engine();
    fake.mediaWork = { proxies: { done: 0, total: 2 }, remux: { done: 0, total: 0 }, failed: 0 };
    const { editor } = await started(fake);
    await Promise.resolve();
    expect(mediaWorkLine(editor.getState().mediaWork)).toBe("Making proxies 1 of 2");
    fake.sources = { asset_0001: { proxy: "half" } };
    fake.emit("media-work", { proxies: { done: 1, total: 2 }, remux: { done: 0, total: 0 }, failed: 0 });
    await Promise.resolve();
    await Promise.resolve();
    expect(editor.getState().sources).toEqual({ asset_0001: { proxy: "half" } });
    fake.emit("open-settings", null);
    expect(editor.getState().settings.open, "the app menu opens Settings").toBe(true);
  });
});

describe("the media bin's marks", () => {
  it("marks assets the preview decodes from a proxy or a remuxed copy", async () => {
    const { sourceMark } = await import("../panels/Media");
    expect(sourceMark(undefined)).toBeNull();
    expect(sourceMark("original")).toBeNull();
    expect(sourceMark({ proxy: "half" })).toEqual({ label: t("media.mark.proxy"), help: t("media.mark.proxy.half") });
    expect(sourceMark("remux")?.label).toBe("Remuxed");
    setLanguage("hu");
    expect(sourceMark("remux")?.label).toBe("Átcsomagolt");
  });
});
