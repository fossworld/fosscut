import { afterEach, describe, expect, it } from "vitest";

import type { MaskView } from "../engine/types";
import { setLanguage, t } from "../i18n";
import { clip, moment } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import { maskChange, maskFields, maskValues, toggleMaskKey } from "./maskFields";

describe("mask fields", () => {
  afterEach(() => setLanguage("en"));
  const video = clip("clip_0003", "video", 0, 4);
  const box: MaskView = { id: "mask_0007", center: [0.3, 0.5], roundness: 0.2 };

  it("show a mask's values with defaults, and only the fields its shape has", () => {
    expect(maskFields(video, box, 0).map((f) => [f.id, f.value])).toEqual([
      ["x", 30],
      ["y", 50],
      ["width", 50],
      ["height", 50],
      ["rotation", 0],
      ["feather", 0],
      ["expansion", 0],
      ["roundness", 20],
    ]);
    expect(maskFields(video, { id: "m", shape: "ellipse" }, 0).map((f) => f.id)).not.toContain("roundness");
    expect(maskFields(video, { id: "m", shape: "brush" }, 0).map((f) => f.id)).toEqual(["feather", "expansion"]);
  });

  it("are labelled in the interface's language", () => {
    expect(maskFields(video, box, 0).map((f) => f.label)).toEqual(
      ["x", "y", "width", "height", "rotation", "feather", "expansion", "roundness"].map((id) =>
        t(`masks.field.${id as "x"}`),
      ),
    );
    expect(maskFields(video, box, 0)[7]?.label).toBe("Corners");
    setLanguage("hu");
    expect(maskFields(video, box, 0).map((f) => f.label)).toEqual([
      "X középpont",
      "Y középpont",
      "Szélesség",
      "Magasság",
      "Elforgatás",
      "Lágyítás",
      "Kiterjesztés",
      "Sarkok",
    ]);
    setLanguage("en");
  });

  it("turn edits into mask.set, or keyframes when animated", () => {
    expect(maskChange(video, box, "x", 40, 0)).toEqual({
      command: "mask.set",
      args: { clip: "clip_0003", mask: "mask_0007", center: [0.4, 0.5] },
    });
    expect(maskChange(video, box, "height", 80, 0)?.args).toEqual({ clip: "clip_0003", mask: "mask_0007", size: [0.5, 0.8] });
    expect(maskChange(video, box, "feather", 50, 0)?.args).toEqual({ clip: "clip_0003", mask: "mask_0007", feather: 0.5 });
    expect(maskChange(video, box, "feather", 150, 0), "out of range").toBeNull();
    expect(maskChange(video, box, "x", 30, 0), "unchanged").toBeNull();
    const moving: MaskView = {
      ...box,
      keyframes: {
        center: [
          { at: moment(0), offset: moment(0), value: [0, 0.5], interpolation: "linear" },
          { at: moment(2), offset: moment(2), value: [1, 0.5], interpolation: "linear" },
        ],
      },
    };
    expect(maskValues(video, moving, FLICKS_PER_SECOND).center).toEqual([0.5, 0.5]);
    expect(maskChange(video, moving, "x", 60, FLICKS_PER_SECOND)).toEqual({
      command: "keyframe.set",
      args: { clip: "clip_0003", property: "masks.mask_0007.center", at: { flicks: FLICKS_PER_SECOND }, value: [0.6, 0.5] },
    });
    expect(toggleMaskKey(video, moving, "center", 0).command).toBe("keyframe.remove");
    expect(toggleMaskKey(video, moving, "feather", 0).command).toBe("keyframe.set");
  });
});
