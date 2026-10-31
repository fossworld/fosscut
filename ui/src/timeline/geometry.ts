// Where things are on the timeline canvas: time and pixels, track rows, what a click hits,
// and zooming around the pointer. Pure, so it is tested without a browser.

import type { ClipView, TimelineView, TrackView } from "../engine/types";
import { FLICKS_PER_SECOND, type Rate, framesToFlicks } from "../transport/time";

/** Height of the time ruler at the top, in CSS pixels. */
export const RULER = 28;
/** Height of one track row. */
export const ROW = 50;
/** Width of the track headers on the left. */
export const HEADER = 152;
/** Zoom limits, in pixels per second. */
export const MIN_ZOOM = 2;
export const MAX_ZOOM = 800;

/** The visible window onto the timeline. */
export interface View {
  /** Pixels per second. */
  zoom: number;
  /** Seconds scrolled past on the left. */
  scroll: number;
  /** Canvas width in CSS pixels. */
  width: number;
  /** Pixels the tracks are scrolled up under the ruler; 0 when omitted. */
  top?: number;
}

/** The x position of `flicks`. */
export function timeToX(flicks: number, view: View): number {
  return HEADER + (flicks / FLICKS_PER_SECOND - view.scroll) * view.zoom;
}

/** The time at x, never before zero. */
export function xToTime(x: number, view: View): number {
  const seconds = (x - HEADER) / view.zoom + view.scroll;
  return Math.max(0, Math.round(seconds * FLICKS_PER_SECOND));
}

/**
 * Tracks top to bottom: the engine lists them bottom layer first, and the top layer is
 * drawn at the top, as in most editors.
 */
export function rows(timeline: TimelineView): TrackView[] {
  return [...timeline.tracks].reverse();
}

/** The top edge of row `row`, with the tracks scrolled up by `top` pixels. */
export function rowY(row: number, top = 0): number {
  return RULER + row * ROW - top;
}

/** The row at y, with the tracks scrolled up by `top` pixels; it may be past the last. */
export function rowAt(y: number, top = 0): number {
  return Math.floor((y - RULER + top) / ROW);
}

/** How far the tracks scroll: none when they fit in `height`, else until the last shows. */
export function maxTop(timeline: TimelineView | null, height: number): number {
  const tracks = timeline?.tracks.length ?? 0;
  return Math.max(0, RULER + tracks * ROW + ROW / 2 - height);
}

export interface Hit {
  track: TrackView;
  clip: ClipView | null;
}

/** What is at (x, y): a track row and the clip under the pointer, if any. */
export function hitTest(timeline: TimelineView, view: View, x: number, y: number): Hit | null {
  if (y < RULER || x < HEADER) return null;
  const ordered = rows(timeline);
  const track = ordered[rowAt(y, view.top)];
  if (!track) return null;
  const time = xToTime(x, view);
  const clip = track.clips.find((c) => c.start.flicks <= time && time < c.end.flicks) ?? null;
  return { track, clip };
}

/** True when y is on the time ruler, where dragging moves the playhead. */
export function onRuler(y: number): boolean {
  return y >= 0 && y < RULER;
}

/** Zooms by `factor` keeping the time under x where it is. */
export function zoomAround(view: View, factor: number, x: number): View {
  const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, view.zoom * factor));
  const anchor = (x - HEADER) / view.zoom + view.scroll;
  const scroll = Math.max(0, anchor - (x - HEADER) / zoom);
  return { ...view, zoom, scroll };
}

/** Scrolls by `pixels`, never before zero. */
export function scrollBy(view: View, pixels: number): View {
  return { ...view, scroll: Math.max(0, view.scroll + pixels / view.zoom) };
}

/** A zoom that shows the whole timeline in the canvas, with a little room after it. */
export function zoomToFit(timeline: TimelineView, width: number): View {
  const seconds = Math.max(timeline.duration.flicks / FLICKS_PER_SECOND, 1) * 1.1;
  const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, (width - HEADER) / seconds));
  return { zoom, scroll: 0, width };
}

/** Seconds between ruler labels: about one per 80 pixels, on a 1-2-5 step. */
export function rulerStep(zoom: number): number {
  const target = 80 / zoom;
  for (const step of [0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600]) {
    if (step >= target) return step;
  }
  return 3600;
}

/** The clip with this id, if the timeline has it. */
export function findClip(timeline: TimelineView, id: string): ClipView | null {
  for (const track of timeline.tracks) {
    const clip = track.clips.find((c) => c.id === id);
    if (clip) return clip;
  }
  return null;
}

