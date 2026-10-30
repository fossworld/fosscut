import { describe, expect, it } from "vitest";

import { FakeEngine } from "../engine/fake";
import type { EditRequest, Opened } from "../engine/types";
import { entry, moment, timeline } from "../test/fixtures";
import { FLICKS_PER_SECOND, framesToFlicks, parseRate } from "../transport/time";
import { Editor, levelled } from "./editor";

const fps30 = parseRate("30");

function opened(revision = 4): Opened {
  return {
    bundle: "/work/Demo.fosscut",
    info: {
      id: "project_1",
      name: "Demo",
      revision,
      can_undo: true,
      can_redo: false,
      primary_timeline: "timeline_0000",
      timelines: [],
      assets: 1,
    },
  };
}

/** An engine with Demo open: the fixture timeline and two history entries. */
function engine(): FakeEngine {
  const fake = new FakeEngine();
  fake.revision = 4;
  fake.opened = opened();
  fake.recent = ["/work/Demo.fosscut"];
  fake.answers["project.info"] = () => fake.opened?.info;
  fake.answers["timeline.read"] = () => timeline(fake.revision);
  fake.answers["history.list"] = { entries: [entry(4, "agent:claude-code", ["clip.split"]), entry(3, "human", ["text.add"])] };
  return fake;
}

async function started(fake = engine(), confirm: (question: string) => boolean = () => false) {
  const editor = new Editor(fake, confirm);
  await editor.start();
  return { editor, fake };
}

