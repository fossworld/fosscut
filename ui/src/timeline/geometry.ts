// Where things are on the timeline canvas: time and pixels, track rows, what a click hits,
// and zooming around the pointer. Pure, so it is tested without a browser.

import type { ClipView, TimelineView, TrackView } from "../engine/types";
import { FLICKS_PER_SECOND, type Rate, framesToFlicks } from "../transport/time";

/** Height of the time ruler at the top, in CSS pixels. */
export const RULER = 24;
/** Height of one track row. */
export const ROW = 46;
/** Width of the track labels on the left. */
export const HEADER = 96;
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

/** The top edge of row `row`. */
export function rowY(row: number): number {
  return RULER + row * ROW;
}

export interface Hit {
  track: TrackView;
  clip: ClipView | null;
}

/** What is at (x, y): a track row and the clip under the pointer, if any. */
export function hitTest(timeline: TimelineView, view: View, x: number, y: number): Hit | null {
  if (y < RULER || x < HEADER) return null;
  const ordered = rows(timeline);
  const track = ordered[Math.floor((y - RULER) / ROW)];
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
export function dragTarget(drag: Drag, timeline: TimelineView, view: View, x: number, y: number, rate: Rate): DragTarget {
  const { clip } = drag;
  const frame = framesToFlicks(1, rate);
  const delta = framesToFlicks(framesMoved(drag, x, view, rate), rate);
  if (drag.mode === "trim-start") {
    const start = Math.min(Math.max(0, clip.start.flicks + delta), clip.end.flicks - frame);
    return { start, end: clip.end.flicks, track: drag.track };
  }
  if (drag.mode === "trim-end") {
    const end = Math.max(clip.start.flicks + frame, clip.end.flicks + delta);
    return { start: clip.start.flicks, end, track: drag.track };
  }
  const start = Math.max(0, clip.start.flicks + delta);
  const under = rows(timeline)[Math.floor((y - RULER) / ROW)];
  const track = under && under.kind === drag.track.kind ? under : drag.track;
  return { start, end: start + clip.duration.flicks, track };
}

/** True when a drag would change nothing. */
export function unchanged(drag: Drag, target: DragTarget): boolean {
  return (
    target.start === drag.clip.start.flicks && target.end === drag.clip.end.flicks && target.track.id === drag.track.id
  );
}
