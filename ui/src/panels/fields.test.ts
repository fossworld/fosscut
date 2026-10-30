import { describe, expect, it } from "vitest";

import type { KeyframeView } from "../engine/types";
import { clip, moment } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import { change, fieldsFor, keyedPaths, toggleKey } from "./fields";

const at = (seconds: number) => ({ playhead: seconds * FLICKS_PER_SECOND, separate: false });

function key(seconds: number, value: number | [number, number], interpolation: KeyframeView["interpolation"] = "linear"): KeyframeView {
  return { at: moment(seconds), offset: moment(seconds), value, interpolation };
}

describe("inspector", () => {
  it("shows picture fields for pictures and sound fields for sound", () => {
    const video = clip("c1", "video", 0, 2, {
      transform: { position: [0.25, -0.1], scale: 1.5 },
      opacity: 0.8,
      crop: { left: 0.1 },
      audio: { fade_in: FLICKS_PER_SECOND / 2, pan: -0.5 },
    });
    expect(fieldsFor(video, at(0)).map((f) => [f.id, f.value])).toEqual([
      ["x", 25],
      ["y", -10],
      ["scale", 150],
      ["rotation", 0],
      ["opacity", 80],
      ["crop_left", 10],
      ["crop_right", 0],
      ["crop_top", 0],
      ["crop_bottom", 0],
      ["volume", 100],
      ["pan", -50],
      ["fade_in", 0.5],
      ["fade_out", 0],
    ]);
    expect(fieldsFor(clip("a", "audio", 0, 2), at(0)).map((f) => f.id)).toEqual(["volume", "pan", "fade_in", "fade_out"]);
    const title = fieldsFor(clip("t", "text", 0, 2), at(0)).map((f) => f.id);
    expect(title).not.toContain("volume");
    expect(title, "titles have no crop").not.toContain("crop_left");
  });

  it("shows width and height scale for stretched pictures or when asked", () => {
    const wide = clip("c1", "video", 0, 2, { transform: { scale: 2, stretch: [1.5, 1] } });
    expect(fieldsFor(wide, at(0)).filter((f) => f.id.startsWith("scale")).map((f) => [f.id, f.value])).toEqual([
      ["scale_x", 300],
      ["scale_y", 200],
    ]);
    expect(change(wide, "scale_y", 100, at(0))).toEqual({
      command: "clip.set",
      args: { clip: "c1", set: { "transform.stretch": [1.5, 0.5] } },
    });
    const plain = clip("c2", "video", 0, 2);
    const separate = { ...at(0), separate: true };
    expect(fieldsFor(plain, separate).map((f) => f.id)).toContain("scale_x");
    expect(change(plain, "scale_x", 120, separate)?.args).toEqual({ clip: "c2", set: { "transform.stretch": [1.2, 1] } });
  });

  it("turns edits into clip.set changes in the engine's units", () => {
    const video = clip("c1", "video", 0, 2, { transform: { position: [0.25, -0.1] } });
    const set = (id: Parameters<typeof change>[1], value: number) => change(video, id, value, at(0))?.args;
    expect(set("x", 40)).toEqual({ clip: "c1", set: { "transform.position": [0.4, -0.1] } });
    expect(set("y", 5)).toEqual({ clip: "c1", set: { "transform.position": [0.25, 0.05] } });
    expect(set("scale", 200)).toEqual({ clip: "c1", set: { "transform.scale": 2 } });
    expect(set("rotation", -15)).toEqual({ clip: "c1", set: { "transform.rotation": -15 } });
    expect(set("opacity", 50)).toEqual({ clip: "c1", set: { opacity: 0.5 } });
    expect(set("crop_bottom", 25)).toEqual({ clip: "c1", set: { "crop.bottom": 0.25 } });
    expect(set("volume", 0)).toEqual({ clip: "c1", set: { volume: 0 } });
    expect(set("pan", 100)).toEqual({ clip: "c1", set: { "audio.pan": 1 } });
    expect(set("fade_out", 1.5)).toEqual({ clip: "c1", set: { "audio.fade_out": { flicks: 1.5 * FLICKS_PER_SECOND } } });
  });

  it("ignores values that are unchanged, out of range or not numbers", () => {
    const video = clip("c1", "video", 0, 2);
    expect(change(video, "x", 0, at(0))).toBeNull();
    expect(change(video, "scale", 0, at(0))).toBeNull();
    expect(change(video, "opacity", 120, at(0))).toBeNull();
    expect(change(video, "crop_left", 100, at(0))).toBeNull();
    expect(change(video, "volume", -1, at(0))).toBeNull();
    expect(change(video, "fade_in", -1, at(0))).toBeNull();
    expect(change(video, "rotation", Number.NaN, at(0))).toBeNull();
    expect(change(clip("a", "audio", 0, 2), "x", 10, at(0)), "audio has no position").toBeNull();
  });

  it("shows animated values at the playhead and edits them by keyframe", () => {
    const video = clip("c1", "video", 0, 4, {
      opacity: 1,
      keyframes: { opacity: [key(0, 0), key(2, 1)], "transform.position": [key(1, [0.5, 0])] },
    });
    const fields = fieldsFor(video, at(1));
    const opacity = fields.find((f) => f.id === "opacity")!;
    expect(opacity.value, "half way between the keyframes").toBe(50);
    expect([opacity.animated, opacity.keyed]).toEqual([true, false]);
    const x = fields.find((f) => f.id === "x")!;
    expect([x.value, x.keyed]).toEqual([50, true]);
    expect(fields.find((f) => f.id === "rotation")!.animated).toBe(false);
    expect(fields.find((f) => f.id === "fade_in")!.animatable).toBe(false);
    expect(change(video, "opacity", 75, at(1))).toEqual({
      command: "keyframe.set",
      args: { clip: "c1", property: "opacity", at: { flicks: FLICKS_PER_SECOND }, value: 0.75 },
    });
    expect(keyedPaths(video, FLICKS_PER_SECOND)).toEqual(["transform.position"]);
    expect(toggleKey(video, "transform.position", FLICKS_PER_SECOND).command).toBe("keyframe.remove");
    expect(toggleKey(video, "opacity", FLICKS_PER_SECOND)).toEqual({
      command: "keyframe.set",
      args: { clip: "c1", property: "opacity", at: { flicks: FLICKS_PER_SECOND } },
    });
  });
});
