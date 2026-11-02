// Any animatable number of a clip as the inspector edits it (ADR-0011, item 3): its value
// at the playhead in the unit people read, whether it has keyframes and one at the
// playhead, and the command a new value turns into. A property with keyframes changes by a
// keyframe at the playhead; one without changes its plain value with the command its kind
// uses (`clip.set`, `shape.set`, `camera.set`, `particles.set`, `clip.follow`).

import { keyAt, keysOf, numberAt, pairAt } from "../app/animation";
import type { ClipView, CommandCall } from "../engine/types";

/** A number field as the inspector shows it, in its own unit. */
export interface KeyedSpec {
  /** The property path, and which half of a pair this field shows. */
  path: string;
  part?: 0 | 1;
  label: string;
  /** The value shown. */
  value: number;
  unit: string;
  /** What may be typed, in the unit shown. */
  min?: number;
  max?: number;
  /** What the slider covers, in the unit shown. */
  range: [number, number];
  step: number;
  /** Whether the property takes keyframes, has any, and has one at the playhead. */
  animatable: boolean;
  animated: boolean;
  keyed: boolean;
  /** A longer explanation, shown as the field's tooltip. */
  help?: string;
}

/** How one field reads a property: the plain value, the scale to the unit shown, and its limits. */
export interface KeyedField {
  path: string;
  /** For a pair, the half shown. */
  part?: 0 | 1;
  /** The stored value when there are no keyframes: a number, or a pair for `part`. */
  plain: number | [number, number];
  label: string;
  unit: string;
  /** Shown value = engine value × scale (100 for percentages). */
  scale?: number;
  min?: number;
  max?: number;
  range: [number, number];
  step?: number;
  /** False for values that never take keyframes. */
  animatable?: boolean;
  help?: string;
}

const round = (value: number) => Math.round(value * 1000) / 1000;

/** The field `field` shows for `clip` at `playhead`. */
export function keyedSpec(clip: ClipView, field: KeyedField, playhead: number): KeyedSpec {
  const scale = field.scale ?? 1;
  const animatable = field.animatable ?? true;
  const engine =
    field.part === undefined
      ? numberAt(clip, field.path, playhead, field.plain as number)
      : pairAt(clip, field.path, playhead, field.plain as [number, number])[field.part];
  return {
    path: field.path,
    part: field.part,
    label: field.label,
    value: round(engine * scale),
    unit: field.unit,
    min: field.min,
    max: field.max,
    range: field.range,
    step: field.step ?? 1,
    animatable,
    animated: animatable && keysOf(clip, field.path) !== undefined,
    keyed: animatable && keyAt(clip, field.path, playhead) !== undefined,
    help: field.help,
  };
}

/** The engine value for a shown value of `field`: a number, or the pair with this half changed. */
export function engineValue(clip: ClipView, field: KeyedField, shown: number, playhead: number): number | [number, number] {
  const value = shown / (field.scale ?? 1);
  if (field.part === undefined) return value;
  const pair = pairAt(clip, field.path, playhead, field.plain as [number, number]);
  return field.part === 0 ? [value, pair[1]] : [pair[0], value];
}

/** True when `shown` may be sent for `field`: a number within its limits. */
export function allowed(field: Pick<KeyedField, "min" | "max">, shown: number): boolean {
  return Number.isFinite(shown) && (field.min === undefined || shown >= field.min) && (field.max === undefined || shown <= field.max);
}

/**
 * The command for a new engine value of `path`: a keyframe at the playhead when the
 * property has keyframes, otherwise `plain`.
 */
export function keyedChange(clip: ClipView, path: string, value: unknown, playhead: number, plain: CommandCall): CommandCall {
  if (keysOf(clip, path)) {
    return { command: "keyframe.set", args: { clip: clip.id, property: path, at: { flicks: playhead }, value } };
  }
  return plain;
}

/** A merge patch that sets `path` (dotted, such as `emitter.position`) to `value`. */
export function patchAt(path: string, value: unknown): Record<string, unknown> {
  const parts = path.split(".");
  let patch: unknown = value;
  for (const part of parts.reverse()) patch = { [part]: patch };
  return patch as Record<string, unknown>;
}
