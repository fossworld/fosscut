// Where a clip's picture lands in the preview, mirroring fosscut-render's layer quad and
// the desktop preview's letterboxing: the canvas fitted into the preview box, the picture
// fitted into the canvas, then moved, scaled and turned by its transform. Masks and the
// overlay that draws them use the picture's own coordinates, 0 to 1 from its top-left.

import { numberAt, pairAt } from "../app/animation";
import type { ClipView } from "../engine/types";

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The canvas of `aspect` (width over height) fitted inside `hole`, centred. */
export function fitCanvas(hole: Box, aspect: number): Box {
  if (hole.width <= 0 || hole.height <= 0 || aspect <= 0) return { x: hole.x, y: hole.y, width: 0, height: 0 };
  const [width, height] = hole.width / hole.height > aspect ? [hole.height * aspect, hole.height] : [hole.width, hole.width / aspect];
  return { x: hole.x + (hole.width - width) / 2, y: hole.y + (hole.height - height) / 2, width, height };
}

/** A picture on the screen: its centre, half width and height, and clockwise rotation. */
export interface Frame {
  center: [number, number];
  half: [number, number];
  rotation: number;
}

/**
 * The picture of `clip` at the playhead, placed in `screen` (the fitted canvas, in CSS
 * pixels) for a timeline `canvas` pixels large; null for clips without a picture size.
 */
export function pictureFrame(clip: ClipView, canvas: [number, number], screen: Box, playhead: number): Frame | null {
  const picture = clip.picture;
  if (!picture || picture[0] <= 0 || picture[1] <= 0) return null;
  const t = clip.transform ?? {};
  const scale = numberAt(clip, "transform.scale", playhead, t.scale ?? 1);
  const [sx, sy] = pairAt(clip, "transform.stretch", playhead, t.stretch ?? [1, 1]);
  const [px, py] = pairAt(clip, "transform.position", playhead, t.position ?? [0, 0]);
  const rotation = numberAt(clip, "transform.rotation", playhead, t.rotation ?? 0);
  const fit = Math.min(canvas[0] / picture[0], canvas[1] / picture[1]);
  const toScreen = screen.width / canvas[0];
  const half: [number, number] = [
    ((picture[0] * fit * scale * sx) / 2) * toScreen,
    ((picture[1] * fit * scale * sy) / 2) * toScreen,
  ];
  const center: [number, number] = [screen.x + (0.5 + px) * screen.width, screen.y + (0.5 + py) * screen.height];
  return { center, half, rotation };
}

/** A point of the picture (0 to 1) on the screen. */
export function toScreen(frame: Frame, [u, v]: [number, number]): [number, number] {
  const dx = (2 * u - 1) * frame.half[0];
  const dy = (2 * v - 1) * frame.half[1];
  const a = (frame.rotation * Math.PI) / 180;
  const [sin, cos] = [Math.sin(a), Math.cos(a)];
  return [frame.center[0] + dx * cos - dy * sin, frame.center[1] + dx * sin + dy * cos];
}

/** A screen point in the picture's coordinates, 0 to 1 (outside the picture beyond them). */
export function toPicture(frame: Frame, [x, y]: [number, number]): [number, number] {
  const a = (frame.rotation * Math.PI) / 180;
  const [sin, cos] = [Math.sin(a), Math.cos(a)];
  const [dx, dy] = [x - frame.center[0], y - frame.center[1]];
  const [rx, ry] = [dx * cos + dy * sin, -dx * sin + dy * cos];
  return [(rx / frame.half[0] + 1) / 2, (ry / frame.half[1] + 1) / 2];
}

/**
 * A point of a mask's box (-1 to 1) in the picture (0 to 1): the mask's own centre, size
 * and rotation, which turns in the picture's pixels, so `aspect` is the picture's width
 * over its height.
 */
export function maskToPicture(
  [qx, qy]: [number, number],
  center: [number, number],
  size: [number, number],
  rotation: number,
  aspect: number,
): [number, number] {
  const [hx, hy] = [(size[0] * aspect) / 2, size[1] / 2];
  const [x, y] = [qx * hx, qy * hy];
  const a = (rotation * Math.PI) / 180;
  const [sin, cos] = [Math.sin(a), Math.cos(a)];
  const [px, py] = [x * cos - y * sin, x * sin + y * cos];
  return [center[0] + px / aspect, center[1] + py];
}

/** The inverse of [`maskToPicture`]. */
export function pictureToMask(
  [u, v]: [number, number],
  center: [number, number],
  size: [number, number],
  rotation: number,
  aspect: number,
): [number, number] {
  const [dx, dy] = [(u - center[0]) * aspect, v - center[1]];
  const a = (rotation * Math.PI) / 180;
  const [sin, cos] = [Math.sin(a), Math.cos(a)];
  const [x, y] = [dx * cos + dy * sin, -dx * sin + dy * cos];
  const [hx, hy] = [Math.max((size[0] * aspect) / 2, 1e-9), Math.max(size[1] / 2, 1e-9)];
  return [x / hx, y / hy];
}
