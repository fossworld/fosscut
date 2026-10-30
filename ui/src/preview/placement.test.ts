import { describe, expect, it } from "vitest";

import { clip, moment } from "../test/fixtures";
import { fitCanvas, maskToPicture, pictureFrame, pictureToMask, toPicture, toScreen } from "./placement";

const close = (a: number[], b: number[]) => a.every((v, i) => Math.abs(v - b[i]!) < 1e-9);

describe("placement", () => {
  it("letterboxes the canvas like the desktop preview", () => {
    expect(fitCanvas({ x: 0, y: 0, width: 400, height: 300 }, 16 / 9)).toEqual({ x: 0, y: 37.5, width: 400, height: 225 });
    expect(fitCanvas({ x: 10, y: 0, width: 400, height: 100 }, 1)).toEqual({ x: 160, y: 0, width: 100, height: 100 });
    expect(fitCanvas({ x: 0, y: 0, width: 0, height: 100 }, 1).width).toBe(0);
  });

  it("places a picture by fit and transform, and maps points both ways", () => {
    const screen = { x: 0, y: 0, width: 960, height: 540 };
    const square = clip("c1", "video", 0, 2, {
      picture: [1000, 1000],
      transform: { position: [0.25, 0], scale: 0.5, rotation: 90 },
    });
    const frame = pictureFrame(square, [1920, 1080], screen, 0)!;
    expect(frame.center).toEqual([720, 270]);
    expect(frame.half, "fitted to the height, then halved").toEqual([135, 135]);
    expect(frame.rotation).toBe(90);
    const corner = toScreen(frame, [0, 0]);
    expect(close(corner, [855, 135]), `a quarter turn moves the top-left corner right: ${corner}`).toBe(true);
    expect(close(toPicture(frame, corner), [0, 0])).toBe(true);
    expect(pictureFrame(clip("t", "text", 0, 1), [1920, 1080], screen, 0)).toBeNull();
    const animated = clip("c2", "video", 0, 2, {
      picture: [1920, 1080],
      keyframes: {
        "transform.scale": [
          { at: moment(0), offset: moment(0), value: 1, interpolation: "linear" },
          { at: moment(2), offset: moment(2), value: 2, interpolation: "linear" },
        ],
      },
    });
    expect(pictureFrame(animated, [1920, 1080], screen, moment(1).flicks)!.half).toEqual([720, 405]);
  });

  it("turns mask box points into picture points and back", () => {
    const center: [number, number] = [0.3, 0.6];
    const size: [number, number] = [0.4, 0.2];
    expect(close(maskToPicture([1, 0], center, size, 0, 16 / 9), [0.5, 0.6])).toBe(true);
    expect(close(maskToPicture([0, -1], center, size, 0, 16 / 9), [0.3, 0.5])).toBe(true);
    for (const rotation of [0, 30, 90, -45]) {
      const q: [number, number] = [0.4, -0.7];
      const back = pictureToMask(maskToPicture(q, center, size, rotation, 16 / 9), center, size, rotation, 16 / 9);
      expect(close(back, q), `rotation ${rotation}`).toBe(true);
    }
  });
});
