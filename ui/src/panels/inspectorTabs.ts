// Which tabs the inspector shows for the selected clip, and how its fields group: the
// clip's own controls first, then sound, speed, animation, colour, masks, effects, the
// subject and measured edits, each where people look for it.

import { isMulticam } from "../app/multicam";
import type { AssetView, ClipView, EffectSpec } from "../engine/types";
import type { Field, FieldId } from "./fields";
import { hasPicture, playsSound } from "./fields";

export type InspectorTab =
  | "basic"
  | "text"
  | "style"
  | "shape"
  | "captions"
  | "translate"
  | "audio"
  | "speed"
  | "animation"
  | "adjust"
  | "masks"
  | "effects"
  | "subject"
  | "tools"
  | "shorts"
  | "camera"
  | "particles"
  | "model";

/** The tabs for `clip`, in order; the first is shown when the clip is selected. */
export function inspectorTabs(clip: ClipView, media: AssetView[]): InspectorTab[] {
  const sound = playsSound(clip, media);
  switch (clip.kind) {
    case "video":
      return [
        "basic",
        ...(sound ? (["audio"] as const) : []),
        "speed",
        "animation",
        "adjust",
        "masks",
        "effects",
        ...(clip.asset ? (["subject", "tools", "shorts"] as const) : (["tools"] as const)),
      ];
    case "image":
      return ["basic", "animation", "adjust", "masks", "effects"];
    case "audio":
      return ["audio", "speed"];
    case "text":
      return ["text", "style", "animation", "effects", "masks"];
    case "caption":
      return ["captions", "style", "translate"];
    case "shape":
      return ["shape", "basic", "animation", "effects", "masks"];
    // A camera has only its own fields (ADR-0037).
    case "camera":
      return ["camera"];
    // A particle emitter covers the canvas and takes a picture's placement (ADR-0038).
    case "particles":
      return ["particles", "basic", "animation", "effects", "masks"];
    // A 3D object is always in the 3D scene (ADR-0040).
    case "model":
      return ["model", "basic", "animation", "effects", "masks"];
    case "adjustment":
      return ["adjust", "effects"];
    case "timeline":
      return ["basic", ...(sound ? (["audio"] as const) : []), "animation", "adjust", "masks", "effects"];
    default:
      return hasPicture(clip) ? ["basic", "effects"] : ["basic"];
  }
}

/** The tab to show: the one asked for when the clip has it, else its first. */
export function shownTab(tabs: InspectorTab[], wanted: InspectorTab | null): InspectorTab {
  return wanted && tabs.includes(wanted) ? wanted : (tabs[0] ?? "basic");
}

/** The name the basic tab has for a clip of this kind: what it places. */
export function basicTabName(clip: ClipView): "video" | "image" | "placement" | "compound" | "multicam" {
  if (isMulticam(clip)) return "multicam";
  if (clip.kind === "video") return "video";
  if (clip.kind === "image") return "image";
  if (clip.kind === "timeline") return "compound";
  return "placement";
}

/** The groups of the inspector's numeric fields. */
export type FieldGroup = "placement" | "opacity" | "crop" | "sound" | "space";

const GROUPS: Record<FieldId, FieldGroup> = {
  x: "placement",
  y: "placement",
  scale: "placement",
  scale_x: "placement",
  scale_y: "placement",
  rotation: "placement",
  opacity: "opacity",
  crop_left: "crop",
  crop_right: "crop",
  crop_top: "crop",
  crop_bottom: "crop",
  volume: "sound",
  pan: "sound",
  fade_in: "sound",
  fade_out: "sound",
  depth: "space",
  tilt: "space",
  turn: "space",
  anchor_x: "space",
  anchor_y: "space",
};

/** The fields of one group, in their order. */
export function fieldsIn(fields: Field[], group: FieldGroup): Field[] {
  return fields.filter((field) => GROUPS[field.id] === group);
}

/** The range a field's slider covers: its own limits, or a range people mostly use. */
export function sliderRange(field: Field): [number, number] {
  const typical: Partial<Record<FieldId, [number, number]>> = {
    x: [-100, 100],
    y: [-100, 100],
    scale: [1, 400],
    scale_x: [1, 400],
    scale_y: [1, 400],
    rotation: [-180, 180],
    volume: [0, 200],
    fade_in: [0, 10],
    fade_out: [0, 10],
    depth: [-100, 300],
    tilt: [-180, 180],
    turn: [-180, 180],
    anchor_x: [-50, 50],
    anchor_y: [-50, 50],
  };
  const [low, high] = typical[field.id] ?? [field.min ?? 0, field.max ?? 100];
  return [field.min ?? low, field.max !== undefined && field.id !== "volume" ? field.max : high];
}

/** Which effects a part of the inspector lists: colour, other pictures, or sound. */
export type EffectScope = "all" | "color" | "picture" | "sound";

/** True when `spec` is listed in `scope`. */
export function inScope(spec: Pick<EffectSpec, "category">, scope: EffectScope): boolean {
  if (scope === "all") return true;
  if (scope === "sound") return spec.category === "audio";
  if (scope === "color") return spec.category === "color";
  return spec.category !== "audio" && spec.category !== "color";
}
