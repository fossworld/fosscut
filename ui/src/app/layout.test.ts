import { describe, expect, it } from "vitest";

import { ASPECTS, DEFAULT_LAYOUT, LIMITS, aspectName, clampLayout, parseLayout } from "./layout";

describe("layout", () => {
  it("names canvas shapes as people do", () => {
    expect(aspectName(1920, 1080)).toBe("16:9");
    expect(aspectName(1080, 1920)).toBe("9:16");
    expect(aspectName(1080, 1080)).toBe("1:1");
    expect(aspectName(1080, 1350)).toBe("4:5");
    expect(aspectName(3840, 1608)).toBe("2.39:1");
    expect(aspectName(1000, 300), "others as their reduced ratio").toBe("10:3");
    expect(aspectName(0, 100)).toBe("");
    for (const aspect of ASPECTS) expect(aspectName(aspect.width, aspect.height)).toBe(aspect.id);
  });

  it("keeps panels within their limits and leaves the preview room", () => {
    const tiny = clampLayout({ ...DEFAULT_LAYOUT, browser: 10, inspector: 10_000, timeline: 5 });
    expect(tiny.browser).toBe(LIMITS.browser[0]);
    expect(tiny.inspector).toBe(LIMITS.inspector[1]);
    expect(tiny.timeline).toBe(LIMITS.timeline[0]);
    const narrow = clampLayout({ ...DEFAULT_LAYOUT, browser: 500, inspector: 500 }, 1000, 800);
    expect(narrow.browser + narrow.inspector).toBeLessThanOrEqual(Math.max(667, LIMITS.browser[0] + LIMITS.inspector[0]));
    const hidden = clampLayout({ ...DEFAULT_LAYOUT, browser: 500, inspector: 500, showBrowser: false, showInspector: false }, 1000, 800);
    expect(hidden.browser, "hidden panels keep their size").toBe(500);
    expect(clampLayout({ ...DEFAULT_LAYOUT, timeline: 600 }, 1440, 700).timeline).toBe(700 - 48 - 260);
  });

  it("reads a stored layout, or the default for anything unreadable", () => {
    expect(parseLayout(null)).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout("{oops")).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout(JSON.stringify({ browser: 300, showInspector: false, timeline: "tall" }))).toEqual({
      ...DEFAULT_LAYOUT,
      browser: 300,
      showInspector: false,
    });
  });
});
