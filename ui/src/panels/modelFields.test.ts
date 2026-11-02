import { describe, expect, it } from "vitest";

import { clip } from "../test/fixtures";
import { inspectorTabs } from "./inspectorTabs";
import { keyedSpec } from "./keyedFields";
import { modelAdd, modelFields, modelKeyedSet, modelOf, modelSet } from "./modelFields";

describe("3D object settings", () => {
  const title = clip("clip_0090", "model", 0, 5, {
    object: "text",
    content: { object: { kind: "text", text: "Hello", depth: 0.08 }, material: { color: "#FF7A45" }, floor: {} },
  });
  const model = clip("clip_0091", "model", 0, 5, { object: "asset", content: { object: { kind: "asset", asset: "asset_0020" } } });

  it("read the object with the engine's defaults", () => {
    expect(modelOf(title)).toMatchObject({ kind: "text", text: "Hello", material: true, color: "#FF7A45", roughness: 0.5, floor: true, shadow: 0.6, depth: 0.08 });
    expect(modelOf(model)).toMatchObject({ kind: "asset", material: false, floor: false, azimuth: -35 });
  });

  it("offer the numbers the object has, keyed by their model paths", () => {
    const paths = (fields: { path: string }[]) => fields.map((f) => f.path);
    const fields = modelFields(modelOf(title));
    expect(paths(fields.shape)).toEqual(["model.depth", "model.emissive_strength"]);
    expect(paths(fields.light)).toContain("model.shadow");
    expect(paths(modelFields(modelOf(model)).light), "no floor, no shadow").not.toContain("model.shadow");
    expect(keyedSpec(title, fields.shape[0]!, 0)).toMatchObject({ value: 8, unit: "%" });
  });

  it("change values by sparse merge patches of model.set", () => {
    expect(modelKeyedSet("clip_0090", "model.light.azimuth", 90)).toEqual({ command: "model.set", args: { clip: "clip_0090", set: { light: { azimuth: 90 } } } });
    expect(modelKeyedSet("clip_0090", "model.depth", 0.04), "the default").toEqual({ command: "model.set", args: { clip: "clip_0090", set: { object: { depth: null } } } });
    expect(modelKeyedSet("clip_0090", "model.unknown", 1)).toBeNull();
    expect(modelSet("clip_0090", { floor: null })).toEqual({ command: "model.set", args: { clip: "clip_0090", set: { floor: null } } });
    expect(modelAdd({ preset: "podium" }, 0)).toEqual({ command: "model.add", args: { preset: "podium", start: { flicks: 0 } } });
  });

  it("show the object first, then its placement", () => {
    expect(inspectorTabs(title, [])).toEqual(["model", "basic", "animation", "effects", "masks"]);
    expect(inspectorTabs(clip("cam", "camera", 0, 1), [])).toEqual(["camera"]);
    expect(inspectorTabs(clip("p", "particles", 0, 1), [])[0]).toBe("particles");
  });
});
