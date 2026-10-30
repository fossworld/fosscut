import { describe, expect, it } from "vitest";

import type { AssetView } from "../engine/types";
import { ALL, shown } from "./mediaFilter";

const asset = (id: string, name: string, kind: AssetView["kind"], seconds: number, folder?: string): AssetView => ({
  id,
  kind,
  name,
  duration: { flicks: seconds * 705_600_000, frames: 0, seconds: String(seconds), timecode: "" },
  used_by: [],
  folder,
});

describe("media filter", () => {
  const assets = [
    asset("asset_0001", "Interview 10.mp4", "video", 60, "Footage"),
    asset("asset_0002", "interview 2.mp4", "video", 30, "Footage/Day 1"),
    asset("asset_0003", "Music bed.wav", "audio", 120),
    asset("asset_0004", "film.cube", "lut", 0, "Looks"),
  ];
  const ids = (list: AssetView[]) => list.map((a) => a.id);

  it("searches every word, filters kinds and folders", () => {
    expect(ids(shown(assets, ALL))).toEqual(["asset_0001", "asset_0002", "asset_0003", "asset_0004"]);
    expect(ids(shown(assets, { ...ALL, search: "INTERVIEW 2" }))).toEqual(["asset_0002"]);
    expect(ids(shown(assets, { ...ALL, kind: "audio" }))).toEqual(["asset_0003"]);
    expect(ids(shown(assets, { ...ALL, folder: "Footage" })), "with subfolders").toEqual(["asset_0001", "asset_0002"]);
    expect(ids(shown(assets, { ...ALL, folder: "" })), "the top level").toEqual(["asset_0003"]);
    expect(ids(shown(assets, { ...ALL, folder: "Foot" })), "not a prefix of a name").toEqual([]);
  });

  it("sorts by name with numbers in order, by length and by kind", () => {
    expect(ids(shown(assets, { ...ALL, sort: "name" }))).toEqual(["asset_0004", "asset_0002", "asset_0001", "asset_0003"]);
    expect(ids(shown(assets, { ...ALL, sort: "duration" }))).toEqual(["asset_0003", "asset_0001", "asset_0002", "asset_0004"]);
    expect(ids(shown(assets, { ...ALL, sort: "kind" }))).toEqual(["asset_0003", "asset_0004", "asset_0002", "asset_0001"]);
  });
});
