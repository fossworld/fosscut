// The selected clip's properties as people edit them (percentages, degrees), and the
// `clip.set` changes they turn into (ADR-0011, item 3: property paths).

import type { ClipView } from "../engine/types";

export type FieldId = "x" | "y" | "scale" | "rotation" | "opacity" | "volume";

export interface Field {
  id: FieldId;
  label: string;
  /** The value shown, in the field's unit. */
  value: number;
  unit: string;
  min?: number;
  max?: number;
  step: number;
}

const round = (value: number) => Math.round(value * 100) / 100;

/** True for clips with a picture: everything but audio. */
function hasPicture(clip: ClipView): boolean {
  return clip.kind !== "audio";
}

/** True for clips with sound. */
function hasSound(clip: ClipView): boolean {
  return clip.kind === "audio" || clip.kind === "video";
}

/** The fields the inspector shows for `clip`. */
export function fieldsFor(clip: ClipView): Field[] {
  const fields: Field[] = [];
  const [x, y] = clip.transform?.position ?? [0, 0];
  if (hasPicture(clip)) {
    fields.push(
      { id: "x", label: "Position X", value: round(x * 100), unit: "%", step: 1 },
      { id: "y", label: "Position Y", value: round(y * 100), unit: "%", step: 1 },
      { id: "scale", label: "Scale", value: round((clip.transform?.scale ?? 1) * 100), unit: "%", min: 1, step: 1 },
      { id: "rotation", label: "Rotation", value: round(clip.transform?.rotation ?? 0), unit: "°", step: 1 },
      { id: "opacity", label: "Opacity", value: round((clip.opacity ?? 1) * 100), unit: "%", min: 0, max: 100, step: 1 },
    );
  }
  if (hasSound(clip)) {
    fields.push({ id: "volume", label: "Volume", value: round((clip.volume ?? 1) * 100), unit: "%", min: 0, max: 400, step: 1 });
  }
  return fields;
}

/**
 * The `clip.set` change for a new value of one field, or null when the value is not a
 * number, out of range, or what the clip already has.
 */
export function change(clip: ClipView, id: FieldId, shown: number): Record<string, number | number[]> | null {
  if (!Number.isFinite(shown)) return null;
  const field = fieldsFor(clip).find((f) => f.id === id);
  if (!field || shown === field.value) return null;
  if ((field.min !== undefined && shown < field.min) || (field.max !== undefined && shown > field.max)) return null;
  const [x, y] = clip.transform?.position ?? [0, 0];
  switch (id) {
    case "x":
      return { "transform.position": [shown / 100, y] };
    case "y":
      return { "transform.position": [x, shown / 100] };
    case "scale":
      return { "transform.scale": shown / 100 };
    case "rotation":
      return { "transform.rotation": shown };
    case "opacity":
      return { opacity: shown / 100 };
    case "volume":
      return { volume: shown / 100 };
  }
}
