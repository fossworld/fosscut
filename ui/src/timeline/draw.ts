// Draws the timeline on a 2D canvas (ADR-0010, item 2): only what is visible, so long
// timelines with thousands of clips stay smooth.

import { keyTimes } from "../app/animation";
import type { ClipView, TimelineView } from "../engine/types";
import { FLICKS_PER_SECOND } from "../transport/time";
import { HEADER, ROW, RULER, type View, rowY, rows, rulerLabel, rulerStep, timeToX } from "./geometry";

const COLORS = {
  background: "#16171b",
  ruler: "#1d1f24",
  rowEven: "#1a1c20",
  rowOdd: "#17181c",
  header: "#202227",
  text: "#e6e7ea",
  muted: "#8d8f98",
  tick: "#3a3d45",
  playhead: "#ff7a45",
  selection: "#ffffff",
} as const;

const CLIP_COLORS: Record<string, string> = {
  video: "#3a6ea5",
  image: "#3f7f8c",
  audio: "#3f8a5a",
  text: "#8a5aa5",
  caption: "#a5793a",
};

const KIND_NAMES: Record<string, string> = {
  video: "Video",
  audio: "Audio",
  text: "Titles",
  caption: "Captions",
};

export interface DrawState {
  playhead: number;
  selection: string | null;
  /** A clip being dragged, and where it would land. */
  drag?: { clip: string; start: number; end: number; track: string } | null;
}

/** Draws a clip's block and label between x0 and x1 on the row at y. */
function block(ctx: CanvasRenderingContext2D, clip: ClipView, x0: number, x1: number, y: number, selected: boolean): void {
  const enabled = clip.enabled !== false;
  ctx.save();
  ctx.globalAlpha *= enabled ? 1 : 0.4;
  ctx.fillStyle = CLIP_COLORS[clip.kind] ?? "#5a5d66";
  ctx.beginPath();
  ctx.roundRect(x0 + 0.5, y + 4.5, x1 - x0 - 1, ROW - 9, 4);
  ctx.fill();
  ctx.restore();
  if (selected) {
    ctx.strokeStyle = COLORS.selection;
    ctx.lineWidth = 2;
    ctx.stroke();
  }
  if (x1 - x0 > 24) {
    ctx.save();
    ctx.beginPath();
    ctx.rect(x0 + 6, y, x1 - x0 - 12, ROW);
    ctx.clip();
    ctx.fillStyle = COLORS.text;
    ctx.font = "12px system-ui, sans-serif";
    ctx.textBaseline = "middle";
    ctx.fillText(label(clip), x0 + 8, y + ROW / 2);
    ctx.restore();
  }
}

function label(clip: ClipView): string {
  const name = clip.text ?? clip.asset_name ?? clip.kind;
  return clip.enabled === false ? `${name} (off)` : name;
}

/** Fade ramps at the clip's edges and diamonds at its keyframes. */
function marks(ctx: CanvasRenderingContext2D, clip: ClipView, view: View, x0: number, x1: number, y: number): void {
  const top = y + 5;
  const bottom = y + ROW - 5;
  ctx.fillStyle = "rgba(0, 0, 0, 0.35)";
  for (const [edge, length] of [
    ["in", clip.audio?.fade_in ?? 0],
    ["out", clip.audio?.fade_out ?? 0],
  ] as const) {
    if (length <= 0) continue;
    const width = (length / FLICKS_PER_SECOND) * view.zoom;
    const from = edge === "in" ? timeToX(clip.start.flicks, view) : timeToX(clip.end.flicks, view);
    const to = edge === "in" ? from + width : from - width;
    ctx.beginPath();
    ctx.moveTo(from, top);
    ctx.lineTo(from, bottom);
    ctx.lineTo(to, top);
    ctx.closePath();
    ctx.fill();
  }
  ctx.fillStyle = COLORS.text;
  for (const time of keyTimes(clip)) {
    const x = timeToX(time, view);
    if (x < x0 || x > x1) continue;
    ctx.beginPath();
    ctx.moveTo(x, bottom - 9);
    ctx.lineTo(x + 4, bottom - 5);
    ctx.lineTo(x, bottom - 1);
    ctx.lineTo(x - 4, bottom - 5);
    ctx.closePath();
    ctx.fill();
  }
}

