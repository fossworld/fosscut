import { describe, expect, it } from "vitest";

import type { ProjectSummary } from "../engine/types";
import { matching, readableSize } from "./Start";

describe("project browser", () => {
  it("writes sizes as people read them", () => {
    expect(readableSize(512)).toBe("512 bytes");
    expect(readableSize(12_300)).toBe("12 KB");
    expect(readableSize(3_400_000)).toBe("3.4 MB");
    expect(readableSize(5_000_000_000)).toBe("5.0 GB");
  });

  it("finds projects by every word of their name or folder", () => {
    const project = (name: string, path: string): ProjectSummary => ({
      path,
      name,
      duration: { flicks: 0, frames: 0, seconds: "0", timecode: "00:00:00:00" },
      timelines: 1,
      assets: 0,
      bytes: 0,
      modified_ms: 0,
    });
    const all = [project("Trip", "/work/2026/Trip.fosscut"), project("Launch video", "/clients/acme/Launch.fosscut")];
    expect(matching(all, "").map((p) => p.name)).toEqual(["Trip", "Launch video"]);
    expect(matching(all, "launch ACME").map((p) => p.name)).toEqual(["Launch video"]);
    expect(matching(all, "2026").map((p) => p.name)).toEqual(["Trip"]);
    expect(matching(all, "beach")).toEqual([]);
  });
});
