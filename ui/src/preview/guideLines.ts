// Rulers and guides over the preview: where they sit on the screen, and which guide a
// pointer grabs. Guides are fractions of the canvas, so they stay put at any window size.

import type { Guide } from "../app/editor";
import type { Box } from "./placement";

/** Ruler thickness in CSS pixels. */
export const RULER = 16;

/** Pixels from a guide within which a press grabs it. */
const GRAB = 5;

/** A guide's position on the screen: an x for vertical guides, a y for horizontal ones. */
export function guideToScreen(guide: Guide, screen: Box): number {
  return guide.axis === "x" ? screen.x + guide.at * screen.width : screen.y + guide.at * screen.height;
}

/** The guide on `axis` at screen position `position`, as a fraction of the canvas. */
export function screenToGuide(axis: "x" | "y", position: number, screen: Box): Guide {
  const span = axis === "x" ? screen.width : screen.height;
  const start = axis === "x" ? screen.x : screen.y;
  const at = span > 0 ? (position - start) / span : 0;
  return { axis, at: Math.min(Math.max(at, 0), 1) };
}

/** The index of the guide near the point, or -1. */
export function guideAt(guides: Guide[], screen: Box, x: number, y: number): number {
  return guides.findIndex((g) => Math.abs(guideToScreen(g, screen) - (g.axis === "x" ? x : y)) <= GRAB);
}

/** Tick marks along a ruler of `length` pixels for a canvas side of `size` pixels: every
 * round number of canvas pixels that leaves at least 40 screen pixels between labels. */
export function ticks(size: number, length: number): number[] {
  if (size <= 0 || length <= 0) return [];
  const steps = [10, 20, 50, 100, 200, 250, 500, 1000];
  const step = steps.find((s) => (s / size) * length >= 40) ?? size;
  const out: number[] = [];
  for (let at = 0; at <= size; at += step) out.push(at);
  return out;
}
