// Effect parameters at the playhead and the commands an edit turns into (ADR-0011,
// items 1 and 2): numbers and pairs with keyframes change by a keyframe at the playhead
// (`effects.<id>.<param>`), everything else with `effect.set`. Curves are edited point by
// point here, then sent whole.

import { evaluate } from "../app/animation";
import type { ClipView, CommandCall, EffectSpec, EffectView, KeyframeView, ParamSpec } from "../engine/types";

export type Point = [number, number];

/** The property path of an effect parameter for keyframe commands. */
export function paramPath(effect: EffectView, param: string): string {
  return `effects.${effect.id}.${param}`;
}

function keysOf(effect: EffectView, param: string): KeyframeView[] | undefined {
  const keys = effect.keyframes?.[param];
  return keys && keys.length > 0 ? keys : undefined;
}

/** True when the parameter has keyframes. */
export function animated(effect: EffectView, param: string): boolean {
  return keysOf(effect, param) !== undefined;
}

/** True when one of its keyframes is on the frame at `playhead`. */
export function keyed(effect: EffectView, param: string, playhead: number): boolean {
  return keysOf(effect, param)?.some((k) => k.at.flicks === playhead) ?? false;
}

/** The value of a parameter at the playhead: keyframes, the stored value, or the default. */
export function paramAt(clip: ClipView, effect: EffectView, spec: ParamSpec, playhead: number): unknown {
  const keys = keysOf(effect, spec.name);
  if (keys) {
    const value = evaluate(keys, playhead - clip.start.flicks);
    if (value !== undefined) return value;
  }
  const stored = effect.params?.[spec.name];
  if (stored !== undefined) return stored;
  if (spec.kind === "curve" && spec.default === "identity") return [[0, 0], [1, 1]];
  return spec.default;
}

function inRange(spec: ParamSpec, value: number): boolean {
  return (
    Number.isFinite(value) &&
    (spec.min === undefined || value >= spec.min) &&
    (spec.max === undefined || value <= spec.max)
  );
}

function same(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * The command that gives a parameter a new value: a keyframe at the playhead when it is
 * animated, otherwise `effect.set`. Null when the value is out of range or unchanged.
 */
export function paramChange(
  clip: ClipView,
  effect: EffectView,
  spec: ParamSpec,
  value: unknown,
  playhead: number,
): CommandCall | null {
  if (spec.kind === "number" && !(typeof value === "number" && inRange(spec, value))) return null;
  if (spec.kind === "pair") {
    if (!Array.isArray(value) || value.length !== 2 || !value.every((v) => typeof v === "number" && inRange(spec, v))) {
      return null;
    }
  }
  if (same(value, paramAt(clip, effect, spec, playhead))) return null;
  if (animated(effect, spec.name)) {
    return {
      command: "keyframe.set",
      args: { clip: clip.id, property: paramPath(effect, spec.name), at: { flicks: playhead }, value },
    };
  }
  return { command: "effect.set", args: { clip: clip.id, effect: effect.id, params: { [spec.name]: value } } };
}

/** Adds a keyframe of the parameter at the playhead, or removes the one there. */
export function toggleParamKey(clip: ClipView, effect: EffectView, spec: ParamSpec, playhead: number): CommandCall {
  const args = { clip: clip.id, property: paramPath(effect, spec.name), at: { flicks: playhead } };
  return keyed(effect, spec.name, playhead) ? { command: "keyframe.remove", args } : { command: "keyframe.set", args };
}

/** The spec of an effect's type, if the catalog has it. */
export function specOf(catalog: EffectSpec[], effect: EffectView): EffectSpec | undefined {
  return catalog.find((s) => s.type === effect.type);
}

const clamp = (v: number) => Math.min(Math.max(v, 0), 1);

/** Adds a curve point, keeping the points in order of x; null if one is already there. */
export function addPoint(points: Point[], point: Point): Point[] | null {
  const [x, y] = [clamp(point[0]), clamp(point[1])];
  if (points.some((p) => Math.abs(p[0] - x) < 0.01)) return null;
  return [...points, [x, y] as Point].sort((a, b) => a[0] - b[0]);
}

/** Moves point `index`, keeping it between its neighbours. */
export function movePoint(points: Point[], index: number, point: Point): Point[] {
  const low = index > 0 ? points[index - 1]![0] + 0.01 : 0;
  const high = index < points.length - 1 ? points[index + 1]![0] - 0.01 : 1;
  return points.map((p, i) => (i === index ? [Math.min(Math.max(point[0], low), high), clamp(point[1])] : p));
}

/** Removes point `index`, keeping at least two. */
export function removePoint(points: Point[], index: number): Point[] {
  return points.length <= 2 ? points : points.filter((_, i) => i !== index);
}
