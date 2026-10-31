// Draws the timeline on a 2D canvas (ADR-0010, item 2): only what is visible, so long
// timelines with thousands of clips stay smooth. Video and image clips show a strip of
// their frames, clips with sound their waveform, and other clips a colour by kind.

import { keyTimes } from "../app/animation";
import { angleNumber } from "../app/multicam";
import { type Peaks, type PictureAsset, gridStep, onGrid, peakBetween } from "../app/pictures";
import { speedLabel } from "../app/speed";
import type { ClipView, TimelineView } from "../engine/types";
import { t } from "../i18n";
import { FLICKS_PER_SECOND } from "../transport/time";
import { HEADER, ROW, RULER, type View, rowY, rows, rulerLabel, rulerStep, timeToX } from "./geometry";
import { DARK, type Palette, clipColors } from "./palette";

export interface DrawState {
  playhead: number;
  /** The time the preview shows while skimming. */
  skim?: number | null;
  selection: string | null;
  /** More selected clips. */
  also?: string[];
  /** A clip being dragged, and where it would land. */
  drag?: { clip: string; start: number; end: number; track: string } | null;
  /** Where media dragged from the browser would land: a time range on a row. */
  drop?: { start: number; end: number; row: number } | null;
  /** The theme's colours. */
  palette?: Palette;
  /** The asset a clip shows, for its frames and its sound. */
  asset?: (id: string) => PictureAsset | undefined;
  /** A decoded frame of an asset at a source time, or null while it loads. */
  frame?: (asset: PictureAsset, flicks: number) => CanvasImageSource | null;
  /** An asset's peaks, or null while they load. */
  peaks?: (asset: PictureAsset) => Peaks | null;
}

/** Top inset of a clip's block in its row, and its height. */
const INSET = 3;
const BLOCK = ROW - 2 * INSET;
/** Height of the sound band under a video clip's frames. */
const BAND = 14;

/** How much faster than the timeline a clip plays its media, on average. */
function rateOf(clip: ClipView): number {
  if (clip.speed && clip.source_used && clip.duration.flicks > 0) return clip.source_used.flicks / clip.duration.flicks;
  return 1;
}

/** The clip's source time at timeline time `flicks`. */
function sourceAt(clip: ClipView, flicks: number): number {
  return (clip.source_in?.flicks ?? 0) + (flicks - clip.start.flicks) * rateOf(clip);
}

function label(clip: ClipView): string {
  const name = clip.text ?? clip.asset_name ?? clip.timeline_name ?? clip.shape ?? clip.kind;
  const speed = speedLabel(clip);
  const shown = speed ? `${name} (${speed})` : name;
  return clip.enabled === false ? t("timeline.clip.off", { name: shown }) : shown;
}

/** A strip of the clip's frames across its block, each tile as wide as the picture's aspect. */
function filmstrip(ctx: CanvasRenderingContext2D, clip: ClipView, asset: PictureAsset, view: View, x0: number, x1: number, top: number, height: number, state: DrawState): void {
  const frame = state.frame;
  if (!frame) return;
  const aspect = asset.video && asset.video.height > 0 ? asset.video.width / asset.video.height : 16 / 9;
  const tile = Math.max(24, height * aspect);
  const clipX = timeToX(clip.start.flicks, view);
  // Tiles line up from the clip's start, so they stay put while it scrolls.
  const first = Math.max(0, Math.floor((x0 - clipX) / tile));
  const step = gridStep((tile / view.zoom) * rateOf(clip));
  for (let x = clipX + first * tile; x < x1; x += tile) {
    const at = ((x - HEADER) / view.zoom + view.scroll) * FLICKS_PER_SECOND;
    const source = asset.kind === "image" ? 0 : onGrid(sourceAt(clip, at) / FLICKS_PER_SECOND, step);
    const image = frame(asset, source);
    if (image) {
      ctx.drawImage(image, x, top, tile, height);
    } else {
      // A quiet placeholder while the frame is read.
      ctx.fillStyle = "rgba(0, 0, 0, 0.18)";
      ctx.fillRect(x + 1, top + 1, tile - 2, height - 2);
    }
  }
}