/** Clips that `flicks` falls strictly inside, so they can be split there. */
export function clipsAt(timeline: TimelineView, flicks: number): ClipView[] {
  return timeline.tracks.flatMap((t) =>
    t.clips.filter((c) => c.start.flicks < flicks && flicks < c.end.flicks),
  );
}

/** A ruler label: minutes and seconds, with tenths when the step is below a second. */
export function rulerLabel(seconds: number, step: number): string {
  const whole = Math.floor(seconds + 1e-9);
  const minutes = Math.floor(whole / 60);
  const rest = String(whole % 60).padStart(2, "0");
  if (step >= 1) return `${minutes}:${rest}`;
  const tenths = Math.round((seconds - whole) * 10) % 10;
  return `${minutes}:${rest}.${tenths}`;
}

/** How close to a clip's edge, in pixels, a press grabs the edge instead of the clip. */
export const EDGE = 6;

export type DragMode = "move" | "trim-start" | "trim-end";

/** What a press at x on `clip` grabs: an edge to trim, or the clip to move. */
export function grabAt(clip: ClipView, view: View, x: number): DragMode {
  const left = timeToX(clip.start.flicks, view);
  const right = timeToX(clip.end.flicks, view);
  if (right - left > 3 * EDGE) {
    if (x - left <= EDGE) return "trim-start";
    if (right - x <= EDGE) return "trim-end";
  }
  return "move";
}

/** A drag in progress: what was grabbed and where the press began. */
export interface Drag {
  clip: ClipView;
  track: TrackView;
  mode: DragMode;
  fromX: number;
}

/** Where a drag would put the clip. */
export interface DragTarget {
  start: number;
  end: number;
  track: TrackView;
}

/** Whole frames moved by the pointer since the press. */
function framesMoved(drag: Drag, x: number, view: View, rate: Rate): number {
  const seconds = (x - drag.fromX) / view.zoom;
  return Math.round((seconds * rate.num) / rate.den);
}

/**
 * Where the clip lands with the pointer at (x, y), in whole frames: moved (onto another
 * track of the same kind under the pointer, when there is one), or with one edge moved,
 * never shorter than a frame or before zero.
 */
export function dragTarget(
  drag: Drag,
  timeline: TimelineView,
  view: View,
  x: number,
  y: number,
  rate: Rate,
  snaps: number[] = [],
): DragTarget {
  const { clip } = drag;
  const frame = framesToFlicks(1, rate);
  const delta = framesToFlicks(framesMoved(drag, x, view, rate), rate);
  if (drag.mode === "trim-start") {
    const wanted = snap(clip.start.flicks + delta, snaps, view);
    const start = Math.min(Math.max(0, wanted), clip.end.flicks - frame);
    return { start, end: clip.end.flicks, track: drag.track };
  }
  if (drag.mode === "trim-end") {
    const end = Math.max(clip.start.flicks + frame, snap(clip.end.flicks + delta, snaps, view));
    return { start: clip.start.flicks, end, track: drag.track };
  }
  // A moved clip snaps by whichever edge lands nearer a snap point.
  const moved = clip.start.flicks + delta;
  const byStart = snap(moved, snaps, view) - moved;
  const byEnd = snap(moved + clip.duration.flicks, snaps, view) - (moved + clip.duration.flicks);
  const shift = byStart === 0 ? byEnd : byEnd === 0 ? byStart : Math.abs(byStart) <= Math.abs(byEnd) ? byStart : byEnd;
  const start = Math.max(0, moved + shift);
  const under = rows(timeline)[rowAt(y, view.top)];
  const track = under && under.kind === drag.track.kind ? under : drag.track;
  return { start, end: start + clip.duration.flicks, track };
}

/** Where dragged edges snap: every other clip's edges, the playhead, markers and zero. */
export function snapPoints(timeline: TimelineView, except: string, playhead: number): number[] {
  const points = [0, playhead, ...(timeline.markers ?? []).map((m) => m.at.flicks)];
  for (const track of timeline.tracks) {
    for (const clip of track.clips) {
      if (clip.id !== except) points.push(clip.start.flicks, clip.end.flicks);
    }
  }
  return points;
}

/** `time`, or the nearest of `points` within `pixels` of it on screen. */
export function snap(time: number, points: number[], view: View, pixels = 8): number {
  let best = time;
  let distance = pixels;
  for (const point of points) {
    const away = (Math.abs(point - time) / FLICKS_PER_SECOND) * view.zoom;
    if (away <= distance) {
      best = point;
      distance = away;
    }
  }
  return best;
}

/** True when a drag would change nothing. */
export function unchanged(drag: Drag, target: DragTarget): boolean {
  return (
    target.start === drag.clip.start.flicks && target.end === drag.clip.end.flicks && target.track.id === drag.track.id
  );
}
