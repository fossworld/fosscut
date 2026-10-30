import { describe, expect, it } from "vitest";

import { clip } from "../test/fixtures";
import { change, fieldsFor } from "./fields";

describe("inspector", () => {
  it("shows picture fields for pictures and volume for sound", () => {
    const video = clip("c1", "video", 0, 2, { transform: { position: [0.25, -0.1], scale: 1.5 }, opacity: 0.8 });
    expect(fieldsFor(video).map((f) => [f.id, f.value])).toEqual([
      ["x", 25],
      ["y", -10],
      ["scale", 150],
      ["rotation", 0],
      ["opacity", 80],
      ["volume", 100],
    ]);
    expect(fieldsFor(clip("a", "audio", 0, 2)).map((f) => f.id)).toEqual(["volume"]);
    expect(fieldsFor(clip("t", "text", 0, 2)).map((f) => f.id)).not.toContain("volume");
  });

  it("turns edits into clip.set changes in the engine's units", () => {
    const video = clip("c1", "video", 0, 2, { transform: { position: [0.25, -0.1] } });
    expect(change(video, "x", 40)).toEqual({ "transform.position": [0.4, -0.1] });
    expect(change(video, "y", 5)).toEqual({ "transform.position": [0.25, 0.05] });
    expect(change(video, "scale", 200)).toEqual({ "transform.scale": 2 });
    expect(change(video, "rotation", -15)).toEqual({ "transform.rotation": -15 });
    expect(change(video, "opacity", 50)).toEqual({ opacity: 0.5 });
    expect(change(video, "volume", 0)).toEqual({ volume: 0 });
  });

  it("ignores values that are unchanged, out of range or not numbers", () => {
    const video = clip("c1", "video", 0, 2);
    expect(change(video, "x", 0)).toBeNull();
    expect(change(video, "scale", 0)).toBeNull();
    expect(change(video, "opacity", 120)).toBeNull();
    expect(change(video, "volume", -1)).toBeNull();
    expect(change(video, "rotation", Number.NaN)).toBeNull();
    expect(change(clip("a", "audio", 0, 2), "x", 10), "audio has no position").toBeNull();
  });
});
