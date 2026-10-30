// The region to track, drawn as a rectangle over the preview: pointer positions on the
// screen become a centre and size as fractions of the clip's picture as displayed (0 to 1
// from its top-left), which is what the engine's tracking takes.

import type { Region } from "../engine/types";
import { type Frame, toPicture, toScreen } from "./placement";

/** How the clip is mirrored on the canvas: the picture shows flipped about its centre. */
export interface Mirror {
  horizontal?: boolean;
  vertical?: boolean;
}

/** Regions smaller than this on either side, as a fraction of the picture, are a click. */
export const MIN_REGION = 0.01;

const clamp = (value: number) => Math.min(1, Math.max(0, value));
const round = (value: number) => Math.round(value * 10_000) / 10_000;

/** A point of the picture as shown, through the clip's mirroring, in the picture's own terms. */
function unmirror([u, v]: [number, number], mirror: Mirror): [number, number] {
  return [mirror.horizontal ? 1 - u : u, mirror.vertical ? 1 - v : v];
}

/**
 * The region a drag from `from` to `to` (screen points, in the overlay's CSS pixels)
 * covers in the picture placed at `frame`: the box around the dragged rectangle's corners
 * (a turned picture sees the rectangle turned), clipped to the picture. Null when it
 * covers less than `MIN_REGION` of the picture either way, as a click does.
 */
export function regionFromDrag(frame: Frame, from: [number, number], to: [number, number], mirror: Mirror = {}): Region | null {
  const corners: [number, number][] = [
    [from[0], from[1]],
    [to[0], from[1]],
    [to[0], to[1]],
    [from[0], to[1]],
  ];
  const picture = corners.map((corner) => unmirror(toPicture(frame, corner), mirror));
  const xs = picture.map(([u]) => clamp(u));
  const ys = picture.map(([, v]) => clamp(v));
  const [left, right] = [Math.min(...xs), Math.max(...xs)];
  const [top, bottom] = [Math.min(...ys), Math.max(...ys)];
  if (right - left < MIN_REGION || bottom - top < MIN_REGION) return null;
  return {
    x: round((left + right) / 2),
    y: round((top + bottom) / 2),
    width: round(right - left),
    height: round(bottom - top),
  };
}

/** A region's corners on the screen, clockwise from its top-left, for drawing it. */
export function regionCorners(frame: Frame, region: Region, mirror: Mirror = {}): [number, number][] {
  const [hw, hh] = [region.width / 2, region.height / 2];
  const corners: [number, number][] = [
    [region.x - hw, region.y - hh],
    [region.x + hw, region.y - hh],
    [region.x + hw, region.y + hh],
    [region.x - hw, region.y + hh],
  ];
  return corners.map((corner) => toScreen(frame, unmirror(corner, mirror)));
}
