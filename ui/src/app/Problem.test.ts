import { describe, expect, it } from "vitest";

import { problemText } from "./Problem";

describe("problemText", () => {
  it("makes the engine's message a sentence before its hint", () => {
    expect(
      problemText({ code: "fosscut.analysis.missing", message: "the transcript analysis of asset_0001 is not cached", hint: "Analyze it first." }),
    ).toBe("The transcript analysis of asset_0001 is not cached. Analyze it first.");
  });

  it("keeps a message that already ends a sentence, and one without a hint", () => {
    expect(problemText({ code: "x", message: "Nothing to do.", hint: "Add clips." })).toBe("Nothing to do. Add clips.");
    expect(problemText({ code: "x", message: "the clip is gone" })).toBe("The clip is gone");
  });
});
