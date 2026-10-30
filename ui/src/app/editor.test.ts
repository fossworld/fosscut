import { describe, expect, it } from "vitest";

import { FakeEngine } from "../engine/fake";
import type { EditRequest, Opened } from "../engine/types";
import { entry, moment, timeline } from "../test/fixtures";
import { FLICKS_PER_SECOND, framesToFlicks, parseRate } from "../transport/time";
import { Editor } from "./editor";

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
});
