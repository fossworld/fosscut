import { describe, expect, it } from "vitest";

import { FakeEngine } from "../engine/fake";
import { t } from "../i18n";
import type { EditRequest, TimelineView } from "../engine/types";
import { clip, entry, moment, timeline, track } from "../test/fixtures";
import { Editor } from "./editor";

const ANGLES = [
  { id: "track_0011", name: "Angle 1: cam-a.mp4" },
  { id: "track_0010", name: "Angle 2: cam-b.mp4" },
];

/** The fixture timeline with a masked video clip, and a multicam clip from 6 s to 12 s. */
function view(revision: number): TimelineView {
  const base = timeline(revision);
  return {
    ...base,
    duration: moment(12),
    tracks: [
      track("track_0002", "video", 0, [
        clip("clip_0003", "video", 0, 6, { asset_name: "clip.mp4", picture: [1920, 1080], masks: [{ id: "mask_0020", shape: "ellipse" }] }),
        clip("clip_0012", "timeline", 6, 12, { timeline: "timeline_0008", timeline_name: "Interview", angle: "track_0011", angles: ANGLES }),
      ]),
      ...base.tracks.slice(1),
    ],
  };
}

function engine(): FakeEngine {
  const fake = new FakeEngine();
  fake.revision = 4;
  fake.opened = {
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
  fake.answers["project.info"] = () => fake.opened?.info;
  fake.answers["timeline.read"] = () => view(fake.revision);
  fake.answers["history.list"] = { entries: [entry(4, "human", ["clip.add"])] };
  return fake;
}

async function started(fake = engine()) {
  const editor = new Editor(fake);
  await editor.start();
  return { editor, fake };
}

describe("measured edits", () => {
  it("stabilizes at the strength chosen against the revision on screen, busy until done", async () => {
    const { editor, fake } = await started();
    let release: () => void = () => undefined;
    const original = fake.measure.stabilize;
    fake.measure.stabilize = (...args) => new Promise((resolve) => (release = () => void original(...args).then(resolve)));
    const running = editor.stabilize("clip_0003", 0.75);
    expect(editor.getState().measurement).toEqual({ kind: "stabilize", clip: "clip_0003", busy: true, report: null, error: null });
    await editor.splitAtScenes("clip_0003");
    expect(editor.getState().notice?.text, "one measurement at a time").toContain("Another measurement is running");
    expect(fake.callsOf("measure.scenes")).toHaveLength(0);
    release();
    await running;
    expect(fake.callsOf("measure.stabilize")).toEqual([["clip_0003", 0.75, 4]]);
    expect(editor.getState().measurement).toEqual({
      kind: "stabilize",
      clip: "clip_0003",
      busy: false,
      report: t("motion.stabilized.zoom", { count: 90, zoom: "1.09" }),
      error: null,
    });
    expect(editor.getState().timeline?.revision, "read again after the engine applied it").toBe(5);
    editor.dismissMeasurement();
    expect(editor.getState().measurement).toBeNull();
  });

  it("splits at scenes and says where, or that there were none", async () => {
    const { editor, fake } = await started();
    await editor.splitAtScenes("clip_0003");
    expect(editor.getState().measurement?.report).toBe(t("motion.scenes.found.none"));
    fake.sceneCuts = [moment(2).flicks, moment(4).flicks];
    await editor.splitAtScenes("clip_0003");
    expect(fake.callsOf("measure.scenes")).toEqual([
      ["clip_0003", 4],
      ["clip_0003", 4],
    ]);
    expect(editor.getState().measurement?.report).toBe(
      t("motion.scenes.found", { count: 3, cuts: `00:00:02:00 ${t("motion.and")} 00:00:04:00` }),
    );
  });

  it("shows refusals with their hint, and reloads on a newer revision", async () => {
    const { editor, fake } = await started();
    fake.measureRefusal = {
      code: "fosscut.request.invalid_arguments",
      message: "clip_0005 is a text clip; stabilization works on video clips",
      hint: "Select a video clip.",
    };
    await editor.stabilize("clip_0005");
    expect(editor.getState().measurement).toMatchObject({ kind: "stabilize", busy: false, report: null, error: fake.measureRefusal });
    fake.measureRefusal = { code: "fosscut.revision.conflict", message: "the project is at revision 6, not 4" };
    await editor.stabilize("clip_0003");
    expect(editor.getState().measurement).toBeNull();
    expect(editor.getState().notice?.text).toContain("The project changed while you were editing");
  });

  it("tracks a region drawn at the playhead with a mask or another clip, and reports where it was lost", async () => {
    const { editor, fake } = await started();
    await editor.seek(moment(8).flicks);
    editor.select("clip_0003");
    await editor.pickRegion("clip_0003");
    expect(editor.getState().playhead, "the playhead moves into the clip").toBe(0);
    expect(editor.getState().regionPick).toEqual({ clip: "clip_0003", drawing: true, region: null, at: 0 });
    await editor.track("mask:mask_0020");
    expect(editor.getState().notice?.text, "nothing drawn yet").toContain("Draw the region");
    await editor.seek(moment(1).flicks);
    const region = { x: 0.4, y: 0.5, width: 0.2, height: 0.3 };
    editor.setRegion(region);
    expect(editor.getState().regionPick).toEqual({ clip: "clip_0003", drawing: false, region, at: moment(1).flicks });
    fake.trackLost = [{ start: moment(2).flicks, end: moment(3).flicks }];
    await editor.track("mask:mask_0020");
    expect(fake.callsOf("measure.track")).toEqual([
      [{ clip: "clip_0003", region, at: moment(1).flicks, mask: "mask_0020", expected_revision: 4 }],
    ]);
    expect(editor.getState().measurement?.report).toBe(
      t("motion.tracked.lost", {
        done: t("motion.tracked", { count: 60 }),
        ranges: t("motion.tracked.range", { from: "00:00:02:00", to: "00:00:03:00" }),
      }),
    );
    expect(editor.getState().regionPick, "done with the region").toBeNull();

    await editor.pickRegion("clip_0003");
    editor.setRegion(region);
    await editor.track("clip:clip_0005");
    expect(fake.callsOf("measure.track")[1]?.[0]).toMatchObject({ follow: "clip_0005", expected_revision: 5 });
    await editor.pickRegion("clip_0003");
    editor.select("clip_0005");
    expect(editor.getState().regionPick, "selecting another clip forgets the region").toBeNull();
  });

  it("makes a multicam clip against the revision on screen and selects it", async () => {
    const { editor, fake } = await started();
    const made = await editor.createMulticam({ angles: ["asset_0001", "asset_0002"], sync: true, start: moment(2).flicks });
    expect(made).toBe(true);
    expect(fake.callsOf("measure.multicam")).toEqual([
      [{ angles: ["asset_0001", "asset_0002"], sync: true, start: moment(2).flicks, expected_revision: 4 }],
    ]);
    expect(editor.getState().selection).toBe("clip_0057");
    expect(editor.getState().measurement?.report).toBe(
      t("motion.multicam.lined", {
        made: t("motion.multicam.made", { count: 2 }),
        offsets: t("motion.multicam.after", { number: 2, amount: t("motion.multicam.seconds", { seconds: "0.50" }) }),
      }),
    );
    expect(await editor.createMulticam({ angles: ["asset_0001"], sync: false })).toBe(false);
    expect(editor.getState().measurement?.error).toMatchObject({ code: "fosscut.multicam.invalid", hint: expect.stringContaining("two or more") });
  });

  it("cuts the selected multicam clip to an angle at the playhead, and follows the part after the cut", async () => {
    const fake = engine();
    fake.onExecute = () => {
      fake.revision += 1;
      return { committed: true, dry_run: false, replayed: false, revision: fake.revision, results: [{ clip: { clip: "clip_0013" }, angle: "track_0010" }] };
    };
    const { editor } = await started(fake);
    await editor.switchAngle(2);
    expect(editor.getState().notice?.text, "no multicam clip selected").toContain("Select a multicam clip");
    editor.select("clip_0012");
    await editor.seek(moment(8).flicks);
    await editor.switchAngle(1);
    expect(fake.callsOf("execute"), "angle 1 shows there already").toHaveLength(0);
    await editor.switchAngle(3);
    expect(editor.getState().notice?.text).toBe("This multicam clip has 2 angles.");
    await editor.switchAngle(2);
    const requests = fake.callsOf("execute").map(([request]) => request as EditRequest);
    expect(requests).toEqual([
      {
        commands: [{ command: "multicam.switch", args: { clip: "clip_0012", at: { flicks: moment(8).flicks }, angle: 2 } }],
        expected_revision: 4,
        reason: "Cut to angle 2",
      },
    ]);
    expect(editor.getState().selection, "the part after the cut").toBe("clip_0013");
  });
});
