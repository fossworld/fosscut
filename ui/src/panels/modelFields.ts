// A 3D object clip (ADR-0040) as the inspector edits it: the engine's defaults for what it
// leaves out, its animatable numbers as keyed fields (`model.<path>`), and the merge
// patches `model.set` takes. A value set back to its default sends null, which removes
// it, so the stored object stays sparse.

import type { ClipView, CommandCall, ModelContentView } from "../engine/types";
import { t } from "../i18n";
import type { KeyedField } from "./keyedFields";

/** The engine's defaults for what an object leaves out. */
export const MODEL_DEFAULTS = {
  color: "#D0D0D0",
  metallic: 0,
  roughness: 0.5,
  emissive_strength: 1,
  azimuth: -35,
  elevation: 50,
  intensity: 1,
  ambient: 0.35,
  shadow: 0.6,
  depth: 0.04,
  growth: 1,
} as const;

/** The kinds of object, as the engine names them. */
export const MODEL_KINDS = ["asset", "box", "sphere", "cylinder", "plane", "torus", "text", "path", "blocks"] as const;

/** The clip's object as stored, with nothing filled in. */
function contentOf(clip: ClipView): ModelContentView {
  return clip.kind === "model" ? ((clip.content as ModelContentView | undefined) ?? {}) : {};
}

const number = (value: unknown, fallback: number) => (typeof value === "number" ? value : fallback);

/** Every setting the panel shows, at its stored value or its default. */
export function modelOf(clip: ClipView) {
  const c = contentOf(clip);
  const object = (c.object ?? {}) as Record<string, unknown>;
  const material = (c.material ?? null) as Record<string, unknown> | null;
  const light = (c.light ?? {}) as Record<string, unknown>;
  const floor = (c.floor ?? null) as Record<string, unknown> | null;
  const d = MODEL_DEFAULTS;
  const kind = String(object.kind ?? clip.object ?? "box");
  return {
    kind,
    text: typeof object.text === "string" ? object.text : "",
    /** False for a model asset that keeps its file's own materials. */
    material: material !== null,
    color: typeof material?.color === "string" ? material.color : d.color,
    metallic: number(material?.metallic, d.metallic),
    roughness: number(material?.roughness, d.roughness),
    emissive: typeof material?.emissive === "string" ? material.emissive : null,
    emissive_strength: number(c.emissive_strength, d.emissive_strength),
    azimuth: number(light.azimuth, d.azimuth),
    elevation: number(light.elevation, d.elevation),
    intensity: number(light.intensity, d.intensity),
    ambient: number(light.ambient, d.ambient),
    floor: floor !== null,
    shadow: number(floor?.shadow, d.shadow),
    depth: number(object.depth, d.depth),
    growth: number(object.growth, d.growth),
  };
}

export type ModelSettings = ReturnType<typeof modelOf>;

/** The object's keyed numbers by group; depth, growth and shadow only where the object has them. */
export function modelFields(m: ModelSettings): Record<"shape" | "light", KeyedField[]> {
  const shape: KeyedField[] = [];
  if (m.kind === "text" || m.kind === "path") {
    shape.push({ path: "model.depth", plain: m.depth, label: t("objects.depth"), unit: "%", scale: 100, min: 0, range: [0, 20], step: 0.1, help: t("objects.depth.help") });
  }
  if (m.kind === "blocks") {
    shape.push({ path: "model.growth", plain: m.growth, label: t("objects.growth"), unit: "%", scale: 100, min: 0, max: 100, range: [0, 100], help: t("objects.growth.help") });
  }
  shape.push({ path: "model.emissive_strength", plain: m.emissive_strength, label: t("objects.glow.strength"), unit: "×", min: 0, max: 100, range: [0, 10], step: 0.1, help: t("objects.glow.strength.help") });
  const light: KeyedField[] = [
    { path: "model.light.azimuth", plain: m.azimuth, label: t("objects.light.azimuth"), unit: "°", range: [-180, 180], help: t("objects.light.azimuth.help") },
    { path: "model.light.elevation", plain: m.elevation, label: t("objects.light.elevation"), unit: "°", min: -90, max: 90, range: [0, 90] },
    { path: "model.light.intensity", plain: m.intensity, label: t("objects.light.intensity"), unit: "×", min: 0, max: 10, range: [0, 3], step: 0.05 },
    { path: "model.light.ambient", plain: m.ambient, label: t("objects.light.ambient"), unit: "×", min: 0, max: 4, range: [0, 2], step: 0.05, help: t("objects.light.ambient.help") },
  ];
  if (m.floor) {
    light.push({ path: "model.shadow", plain: m.shadow, label: t("objects.floor.shadow"), unit: "%", scale: 100, min: 0, max: 100, range: [0, 100] });
  }
  return { shape, light };
}

/** The merge patch that sets a keyed path of an object to `value`, sparse at its default. */
function keyedPatch(path: string, value: number): Record<string, unknown> | null {
  const d = MODEL_DEFAULTS;
  const sparse = (v: number, fallback: number) => (v === fallback ? null : v);
  switch (path) {
    case "model.emissive_strength":
      return { emissive_strength: sparse(value, d.emissive_strength) };
    case "model.light.azimuth":
      return { light: { azimuth: sparse(value, d.azimuth) } };
    case "model.light.elevation":
      return { light: { elevation: sparse(value, d.elevation) } };
    case "model.light.intensity":
      return { light: { intensity: sparse(value, d.intensity) } };
    case "model.light.ambient":
      return { light: { ambient: sparse(value, d.ambient) } };
    case "model.shadow":
      return { floor: { shadow: sparse(value, d.shadow) } };
    case "model.depth":
      return { object: { depth: sparse(value, d.depth) } };
    case "model.growth":
      return { object: { growth: sparse(value, d.growth) } };
    default:
      return null;
  }
}

/** The `model.set` call for a merge patch of the object. */
export function modelSet(clip: string, set: Record<string, unknown>): CommandCall {
  return { command: "model.set", args: { clip, set } };
}

/** The `model.set` call for a keyed path's new plain value, or null for a path the object lacks. */
export function modelKeyedSet(clip: string, path: string, value: number): CommandCall | null {
  const patch = keyedPatch(path, value);
  return patch ? modelSet(clip, patch) : null;
}

/** The `model.add` arguments for a preset or a model asset at `flicks`. */
export function modelAdd(source: { preset: string } | { asset: string }, flicks: number): CommandCall {
  return { command: "model.add", args: { ...source, start: { flicks } } };
}
