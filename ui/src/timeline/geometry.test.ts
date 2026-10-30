import { describe, expect, it } from "vitest";

import type { TrackView } from "../engine/types";
import { timeline, track } from "../test/fixtures";
import { FLICKS_PER_SECOND, framesToFlicks } from "../transport/time";
import {
  HEADER,
  MAX_ZOOM,
  MIN_ZOOM,
  ROW,
  RULER,
  clipsAt,
  dragTarget,
  findClip,
  grabAt,
  hitTest,
  onRuler,
  rowY,
  rows,
  rulerLabel,
  rulerStep,
  scrollBy,
  snap,
  snapPoints,
  timeToX,
  unchanged,
  xToTime,
  zoomAround,
  zoomToFit,
} from "./geometry";

const view = { zoom: 100, scroll: 1, width: 1000 };

describe("timeline geometry", () => {
  it("maps time to x and back, with the scroll and the track labels", () => {
    expect(timeToX(FLICKS_PER_SECOND, view)).toBe(HEADER);
    expect(timeToX(3 * FLICKS_PER_SECOND, view)).toBe(HEADER + 200);
    expect(xToTime(HEADER + 200, view)).toBe(3 * FLICKS_PER_SECOND);
    expect(xToTime(0, { ...view, scroll: 0 }), "never before zero").toBe(0);
  });

  it("draws the top layer at the top and hits the clip under the pointer", () => {
    const t = timeline();
    expect(rows(t).map((r) => r.id)).toEqual(["track_0004", "track_0002"]);
    expect(rowY(1)).toBe(RULER + ROW);
    const flat = { zoom: 100, scroll: 0, width: 1000 };
    const title = hitTest(t, flat, HEADER + 50, RULER + 5);
    expect(title?.clip?.id).toBe("clip_0005");
    const emptyPart = hitTest(t, flat, HEADER + 300, RULER + 5);
    expect(emptyPart?.track.id).toBe("track_0004");
    expect(emptyPart?.clip).toBeNull();
    expect(hitTest(t, flat, HEADER + 300, RULER + ROW + 5)?.clip?.id).toBe("clip_0003");
    expect(hitTest(t, flat, HEADER + 50, RULER + 3 * ROW)).toBeNull();
    expect(hitTest(t, flat, 10, RULER + 5), "the labels are not clips").toBeNull();
    expect(hitTest(t, flat, HEADER + 50, 5), "nor is the ruler").toBeNull();
    expect(onRuler(5) && !onRuler(RULER)).toBe(true);
  });

  it("zooms around the pointer within limits, and scrolls", () => {
    const zoomed = zoomAround(view, 2, HEADER + 100);
    expect(zoomed.zoom).toBe(200);
    expect(xToTime(HEADER + 100, zoomed)).toBe(xToTime(HEADER + 100, view));
    expect(zoomAround(view, 1e6, HEADER).zoom).toBe(MAX_ZOOM);
    expect(zoomAround(view, 1e-6, HEADER).zoom).toBe(MIN_ZOOM);
    expect(zoomAround({ ...view, scroll: 0 }, 0.5, HEADER + 400).scroll).toBe(0);
    expect(scrollBy(view, 100).scroll).toBe(2);
    expect(scrollBy(view, -1000).scroll).toBe(0);
  });

  it("fits the whole timeline and labels the ruler on round steps", () => {
    const fit = zoomToFit(timeline(), HEADER + 660);
    expect(fit.zoom).toBeCloseTo(100);
    expect(fit.scroll).toBe(0);
    expect(rulerStep(100)).toBe(1);
    expect(rulerStep(30)).toBe(5);
    expect(rulerStep(800)).toBe(0.1);
    expect(rulerStep(0.001)).toBe(3600);
    expect(rulerLabel(0, 1)).toBe("0:00");
    expect(rulerLabel(65, 5)).toBe("1:05");
    expect(rulerLabel(2.5, 0.5)).toBe("0:02.5");
    expect(rulerLabel(0.30000000000000004, 0.1)).toBe("0:00.3");
  });

  it("finds clips by id and those a time falls inside", () => {
    const t = timeline();
    expect(findClip(t, "clip_0005")?.text).toBe("Live edit");
    expect(findClip(t, "clip_9")).toBeNull();
    expect(clipsAt(t, FLICKS_PER_SECOND).map((c) => c.id)).toEqual(["clip_0003", "clip_0005"]);
    expect(clipsAt(t, 2 * FLICKS_PER_SECOND).map((c) => c.id), "not at an edge").toEqual(["clip_0003"]);
    expect(clipsAt(t, 0)).toEqual([]);
  });
});

