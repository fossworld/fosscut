// A shape clip's shape as people edit it, with the engine's defaults filled in, and the
// merge patch each edit turns into for `shape.set` (ADR-0028). Patches are sparse: a value
// set back to its default sends null, which removes it, so the stored shape stays sparse.
// An outline always sends `enabled`, which the engine needs to read it.

import type { ClipView, ShapeContentView, ShapeKind } from "../engine/types";
import type { MessageKey } from "../i18n";
import { TEXT_DEFAULTS, normalColor } from "./textFields";

/** Every field of a shape with a value, flat, as the panel shows them. */
export interface ShapeFields {
  shape: ShapeKind;
  /** Width and height as fractions of the canvas. */
  width: number;
  height: number;
  roundness: number;
  color: string;
  gradient: boolean;
  gradient_to: string;
  /** Degrees: 0 runs left to right, 90 top to bottom. */
  angle: number;
  radial: boolean;
  outline: boolean;
  outline_color: string;
  /** A fraction of the shape's shorter side. */
  outline_width: number;
  /** The label's words; empty when the shape has no label. */
  label: string;
  label_color: string;
  /** A fraction of the canvas height, as a text's size. */
  label_size: number;
  /** Where the label's centre sits in the shape's box, -1 to 1. */
  label_x: number;
  label_y: number;
}

export type ShapeField = keyof ShapeFields;

/** The engine's defaults for what a shape leaves out. */
export const SHAPE_DEFAULTS = {
  roundness: 0,
  color: "#FFFFFF",
  gradient_to: "#000000",
  angle: 0,
  radial: false,
  outline_color: "#000000",
  outline_width: 0.04,
  label_color: TEXT_DEFAULTS.color,
  label_size: TEXT_DEFAULTS.size,
  label_x: 0,
  label_y: 0,
} as const;

/** The outlines a shape can switch to; a pen path keeps its points, so only paths offer it. */
export const SHAPES: ShapeKind[] = ["rectangle", "ellipse", "star", "heart"];

/** The outlines to offer for `clip`: the drawn ones, and `path` when it is one. */
export function shapeKinds(clip: ClipView): ShapeKind[] {
  return shapeOf(clip).shape === "path" ? [...SHAPES, "path"] : SHAPES;
}

/** How a number field is shown: `scale` times the engine value, within `min` and `max` shown. */
export interface ShapeNumber {
  /** The field's name, translated where it shows. */
  label: MessageKey;
  min: number;
  max: number;
  step: number;
  scale: number;
  unit: string;
}

export const SHAPE_NUMBERS: Partial<Record<ShapeField, ShapeNumber>> = {
  width: { label: "shape.number.width", min: 0.01, max: 400, step: 1, scale: 100, unit: "%" },
  height: { label: "shape.number.height", min: 0.01, max: 400, step: 1, scale: 100, unit: "%" },
  roundness: { label: "shape.number.roundness", min: 0, max: 100, step: 5, scale: 100, unit: "%" },
  angle: { label: "shape.number.angle", min: -360, max: 360, step: 15, scale: 1, unit: "deg" },
  outline_width: { label: "shape.number.outline_width", min: 0, max: 50, step: 0.5, scale: 100, unit: "%" },
  label_size: { label: "shape.number.label_size", min: 0.5, max: 100, step: 0.5, scale: 100, unit: "%" },
  label_x: { label: "shape.number.label_x", min: -100, max: 100, step: 5, scale: 100, unit: "%" },
  label_y: { label: "shape.number.label_y", min: -100, max: 100, step: 5, scale: 100, unit: "%" },
};

const COLORS: ShapeField[] = ["color", "gradient_to", "outline_color", "label_color"];
const SWITCHES: ShapeField[] = ["gradient", "radial", "outline"];

/** The clip's shape, or a plain white rectangle when the clip shows none. */
function contentOf(clip: ClipView): ShapeContentView {
  return clip.content ?? { shape: clip.shape ?? "rectangle", size: [0.3, 0.2] };
}

