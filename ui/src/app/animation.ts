// Keyframed clip properties on the UI side, mirroring fosscut-model's evaluation
// (ADR-0011, item 3): values hold before the first keyframe and after the last, and move
// along each keyframe's easing in between. The inspector shows values at the playhead.

import type { ClipView, Interpolation, KeyframeView } from "../engine/types";

/** CSS-style easing curves, as the engine names them. */
export const EASINGS = {
  linear: "linear",
  hold: "hold",
  ease_in: { bezier: [0.42, 0, 1, 1] },
  ease_out: { bezier: [0, 0, 0.58, 1] },
  ease_in_out: { bezier: [0.42, 0, 0.58, 1] },
} as const satisfies Record<string, Interpolation>;

export type EasingName = keyof typeof EASINGS;

/** The easing name of an interpolation, or "custom" for another Bezier curve. */
export function easingName(interpolation: Interpolation): EasingName | "custom" {
  if (typeof interpolation === "string") return interpolation;
  const [x1, y1, x2, y2] = interpolation.bezier;
  for (const [name, known] of Object.entries(EASINGS)) {
    if (typeof known !== "string" && known.bezier.every((v, i) => v === [x1, y1, x2, y2][i])) {
      return name as EasingName;
    }
  }
  return "custom";
}

function cubic(a: number, b: number, s: number): number {
  const r = 1 - s;
  return 3 * r * r * s * a + 3 * r * s * s * b + s * s * s;
}

function slope(a: number, b: number, s: number): number {
  const r = 1 - s;
  return 3 * r * r * a + 6 * r * s * (b - a) + 3 * s * s * (1 - b);
}

/** The fraction of the change at fraction `t` of the time between two keyframes. */
export function progress(interpolation: Interpolation, t: number): number {
  const x = Math.min(Math.max(t, 0), 1);
  if (interpolation === "hold") return 0;
  if (interpolation === "linear") return x;
  const [x1raw, y1, x2raw, y2] = interpolation.bezier;
  const x1 = Math.min(Math.max(x1raw, 0), 1);
  const x2 = Math.min(Math.max(x2raw, 0), 1);
  let s = x;
  for (let i = 0; i < 8; i++) {
    const error = cubic(x1, x2, s) - x;
    if (Math.abs(error) < 1e-9) return cubic(y1, y2, s);
    const d = slope(x1, x2, s);
    if (Math.abs(d) < 1e-6) break;
    s = Math.min(Math.max(s - error / d, 0), 1);
  }
  let low = 0;
  let high = 1;
  s = x;
  for (let i = 0; i < 60; i++) {
    const value = cubic(x1, x2, s);
    if (Math.abs(value - x) < 1e-9) break;
    if (value < x) low = s;
    else high = s;
    s = (low + high) / 2;
  }
  return cubic(y1, y2, s);
}

type Value = number | [number, number];

function lerp(a: Value, b: Value, t: number): Value {
  if (typeof a === "number" && typeof b === "number") return a + (b - a) * t;
  if (Array.isArray(a) && Array.isArray(b)) return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
  return a;
}

/** The value of keyframes at `offset` flicks from the clip's start; undefined without any. */
export function evaluate(keys: KeyframeView[], offset: number): Value | undefined {
  const first = keys[0];
  if (!first) return undefined;
  if (offset <= first.offset.flicks) return first.value;
  let after = keys.findIndex((k) => k.offset.flicks > offset);
  if (after === -1) after = keys.length;
  const before = keys[after - 1]!;
  const next = keys[after];
  if (!next) return before.value;
  const t = (offset - before.offset.flicks) / (next.offset.flicks - before.offset.flicks);
  return lerp(before.value, next.value, progress(before.interpolation, t));
}

/** The keyframes of `path`, if it is animated. */
export function keysOf(clip: ClipView, path: string): KeyframeView[] | undefined {
  const keys = clip.keyframes?.[path];
  return keys && keys.length > 0 ? keys : undefined;
}

/** The keyframe of `path` on the frame at `playhead` (timeline flicks), if there is one. */
export function keyAt(clip: ClipView, path: string, playhead: number): KeyframeView | undefined {
  return keysOf(clip, path)?.find((k) => k.at.flicks === playhead);
}

/** A number property at the playhead: from its keyframes when it has any. */
export function numberAt(clip: ClipView, path: string, playhead: number, plain: number): number {
  const keys = keysOf(clip, path);
  const value = keys ? evaluate(keys, playhead - clip.start.flicks) : undefined;
  return typeof value === "number" ? value : plain;
}

/** A pair property at the playhead: from its keyframes when it has any. */
export function pairAt(
  clip: ClipView,
  path: string,
  playhead: number,
  plain: [number, number],
): [number, number] {
  const keys = keysOf(clip, path);
  const value = keys ? evaluate(keys, playhead - clip.start.flicks) : undefined;
  return Array.isArray(value) ? value : plain;
}

/** Timeline times of every keyframe of the clip, sorted and without repeats. */
export function keyTimes(clip: ClipView): number[] {
  const times = new Set<number>();
  for (const keys of Object.values(clip.keyframes ?? {})) for (const key of keys) times.add(key.at.flicks);
  return [...times].sort((a, b) => a - b);
}

/**
 * A curve through `points` sampled at `count` positions from 0 to 1, with the monotone
 * cubic interpolation fosscut-render uses for `color.curves`: smooth, no overshoot, flat
 * before the first point and after the last.
 */
export function sampleCurve(points: [number, number][], count = 256): number[] {
  const out: number[] = [];
  if (points.length < 2) {
    for (let i = 0; i < count; i++) out.push(i / (count - 1));
    return out;
  }
  const n = points.length;
  const x = points.map((p) => p[0]);
  const y = points.map((p) => p[1]);
  const secants = x.slice(0, -1).map((xi, i) => (y[i + 1]! - y[i]!) / Math.max(x[i + 1]! - xi, 1e-9));
  const tangents = new Array<number>(n).fill(0);
  tangents[0] = secants[0]!;
  tangents[n - 1] = secants[n - 2]!;
  for (let i = 1; i < n - 1; i++) {
    tangents[i] = secants[i - 1]! * secants[i]! <= 0 ? 0 : (secants[i - 1]! + secants[i]!) / 2;
  }
  for (let i = 0; i < n - 1; i++) {
    const s = secants[i]!;
    if (s === 0) {
      tangents[i] = 0;
      tangents[i + 1] = 0;
      continue;
    }
    const a = tangents[i]! / s;
    const b = tangents[i + 1]! / s;
    const r = a * a + b * b;
    if (r > 9) {
      const t = 3 / Math.sqrt(r);
      tangents[i] = t * a * s;
      tangents[i + 1] = t * b * s;
    }
  }
  for (let index = 0; index < count; index++) {
    const t = index / (count - 1);
    let v: number;
    if (t <= x[0]!) v = y[0]!;
    else if (t >= x[n - 1]!) v = y[n - 1]!;
    else {
      let i = 0;
      while (i < n - 2 && x[i + 1]! <= t) i++;
      const h = x[i + 1]! - x[i]!;
      const s = (t - x[i]!) / h;
      const s2 = s * s;
      const s3 = s2 * s;
      v =
        (2 * s3 - 3 * s2 + 1) * y[i]! +
        (s3 - 2 * s2 + s) * h * tangents[i]! +
        (-2 * s3 + 3 * s2) * y[i + 1]! +
        (s3 - s2) * h * tangents[i + 1]!;
    }
    out.push(Math.min(Math.max(v, 0), 1));
  }
  return out;
}
