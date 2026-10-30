// The selected clip's properties as people edit them (percentages, degrees, seconds), at
// the playhead, and the commands an edit turns into (ADR-0011, item 3: property paths).
// A property with keyframes changes by a keyframe at the playhead; one without changes
// its plain value with `clip.set`.

import { keyAt, keysOf, numberAt, pairAt } from "../app/animation";
import type { ClipView, CommandCall } from "../engine/types";
import { FLICKS_PER_SECOND } from "../transport/time";

export type FieldId =
  | "x"
  | "y"
  | "scale"
  | "scale_x"
  | "scale_y"
  | "rotation"
  | "crop_left"
  | "crop_right"
  | "crop_top"
  | "crop_bottom"
  | "opacity"
  | "volume"
  | "pan"
  | "fade_in"
  | "fade_out";

export interface Field {
  id: FieldId;
  label: string;
  /** The property path it edits. */
  path: string;
  /** The value shown, in the field's unit. */
  value: number;
  unit: string;
  min?: number;
  max?: number;
  step: number;
  /** Whether the property takes keyframes. */
  animatable: boolean;
  /** Whether it has keyframes. */
  animated: boolean;
  /** Whether one of them is at the playhead. */
  keyed: boolean;
}

/** How the inspector shows a clip: at which time, and scale as one value or two. */
export interface Viewing {
  playhead: number;
  /** Show width and height scale separately. */
  separate: boolean;
}

const round = (value: number) => Math.round(value * 100) / 100;

/** True for clips with a picture: everything but audio. */
export function hasPicture(clip: ClipView): boolean {
  return clip.kind !== "audio";
}

/** True for clips with sound. */
export function hasSound(clip: ClipView): boolean {
  return clip.kind === "audio" || clip.kind === "video";
}

/** True when the clip's picture is stretched, so its scale shows as two values. */
export function stretched(clip: ClipView): boolean {
  const [sx, sy] = clip.transform?.stretch ?? [1, 1];
  return sx !== 1 || sy !== 1 || keysOf(clip, "transform.stretch") !== undefined;
}

/** Values of the clip's properties at the playhead, in the engine's units. */
function values(clip: ClipView, playhead: number) {
  const t = clip.transform ?? {};
  const crop = clip.crop ?? {};
  return {
    position: pairAt(clip, "transform.position", playhead, t.position ?? [0, 0]),
    scale: numberAt(clip, "transform.scale", playhead, t.scale ?? 1),
    stretch: pairAt(clip, "transform.stretch", playhead, t.stretch ?? [1, 1]),
    rotation: numberAt(clip, "transform.rotation", playhead, t.rotation ?? 0),
    crop: {
      left: numberAt(clip, "crop.left", playhead, crop.left ?? 0),
      right: numberAt(clip, "crop.right", playhead, crop.right ?? 0),
      top: numberAt(clip, "crop.top", playhead, crop.top ?? 0),
      bottom: numberAt(clip, "crop.bottom", playhead, crop.bottom ?? 0),
    },
    opacity: numberAt(clip, "opacity", playhead, clip.opacity ?? 1),
    volume: numberAt(clip, "volume", playhead, clip.volume ?? 1),
    pan: numberAt(clip, "audio.pan", playhead, clip.audio?.pan ?? 0),
  };
}

