import { describe, expect, it } from "vitest";

import type { AssetView, EffectSpec, EffectView, ParamSpec } from "../engine/types";
import { clip, moment } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import {
  addPoint,
  animated,
  effectGroups,
  groupOf,
  isSoundEffect,
  keyed,
  keyframeable,
  offeredEffects,
  movePoint,
  paramAt,
  paramChange,
  paramPath,
  removePoint,
  specOf,
  targetsOf,
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

describe("sound and picture effects", () => {
  const effect = (type: string, category: string, params: ParamSpec[] = []): EffectSpec => ({
    type,
    version: 1,
    title: type,
    category,
    summary: "",
    params,
  });
  const amount: ParamSpec = { name: "amount", kind: "number", default: 0.7, min: 0, max: 1, animatable: false, meaning: "" };
  const mode: ParamSpec = {
    name: "mode",
    kind: "choice",
    default: "isolate",
    choices: ["isolate", "remove"],
    animatable: false,
    meaning: "",
  };
  const catalog = [
    effect("color.adjust", "color", [exposure]),
    effect("audio.denoise", "audio", [amount]),
    effect("blur.gaussian", "blur"),
    effect("audio.vocals", "audio", [mode]),
  ];
  const media: AssetView[] = [
    { id: "asset_0001", kind: "video", name: "talk.mp4", used_by: [], audio: { channels: 2 } },
    { id: "asset_0002", kind: "video", name: "silent.mp4", used_by: [], audio: null },
    { id: "asset_0003", kind: "audio", name: "music.m4a", used_by: [] },
  ];
  const offered = (kind: string, extra: Record<string, unknown> = {}) =>
    offeredEffects(catalog, targetsOf(clip("clip_0001", kind, 0, 2, extra), media)).map((e) => e.type);

  it("offer sound effects only where there is sound, and picture effects only where there is a picture", () => {
    expect(offered("audio", { asset: "asset_0003" }), "audio clips: sound only").toEqual(["audio.denoise", "audio.vocals"]);
    expect(offered("video", { asset: "asset_0001" }), "a video with sound: both").toEqual([
      "color.adjust",
      "audio.denoise",
      "blur.gaussian",
      "audio.vocals",
    ]);
    expect(offered("video", { asset: "asset_0002" }), "a silent video: pictures only").toEqual(["color.adjust", "blur.gaussian"]);
    expect(offered("video", { asset: "asset_0009" }), "an unlisted asset is left to the engine").toHaveLength(4);
    expect(offered("timeline"), "compound clips: both").toHaveLength(4);
    for (const kind of ["text", "image", "caption", "adjustment"]) {
      expect(offered(kind), kind).toEqual(["color.adjust", "blur.gaussian"]);
    }
    expect(catalog.map(isSoundEffect)).toEqual([false, true, false, true]);
  });

  it("never keyframe parameters that are not animatable", () => {
    const sound = clip("clip_0001", "audio", 0, 2);
    const denoise: EffectView = {
      id: "effect_0002",
      type: "audio.denoise",
      params: { amount: 0.5 },
      keyframes: { amount: [{ at: moment(1), offset: moment(1), value: 0.9, interpolation: "linear" }] },
    };
    expect([keyframeable(exposure), keyframeable(amount), keyframeable(mode), keyframeable(master)]).toEqual([
      true,
      false,
      false,
      false,
    ]);
    expect(paramAt(sound, denoise, amount, second), "stray keys are not read").toBe(0.5);
    expect(paramChange(sound, denoise, amount, 0.8, second)).toEqual({
      command: "effect.set",
      args: { clip: "clip_0001", effect: "effect_0002", params: { amount: 0.8 } },
    });
    const vocals: EffectView = { id: "effect_0003", type: "audio.vocals" };
    expect(paramAt(sound, vocals, mode, 0)).toBe("isolate");
    expect(paramChange(sound, vocals, mode, "remove", 0)?.args).toEqual({
      clip: "clip_0001",
      effect: "effect_0003",
      params: { mode: "remove" },
    });
    expect(paramChange(sound, vocals, mode, "isolate", 0), "unchanged").toBeNull();
    expect(paramChange(sound, vocals, mode, "louder", 0), "not a choice").toBeNull();
  });
});

describe("the effects browser", () => {
  const spec = (type: string, category: string): EffectSpec => ({ type, version: 1, title: type, category, summary: "", params: [] });
  const color: ParamSpec = { name: "color", kind: "color", default: "#FFFFFF", animatable: false, meaning: "" };

  it("lists picture effects, then body effects, then sound effects", () => {
    const offered = [
      spec("body.glow", "body"),
      spec("color.adjust", "color"),
      spec("audio.denoise", "audio"),
      spec("key.subject", "key"),
      spec("body.outline", "body"),
    ];
    expect(offered.map(groupOf)).toEqual(["body", "picture", "sound", "picture", "body"]);
    expect(effectGroups(offered).map((g) => [g.kind, g.effects.map((e) => e.type)])).toEqual([
      ["picture", ["color.adjust", "key.subject"]],
      ["body", ["body.glow", "body.outline"]],
      ["sound", ["audio.denoise"]],
    ]);
    expect(effectGroups([spec("color.adjust", "color")]).map((g) => g.kind), "one group needs no headings").toEqual(["picture"]);
    expect(effectGroups([])).toEqual([]);
  });

  it("set colour parameters with effect.set", () => {
    const glow: EffectView = { id: "effect_0020", type: "body.glow", params: {} };
    const video = clip("clip_0003", "video", 0, 4);
    expect(paramAt(video, glow, color, 0)).toBe("#FFFFFF");
    expect(paramChange(video, glow, color, "#66CCFF", 0)).toEqual({
      command: "effect.set",
      args: { clip: "clip_0003", effect: "effect_0020", params: { color: "#66CCFF" } },
    });
    expect(paramChange(video, glow, color, "#FFFFFF", 0), "unchanged").toBeNull();
  });
});