describe("editor", () => {
  it("loads the open project and puts the playhead at the start", async () => {
    const { editor, fake } = await started();
    const state = editor.getState();
    expect(state.opened?.info.name).toBe("Demo");
    expect(state.timeline?.tracks).toHaveLength(2);
    expect(state.history.map((e) => e.seq)).toEqual([4, 3]);
    expect(state.recent).toEqual(["/work/Demo.fosscut"]);
    expect(fake.callsOf("preview.show")).toEqual([[0]]);
    expect(fake.callsOf("query")).toContainEqual(["timeline.read", { detail: "full", timeline: "timeline_0000" }]);
    expect(fake.callsOf("query"), "the effect catalog, once").toContainEqual(["capabilities", { detail: "full" }]);
  });

  it("starts empty when no project is open, and opens one by path or dialog", async () => {
    const fake = engine();
    fake.opened = null;
    const { editor } = await started(fake);
    expect(editor.getState().opened).toBeNull();
    await editor.open("/nowhere.fosscut");
    expect(editor.getState().notice?.text).toContain("not a FossCut project");
    fake.opened = opened();
    await editor.pick();
    expect(editor.getState().timeline?.name).toBe("Timeline 1");
    await editor.create();
    expect(fake.callsOf("newProject")).toHaveLength(1);
    await editor.open("/work/Demo.fosscut");
    expect(fake.callsOf("openProject")).toEqual([["/nowhere.fosscut"], ["/work/Demo.fosscut"]]);
    await editor.close();
    expect(editor.getState().opened).toBeNull();
    expect(editor.getState().recent).toEqual(["/work/Demo.fosscut"]);
  });

  it("follows commits by anyone, and the playhead the preview reports", async () => {
    const { editor, fake } = await started();
    const seen: number[] = [];
    editor.subscribe(() => seen.push(editor.getState().timeline?.revision ?? -1));
    fake.revision = 9;
    fake.emit("project-changed", { revision: 9, seq: 9, kind: "commit", actor: "agent:codex", commands: ["clip.add"] });
    await editor.refresh();
    expect(editor.getState().timeline?.revision).toBe(9);
    expect(seen).toContain(9);

    fake.emit("playhead", { time: moment(2), playing: true });
    expect(editor.getState()).toMatchObject({ playhead: 2 * FLICKS_PER_SECOND, playing: true });
    fake.emit("preview-error", "clip.mp4 is missing");
    expect(editor.getState()).toMatchObject({ playing: false, notice: { kind: "error", text: "clip.mp4 is missing" } });
    editor.dismissNotice();
    expect(editor.getState().notice).toBeNull();
    fake.emit("project-closed", null);
    expect(editor.getState()).toMatchObject({ opened: null, playhead: 0 });
    editor.stop();
    fake.emit("playhead", { time: moment(1), playing: true });
    expect(editor.getState().playing, "stopped editors no longer listen").toBe(false);
  });

  it("reads once more, not many times, when commits arrive during a read", async () => {
    const { editor, fake } = await started();
    const before = fake.callsOf("query").length;
    await Promise.all([editor.refresh(), editor.refresh(), editor.refresh()]);
    expect(fake.callsOf("query").length - before, "two reads of four queries").toBe(8);
  });

  it("moves the playhead by frames and seconds within the timeline", async () => {
    const { editor, fake } = await started();
    await editor.seek(FLICKS_PER_SECOND + 1000);
    expect(editor.getState().playhead).toBe(FLICKS_PER_SECOND);
    await editor.step(1);
    expect(editor.getState().playhead).toBe(framesToFlicks(31, fps30));
    await editor.stepSeconds(-10);
    expect(editor.getState().playhead).toBe(0);
    await editor.seekEnd();
    expect(editor.getState().playhead).toBe(framesToFlicks(179, fps30));
    await editor.stepSeconds(5);
    expect(editor.getState().playhead, "held at the last frame").toBe(framesToFlicks(179, fps30));
    await editor.togglePlay();
    expect(editor.getState().playing).toBe(true);
    await editor.togglePlay();
    expect(fake.callsOf("preview.play")).toHaveLength(1);
    await editor.setViewport({ x: 1, y: 2, width: 3, height: 4 });
    expect(fake.callsOf("preview.viewport")).toEqual([[{ x: 1, y: 2, width: 3, height: 4 }]]);
    expect(fake.callsOf("preview.pause")).toHaveLength(1);
  });

  it("edits against the revision on screen and reloads after", async () => {
    const { editor, fake } = await started();
    editor.select("clip_0005");
    await editor.deleteSelection();
    const [request] = fake.callsOf("execute")[0] as [EditRequest];
    expect(request).toEqual({ commands: [{ command: "clip.remove", args: { clip: "clip_0005" } }], expected_revision: 4 });
    expect(editor.getState().selection).toBeNull();
    expect(editor.getState().timeline?.revision).toBe(5);
    await editor.undo();
    expect((fake.callsOf("execute")[1] as [EditRequest])[0].commands).toEqual([{ command: "history.undo" }]);
    await editor.redo();
    expect(fake.callsOf("execute"), "nothing to redo").toHaveLength(2);
  });

  it("reloads and says so when someone else changed the project first", async () => {
    const { editor, fake } = await started();
    fake.onExecute = () => {
      throw { code: "fosscut.revision.conflict", message: "the project is at revision 7, not 4" };
    };
    fake.revision = 7;
    editor.select("clip_0003");
    await editor.deleteSelection();
    expect(editor.getState().timeline?.revision).toBe(7);
    expect(editor.getState().notice?.kind).toBe("info");
    expect(editor.getState().selection, "still selected: the clip is still there").toBe("clip_0003");
  });

  it("asks before destructive edits and applies them only with consent", async () => {
    const guarded = (request: EditRequest) => {
      if (!request.allow_destructive) {
        throw { code: "fosscut.guard.destructive", message: "removing clip_0003 removes main footage" };
      }
      return { committed: true, dry_run: false, replayed: false, revision: 5, results: [] };
    };
    const asked: string[] = [];
    const no = await started(engine(), (q) => (asked.push(q), false));
    no.fake.onExecute = guarded;
    no.editor.select("clip_0003");
    await no.editor.deleteSelection();
    expect(no.fake.callsOf("execute")).toHaveLength(1);
    expect(no.editor.getState().notice).toBeNull();
    expect(asked[0]).toContain("main footage");

    const yes = await started(engine(), () => true);
    yes.fake.onExecute = guarded;
    yes.editor.select("clip_0003");
    await yes.editor.deleteSelection();
    const retried = yes.fake.callsOf("execute")[1] as [EditRequest];
    expect(retried[0].allow_destructive).toBe(true);
    expect(yes.editor.getState().selection).toBeNull();
  });

  it("shows other errors with their hint", async () => {
    const { editor, fake } = await started();
    fake.onExecute = () => {
      throw { code: "fosscut.clip.overlap", message: "the clip would overlap", hint: "Choose a free position" };
    };
    editor.select("clip_0003");
    await editor.deleteSelection();
    expect(editor.getState().notice?.text).toBe("the clip would overlap. Choose a free position");
    fake.onExecute = () => {
      throw new Error("pipe broke");
    };
    await editor.undo();
    expect(editor.getState().notice?.text).toContain("pipe broke");
  });

  it("splits the selected clip, or the only one, at the playhead", async () => {
    const { editor, fake } = await started();
    await editor.seek(3 * FLICKS_PER_SECOND);
    await editor.splitAtPlayhead();
    const [first] = fake.callsOf("execute")[0] as [EditRequest];
    expect(first.commands).toEqual([{ command: "clip.split", args: { clip: "clip_0003", at: { flicks: 3 * FLICKS_PER_SECOND } } }]);

    await editor.seek(FLICKS_PER_SECOND);
    await editor.splitAtPlayhead();
    expect(editor.getState().notice?.text).toContain("select the one");
    editor.select("clip_0005");
    await editor.splitAtPlayhead();
    expect((fake.callsOf("execute")[1] as [EditRequest])[0].commands[0]?.args).toMatchObject({ clip: "clip_0005" });

    await editor.seek(4 * FLICKS_PER_SECOND);
    await editor.splitAtPlayhead();
    expect(editor.getState().notice?.text).toContain("not inside the selected clip");
    editor.select(null);
    await editor.seek(0);
    await editor.splitAtPlayhead();
    expect(editor.getState().notice?.text).toContain("No clip");
    expect(fake.callsOf("execute")).toHaveLength(2);
  });

  it("drops a selection whose clip is gone after a reload, and polls clients", async () => {
    const { editor, fake } = await started();
    editor.select("clip_9999");
    await editor.refresh();
    expect(editor.getState().selection).toBeNull();
    fake.connected = [{ id: 1, name: "agent:claude-code", since_ms: 5 }];
    await editor.refreshClients();
    expect(editor.getState().clients.map((c) => c.name)).toEqual(["agent:claude-code"]);
  });

  it("moves, trims and sets properties as single commands", async () => {
    const { editor, fake } = await started();
    await editor.moveClip("clip_0005", 3 * FLICKS_PER_SECOND);
    await editor.moveClip("clip_0003", 0, "track_0009");
    await editor.trimClip("clip_0003", { start: FLICKS_PER_SECOND });
    await editor.trimClip("clip_0003", { end: 2 * FLICKS_PER_SECOND });
    await editor.setProperties("clip_0003", { opacity: 0.5, "transform.scale": 2 });
    await editor.setProperties("clip_0003", {});
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([
      { command: "clip.move", args: { clip: "clip_0005", start: { flicks: 3 * FLICKS_PER_SECOND } } },
      { command: "clip.move", args: { clip: "clip_0003", start: { flicks: 0 }, track: "track_0009" } },
      { command: "clip.trim", args: { clip: "clip_0003", start: { flicks: FLICKS_PER_SECOND } } },
      { command: "clip.trim", args: { clip: "clip_0003", end: { flicks: 2 * FLICKS_PER_SECOND } } },
      { command: "clip.set", args: { clip: "clip_0003", set: { opacity: 0.5, "transform.scale": 2 } } },
    ]);
  });

  it("reads the effect catalog and LUTs, and adds effects, looks and adjustment layers", async () => {
    const fake = engine();
    fake.answers["capabilities"] = {
      effects: [{ type: "color.adjust", version: 1, title: "Adjust", category: "color", summary: "", params: [] }],
      looks: [{ name: "warm", title: "Warm", description: "" }],
    };
    fake.answers["media.list"] = {
      assets: [
        { id: "asset_0001", name: "clip.mp4", kind: "video" },
        { id: "asset_0002", name: "film.cube", kind: "lut" },
      ],
    };
    const { editor } = await started(fake);
    expect(editor.getState().catalog?.looks[0]?.name).toBe("warm");
    expect(editor.getState().luts).toEqual([{ id: "asset_0002", name: "film.cube" }]);
    await editor.addEffect("clip_0003", "color.adjust");
    await editor.applyLook("clip_0003", "warm");
    await editor.seek(FLICKS_PER_SECOND);
    await editor.addAdjustment();
    await editor.autoColor("clip_0003");
    await editor.autoColor("clip_0003", "clip_0009");
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([
      { command: "effect.add", args: { clip: "clip_0003", type: "color.adjust" } },
      { command: "look.apply", args: { clip: "clip_0003", look: "warm" } },
      { command: "adjustment.add", args: { start: { flicks: FLICKS_PER_SECOND }, duration: { seconds: "3" } } },
      { command: "effect.add", args: { clip: "clip_0003", type: "color.adjust", params: { temperature: 0.2 }, index: 0 } },
      { command: "effect.add", args: { clip: "clip_0003", type: "color.adjust", params: { saturation: -0.1 }, index: 0 } },
    ]);
    expect(fake.callsOf("colorSuggest")).toEqual([
      ["clip_0003", undefined],
      ["clip_0003", "clip_0009"],
    ]);
  });

  it("adds shape masks at once, and path and brush masks after drawing them", async () => {
    const { editor, fake } = await started();
    await editor.addMask("clip_0003", "ellipse");
    await editor.addMask("clip_0003", "text");
    await editor.addMask("clip_0003", "path");
    expect(editor.getState().maskTool).toMatchObject({ clip: "clip_0003", mask: "", mode: "pen" });
    await editor.finishMask({ points: [{ at: [0, 0] }] });
    expect(editor.getState().notice?.text).toContain("three points");
    const triangle = [{ at: [-1, -1] as [number, number] }, { at: [1, -1] as [number, number] }, { at: [0, 1] as [number, number] }];
    await editor.finishMask({ points: triangle });
    expect(editor.getState().maskTool, "done drawing").toBeNull();
    await editor.addMask("clip_0003", "brush");
    await editor.finishMask({ stroke: { points: [[0.2, 0.2]], radius: 0.04, erase: false } });
    editor.setMaskTool(null);
    await editor.finishMask({ stroke: { points: [[0.2, 0.2]], radius: 0.04, erase: false } });
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([
      { command: "mask.add", args: { clip: "clip_0003", shape: "ellipse" } },
      { command: "mask.add", args: { clip: "clip_0003", shape: "text", text: "TEXT" } },
      { command: "mask.add", args: { clip: "clip_0003", shape: "path", size: [1, 1], points: triangle } },
      {
        command: "mask.add",
        args: { clip: "clip_0003", shape: "brush", strokes: [{ points: [[0.2, 0.2]], radius: 0.04, erase: false }] },
      },
    ]);
  });

  it("sets and removes transitions and animations", async () => {
    const fake = engine();
    fake.answers["capabilities"] = {
      effects: [],
      looks: [],
      transitions: [{ type: "wipe", title: "Wipe", summary: "", params: [] }],
      animations: [{ preset: "pulse", applies: "combo", meaning: "" }],
    };
    const { editor } = await started(fake);
    expect(editor.getState().catalog?.transitions[0]?.type).toBe("wipe");
    expect(editor.getState().catalog?.animations[0]?.applies).toBe("combo");
    await editor.setTransition("clip_0003", "wipe", 1, { direction: "up" });
    await editor.setTransition("clip_0003", "dissolve");
    await editor.setTransition("clip_0003", "");
    await editor.setAnimation("clip_0003", "in", "fade");
    await editor.setAnimation("clip_0003", "combo", "pulse", 2);
    await editor.setAnimation("clip_0003", "out", "");
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([
      {
        command: "transition.set",
        args: { clip: "clip_0003", type: "wipe", duration: { seconds: "1" }, params: { direction: "up" } },
      },
      { command: "transition.set", args: { clip: "clip_0003", type: "dissolve", duration: { seconds: "0.5" }, params: {} } },
      { command: "transition.remove", args: { clip: "clip_0003" } },
      { command: "animation.set", args: { clip: "clip_0003", in: { preset: "fade" } } },
      { command: "animation.set", args: { clip: "clip_0003", combo: { preset: "pulse", duration: { seconds: "2" } } } },
      { command: "animation.set", args: { clip: "clip_0003", clear: ["out"] } },
    ]);
  });

  it("changes speed with ripple and freezes the frame at the playhead", async () => {
    const fake = engine();
    fake.answers["capabilities"] = {
      effects: [],
      looks: [],
      speed_presets: [{ preset: "hero", meaning: "" }],
    };
    const { editor } = await started(fake);
    expect(editor.getState().catalog?.speed_presets[0]?.preset).toBe("hero");
    await editor.setSpeed("clip_0003", { rate: 2 });
    await editor.setSpeed("clip_0003", { preset: "hero" });
    await editor.seek(FLICKS_PER_SECOND);
    await editor.freeze("clip_0003");
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([
      { command: "clip.speed", args: { clip: "clip_0003", ripple: true, rate: 2 } },
      { command: "clip.speed", args: { clip: "clip_0003", ripple: true, preset: "hero" } },
      {
        command: "clip.freeze",
        args: { clip: "clip_0003", at: { flicks: FLICKS_PER_SECOND }, duration: { seconds: "2" } },
      },
    ]);
  });

  it("copies, cuts and pastes clips, and pastes attributes", async () => {
    const fake = engine();
    fake.answers["clips.copy"] = { clips: [{ track: "track_0002", clip: { id: "clip_0003" } }] };
    const { editor } = await started(fake);
    await editor.paste();
    editor.select("clip_0003");
    await editor.copy();
    expect(editor.getState().clipboard).toHaveLength(1);
    await editor.seek(FLICKS_PER_SECOND);
    await editor.paste();
    editor.copyAttributes();
    editor.select("clip_0005");
    await editor.pasteAttributes();
    await editor.cut();
    expect(fake.callsOf("query").filter(([name]) => name === "clips.copy")).toEqual([
      ["clips.copy", { clips: ["clip_0003"] }],
      ["clips.copy", { clips: ["clip_0005"] }],
    ]);
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands);
    expect(commands).toEqual([
      [
        {
          command: "clips.paste",
          args: { clips: [{ track: "track_0002", clip: { id: "clip_0003" } }], at: { flicks: FLICKS_PER_SECOND } },
        },
      ],
      [{ command: "clip.copy_attributes", args: { from: "clip_0003", to: ["clip_0005"] } }],
      [{ command: "clip.remove", args: { clip: "clip_0005" } }],
    ]);
  });

  it("groups selections, follows the linkage and magnet switches, and edits around the playhead", async () => {
    const { editor, fake } = await started();
    editor.select("clip_0003");
    await editor.group();
    expect(editor.getState().notice?.text).toContain("two or more");
    editor.toggleSelected("clip_0005");
    expect(editor.selected()).toEqual(["clip_0003", "clip_0005"]);
    await editor.group();
    await editor.ungroup();
    editor.toggleSelected("clip_0005");
    expect(editor.selected()).toEqual(["clip_0003"]);
    editor.setLinkage(false);
    editor.setMagnet(true);
    await editor.moveClip("clip_0003", FLICKS_PER_SECOND);
    await editor.deleteSide("left");
    expect(editor.getState().notice?.text).toContain("inside the selected clip");
    await editor.seek(2 * FLICKS_PER_SECOND);
    await editor.deleteSide("left");
    await editor.deleteSide("right");
    await editor.splitAll();
    await editor.toggleExtractedAudio("clip_0003");
    await editor.align("clip_0003", { horizontal: "left" });
    await editor.deleteSelection();
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands);
    const at = { flicks: 2 * FLICKS_PER_SECOND };
    expect(commands).toEqual([
      [{ command: "clip.group", args: { clips: ["clip_0003", "clip_0005"] } }],
      [{ command: "clip.ungroup", args: { clips: ["clip_0003", "clip_0005"] } }],
      [{ command: "clip.move", args: { clip: "clip_0003", start: { flicks: FLICKS_PER_SECOND }, linked: false } }],
      [{ command: "clip.trim", args: { clip: "clip_0003", start: at, ripple: true, linked: false } }],
      [{ command: "clip.trim", args: { clip: "clip_0003", end: at, ripple: true, linked: false } }],
      [{ command: "timeline.split", args: { at } }],
      [{ command: "clip.extract_audio", args: { clip: "clip_0003" } }],
      [{ command: "clip.align", args: { clip: "clip_0003", horizontal: "left" } }],
      [{ command: "clip.remove", args: { clip: "clip_0003", linked: false, ripple: true } }],
    ]);
  });

  it("adds, removes and jumps between markers", async () => {
    const fake = engine();
    fake.answers["timeline.read"] = () => ({
      ...timeline(fake.revision),
      markers: [
        { id: "marker_0010", at: moment(1) },
        { id: "marker_0011", at: moment(3) },
      ],
    });
    const { editor } = await started(fake);
    await editor.seekMarker(1);
    expect(editor.getState().playhead).toBe(FLICKS_PER_SECOND);
    await editor.seekMarker(1);
    expect(editor.getState().playhead).toBe(3 * FLICKS_PER_SECOND);
    await editor.seekMarker(-1);
    expect(editor.getState().playhead).toBe(FLICKS_PER_SECOND);
    await editor.toggleMarker();
    await editor.seek(2 * FLICKS_PER_SECOND);
    await editor.toggleMarker();
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([
      { command: "marker.remove", args: { marker: "marker_0010" } },
      { command: "marker.add", args: { at: { flicks: 2 * FLICKS_PER_SECOND } } },
    ]);
  });

  it("shuttles, skims, cuts with the blade and changes the view", async () => {
    const { editor, fake } = await started();
    const rates: number[] = [];
    for (const press of [1, 1, 1, 1, -1, -1, 0] as const) {
      await editor.shuttle(press);
      rates.push(editor.getState().shuttle);
    }
    expect(rates, "L doubles to 8x, J turns back and doubles, K stops").toEqual([1, 2, 4, 8, -1, -2, 0]);
    expect(fake.callsOf("preview.shuttle").map(([rate]) => rate)).toEqual([1, 2, 4, 8, -1, -2, 0]);
    await editor.shuttle(1);
    fake.emit("playhead", { time: moment(3), playing: false });
    expect(editor.getState().shuttle, "stopping at an end resets the shuttle").toBe(0);
    await editor.skim(2 * FLICKS_PER_SECOND);
    expect(editor.getState().skim, "skimming is off").toBeNull();
    editor.setSkimming(true);
    await editor.skim(2 * FLICKS_PER_SECOND);
    await editor.skim(2 * FLICKS_PER_SECOND);
    await editor.skim(null);
    expect(fake.callsOf("preview.show").slice(-2), "the frame under the pointer, then back").toEqual([
      [2 * FLICKS_PER_SECOND],
      [3 * FLICKS_PER_SECOND],
    ]);
    editor.setTool("blade");
    await editor.bladeAt("clip_0003", FLICKS_PER_SECOND);
    expect(editor.getState().tool).toBe("blade");
    await editor.toggleFullscreen();
    await editor.toggleFullscreen();
    expect(fake.callsOf("fullscreen")).toEqual([[true], [false]]);
    editor.toggleRulers();
    editor.setGuide(0, { axis: "x", at: 0.5 });
    editor.setGuide(1, { axis: "y", at: 0.25 });
    editor.setGuide(0, { axis: "x", at: 0.6 });
    editor.setGuide(1, null);
    expect(editor.getState().guides).toEqual([{ axis: "x", at: 0.6 }]);
    editor.zoom("fit");
    editor.zoom("in");
    expect(editor.getState().zoomRequest).toEqual({ kind: "in", seq: 2 });
    editor.setSnapping(false);
    expect(editor.getState().snapping).toBe(false);
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([{ command: "clip.split", args: { clip: "clip_0003", at: { flicks: FLICKS_PER_SECOND } } }]);
  });

  it("manages the media bin and the project's timelines", async () => {
    const fake = engine();
    fake.answers["media.list"] = {
      assets: [
        { id: "asset_0001", name: "clip.mp4", kind: "video", used_by: ["clip_0003"], folder: "Footage" },
        { id: "asset_0002", name: "gone.mp4", kind: "video", used_by: [], missing: true },
      ],
      folders: ["Footage"],
    };
    const { editor } = await started(fake);
    expect(editor.getState().folders).toEqual(["Footage"]);
    expect(editor.getState().media[1]?.missing).toBe(true);
    await editor.renameAsset("asset_0002", "Found");
    await editor.renameAsset("asset_0002", "  ");
    await editor.removeAsset("asset_0001");
    await editor.removeAsset("asset_0002");
    await editor.moveAssets(["asset_0002"], "Footage/Day 1");
    await editor.moveAssets(["asset_0002"], "");
    await editor.moveAssets([], "Footage");
    await editor.folder("rename", "Footage", "Shots");
    await editor.revealAsset("asset_0001");
    await editor.relinkAsset("asset_0002");
    fake.relinkChooses = false;
    await editor.relinkAsset("asset_0002");
    await editor.seek(FLICKS_PER_SECOND);
    await editor.addToTimeline("asset_0001");
    await editor.addTimeline(["asset_0001", "asset_0002"]);
    await editor.showTimeline("timeline_0000");
    await editor.showTimeline("timeline_0009");
    expect(editor.getState().playhead, "a new timeline starts at its beginning").toBe(0);
    await editor.renameTimeline("timeline_0009", "Shorts");
    await editor.removeTimeline("timeline_0009");
    expect(fake.callsOf("media.reveal")).toEqual([["asset_0001"]]);
    expect(fake.callsOf("media.relink")).toEqual([["asset_0002"], ["asset_0002"]]);
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([
      { command: "media.rename", args: { asset: "asset_0002", name: "Found" } },
      { command: "media.remove", args: { asset: "asset_0001", remove_clips: true } },
      { command: "media.remove", args: { asset: "asset_0002" } },
      { command: "media.move", args: { assets: ["asset_0002"], folder: "Footage/Day 1" } },
      { command: "media.move", args: { assets: ["asset_0002"] } },
      { command: "media.folder", args: { action: "rename", folder: "Footage", to: "Shots" } },
      { command: "clip.add", args: { asset: "asset_0001", start: { flicks: FLICKS_PER_SECOND } } },
      { command: "timeline.add", args: { assets: ["asset_0001", "asset_0002"], primary: true } },
      { command: "timeline.set_primary", args: { timeline: "timeline_0009" } },
      { command: "timeline.rename", args: { timeline: "timeline_0009", name: "Shorts" } },
      { command: "timeline.remove", args: { timeline: "timeline_0009" } },
    ]);
  });

  it("browses projects with thumbnails, and moves them to the trash and back", async () => {
    const fake = new FakeEngine();
    const summary = {
      path: "/work/Old.fosscut",
      name: "Old",
      duration: moment(3),
      timelines: 1,
      assets: 2,
      bytes: 4096,
      modified_ms: 1,
      thumbnail: "data:image/png;base64,AA==",
    };
    fake.summaries = [summary];
    let asked = "";
    const editor = new Editor(fake, (question) => {
      asked = question;
      return true;
    });
    await editor.start();
    expect(editor.getState().projects[0]?.thumbnail).toBe("data:image/png;base64,AA==");
    expect(fake.callsOf("projectSummaries"), "names first, then pictures").toEqual([[false], [true]]);
    await editor.trashProject("/work/Old.fosscut");
    expect(editor.getState().projects).toEqual([]);
    expect(editor.getState().trash.map((t) => t.original)).toEqual(["/work/Old.fosscut"]);
    await editor.restoreProject(editor.getState().trash[0]!.id);
    expect(editor.getState().notice?.text).toContain("/work/Old.fosscut");
    await editor.trashProject("/work/Old.fosscut");
    await editor.deleteTrashed(editor.getState().trash[0]!.id);
    expect(asked).toContain("cannot be undone");
    expect(editor.getState().trash).toEqual([]);
    expect(fake.callsOf("trash.remove")).toHaveLength(1);
  });

  it("makes, places and breaks compound clips", async () => {
    const { editor, fake } = await started();
    await editor.compound();
    editor.select("clip_0003");
    editor.toggleSelected("clip_0005");
    await editor.compound();
    expect(editor.selected(), "the clips are inside it now").toEqual([]);
    await editor.seek(FLICKS_PER_SECOND);
    await editor.addCompound("timeline_0020");
    await editor.decompose("clip_0021");
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([
      { command: "clip.compound", args: { clips: ["clip_0003", "clip_0005"] } },
      { command: "clip.add", args: { compound: "timeline_0020", start: { flicks: FLICKS_PER_SECOND } } },
      { command: "clip.decompose", args: { clip: "clip_0021" } },
    ]);
  });

  it("works without a catalog and shows color errors", async () => {
    const fake = engine();
    fake.colorSuggest = async () => {
      throw { code: "fosscut.request.invalid_arguments", message: "a clip cannot be matched to itself" };
    };
    const { editor } = await started(fake);
    expect(editor.getState().catalog).toBeNull();
    await editor.autoColor("clip_0003", "clip_0003");
    expect(editor.getState().notice?.text).toContain("matched to itself");
    expect(fake.callsOf("execute")).toHaveLength(0);
  });

  it("deactivates the selected clip and activates it again", async () => {
    const fake = engine();
    let enabled = true;
    fake.answers["timeline.read"] = () => {
      const view = timeline(fake.revision);
      if (!enabled) view.tracks[0]!.clips[0]!.enabled = false;
      return view;
    };
    const { editor } = await started(fake);
    await editor.toggleEnabled();
    expect(fake.callsOf("execute"), "nothing selected").toHaveLength(0);
    editor.select("clip_0003");
    await editor.toggleEnabled();
    enabled = false;
    await editor.refresh();
    await editor.toggleEnabled();
    const sets = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]?.args);
    expect(sets).toEqual([
      { clip: "clip_0003", set: { enabled: false } },
      { clip: "clip_0003", set: { enabled: true } },
    ]);
  });

  it("levels the loudness of clips as one step and says what it did", async () => {
    const { editor, fake } = await started();
    await editor.normalizeLoudness([]);
    expect(editor.getState().notice?.text).toContain("Select clips with sound");
    expect(fake.callsOf("audioNormalize"), "nothing to measure").toHaveLength(0);

    await editor.normalizeLoudness(["clip_0003", "clip_0008"]);
    expect(fake.callsOf("audioNormalize")).toEqual([[["clip_0003", "clip_0008"], undefined]]);
    const requests = fake.callsOf("execute").map(([request]) => request as EditRequest);
    expect(requests, "one undoable step").toHaveLength(1);
    expect(requests[0]).toEqual({
      commands: [
        { command: "clip.set", args: { clip: "clip_0003", set: { volume: 1.413 } } },
        { command: "clip.set", args: { clip: "clip_0008", set: { volume: 1.413 } } },
      ],
      expected_revision: 4,
      reason: "Normalize loudness",
    });
    expect(editor.getState().notice).toEqual({ kind: "info", text: "Levelled to -14 LUFS." });
    await editor.normalizeLoudness(["clip_0003"], -16);
    expect(fake.callsOf("audioNormalize")[1]).toEqual([["clip_0003"], -16]);
    expect(editor.getState().notice?.text).toBe("Levelled to -16 LUFS.");
  });

  it("applies nothing when the clips are silent, and shows the engine's refusals", async () => {
    const fake = engine();
    fake.audioNormalize = async (clips) => ({
      target_lufs: -14,
      ceiling_dbfs: -1,
      clips: clips.map((clip) => ({ clip, measured_lufs: null, peak_dbfs: null, gain_db: 0, volume: 1, limited: false })),
      commands: [],
    });
    const { editor } = await started(fake);
    await editor.normalizeLoudness(["clip_0003"]);
    expect(editor.getState().notice?.text).toContain("Nothing to level");
    fake.audioNormalize = async () => {
      throw { code: "fosscut.request.invalid_arguments", message: "clip_0003 has volume keyframes" };
    };
    await editor.normalizeLoudness(["clip_0003"]);
    expect(editor.getState().notice).toMatchObject({ kind: "error", text: "clip_0003 has volume keyframes" });
    expect(fake.callsOf("execute")).toHaveLength(0);
  });

  it("describes clips held back by the ceiling and silent ones left alone", () => {
    const clip = (id: string, measured: number | null, limited = false) => ({
      clip: id,
      measured_lufs: measured,
      peak_dbfs: measured === null ? null : -3,
      gain_db: 0,
      volume: 1,
      limited,
    });
    const result = (clips: ReturnType<typeof clip>[]) => levelled({ target_lufs: -14, ceiling_dbfs: -1, clips, commands: [] });
    expect(result([clip("clip_0001", -20)])).toBe("Levelled to -14 LUFS.");
    expect(result([clip("clip_0001", -20, true), clip("clip_0002", null)])).toBe(
      "Levelled to -14 LUFS; one clip held back by the -1 dBFS peak ceiling; one silent clip left alone.",
    );
    expect(result([clip("clip_0001", -20, true), clip("clip_0002", -30, true), clip("clip_0003", null), clip("clip_0004", null)])).toBe(
      "Levelled to -14 LUFS; 2 clips held back by the -1 dBFS peak ceiling; 2 silent clips left alone.",
    );
  });

  it("edits text clips, saves their style as presets and adds titles at the playhead", async () => {
    const fake = engine();
    fake.answers["project.info"] = () => ({ ...fake.opened?.info, text_presets: ["brand"] });
    fake.answers["capabilities"] = {
      effects: [],
      looks: [],
      text: { templates: [{ name: "lower-third", description: "", lines: ["Alex Morgan", "Founder"] }] },
    };
    const asked: string[] = [];
    let replace = false;
    const { editor } = await started(fake, (question) => {
      asked.push(question);
      return replace;
    });
    expect(editor.getState().catalog?.text.templates[0]?.name).toBe("lower-third");
    expect(editor.getState().catalog?.text.reveal.units, "defaults for what capabilities leaves out").toEqual([
      "letter",
      "word",
      "line",
    ]);
    await editor.setText("clip_0005", null);
    await editor.setText("clip_0005", {});
    await editor.setText("clip_0005", { style: { bold: true } });
    await editor.setText("clip_0005", { preset: "neon" });
    expect(await editor.saveTextPreset("clip_0005", "  "), "a name is needed").toBe(false);
    expect(await editor.saveTextPreset("clip_0005", " lower ")).toBe(true);
    expect(editor.getState().notice?.text).toBe("Saved the style as lower.");
    expect(await editor.saveTextPreset("clip_0005", "brand"), "declined replacing").toBe(false);
    replace = true;
    expect(await editor.saveTextPreset("clip_0005", "brand")).toBe(true);
    expect(asked).toHaveLength(2);
    expect(asked[0]).toContain("Replace it?");

    fake.onExecute = (request) => {
      fake.revision += 1;
      const placed = request.commands[0]?.command.startsWith("text.") ? [{ clip: "clip_0040" }] : [];
      return { committed: true, dry_run: false, replayed: false, revision: fake.revision, results: placed };
    };
    await editor.seek(FLICKS_PER_SECOND);
    await editor.addTitle("lower-third");
    expect(editor.getState().selection, "the new title is selected").toBe("clip_0040");
    await editor.addTitle("");
    const commands = fake.callsOf("execute").map(([request]) => (request as EditRequest).commands[0]);
    expect(commands).toEqual([
      { command: "text.set", args: { clip: "clip_0005", style: { bold: true } } },
      { command: "text.set", args: { clip: "clip_0005", preset: "neon" } },
      { command: "text.preset_save", args: { name: "lower", clip: "clip_0005" } },
      { command: "text.preset_save", args: { name: "brand", clip: "clip_0005", replace: true } },
      { command: "text.template", args: { template: "lower-third", start: { flicks: FLICKS_PER_SECOND } } },
      { command: "text.add", args: { text: "Title", start: { flicks: FLICKS_PER_SECOND } } },
    ]);
  });
});
