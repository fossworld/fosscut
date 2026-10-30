import { describe, expect, it } from "vitest";

import { clip } from "../test/fixtures";
import { pictureFrame } from "./placement";
import { regionCorners, regionFromDrag } from "./region";

const close = (a: number[], b: number[]) => a.every((v, i) => Math.abs(v - b[i]!) < 1e-3);

// A 1920x1080 canvas shown 960x540 at the overlay's top-left: a full-frame 16:9 picture
// covers it exactly, so screen pixels halve into picture fractions of 960 and 540.
const screen = { x: 0, y: 0, width: 960, height: 540 };
const full = pictureFrame(clip("c1", "video", 0, 4, { picture: [1920, 1080] }), [1920, 1080], screen, 0)!;

describe("the region to track", () => {
  it("turns a drag on the preview into a centre and size in the picture", () => {
    const region = regionFromDrag(full, [96, 54], [288, 216]);
    expect(region).toEqual({ x: 0.2, y: 0.25, width: 0.2, height: 0.3 });
    expect(regionFromDrag(full, [288, 216], [96, 54]), "dragged up and left, the same region").toEqual(region);
  });

  it("measures in the picture, not the canvas, wherever the picture is placed", () => {
    // A square picture at half size, a quarter of the canvas right of centre.
    const square = clip("c2", "video", 0, 4, { picture: [1000, 1000], transform: { position: [0.25, 0], scale: 0.5 } });
    const frame = pictureFrame(square, [1920, 1080], screen, 0)!;
    expect(frame.center).toEqual([720, 270]);
    expect(frame.half).toEqual([135, 135]);
    // The picture's left half, top to bottom.
    expect(regionFromDrag(frame, [585, 135], [720, 405])).toEqual({ x: 0.25, y: 0.5, width: 0.5, height: 1 });
  });

  it("clips the region to the picture and ignores clicks", () => {
    expect(regionFromDrag(full, [-50, -50], [96, 54]), "only the part over the picture").toEqual({
      x: 0.05,
      y: 0.05,
      width: 0.1,
      height: 0.1,
    });
    expect(regionFromDrag(full, [100, 100], [102, 300]), "too narrow").toBeNull();
    expect(regionFromDrag(full, [100, 100], [100, 100]), "a click").toBeNull();
    expect(regionFromDrag(full, [-90, 10], [-10, 90]), "beside the picture").toBeNull();
  });

  it("follows a turned picture and a mirrored one", () => {
    // A quarter turn clockwise: right of the centre on the screen is above it in the picture.
    const turned = clip("c3", "video", 0, 4, { picture: [1080, 1080], transform: { rotation: 90 } });
    const frame = pictureFrame(turned, [1920, 1080], screen, 0)!;
    const region = regionFromDrag(frame, [480, 135], [615, 270])!;
    expect(close([region.x, region.y, region.width, region.height], [0.375, 0.375, 0.25, 0.25]), JSON.stringify(region)).toBe(true);
    // Mirrored, the left of the screen is the right of the picture.
    const mirrored = regionFromDrag(full, [96, 54], [288, 216], { horizontal: true });
    expect(mirrored).toEqual({ x: 0.8, y: 0.25, width: 0.2, height: 0.3 });
  });

  it("draws a region back where it was dragged", () => {
    const region = regionFromDrag(full, [96, 54], [288, 216], { vertical: true })!;
    const corners = regionCorners(full, region, { vertical: true });
    const xs = corners.map(([x]) => x);
    const ys = corners.map(([, y]) => y);
    expect(close([Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)], [96, 54, 288, 216])).toBe(true);
  });
});