/** The clip's waveform between `top` and `top + height`, scaled by its volume. */
function waveform(ctx: CanvasRenderingContext2D, clip: ClipView, asset: PictureAsset, view: View, x0: number, x1: number, top: number, height: number, color: string, state: DrawState): void {
  const peaks = state.peaks?.(asset);
  const middle = top + height / 2;
  ctx.fillStyle = color;
  if (!peaks || peaks.duration <= 0) {
    ctx.globalAlpha *= 0.5;
    ctx.fillRect(x0, middle, x1 - x0, 1);
    ctx.globalAlpha /= 0.5;
    return;
  }
  const gain = Math.min(1.6, clip.volume ?? 1);
  const perPixel = FLICKS_PER_SECOND / view.zoom;
  for (let x = Math.floor(x0); x < x1; x += 1) {
    const at = ((x - HEADER) / view.zoom + view.scroll) * FLICKS_PER_SECOND;
    const from = sourceAt(clip, at) / peaks.duration;
    const to = sourceAt(clip, at + perPixel) / peaks.duration;
    if (to < 0 || from > 1) continue;
    const [low, high] = peakBetween(peaks.peaks, Math.max(0, from), Math.min(1, to));
    const up = Math.min(1, high * gain) * (height / 2);
    const down = Math.min(1, -low * gain) * (height / 2);
    ctx.fillRect(x, middle - up, 1, Math.max(1, up + down));
  }
}

/** Draws a clip's block between x0 and x1 on the row at y: its picture or sound, and its label. */
function block(ctx: CanvasRenderingContext2D, clip: ClipView, view: View, x0: number, x1: number, y: number, selected: boolean, state: DrawState): void {
  const palette = state.palette ?? DARK;
  const colors = clipColors(palette, clip.kind);
  const enabled = clip.enabled !== false;
  const top = y + INSET;
  const width = x1 - x0;
  ctx.save();
  ctx.globalAlpha *= enabled ? 1 : 0.45;
  ctx.beginPath();
  ctx.roundRect(x0 + 0.5, top, Math.max(1, width - 1), BLOCK, 6);
  ctx.fillStyle = colors.fill;
  ctx.fill();
  ctx.clip();
  const asset = clip.asset ? state.asset?.(clip.asset) : undefined;
  if ((clip.kind === "video" || clip.kind === "image") && asset && !asset.missing) {
    const sound = clip.kind === "video" && !clip.audio_detached && asset.kind === "video" && state.peaks;
    const pictureHeight = sound ? BLOCK - BAND : BLOCK;
    filmstrip(ctx, clip, asset, view, x0, x1, top, pictureHeight, state);
    // Darker at the top, so the label reads over any picture.
    const shade = ctx.createLinearGradient(0, top, 0, top + 20);
    shade.addColorStop(0, "rgba(0, 0, 0, 0.55)");
    shade.addColorStop(1, "rgba(0, 0, 0, 0)");
    ctx.fillStyle = shade;
    ctx.fillRect(x0, top, width, 20);
    if (sound) {
      ctx.fillStyle = "rgba(0, 0, 0, 0.45)";
      ctx.fillRect(x0, top + pictureHeight, width, BAND);
      waveform(ctx, clip, asset, view, x0, x1, top + pictureHeight + 1, BAND - 2, colors.tone, state);
    }
  } else if (clip.kind === "audio" && asset) {
    waveform(ctx, clip, asset, view, x0, x1, top + 15, BLOCK - 18, colors.tone, state);
  } else if (clip.kind === "adjustment") {
    // Stripes: it changes what lies below rather than showing a picture.
    ctx.strokeStyle = "rgba(255, 255, 255, 0.08)";
    ctx.lineWidth = 6;
    for (let x = x0 - BLOCK; x < x1; x += 16) {
      ctx.beginPath();
      ctx.moveTo(x, top + BLOCK);
      ctx.lineTo(x + BLOCK, top);
      ctx.stroke();
    }
  } else if (clip.kind === "timeline") {
    // A compound clip: the edges of the clips stacked inside it.
    ctx.fillStyle = "rgba(255, 255, 255, 0.1)";
    ctx.fillRect(x0, top + BLOCK - 7, width, 3);
    ctx.fillRect(x0, top + BLOCK - 13, width, 3);
  }
  // The kind's own tone along the top edge.
  ctx.fillStyle = colors.tone;
  ctx.globalAlpha *= 0.9;
  ctx.fillRect(x0, top, width, 2);
  ctx.restore();

  if (selected) {
    ctx.save();
    ctx.beginPath();
    ctx.roundRect(x0 + 1, top + 0.5, Math.max(1, width - 2), BLOCK - 1, 6);
    ctx.strokeStyle = palette.selection;
    ctx.lineWidth = 2;
    ctx.stroke();
    ctx.restore();
  }

  if (width > 28) {
    ctx.save();
    ctx.beginPath();
    ctx.rect(x0 + 6, y, width - 12, ROW);
    ctx.clip();
    ctx.textBaseline = "middle";
    const line = top + 10;
    // A multicam part: the number of the angle it shows, in a small box before the label.
    const angle = angleNumber(clip);
    let text = x0 + 8;
    if (angle !== null) {
      ctx.fillStyle = "rgba(0, 0, 0, 0.45)";
      ctx.beginPath();
      ctx.roundRect(x0 + 6, line - 7, 14, 14, 3);
      ctx.fill();
      ctx.fillStyle = palette.clipLabel;
      ctx.font = "600 10px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText(String(angle), x0 + 13, line + 0.5);
      ctx.textAlign = "left";
      text += 16;
    }
    ctx.fillStyle = palette.clipLabel;
    ctx.font = "500 11px system-ui, sans-serif";
    ctx.shadowColor = "rgba(0, 0, 0, 0.6)";
    ctx.shadowBlur = 2;
    ctx.fillText(label(clip), text, line);
    ctx.restore();
  }
}

