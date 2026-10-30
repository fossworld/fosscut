import { describe, expect, it } from "vitest";

import type { KeyframeView } from "../engine/types";
import { clip, moment } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import { EASINGS, easingName, evaluate, keyAt, keyTimes, numberAt, pairAt, progress, sampleCurve } from "./animation";

function key(seconds: number, value: number | [number, number], interpolation: KeyframeView["interpolation"] = "linear"): KeyframeView {
  return { at: moment(seconds), offset: moment(seconds), value, interpolation };
}

const s = (seconds: number) => seconds * FLICKS_PER_SECOND;

describe("keyframes", () => {
  it("hold outside the keys and interpolate between them, as the engine does", () => {
    const keys = [key(1, 0), key(3, 1, "hold"), key(5, 0)];
    expect(evaluate([], 0)).toBeUndefined();
    expect(evaluate(keys, s(0))).toBe(0);
    expect(evaluate(keys, s(2))).toBe(0.5);
    expect(evaluate(keys, s(4.9)), "hold keeps the value").toBe(1);
    expect(evaluate(keys, s(5))).toBe(0);
    expect(evaluate(keys, s(9))).toBe(0);
    expect(evaluate([key(0, [0, 1]), key(2, [1, -1])], s(1))).toEqual([0.5, 0]);
  });

  it("ease along CSS Bezier curves", () => {
    expect(progress("hold", 0.7)).toBe(0);
    expect(progress("linear", 1.5)).toBe(1);
    expect(progress(EASINGS.ease_in, 0.5)).toBeLessThan(0.5);
    expect(progress(EASINGS.ease_out, 0.5)).toBeGreaterThan(0.5);
    expect(progress(EASINGS.ease_in_out, 0.5)).toBeCloseTo(0.5, 6);
    // The value fosscut-model's test checks for the same curve.
    expect(progress(EASINGS.ease_in, 0.25)).toBeCloseTo(0.0934, 2);
    expect(progress({ bezier: [0.3, 1.6, 0.6, 1] }, 0.6), "overshoot").toBeGreaterThan(1);
    expect(easingName({ bezier: [0.42, 0, 0.58, 1] })).toBe("ease_in_out");
    expect(easingName({ bezier: [0.1, 0.2, 0.3, 0.4] })).toBe("custom");
    expect(easingName("hold")).toBe("hold");
  });

  it("read clip properties at the playhead", () => {
    const video = clip("c1", "video", 2, 6, {
      keyframes: { opacity: [key(2, 0), key(4, 1)], "transform.position": [key(3, [0.1, 0.2])] },
    });
    // Offsets are from the clip's start at 2 s; the fixture uses timeline times for both.
    video.keyframes!.opacity = [
      { at: moment(2), offset: moment(0), value: 0, interpolation: "linear" },
      { at: moment(4), offset: moment(2), value: 1, interpolation: "linear" },
    ];
    expect(numberAt(video, "opacity", s(3), 1)).toBe(0.5);
    expect(numberAt(video, "volume", s(3), 0.7), "not animated").toBe(0.7);
    expect(pairAt(video, "transform.position", s(5), [0, 0])).toEqual([0.1, 0.2]);
    expect(keyAt(video, "opacity", s(4))?.value).toBe(1);
    expect(keyAt(video, "opacity", s(3))).toBeUndefined();
    expect(keyTimes(video)).toEqual([s(2), s(3), s(4)]);
    expect(keyTimes(clip("c2", "video", 0, 1))).toEqual([]);
  });
});

describe("curves", () => {
  it("sample like the renderer: through the points, monotone, flat outside", () => {
    const straight = sampleCurve([[0, 0], [1, 1]]);
    expect(straight[128]).toBeCloseTo(128 / 255, 6);
    const curve = sampleCurve([[0, 0], [0.25, 0.4], [0.5, 0.5], [1, 1]]);
    expect(curve[Math.round(0.5 * 255)]).toBeCloseTo(0.5, 1);
    expect(curve.every((v, i) => i === 0 || v >= curve[i - 1]! - 1e-6), "monotone").toBe(true);
    const flat = sampleCurve([[0.2, 0.3], [0.8, 0.7]]);
    expect([flat[0], flat[255]]).toEqual([0.3, 0.7]);
    expect(sampleCurve([[0.5, 0.5]], 3)).toEqual([0, 0.5, 1]);
    const wave = sampleCurve([[0, 0], [0.3, 0.8], [0.6, 0.2], [1, 1]]);
    expect(Math.max(...wave)).toBeLessThanOrEqual(1);
    expect(wave[Math.round(0.3 * 255)]!).toBeLessThanOrEqual(0.81);
  });
});
