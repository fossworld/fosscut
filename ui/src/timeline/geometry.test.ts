import { describe, expect, it } from "vitest";

import { timeline } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import {
  HEADER,
  MAX_ZOOM,
  MIN_ZOOM,
  ROW,
  RULER,
  clipsAt,
  findClip,
  hitTest,
  onRuler,
  rowY,
  rows,
  rulerLabel,
  rulerStep,
  scrollBy,
  timeToX,
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