/** The fields the inspector shows for `clip` at the playhead. */
export function fieldsFor(clip: ClipView, viewing: Viewing): Field[] {
  const { playhead } = viewing;
  const v = values(clip, playhead);
  const field = (
    id: FieldId,
    label: string,
    path: string,
    value: number,
    unit: string,
    extra: Partial<Field> = {},
  ): Field => {
    const animatable = extra.animatable ?? true;
    return {
      id,
      label,
      path,
      value: round(value),
      unit,
      step: 1,
      animatable,
      animated: animatable && keysOf(clip, path) !== undefined,
      keyed: animatable && keyAt(clip, path, playhead) !== undefined,
      ...extra,
    };
  };
  const fields: Field[] = [];
  if (hasPicture(clip)) {
    const [x, y] = v.position;
    fields.push(
      field("x", "Position X", "transform.position", x * 100, "%"),
      field("y", "Position Y", "transform.position", y * 100, "%"),
    );
    if (viewing.separate || stretched(clip)) {
      fields.push(
        field("scale_x", "Scale X", "transform.stretch", v.scale * v.stretch[0] * 100, "%", { min: 1 }),
        field("scale_y", "Scale Y", "transform.stretch", v.scale * v.stretch[1] * 100, "%", { min: 1 }),
      );
    } else {
      fields.push(field("scale", "Scale", "transform.scale", v.scale * 100, "%", { min: 1 }));
    }
    fields.push(
      field("rotation", "Rotation", "transform.rotation", v.rotation, "°"),
      field("opacity", "Opacity", "opacity", v.opacity * 100, "%", { min: 0, max: 100 }),
    );
    if (clip.kind === "video" || clip.kind === "image") {
      fields.push(
        field("crop_left", "Crop left", "crop.left", v.crop.left * 100, "%", { min: 0, max: 99 }),
        field("crop_right", "Crop right", "crop.right", v.crop.right * 100, "%", { min: 0, max: 99 }),
        field("crop_top", "Crop top", "crop.top", v.crop.top * 100, "%", { min: 0, max: 99 }),
        field("crop_bottom", "Crop bottom", "crop.bottom", v.crop.bottom * 100, "%", { min: 0, max: 99 }),
      );
    }
  }
  if (hasSound(clip)) {
    const seconds = (flicks: number | undefined) => (flicks ?? 0) / FLICKS_PER_SECOND;
    fields.push(
      field("volume", "Volume", "volume", v.volume * 100, "%", { min: 0, max: 400 }),
      field("pan", "Pan", "audio.pan", v.pan * 100, "%", { min: -100, max: 100 }),
      field("fade_in", "Fade in", "audio.fade_in", seconds(clip.audio?.fade_in), "s", {
        min: 0,
        step: 0.1,
        animatable: false,
      }),
      field("fade_out", "Fade out", "audio.fade_out", seconds(clip.audio?.fade_out), "s", {
        min: 0,
        step: 0.1,
        animatable: false,
      }),
    );
  }
  return fields;
}

/** The engine value of `path` after field `id` shows `shown`. */
function engineValue(clip: ClipView, id: FieldId, shown: number, playhead: number): unknown {
  const v = values(clip, playhead);
  const [x, y] = v.position;
  const time = (seconds: number) => ({ flicks: Math.round(seconds * FLICKS_PER_SECOND) });
  switch (id) {
    case "x":
      return [shown / 100, y];
    case "y":
      return [x, shown / 100];
    case "scale":
      return shown / 100;
    case "scale_x":
      return [shown / 100 / v.scale, v.stretch[1]];
    case "scale_y":
      return [v.stretch[0], shown / 100 / v.scale];
    case "rotation":
      return shown;
    case "crop_left":
    case "crop_right":
    case "crop_top":
    case "crop_bottom":
    case "opacity":
    case "volume":
    case "pan":
      return shown / 100;
    case "fade_in":
    case "fade_out":
      return time(shown);
  }
}

/**
 * The command for a new value of one field: a keyframe at the playhead when the property
 * is animated, otherwise `clip.set`. Null when the value is not a number, out of range,
 * or what the field already shows.
 */
export function change(clip: ClipView, id: FieldId, shown: number, viewing: Viewing): CommandCall | null {
  if (!Number.isFinite(shown)) return null;
  const field = fieldsFor(clip, viewing).find((f) => f.id === id);
  if (!field || shown === field.value) return null;
  if ((field.min !== undefined && shown < field.min) || (field.max !== undefined && shown > field.max)) return null;
  const value = engineValue(clip, id, shown, viewing.playhead);
  if (field.animated) {
    return {
      command: "keyframe.set",
      args: { clip: clip.id, property: field.path, at: { flicks: viewing.playhead }, value },
    };
  }
  return { command: "clip.set", args: { clip: clip.id, set: { [field.path]: value } } };
}

/** Adds a keyframe of `path` at the playhead with its current value, or removes the one there. */
export function toggleKey(clip: ClipView, path: string, playhead: number): CommandCall {
  const args = { clip: clip.id, property: path, at: { flicks: playhead } };
  return keyAt(clip, path, playhead)
    ? { command: "keyframe.remove", args }
    : { command: "keyframe.set", args };
}

/** Paths of the clip with a keyframe at the playhead. */
export function keyedPaths(clip: ClipView, playhead: number): string[] {
  return Object.keys(clip.keyframes ?? {}).filter((path) => keyAt(clip, path, playhead) !== undefined);
}
