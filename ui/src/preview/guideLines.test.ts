import { describe, expect, it } from "vitest";

import { guideAt, guideToScreen, screenToGuide, ticks } from "./guideLines";

describe("guides", () => {
  const screen = { x: 10, y: 20, width: 400, height: 225 };

  it("places guides on the canvas and finds the one under the pointer", () => {
    expect(guideToScreen({ axis: "x", at: 0.5 }, screen)).toBe(210);
    expect(guideToScreen({ axis: "y", at: 1 }, screen)).toBe(245);
    expect(screenToGuide("x", 110, screen)).toEqual({ axis: "x", at: 0.25 });
    expect(screenToGuide("y", 0, screen), "kept on the canvas").toEqual({ axis: "y", at: 0 });
    const guides = [
      { axis: "x" as const, at: 0.5 },
      { axis: "y" as const, at: 0.2 },
    ];
    expect(guideAt(guides, screen, 213, 100)).toBe(0);
    expect(guideAt(guides, screen, 100, 66)).toBe(1);
    expect(guideAt(guides, screen, 100, 100)).toBe(-1);
  });

  it("spaces ruler ticks by round canvas pixels", () => {
    expect(ticks(1920, 400).slice(0, 3)).toEqual([0, 200, 400]);
    expect(ticks(1920, 1920).slice(0, 3), "at full size every 50").toEqual([0, 50, 100]);
    expect(ticks(0, 100)).toEqual([]);
  });
});
