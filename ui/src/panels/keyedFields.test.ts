import { describe, expect, it } from "vitest";

import type { KeyframeView } from "../engine/types";
import { clip, moment } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import { cameraFields, cameraOf, cameraSet } from "./Camera";
import { canFollow, followCall, pathsFor } from "./Follow";
import { allowed, engineValue, keyedChange, keyedSpec, patchAt } from "./keyedFields";
import { particleFields, particlesOf, particlesSet, recolored } from "./particleFields";

const key = (seconds: number, value: number | [number, number]): KeyframeView => ({
  at: moment(seconds),
  offset: moment(seconds),
  value,
  interpolation: "linear",
});

describe("keyed fields", () => {
  const camera = clip("clip_0060", "camera", 0, 4, {
    camera: { field_of_view: 60, orbit: [20, 0], depth_of_field: true },
    keyframes: { "camera.position_z": [key(0, 0), key(2, 0.5)] },
  });

  it("show values at the playhead in their unit, with keyframe state", () => {
    const [fov] = cameraFields(cameraOf(camera)).lens;
    expect(keyedSpec(camera, fov!, 0)).toMatchObject({ value: 60, unit: "°", animated: false, keyed: false });
    const dolly = cameraFields(cameraOf(camera)).place[2]!;
    expect(keyedSpec(camera, dolly, FLICKS_PER_SECOND)).toMatchObject({ value: 25, animated: true, keyed: false });
    expect(keyedSpec(camera, dolly, 2 * FLICKS_PER_SECOND).keyed).toBe(true);
    const pitch = cameraFields(cameraOf(camera)).orbit[1]!;
    expect(engineValue(camera, pitch, 15, 0), "the other half of the pair stays").toEqual([20, 15]);
    expect([allowed(fov!, 0), allowed(fov!, 120), allowed(fov!, Number.NaN)]).toEqual([false, true, false]);
  });

  it("change plain values with the kind's command, and animated ones by keyframe", () => {
    const plain = cameraSet(camera.id, "field_of_view", 50);
    expect(keyedChange(camera, "camera.field_of_view", 50, 0, plain)).toEqual({
      command: "camera.set",
      args: { clip: "clip_0060", set: { field_of_view: 50 } },
    });
    expect(cameraSet(camera.id, "field_of_view", 40), "the default is removed").toEqual({
      command: "camera.set",
      args: { clip: "clip_0060", set: { field_of_view: null } },
    });
    expect(keyedChange(camera, "camera.position_z", 0.3, FLICKS_PER_SECOND, cameraSet(camera.id, "position_z", 0.3))).toEqual({
      command: "keyframe.set",
      args: { clip: "clip_0060", property: "camera.position_z", at: { flicks: FLICKS_PER_SECOND }, value: 0.3 },
    });
    expect(patchAt("emitter.position", [0, 0.5])).toEqual({ emitter: { position: [0, 0.5] } });
  });

  it("read a camera with the engine's defaults", () => {
    expect(cameraOf(clip("clip_0061", "camera", 0, 1))).toMatchObject({ field_of_view: 40, aperture: 0.1, max_blur: 0.02, depth_of_field: false });
    expect(cameraOf(camera)).toMatchObject({ field_of_view: 60, orbit: [20, 0], depth_of_field: true });
  });
});

describe("particle settings", () => {
  const sparks = clip("clip_0070", "particles", 0, 5, {
    content: { rate: 160, speed: 1.3, colors: ["#FFF6D8", "#FF5A1F"], sprite: { kind: "spark" }, emitter: { position: [0, 0.35] } },
  });

  it("read the emitter with the engine's defaults, in shown units", () => {
    const p = particlesOf(sparks);
    expect(p).toMatchObject({ rate: 160, life: 2, speed: 1.3, direction: -90, sprite: "spark", emitter_shape: "point", emitter_position: [0, 0.35] });
    const speed = particleFields(p).motion[0]!;
    expect(keyedSpec(sparks, speed, 0)).toMatchObject({ value: 130, unit: "%/s" });
    expect(particlesOf(clip("clip_0071", "particles", 0, 1))).toMatchObject({ rate: 30, speed: 0.25, size: 0.02, colors: ["#FFFFFF"] });
  });

  it("set settings by sparse merge patches", () => {
    expect(particlesSet(sparks.id, "rate", 60)).toEqual({ command: "particles.set", args: { clip: "clip_0070", set: { rate: 60 } } });
    expect(particlesSet(sparks.id, "rate", 30), "the default").toEqual({ command: "particles.set", args: { clip: "clip_0070", set: { rate: null } } });
    expect(particlesSet(sparks.id, "emitter.position", [0.1, 0])).toEqual({
      command: "particles.set",
      args: { clip: "clip_0070", set: { emitter: { position: [0.1, 0] } } },
    });
    expect(particlesSet(sparks.id, "sprite.kind", "dot")).toEqual({ command: "particles.set", args: { clip: "clip_0070", set: { sprite: { kind: null } } } });
  });

  it("add, change and remove colours, keeping one to eight", () => {
    expect(recolored(["#FFFFFF"], 1, "#FF0000")).toEqual(["#FFFFFF", "#FF0000"]);
    expect(recolored(["#FFFFFF", "#FF0000"], 0, "#000000")).toEqual(["#000000", "#FF0000"]);
    expect(recolored(["#FFFFFF", "#FF0000"], 0, null)).toEqual(["#FF0000"]);
    expect(recolored(["#FFFFFF"], 0, null), "the last colour stays").toBeNull();
    expect(recolored(Array.from({ length: 8 }, () => "#FFFFFF"), 8, "#000000"), "at most eight").toBeNull();
  });
});

describe("following paths", () => {
  const route = clip("clip_0080", "shape", 0, 5, { shape: "path" });
  const badge = clip("clip_0081", "shape", 0, 5, { shape: "ellipse", follow: { clip: "clip_0080" } });
  const dot = clip("clip_0082", "image", 0, 5);
  const timeline = {
    timeline: "timeline_0000",
    name: "T",
    revision: 1,
    format: { width: 1920, height: 1080, rate: "30" },
    duration: moment(5),
    tracks: [{ id: "track_0001", kind: "video", index: 0, clips: [route, badge, dot] }],
  };

  it("offer shape clips that follow nothing, never the clip itself", () => {
    expect(pathsFor(timeline, dot).map((c) => c.id)).toEqual(["clip_0080"]);
    expect(pathsFor(timeline, route).map((c) => c.id), "not itself, nor a follower").toEqual([]);
  });

  it("let pictures follow unless something follows them", () => {
    expect(canFollow(dot, timeline)).toBe(true);
    expect(canFollow(route, timeline), "a followed path stays put").toBe(false);
    expect(canFollow(clip("clip_0083", "camera", 0, 5), timeline)).toBe(false);
    expect(followCall("clip_0082", { path: "clip_0080" })).toEqual({ command: "clip.follow", args: { clip: "clip_0082", path: "clip_0080" } });
  });
});