/** Draws the whole timeline into a canvas of `width` by `height` CSS pixels. */
export function drawTimeline(
  ctx: CanvasRenderingContext2D,
  timeline: TimelineView | null,
  view: View,
  height: number,
  state: DrawState,
): void {
  const width = view.width;
  ctx.fillStyle = COLORS.background;
  ctx.fillRect(0, 0, width, height);
  if (!timeline) return;

  // Rows, then clips that overlap the visible time.
  const ordered = rows(timeline);
  const first = view.scroll * FLICKS_PER_SECOND;
  const last = (view.scroll + (width - HEADER) / view.zoom) * FLICKS_PER_SECOND;
  ordered.forEach((track, row) => {
    const y = rowY(row);
    ctx.fillStyle = row % 2 === 0 ? COLORS.rowEven : COLORS.rowOdd;
    ctx.fillRect(HEADER, y, width - HEADER, ROW);
    for (const clip of track.clips) {
      if (clip.end.flicks < first || clip.start.flicks > last) continue;
      const x0 = Math.max(HEADER, timeToX(clip.start.flicks, view));
      const x1 = Math.min(width, timeToX(clip.end.flicks, view));
      if (x1 - x0 < 1) continue;
      const dragged = state.drag?.clip === clip.id;
      ctx.globalAlpha = dragged ? 0.3 : 1;
      block(ctx, clip, x0, x1, y, !dragged && clip.id === state.selection);
      marks(ctx, clip, view, x0, x1, y);
      ctx.globalAlpha = 1;
    }
  });

  // Where a dragged clip would land.
  const drag = state.drag;
  const row = drag ? ordered.findIndex((t) => t.id === drag.track) : -1;
  const dragged = drag ? timeline.tracks.flatMap((t) => t.clips).find((c) => c.id === drag.clip) : undefined;
  if (drag && dragged && row >= 0) {
    const x0 = Math.max(HEADER, timeToX(drag.start, view));
    const x1 = Math.min(width, timeToX(drag.end, view));
    if (x1 - x0 >= 1) block(ctx, dragged, x0, x1, rowY(row), true);
  }

  // Ruler with labels on round steps.
  ctx.fillStyle = COLORS.ruler;
  ctx.fillRect(0, 0, width, RULER);
  const step = rulerStep(view.zoom);
  const start = Math.floor(view.scroll / step) * step;
  ctx.font = "11px system-ui, sans-serif";
  ctx.textBaseline = "middle";
  for (let seconds = start; ; seconds += step) {
    const x = timeToX(seconds * FLICKS_PER_SECOND, view);
    if (x > width) break;
    if (x < HEADER) continue;
    ctx.fillStyle = COLORS.tick;
    ctx.fillRect(Math.round(x), RULER - 8, 1, 8);
    ctx.fillStyle = COLORS.muted;
    ctx.fillText(rulerLabel(seconds, step), x + 4, RULER / 2);
  }

  // Track labels.
  ctx.fillStyle = COLORS.header;
  ctx.fillRect(0, 0, HEADER, height);
  ctx.font = "12px system-ui, sans-serif";
  ordered.forEach((track, row) => {
    ctx.fillStyle = COLORS.text;
    ctx.fillText(track.name ?? KIND_NAMES[track.kind] ?? track.kind, 10, rowY(row) + ROW / 2);
  });

  // Playhead.
  const x = timeToX(state.playhead, view);
  if (x >= HEADER && x <= width) {
    ctx.fillStyle = COLORS.playhead;
    ctx.fillRect(Math.round(x) - 1, 0, 2, height);
    ctx.beginPath();
    ctx.moveTo(x - 6, 0);
    ctx.lineTo(x + 6, 0);
    ctx.lineTo(x, 8);
    ctx.fill();
  }
}
