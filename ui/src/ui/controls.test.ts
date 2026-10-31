import { describe, expect, it } from "vitest";

import { stripEnds } from "./controls";

describe("stripEnds", () => {
  it("shows a way on while categories are out of sight past the end", () => {
    // Ten categories, 542 px of them, in a 378 px panel.
    expect(stripEnds(0, 378, 542)).toEqual({ start: false, end: true });
    expect(stripEnds(80, 378, 542)).toEqual({ start: true, end: true });
    expect(stripEnds(164, 378, 542)).toEqual({ start: true, end: false });
  });

  it("shows nothing when everything fits, or a pixel of rounding remains", () => {
    expect(stripEnds(0, 378, 378)).toEqual({ start: false, end: false });
    expect(stripEnds(0.5, 378, 379)).toEqual({ start: false, end: false });
  });
});
