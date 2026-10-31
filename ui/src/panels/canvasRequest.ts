// The canvas fill of video and image clips (`clip.canvas`): what shows around a picture
// that does not cover the canvas, such as a wide shot in a vertical video.

import type { CanvasFill, ClipView, CommandCall } from "../engine/types";

/** The kinds of fill the inspector offers. */
export type FillKind = "none" | "blur" | "color";

/** The blur amount the engine takes, as a fraction of the covering picture's height. */
export const BLUR_MAX = 0.2;
export const BLUR_DEFAULT = 0.05;
export const COLOR_DEFAULT = "#000000";

/** True for clips with a canvas fill: video and image clips. */
export function hasCanvas(clip: ClipView): boolean {
  return clip.kind === "video" || clip.kind === "image";
}

/** The kind of a clip's fill. */
export function fillKind(clip: ClipView): FillKind {
  return clip.canvas?.kind ?? "none";
}

/** The clips a fill applies to: the selected video and image clips, `clip` first. */
export function fillTargets(clip: ClipView, selected: ClipView[]): string[] {
  const ids = [clip, ...selected].filter(hasCanvas).map((c) => c.id);
  return [...new Set(ids)];
}

/** A blur amount the engine accepts: from 0 to 0.2, at most three decimals. */
export function blurAmount(amount: number): number {
  if (!Number.isFinite(amount)) return BLUR_DEFAULT;
  return Math.round(Math.min(Math.max(amount, 0), BLUR_MAX) * 1000) / 1000;
}

/** A colour as the engine writes it, `#RRGGBB`, from what a colour picker gives. */
export function fillColor(color: string): string {
  const hex = /^#?([0-9a-f]{6})$/i.exec(color.trim());
  return hex?.[1] ? `#${hex[1].toUpperCase()}` : COLOR_DEFAULT;
}

/**
 * The fill of `kind` for a clip that has `current`: a new blur or colour keeps the value
 * the clip had, or starts at the default.
 */
export function fillOf(kind: FillKind, current: CanvasFill | undefined, value?: number | string): CanvasFill | null {
  if (kind === "none") return null;
  if (kind === "blur") {
    const amount = typeof value === "number" ? value : current?.kind === "blur" ? current.amount : BLUR_DEFAULT;
    return { kind: "blur", amount: blurAmount(amount) };
  }
  const color = typeof value === "string" ? value : current?.kind === "color" ? current.color : COLOR_DEFAULT;
  return { kind: "color", color: fillColor(color) };
}

/** The `clip.canvas` call that gives `clips` the fill, or takes it away when null. */
export function canvasCommand(clips: string[], fill: CanvasFill | null): CommandCall {
  return { command: "clip.canvas", args: fill ? { clips, fill } : { clips } };
}
