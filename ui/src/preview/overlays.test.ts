import { describe, expect, it } from "vitest";

import { skimTime } from "../panels/Media";
import { moment } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import { ZOOMS, clampPan } from "./PreviewHole";
import { SAFE, inset } from "./SafeZones";

describe("preview overlays and zoom", () => {
  it("puts the safe zones inside the canvas, centred", () => {
    const canvas = { x: 10, y: 20, width: 1000, height: 500 };
    expect(inset(canvas, SAFE.title)).toEqual({ x: 60, y: 45, width: 900, height: 450 });
    expect(SAFE.action).toBeGreaterThan(SAFE.title);
  });

  it("pans a zoomed view only as far as it still covers the hole", () => {
    const hole = { width: 800, height: 400 };
    expect(clampPan([1000, -1000], 2, hole)).toEqual([400, -200]);
    expect(clampPan([30, 40], 3, hole)).toEqual([30, 40]);
    expect(clampPan([30, 40], 1, hole), "fitted, it does not move").toEqual([0, 0]);
    expect(ZOOMS[0]).toBe(1);
  });

  it("runs through a video's tile in a few frames on the pictures' grid", () => {
    const asset = { duration: moment(12) };
    expect(skimTime(asset, 0)).toBe(0.5 * FLICKS_PER_SECOND);
    expect(skimTime(asset, 0.999)).toBe(11.5 * FLICKS_PER_SECOND);
    expect(skimTime(asset, 2), "past the end is the last frame").toBe(11.5 * FLICKS_PER_SECOND);
    expect(skimTime({ duration: null }, 0.5)).toBe(0);
  });
});