describe("dragging clips", () => {
  const rate = { num: 30, den: 1 };
  const flat = { zoom: 100, scroll: 0, width: 1000 };
  const second = FLICKS_PER_SECOND;

  it("grabs edges near them and the clip elsewhere", () => {
    const video = timeline().tracks[0]!.clips[0]!;
    expect(grabAt(video, flat, HEADER + 2)).toBe("trim-start");
    expect(grabAt(video, flat, HEADER + 597)).toBe("trim-end");
    expect(grabAt(video, flat, HEADER + 300)).toBe("move");
    const narrow = { ...flat, zoom: 2 };
    expect(grabAt(video, narrow, HEADER + 1), "too narrow to have edges").toBe("move");
  });

  it("moves clips by whole frames, onto a track of the same kind under the pointer", () => {
    const t = {
      ...timeline(),
      tracks: [...timeline().tracks, track("track_0009", "video", 2, [])],
    };
    const [video, title] = t.tracks as [TrackView, TrackView, TrackView];
    const drag = { clip: video.clips[0]!, track: video, mode: "move" as const, fromX: HEADER + 100 };
    const moved = dragTarget(drag, t, flat, HEADER + 151.7, RULER + 2 * ROW + 5, rate);
    expect(moved.start, "half a second and a frame, rounded to frames").toBe(framesToFlicks(16, rate));
    expect(moved.end - moved.start).toBe(6 * second);
    expect(moved.track.id, "same row: still the video track").toBe("track_0002");
    expect(dragTarget(drag, t, flat, HEADER + 150, RULER + 5, rate).track.id, "a video row on top").toBe("track_0009");
    expect(dragTarget(drag, t, flat, HEADER + 150, RULER + ROW + 5, rate).track.id, "titles take no video").toBe("track_0002");
    expect(dragTarget(drag, t, flat, HEADER - 500, RULER + 5, rate).start, "never before zero").toBe(0);
    expect(unchanged(drag, dragTarget(drag, t, flat, HEADER + 100, RULER + 2 * ROW + 5, rate))).toBe(true);
    expect(title.kind).toBe("text");
  });

  it("trims edges by whole frames, never below one frame", () => {
    const t = timeline();
    const video = t.tracks[0]!;
    const clip = video.clips[0]!;
    const start = { clip, track: video, mode: "trim-start" as const, fromX: HEADER };
    expect(dragTarget(start, t, flat, HEADER + 100, 0, rate)).toMatchObject({ start: second, end: 6 * second });
    expect(dragTarget(start, t, flat, HEADER + 5000, 0, rate).start).toBe(6 * second - framesToFlicks(1, rate));
    expect(dragTarget(start, t, flat, HEADER - 100, 0, rate).start).toBe(0);
    const end = { clip, track: video, mode: "trim-end" as const, fromX: HEADER + 600 };
    expect(dragTarget(end, t, flat, HEADER + 400, 0, rate)).toMatchObject({ start: 0, end: 4 * second });
    expect(dragTarget(end, t, flat, HEADER + 800, 0, rate).end, "longer, if the media allows").toBe(8 * second);
    expect(dragTarget(end, t, flat, HEADER - 1000, 0, rate).end).toBe(framesToFlicks(1, rate));
  });

  it("snaps dragged edges to nearby clip edges, the playhead and markers", () => {
    const t = { ...timeline(), markers: [{ id: "marker_0001", at: { flicks: 3 * second, frames: 90, seconds: "3", timecode: "" } }] };
    const video = t.tracks[0]!;
    const clip = video.clips[0]!;
    const points = snapPoints(t, clip.id, 7 * second);
    expect(points).toContain(2 * second);
    expect(points, "markers").toContain(3 * second);
    expect(points, "the playhead").toContain(7 * second);
    expect(points, "not its own edges").not.toContain(6 * second);
    // 8 pixels at 100 pixels a second is 0.08 s.
    expect(snap(2.05 * second, points, flat)).toBe(2 * second);
    expect(snap(2.1 * second, points, flat), "too far").toBe(2.1 * second);
    const end = { clip, track: video, mode: "trim-end" as const, fromX: HEADER + 600 };
    expect(dragTarget(end, t, flat, HEADER + 696, 0, rate, points).end, "onto the playhead").toBe(7 * second);
    const move = { clip, track: video, mode: "move" as const, fromX: HEADER };
    expect(dragTarget(move, t, flat, HEADER + 296, RULER + 5, rate, points).start, "the start onto the marker").toBe(3 * second);
    expect(dragTarget(move, t, flat, HEADER + 296, RULER + 5, rate).start, "no snapping without points").not.toBe(3 * second);
  });
});
