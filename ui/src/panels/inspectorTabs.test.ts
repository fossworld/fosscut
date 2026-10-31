import { describe, expect, it } from "vitest";

import { clip } from "../test/fixtures";
import { type Field, fieldsFor } from "./fields";
import { basicTabName, fieldsIn, inScope, inspectorTabs, shownTab, sliderRange } from "./inspectorTabs";

const media = [
  { id: "asset_0001", name: "a.mp4", kind: "video" as const, used_by: [], audio: { channels: 2 } },
  { id: "asset_0002", name: "silent.mp4", kind: "video" as const, used_by: [], audio: null },
];

describe("inspector tabs", () => {
  it("offers each kind of clip the tabs it has controls for", () => {
    expect(inspectorTabs(clip("c1", "video", 0, 2, { asset: "asset_0001" }), media)).toEqual([
      "basic",
      "audio",
      "speed",
      "animation",
      "adjust",
      "masks",
      "effects",
      "subject",
      "tools",
      "shorts",
    ]);
    expect(inspectorTabs(clip("c2", "video", 0, 2, { asset: "asset_0002" }), media), "a silent video has no sound tab").not.toContain("audio");
    expect(inspectorTabs(clip("c3", "audio", 0, 2), media)).toEqual(["audio", "speed"]);
    expect(inspectorTabs(clip("c4", "text", 0, 2, { text: "Hi" }), media).slice(0, 3)).toEqual(["text", "style", "animation"]);
    expect(inspectorTabs(clip("c5", "caption", 0, 2, { text: "Hi" }), media)).toEqual(["captions", "style", "translate"]);
    expect(inspectorTabs(clip("c6", "adjustment", 0, 2), media)).toEqual(["adjust", "effects"]);
    expect(inspectorTabs(clip("c7", "shape", 0, 2), media)[0]).toBe("shape");
  });

  it("keeps the tab asked for when the clip has it, else shows its first", () => {
    expect(shownTab(["basic", "audio"], "audio")).toBe("audio");
    expect(shownTab(["audio", "speed"], "masks")).toBe("audio");
    expect(shownTab(["basic"], null)).toBe("basic");
  });

  it("names the first tab by what it places", () => {
    expect(basicTabName(clip("c1", "video", 0, 2))).toBe("video");
    expect(basicTabName(clip("c2", "image", 0, 2))).toBe("image");
    expect(basicTabName(clip("c3", "timeline", 0, 2, { timeline: "timeline_0001" }))).toBe("compound");
    expect(basicTabName(clip("c4", "timeline", 0, 2, { angles: [{ id: "t1", name: "A" }, { id: "t2", name: "B" }], angle: "t1" }))).toBe("multicam");
    expect(basicTabName(clip("c5", "shape", 0, 2))).toBe("placement");
  });

  it("groups a video clip's fields into placement, opacity, crop and sound", () => {
    const fields = fieldsFor(clip("c1", "video", 0, 2), { playhead: 0, separate: false });
    expect(fieldsIn(fields, "placement").map((f) => f.id)).toEqual(["x", "y", "scale", "rotation"]);
    expect(fieldsIn(fields, "opacity").map((f) => f.id)).toEqual(["opacity"]);
    expect(fieldsIn(fields, "crop")).toHaveLength(4);
    expect(fieldsIn(fields, "sound").map((f) => f.id)).toEqual(["volume", "pan", "fade_in", "fade_out"]);
  });

  it("gives sliders the field's own limits or a range people use", () => {
    const field = (id: Field["id"], extra: Partial<Field> = {}): Field => ({
      id,
      label: id,
      path: id,
      value: 0,
      unit: "%",
      step: 1,
      animatable: true,
      animated: false,
      keyed: false,
      ...extra,
    });
    expect(sliderRange(field("opacity", { min: 0, max: 100 }))).toEqual([0, 100]);
    expect(sliderRange(field("rotation"))).toEqual([-180, 180]);
    expect(sliderRange(field("scale", { min: 1 }))).toEqual([1, 400]);
    expect(sliderRange(field("volume", { min: 0, max: 400 })), "volume's slider stops at twice as loud").toEqual([0, 200]);
  });

  it("splits effects into colour, other pictures and sound", () => {
    expect(inScope({ category: "color" }, "color")).toBe(true);
    expect(inScope({ category: "color" }, "picture")).toBe(false);
    expect(inScope({ category: "stylize" }, "picture")).toBe(true);
    expect(inScope({ category: "audio" }, "sound")).toBe(true);
    expect(inScope({ category: "audio" }, "picture")).toBe(false);
    expect(inScope({ category: "audio" }, "all")).toBe(true);
  });
});