/** A transition from `clip` into the next: a rounded badge across the cut, as long as it lasts. */
function transitionMark(ctx: CanvasRenderingContext2D, clip: ClipView, view: View, y: number, palette: Palette): void {
  if (!clip.transition) return;
  const cut = timeToX(clip.end.flicks, view);
  const half = Math.max(7, (clip.transition.duration.flicks / FLICKS_PER_SECOND / 2) * view.zoom);
  const top = y + ROW / 2 - 9;
  ctx.save();
  ctx.beginPath();
  ctx.roundRect(cut - half, top, half * 2, 18, 9);
  ctx.fillStyle = "rgba(10, 10, 12, 0.72)";
  ctx.fill();
  ctx.strokeStyle = palette.text;
  ctx.globalAlpha = 0.85;
  ctx.lineWidth = 1;
  ctx.stroke();
  // Two overlapping triangles: one picture giving way to the next.
  ctx.fillStyle = palette.text;
  ctx.beginPath();
  ctx.moveTo(cut - 5, top + 5);
  ctx.lineTo(cut + 1, top + 9);
  ctx.lineTo(cut - 5, top + 13);
  ctx.closePath();
  ctx.fill();
  ctx.globalAlpha = 0.55;
  ctx.beginPath();
  ctx.moveTo(cut + 5, top + 5);
  ctx.lineTo(cut - 1, top + 9);
  ctx.lineTo(cut + 5, top + 13);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

/** Fade ramps at the clip's edges and diamonds at its keyframes. */
function marks(ctx: CanvasRenderingContext2D, clip: ClipView, view: View, x0: number, x1: number, y: number, palette: Palette): void {
  const top = y + INSET;
  const bottom = y + ROW - INSET;
  ctx.fillStyle = "rgba(0, 0, 0, 0.38)";
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
  for (const time of keyTimes(clip)) {
    const x = timeToX(time, view);
    if (x < x0 || x > x1) continue;
    ctx.beginPath();
    ctx.moveTo(x, bottom - 11);
    ctx.lineTo(x + 4.5, bottom - 6.5);
    ctx.lineTo(x, bottom - 2);
    ctx.lineTo(x - 4.5, bottom - 6.5);
    ctx.closePath();
    ctx.fillStyle = palette.playhead;
    ctx.fill();
    ctx.strokeStyle = "rgba(0, 0, 0, 0.6)";
    ctx.lineWidth = 1;
    ctx.stroke();
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
  const palette = state.palette ?? DARK;
  const width = view.width;
  const top = view.top ?? 0;
  ctx.fillStyle = palette.background;
  ctx.fillRect(0, 0, width, height);
  if (!timeline) return;

  // Rows, then clips that overlap the visible time, under the ruler.
  const ordered = rows(timeline);
  const first = view.scroll * FLICKS_PER_SECOND;
  const last = (view.scroll + (width - HEADER) / view.zoom) * FLICKS_PER_SECOND;
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, RULER, width, height - RULER);
  ctx.clip();
  ordered.forEach((track, row) => {
    const y = rowY(row, top);
    if (y + ROW < RULER || y > height) return;
    ctx.fillStyle = row % 2 === 0 ? palette.row : palette.rowAlt;
    ctx.fillRect(HEADER, y, width - HEADER, ROW);
    for (const clip of track.clips) {
      if (clip.end.flicks < first || clip.start.flicks > last) continue;
      const x0 = Math.max(HEADER, timeToX(clip.start.flicks, view));
      const x1 = Math.min(width, timeToX(clip.end.flicks, view));
      if (x1 - x0 < 1) continue;
      const dragged = state.drag?.clip === clip.id;
      ctx.globalAlpha = dragged ? 0.3 : 1;
      const chosen = clip.id === state.selection || (state.also ?? []).includes(clip.id);
      block(ctx, clip, view, x0, x1, y, !dragged && chosen, state);
      if (clip.group && x1 - x0 > 10) {
        // A grouped clip: a short bar under its top edge.
        ctx.fillStyle = "rgba(255, 255, 255, 0.7)";
        ctx.fillRect(x0 + 5, y + ROW - INSET - 5, Math.min(16, x1 - x0 - 10), 2);
      }
      marks(ctx, clip, view, x0, x1, y, palette);
      ctx.globalAlpha = 1;
    }
    // Transitions sit on top of both clips they join.
    for (const clip of track.clips) {
      if (clip.end.flicks >= first && clip.end.flicks <= last) transitionMark(ctx, clip, view, y, palette);
    }
  });

  // Where a dragged clip would land.
  const drag = state.drag;
  const row = drag ? ordered.findIndex((t) => t.id === drag.track) : -1;
  const dragged = drag ? timeline.tracks.flatMap((t) => t.clips).find((c) => c.id === drag.clip) : undefined;
  if (drag && dragged && row >= 0) {
    const x0 = Math.max(HEADER, timeToX(drag.start, view));
    const x1 = Math.min(width, timeToX(drag.end, view));
    if (x1 - x0 >= 1) block(ctx, dragged, view, x0, x1, rowY(row, top), true, state);
  }

  // Where dropped media would land.
  const drop = state.drop;
  if (drop) {
    const x0 = Math.max(HEADER, timeToX(drop.start, view));
    const x1 = Math.min(width, timeToX(drop.end, view));
    const y = rowY(drop.row, top);
    if (x1 - x0 >= 1) {
      ctx.save();
      ctx.beginPath();
      ctx.roundRect(x0 + 0.5, y + INSET, x1 - x0 - 1, BLOCK, 6);
      ctx.fillStyle = palette.drop;
      ctx.fill();
      ctx.setLineDash([5, 4]);
      ctx.strokeStyle = palette.playhead;
      ctx.lineWidth = 1.5;
      ctx.stroke();
      ctx.restore();
    }
  }
  ctx.restore();

  // Ruler with labels on round steps and ticks between them.
  ctx.fillStyle = palette.ruler;
  ctx.fillRect(0, 0, width, RULER);
  ctx.fillStyle = palette.tick;
  ctx.fillRect(0, RULER - 1, width, 1);
  const step = rulerStep(view.zoom);
  const minor = step / 5;
  const start = Math.floor(view.scroll / step) * step;
  ctx.font = "10.5px system-ui, sans-serif";
  ctx.textBaseline = "middle";
  for (let seconds = start; ; seconds += minor) {
    const x = timeToX(seconds * FLICKS_PER_SECOND, view);
    if (x > width) break;
    if (x < HEADER) continue;
    const major = Math.abs(seconds / step - Math.round(seconds / step)) < 1e-6;
    ctx.fillStyle = palette.tick;
    ctx.fillRect(Math.round(x), major ? RULER - 10 : RULER - 5, 1, major ? 10 : 5);
    if (major) {
      ctx.fillStyle = palette.label;
      ctx.fillText(rulerLabel(seconds, step), x + 4, RULER / 2 - 2);
    }
  }

  // Markers: flags on the ruler.
  for (const marker of timeline.markers ?? []) {
    const mx = timeToX(marker.at.flicks, view);
    if (mx < HEADER || mx > width) continue;
    ctx.fillStyle = marker.color ?? palette.marker;
    ctx.beginPath();
    ctx.moveTo(mx, RULER - 15);
    ctx.lineTo(mx + 8, RULER - 11.5);
    ctx.lineTo(mx, RULER - 8);
    ctx.closePath();
    ctx.fill();
    ctx.fillRect(Math.round(mx), RULER - 15, 1.5, 15);
    ctx.globalAlpha = 0.35;
    ctx.fillRect(Math.round(mx), RULER, 1, height - RULER);
    ctx.globalAlpha = 1;
  }

  // The skimming line: where the preview shows while hovering.
  if (state.skim != null) {
    const sx = timeToX(state.skim, view);
    if (sx >= HEADER && sx <= width) {
      ctx.fillStyle = palette.skim;
      ctx.fillRect(Math.round(sx), RULER, 1, height - RULER);
    }
  }

  // Playhead: a line through the tracks and a handle on the ruler.
  const x = timeToX(state.playhead, view);
  if (x >= HEADER && x <= width) {
    ctx.fillStyle = palette.playhead;
    ctx.fillRect(Math.round(x) - 1, 4, 2, height - 4);
    ctx.beginPath();
    ctx.roundRect(x - 6, 2, 12, 12, 3);
    ctx.fill();
    ctx.beginPath();
    ctx.moveTo(x - 6, 11);
    ctx.lineTo(x + 6, 11);
    ctx.lineTo(x, 18);
    ctx.closePath();
    ctx.fill();
  }
}
