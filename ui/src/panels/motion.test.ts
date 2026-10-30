import { describe, expect, it } from "vitest";

import { clip, timeline, track } from "../test/fixtures";
import { nextClip } from "./Motion";

describe("motion", () => {
  it("finds the clip a transition leads into only when it touches", () => {
    const view = {
      ...timeline(),
      tracks: [
        track("track_0002", "video", 0, [
          clip("clip_0003", "video", 0, 4),
          clip("clip_0004", "video", 4, 6),
          clip("clip_0005", "video", 7, 9),
        ]),
      ],
    };
    const [first, second, third] = view.tracks[0]!.clips;
    expect(nextClip(view, first!)?.id).toBe("clip_0004");
    expect(nextClip(view, second!), "a gap before the third").toBeNull();
    expect(nextClip(view, third!), "the last clip").toBeNull();
    expect(nextClip(view, clip("clip_0099", "video", 0, 1)), "not on the timeline").toBeNull();
  });
});