/** The clip's shape with every field at its stored value or its default. */
export function shapeOf(clip: ClipView): ShapeFields {
  const c = contentOf(clip);
  const d = SHAPE_DEFAULTS;
  const gradient = c.fill?.gradient;
  const style = c.label?.style ?? {};
  return {
    shape: c.shape,
    width: c.size[0],
    height: c.size[1],
    roundness: c.roundness ?? d.roundness,
    color: c.fill?.color ?? d.color,
    gradient: gradient !== undefined,
    gradient_to: gradient?.to ?? d.gradient_to,
    angle: gradient?.angle ?? d.angle,
    radial: gradient?.radial ?? d.radial,
    outline: c.outline?.enabled ?? false,
    outline_color: c.outline?.color ?? d.outline_color,
    outline_width: c.outline?.width ?? d.outline_width,
    label: c.label?.text ?? "",
    label_color: style.color ?? d.label_color,
    label_size: style.size ?? d.label_size,
    label_x: c.label_at?.[0] ?? d.label_x,
    label_y: c.label_at?.[1] ?? d.label_y,
  };
}

/** The number a field shows for its engine value. */
export function shownNumber(field: ShapeField, value: number): number {
  const spec = SHAPE_NUMBERS[field];
  return spec ? Math.round(value * spec.scale * 10000) / 10000 : value;
}

/** The engine value of a field from a shown value, or null when it is not valid there. */
function checked(field: ShapeField, shown: unknown, kinds: ShapeKind[]): unknown {
  const spec = SHAPE_NUMBERS[field];
  if (spec) {
    if (typeof shown !== "number" || !Number.isFinite(shown) || shown < spec.min || shown > spec.max) return null;
    return Math.round((shown / spec.scale) * 100000) / 100000;
  }
  if (COLORS.includes(field)) return typeof shown === "string" ? normalColor(shown) : null;
  if (SWITCHES.includes(field)) return typeof shown === "boolean" ? shown : null;
  if (field === "shape") return kinds.includes(shown as ShapeKind) ? shown : null;
  if (field === "label") return typeof shown === "string" ? shown.trim() : null;
  return null;
}

/** `value` unless it is the default, which a merge patch resets with null. */
function sparse(value: unknown, fallback: unknown): unknown {
  return value === fallback ? null : value;
}

/**
 * The `shape.set` merge patch for a new value of one field, in its shown unit (a width of
 * 30 is 30% of the canvas). Null when the value is not valid there or changes nothing:
 * gradient and label fields need a gradient and a label first. An empty label removes it.
 */
export function shapePatch(clip: ClipView, field: ShapeField, shown: unknown): Record<string, unknown> | null {
  const value = checked(field, shown, shapeKinds(clip));
  if (value === null) return null;
  const now = shapeOf(clip);
  if (value === now[field]) return null;
  const d = SHAPE_DEFAULTS;
  const n = value as number;
  const gradient = (change: Record<string, unknown>) => (now.gradient ? { fill: { gradient: change } } : null);
  const outline = (change: Record<string, unknown>) => ({ outline: { enabled: now.outline, ...change } });
  const label = (style: Record<string, unknown>) => (now.label ? { label: { style } } : null);
  const at = (x: number, y: number) => ({ label_at: x === 0 && y === 0 ? null : [x, y] });
  switch (field) {
    case "shape":
      return { shape: value };
    case "width":
      return { size: [n, now.height] };
    case "height":
      return { size: [now.width, n] };
    case "roundness":
      return { roundness: sparse(n, d.roundness) };
    case "color":
      return { fill: { color: value } };
    case "gradient":
      return { fill: { gradient: value ? { to: d.gradient_to } : null } };
    case "gradient_to":
      return gradient({ to: value });
    case "angle":
      return gradient({ angle: sparse(n, d.angle) });
    case "radial":
      return gradient({ radial: value ? true : null });
    case "outline":
      return { outline: { enabled: value } };
    case "outline_color":
      return outline({ color: sparse(value, d.outline_color) });
    case "outline_width":
      return outline({ width: sparse(n, d.outline_width) });
    case "label":
      if (value === "") return now.label ? { label: null } : null;
      return { label: { text: value } };
    case "label_color":
      return label({ color: sparse(value, d.label_color) });
    case "label_size":
      return label({ size: sparse(n, d.label_size) });
    case "label_x":
      return now.label ? at(n, now.label_y) : null;
    case "label_y":
      return now.label ? at(now.label_x, n) : null;
  }
}
