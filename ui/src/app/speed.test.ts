import { describe, expect, it } from "vitest";

import { clip } from "../test/fixtures";
import { speedLabel } from "./speed";

describe("speed", () => {
  it("labels rates, curves, reversal and freezes", () => {
    const at = (speed: Parameters<typeof clip>[4]) => speedLabel(clip("clip_0003", "video", 0, 4, speed));
    expect(at({})).toBeNull();
    expect(at({ speed: { rate: 2 } })).toBe("2x");
    expect(at({ speed: { rate: 0.333 } })).toBe("0.33x");
    expect(at({ speed: { reverse: true } })).toBe("1x, reversed");
    expect(at({ speed: { curve: [{ at: 0, rate: 1 }, { at: 1, rate: 2 }] } })).toBe("curve");
    expect(at({ speed: { curve: [{ at: 0, rate: 1 }], reverse: true } })).toBe("curve, reversed");
    expect(at({ speed: { rate: 0 } })).toBe("freeze");
  });
});
