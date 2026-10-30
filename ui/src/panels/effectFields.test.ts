import { describe, expect, it } from "vitest";

import type { EffectView, ParamSpec } from "../engine/types";
import { clip, moment } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import {
  addPoint,
  animated,
  keyed,
  movePoint,
  paramAt,
  paramChange,
  paramPath,
  removePoint,
  specOf,
  toggleParamKey,
} from "./effectFields";

const exposure: ParamSpec = { name: "exposure", kind: "number", default: 0, min: -4, max: 4, animatable: true, meaning: "" };
const lift: ParamSpec = { name: "lift", kind: "pair", default: [0, 0], min: -1, max: 1, animatable: true, meaning: "" };
const master: ParamSpec = { name: "master", kind: "curve", default: "identity", min: 0, max: 1, animatable: false, meaning: "" };
const second = FLICKS_PER_SECOND;

describe("effect parameters", () => {
  const video = clip("clip_0003", "video", 2, 6);
  const plain: EffectView = { id: "effect_0004", type: "color.adjust", params: { exposure: 0.5 } };

  it("read stored values, defaults and keyframes at the playhead", () => {
    expect(paramAt(video, plain, exposure, 0)).toBe(0.5);
    expect(paramAt(video, { ...plain, params: {} }, exposure, 0)).toBe(0);
    expect(paramAt(video, plain, master, 0)).toEqual([[0, 0], [1, 1]]);
    const moving: EffectView = {
      ...plain,
      keyframes: {
        exposure: [
          { at: moment(2), offset: moment(0), value: 0, interpolation: "linear" },
          { at: moment(4), offset: moment(2), value: 2, interpolation: "linear" },
        ],
      },
    };
    expect(paramAt(video, moving, exposure, 3 * second)).toBe(1);
    expect([animated(moving, "exposure"), keyed(moving, "exposure", 4 * second), keyed(moving, "exposure", 3 * second)]).toEqual([
      true,
      true,
      false,
    ]);
    expect(paramPath(moving, "exposure")).toBe("effects.effect_0004.exposure");
    expect(paramChange(video, moving, exposure, 1.5, 3 * second)).toEqual({
      command: "keyframe.set",
      args: { clip: "clip_0003", property: "effects.effect_0004.exposure", at: { flicks: 3 * second }, value: 1.5 },
    });
    expect(toggleParamKey(video, moving, exposure, 4 * second).command).toBe("keyframe.remove");
    expect(toggleParamKey(video, moving, exposure, 3 * second)).toEqual({
      command: "keyframe.set",
      args: { clip: "clip_0003", property: "effects.effect_0004.exposure", at: { flicks: 3 * second } },
    });
  });

  it("turn edits into effect.set, refusing values out of range or unchanged", () => {
    expect(paramChange(video, plain, exposure, 1, 0)).toEqual({
      command: "effect.set",
      args: { clip: "clip_0003", effect: "effect_0004", params: { exposure: 1 } },
    });
    expect(paramChange(video, plain, exposure, 0.5, 0), "unchanged").toBeNull();
    expect(paramChange(video, plain, exposure, 9, 0), "out of range").toBeNull();
    expect(paramChange(video, plain, exposure, Number.NaN, 0)).toBeNull();
    expect(paramChange(video, plain, lift, [0.2, -0.1], 0)?.args).toEqual({
      clip: "clip_0003",
      effect: "effect_0004",
      params: { lift: [0.2, -0.1] },
    });
    expect(paramChange(video, plain, lift, [2, 0], 0)).toBeNull();
    expect(paramChange(video, plain, lift, 0.2, 0)).toBeNull();
    expect(specOf([{ type: "color.adjust", version: 1, title: "Adjust", category: "color", summary: "", params: [] }], plain)?.title).toBe(
      "Adjust",
    );
    expect(specOf([], plain)).toBeUndefined();
  });

  it("edit curves point by point, in order and within neighbours", () => {
    const straight: [number, number][] = [
      [0, 0],
      [1, 1],
    ];
    const added = addPoint(straight, [0.5, 0.7])!;
    expect(added).toEqual([
      [0, 0],
      [0.5, 0.7],
      [1, 1],
    ]);
    expect(addPoint(added, [0.505, 0.2]), "too close to a point").toBeNull();
    expect(addPoint(straight, [1.5, -1])).toBeNull();
    expect(movePoint(added, 1, [2, 0.4])[1]).toEqual([0.99, 0.4]);
    expect(movePoint(added, 1, [-1, 2])[1]).toEqual([0.01, 1]);
    expect(movePoint(added, 0, [0.3, 0.1])[0]).toEqual([0.3, 0.1]);
    expect(removePoint(added, 1)).toEqual(straight);
    expect(removePoint(straight, 0), "two points stay").toEqual(straight);
  });
});
