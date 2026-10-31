import { afterEach, describe, expect, it } from "vitest";

import { setLanguage, t } from "../i18n";
import { clip, moment, timeline, track } from "../test/fixtures";
import { parseRate } from "../transport/time";
import { multicamMade, splitAtScenes, stabilized, targetOf, trackTargets, tracked } from "./measure";

const fps30 = parseRate("30");
const response = { committed: true, dry_run: false, replayed: false, revision: 5, results: [] };

describe("measured edits", () => {
  it("offers the clip's masks and the clips with a place that show while it does", () => {
    const shot = clip("clip_0003", "video", 2, 6, {
      masks: [{ id: "mask_0010", shape: "ellipse" }, { id: "mask_0011" }],
    });
    const view = {
      ...timeline(),
      tracks: [
        track("track_0002", "video", 0, [shot, clip("clip_0006", "video", 6, 9, { asset_name: "after.mp4" })]),
        track("track_0004", "text", 1, [clip("clip_0005", "text", 0, 3, { text: "Name" }), clip("clip_0007", "text", 7, 8, { text: "Later" })]),
        track("track_0008", "video", 2, [clip("clip_0009", "shape", 5, 7, { shape: "star" })]),
        track("track_0010", "audio", 3, [clip("clip_0012", "audio", 0, 9, { asset_name: "music.m4a" })]),
        track("track_0013", "caption", 4, [clip("clip_0014", "caption", 2, 3, { text: "Hi" })]),
      ],
    };
    expect(trackTargets(shot, view)).toEqual([
      { value: "mask:mask_0010", label: t("motion.target.mask", { number: 1, shape: "ellipse" }) },
      { value: "mask:mask_0011", label: t("motion.target.mask", { number: 2, shape: "rectangle" }) },
      { value: "clip:clip_0005", label: t("motion.target.clip", { kind: t("motion.target.text"), name: "Name" }) },
      { value: "clip:clip_0009", label: t("motion.target.clip", { kind: t("motion.target.shape"), name: "star" }) },
    ]);
    expect(trackTargets(shot, view)[2]?.label).toBe("Title: Name");
    expect(targetOf("mask:mask_0010")).toEqual({ mask: "mask_0010" });
    expect(targetOf("clip:clip_0005")).toEqual({ follow: "clip_0005" });
    expect([targetOf(""), targetOf("clip:"), targetOf("track:track_0002")]).toEqual([null, null, null]);
  });

  it("says what stabilizing and splitting did", () => {
    expect(stabilized({ motion: { clip: "clip_0003", frames: 90, zoom: 1.0834 }, response })).toBe(
      "Stabilized over 90 frames, zoomed in 1.08x to hide the moving edges.",
    );
    expect(stabilized({ motion: { clip: "clip_0003", frames: 12, zoom: 1 }, response })).toBe(
      t("motion.stabilized.still", { count: 12 }),
    );
    expect(stabilized({ motion: { clip: "clip_0003", frames: 1, zoom: 1 }, response })).toBe(
      "Stabilized over 1 frame; no zoom was needed.",
    );
    const split = (seconds: number[]) => ({ scenes: { clip: "clip_0003", cuts: seconds.map((s) => moment(s).flicks) }, response });
    expect(splitAtScenes({ ...split([]), response: null }, fps30)).toBe(t("motion.scenes.found.none"));
    expect(splitAtScenes(split([2, 4.5]), fps30)).toBe("Split into 3 shots at 00:00:02:00 and 00:00:04:15.");
    expect(splitAtScenes(split([1, 2, 3, 4, 5, 6, 7]), fps30)).toBe(
      "Split into 8 shots at 00:00:01:00, 00:00:02:00, 00:00:03:00, 00:00:04:00, 00:00:05:00 and 2 more.",
    );
  });

  it("reports where tracking lost the region", () => {
    const motion = { clip: "clip_0005", frames: 60 };
    expect(tracked({ motion, response }, fps30)).toBe("Tracked over 60 frames.");
    const lost = [
      { start: moment(1).flicks, end: moment(1.5).flicks },
      { start: moment(3).flicks, end: moment(4).flicks },
    ];
    expect(tracked({ motion: { ...motion, lost }, response }, fps30)).toBe(
      "Tracked over 60 frames. Lost the region from 00:00:01:00 to 00:00:01:15 and 00:00:03:00 to 00:00:04:00; it holds its last place there.",
    );
  });

  it("says how the angles line up and which matches are doubtful", () => {
    expect(multicamMade({ sync: null, response }, 2)).toBe("Made a multicam clip of 2 angles, all starting together.");
    const second = 705_600_000;
    expect(multicamMade({ sync: { offsets: [0, 1.5 * second, -0.25 * second], confidence: [1, 0.8, 0.9] }, response }, 3)).toBe(
      "Made a multicam clip of 3 angles, lined up by their sound: angle 2 starts 1.50 s after angle 1 and angle 3 starts 0.25 s before angle 1.",
    );
    expect(multicamMade({ sync: { offsets: [0, 0], confidence: [1, 0.12] }, response }, 2)).toBe(
      "Made a multicam clip of 2 angles, lined up by their sound: angle 2 starts with angle 1. The match for angle 2 is doubtful; check it by eye.",
    );
  });

  describe("in Hungarian", () => {
    afterEach(() => setLanguage("en"));

    it("reports in Hungarian, with Hungarian lists and decimal commas", () => {
      setLanguage("hu");
      expect(stabilized({ motion: { clip: "clip_0003", frames: 90, zoom: 1.0834 }, response })).toBe(
        "Stabilizálva 90 képkockán át, 1,08x nagyítással a mozgó szélek elrejtéséhez.",
      );
      const cuts = [2, 4.5].map((s) => moment(s).flicks);
      expect(splitAtScenes({ scenes: { clip: "clip_0003", cuts }, response }, fps30)).toBe(
        "3 snittre vágva ezeknél: 00:00:02:00 és 00:00:04:15.",
      );
      const second = 705_600_000;
      expect(multicamMade({ sync: { offsets: [0, 1.5 * second], confidence: [1, 0.12] }, response }, 2)).toBe(
        "Többkamerás klip készült 2 kameranézetből, a hangjuk alapján igazítva: 2. kameranézet 1,50 mp késéssel indul az elsőhöz képest. Bizonytalan az egyezés: 2. kameranézet; ellenőrizd szemmel.",
      );
      const shot = clip("clip_0003", "video", 0, 6, { masks: [{ id: "mask_0010", shape: "ellipse" }] });
      expect(trackTargets(shot, { ...timeline(), tracks: [] })).toEqual([{ value: "mask:mask_0010", label: "1. maszk: ellipse" }]);
    });
  });
});
