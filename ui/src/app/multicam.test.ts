import { describe, expect, it } from "vitest";

import type { AssetView } from "../engine/types";
import { clip, moment, timeline, track } from "../test/fixtures";
import { NEW_MULTICAM, angleNumber, angleTitle, canSync, multicamAngles, multicamPartAt, multicamRequest } from "./multicam";

const ANGLES = [
  { id: "track_0011", name: "Angle 1: cam-a.mp4" },
  { id: "track_0010", name: "Angle 2: cam-b.mp4" },
  { id: "track_0009", name: null },
];

const part = (id: string, start: number, end: number, angle: string, timeline = "timeline_0008") =>
  clip(id, "timeline", start, end, { timeline, timeline_name: "Interview", angle, angles: ANGLES });

const asset = (id: string, extra: Partial<AssetView> = {}): AssetView => ({
  id,
  kind: "video",
  name: `${id}.mp4`,
  duration: moment(10),
  used_by: [],
  audio: { channels: 2 },
  ...extra,
});

describe("multicam", () => {
  it("numbers the angle each part shows as multicam.switch does", () => {
    expect(angleNumber(part("clip_0012", 0, 4, "track_0011"))).toBe(1);
    expect(angleNumber(part("clip_0013", 4, 8, "track_0009"))).toBe(3);
    expect(angleNumber(part("clip_0014", 0, 4, "track_0099")), "not one of its angles").toBeNull();
    expect(angleNumber(clip("clip_0015", "timeline", 0, 4, { timeline: "timeline_0020" })), "a plain compound clip").toBeNull();
    expect(angleNumber(clip("clip_0016", "video", 0, 4))).toBeNull();
    const shown = part("clip_0012", 0, 4, "track_0011");
    expect([angleTitle(shown, 1), angleTitle(shown, 3), angleTitle(shown, 4)]).toEqual([
      "Angle 1: cam-a.mp4",
      "Angle 3",
      "Angle 4",
    ]);
  });

  it("offers the chosen video that is there, in the order chosen", () => {
    const media = [asset("asset_0001"), asset("asset_0002", { missing: true }), asset("asset_0003"), asset("asset_0004", { kind: "audio" })];
    const angles = multicamAngles(media, ["asset_0003", "asset_0004", "asset_0002", "asset_0001", "asset_0099"]);
    expect(angles.map((a) => a.id)).toEqual(["asset_0003", "asset_0001"]);
    expect(canSync(angles)).toBe(true);
    expect(canSync([asset("asset_0001"), asset("asset_0005", { audio: null })]), "one angle is silent").toBe(false);
  });

  it("builds the request from the form: synced, at the playhead, sound and name only when chosen", () => {
    const angles = [asset("asset_0003"), asset("asset_0001")];
    expect(multicamRequest(NEW_MULTICAM, angles, 2_000.4)).toEqual({ angles: ["asset_0003", "asset_0001"], sync: true, start: 2000 });
    expect(multicamRequest({ sync: false, sound: 2, name: "  Interview " }, angles, 0)).toEqual({
      angles: ["asset_0003", "asset_0001"],
      sync: false,
      sound: 2,
      name: "Interview",
      start: 0,
    });
    expect(multicamRequest({ ...NEW_MULTICAM, sound: 7 }, angles, 0).sound, "an angle that exists").toBe(2);
    const silent = [asset("asset_0003"), asset("asset_0005", { audio: null })];
    expect(multicamRequest(NEW_MULTICAM, silent, 0).sync, "without sound on every angle, no sync").toBe(false);
  });

  it("finds the part of the selected multicam clip under the playhead", () => {
    const view = {
      ...timeline(),
      tracks: [
        track("track_0002", "video", 0, [
          part("clip_0012", 0, 4, "track_0011"),
          part("clip_0013", 4, 8, "track_0010"),
          part("clip_0017", 8, 9, "track_0010", "timeline_0018"),
        ]),
        track("track_0004", "text", 1, [clip("clip_0005", "text", 0, 2, { text: "Live edit" })]),
      ],
    };
    const at = (seconds: number) => moment(seconds).flicks;
    expect(multicamPartAt(view, "clip_0012", at(1))?.id).toBe("clip_0012");
    expect(multicamPartAt(view, "clip_0012", at(5))?.id, "the part after an earlier cut").toBe("clip_0013");
    expect(multicamPartAt(view, "clip_0012", at(4))?.id, "a part starts at its start").toBe("clip_0013");
    expect(multicamPartAt(view, "clip_0012", at(8.5)), "another multicam clip").toBeNull();
    expect(multicamPartAt(view, "clip_0005", at(1)), "not a multicam clip").toBeNull();
    expect(multicamPartAt(view, null, at(1))).toBeNull();
  });
});
