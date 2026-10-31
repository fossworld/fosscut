// A clip's masks at the playhead and the commands an edit turns into (ADR-0011, item 4):
// animated mask values change by a keyframe at the playhead (`masks.<id>.<property>`),
// others with `mask.set`.

import { evaluate } from "../app/animation";
import type { ClipView, CommandCall, KeyframeView, MaskView } from "../engine/types";
import { t } from "../i18n";

export type MaskFieldId = "x" | "y" | "width" | "height" | "rotation" | "feather" | "expansion" | "roundness";

export interface MaskField {
  id: MaskFieldId;
  label: string;
  /** The mask property it edits. */
  property: "center" | "size" | "rotation" | "feather" | "expansion" | "roundness";
  value: number;
  unit: string;
  min: number;
  max: number;
  animated: boolean;
  keyed: boolean;
}

/** Mask values with defaults filled in. */
export function maskValues(clip: ClipView, mask: MaskView, playhead: number) {
  const at = (name: string, plain: number | [number, number]) => {
    const keys: KeyframeView[] | undefined = mask.keyframes?.[name];
    const value = keys && keys.length > 0 ? evaluate(keys, playhead - clip.start.flicks) : undefined;
    return value ?? plain;
  };
  return {
    center: at("center", mask.center ?? [0.5, 0.5]) as [number, number],
    size: at("size", mask.size ?? [0.5, 0.5]) as [number, number],
    rotation: at("rotation", mask.rotation ?? 0) as number,
    feather: at("feather", mask.feather ?? 0) as number,
    expansion: at("expansion", mask.expansion ?? 0) as number,
    roundness: at("roundness", mask.roundness ?? 0) as number,
  };
}

const round = (v: number) => Math.round(v * 100) / 100;

/** The fields the inspector shows for a mask. */
export function maskFields(clip: ClipView, mask: MaskView, playhead: number): MaskField[] {
  const v = maskValues(clip, mask, playhead);
  const animated = (p: string) => (mask.keyframes?.[p]?.length ?? 0) > 0;
  const keyed = (p: string) => mask.keyframes?.[p]?.some((k) => k.at.flicks === playhead) ?? false;
  // Labels are read in the interface's language each time the fields are shown.
  const field = (
    id: MaskFieldId,
    property: MaskField["property"],
    value: number,
    unit: string,
    min: number,
    max: number,
  ): MaskField => ({
    id,
    label: t(`masks.field.${id}`),
    property,
    value: round(value),
    unit,
    min,
    max,
    animated: animated(property),
    keyed: keyed(property),
  });
  const fields: MaskField[] = [];
  if (mask.shape !== "brush") {
    fields.push(
      field("x", "center", v.center[0] * 100, "%", -100, 200),
      field("y", "center", v.center[1] * 100, "%", -100, 200),
      field("width", "size", v.size[0] * 100, "%", 0, 400),
      field("height", "size", v.size[1] * 100, "%", 0, 400),
      field("rotation", "rotation", v.rotation, "°", -36000, 36000),
    );
  }
  fields.push(
    field("feather", "feather", v.feather * 100, "%", 0, 100),
    field("expansion", "expansion", v.expansion * 100, "%", -100, 100),
  );
  if ((mask.shape ?? "rectangle") === "rectangle") fields.push(field("roundness", "roundness", v.roundness * 100, "%", 0, 100));
  return fields;
}

/**
 * The command for a new value of one mask field: a keyframe at the playhead when the
 * property is animated, otherwise `mask.set`; null when out of range or unchanged.
 */
export function maskChange(clip: ClipView, mask: MaskView, id: MaskFieldId, shown: number, playhead: number): CommandCall | null {
  const field = maskFields(clip, mask, playhead).find((f) => f.id === id);
  if (!field || !Number.isFinite(shown) || shown === field.value || shown < field.min || shown > field.max) return null;
  const v = maskValues(clip, mask, playhead);
  const value: number | [number, number] =
    id === "x"
      ? [shown / 100, v.center[1]]
      : id === "y"
        ? [v.center[0], shown / 100]
        : id === "width"
          ? [shown / 100, v.size[1]]
          : id === "height"
            ? [v.size[0], shown / 100]
            : id === "rotation"
              ? shown
              : shown / 100;
  if (field.animated) {
    return {
      command: "keyframe.set",
      args: { clip: clip.id, property: `masks.${mask.id}.${field.property}`, at: { flicks: playhead }, value },
    };
  }
  return { command: "mask.set", args: { clip: clip.id, mask: mask.id, [field.property]: value } };
}

/** Adds a keyframe of a mask property at the playhead, or removes the one there. */
export function toggleMaskKey(clip: ClipView, mask: MaskView, property: string, playhead: number): CommandCall {
  const args = { clip: clip.id, property: `masks.${mask.id}.${property}`, at: { flicks: playhead } };
  const keyed = mask.keyframes?.[property]?.some((k) => k.at.flicks === playhead) ?? false;
  return keyed ? { command: "keyframe.remove", args } : { command: "keyframe.set", args };
}
