import { afterEach, describe, expect, it } from "vitest";

import type { AssetView, LibraryItem, LibraryKind } from "../engine/types";
import { setLanguage, t } from "../i18n";
import { clip, timeline, track } from "../test/fixtures";
import {
  EVERYTHING,
  KINDS,
  addPlan,
  counts,
  itemDetail,
  kindName,
  kindPlural,
  placingOf,
  seconds,
  shownItems,
  titleOf,
  trackOf,
} from "./libraryFilter";

const item = (name: string, kind: LibraryKind, title: string, description: string, extra: Partial<LibraryItem> = {}): LibraryItem => ({
  name,
  kind,
  title,
  description,
  tags: [],
  commercial: true,
  ...extra,
});

const items = [
  item("badge-sale", "sticker", "Sale badge", "A yellow burst with SALE in black capitals.", { tags: ["promo", "discount"] }),
  item("circle", "shape", "Circle", "A circle."),
  item("sunset", "background", "Sunset", "Warm orange fading to pink."),
  item("lower-third", "title", "lower-third", "A name and a role at the lower left."),
  item("highlight", "text_style", "highlight", "Black letters on a bright yellow box."),
  item("highlight", "caption_style", "highlight", "The spoken word in a yellow box."),
  item("teal_orange", "look", "Teal and orange", "Cool shadows, warm skin."),
  item("vertical-promo", "template", "vertical-promo", "A 9:16 promo."),
  item("whoosh", "sound", "Whoosh", "A quick rush of air.", { seconds: 0.62 }),
  item("lofi-beat", "music", "Lo-fi beat", "A relaxed loop.", { seconds: 16, bpm: 90 }),
];

const asset = (id: string, kind: AssetView["kind"]): AssetView => ({ id, kind, name: id, used_by: [] });

describe("library filter", () => {
  afterEach(() => setLanguage("en"));
  const names = (list: LibraryItem[]) => list.map((i) => `${i.kind}:${i.name}`);

  it("searches every word in names, titles, descriptions and tags, within a kind", () => {
    expect(shownItems(items, EVERYTHING)).toHaveLength(items.length);
    expect(names(shownItems(items, { ...EVERYTHING, search: "YELLOW box" }))).toEqual([
      "text_style:highlight",
      "caption_style:highlight",
    ]);
    expect(names(shownItems(items, { kind: "caption_style", search: "yellow" }))).toEqual(["caption_style:highlight"]);
    expect(names(shownItems(items, { ...EVERYTHING, search: "discount" })), "tags count").toEqual(["sticker:badge-sale"]);
    expect(names(shownItems(items, { ...EVERYTHING, search: "  sale   badge " }))).toEqual(["sticker:badge-sale"]);
    expect(shownItems(items, { kind: "music", search: "whoosh" })).toEqual([]);
  });

  it("counts kinds, names them and reads names used as titles as words", () => {
    const all = counts(items);
    expect(all.sticker).toBe(1);
    expect(Object.keys(all)).toEqual(KINDS.map((k) => k.kind));
    expect(counts([]).music, "kinds without items count zero").toBe(0);
    expect(kindName("music")).toBe(t("media.library.one.music"));
    expect(kindPlural("text_style")).toBe(t("media.library.plural.text_style"));
    expect(kindName("music")).toBe("Music loop");
    setLanguage("hu");
    expect([kindName("music"), kindPlural("sticker")]).toEqual(["Zenei hurok", "Matricák"]);
    setLanguage("en");
    expect(titleOf(items[3]!)).toBe("Lower third");
    expect(titleOf(items[6]!), "a real title stays").toBe("Teal and orange");
  });

  it("gives a sound's length and a music loop's tempo", () => {
    expect(itemDetail(items[8]!)).toBe(t("media.library.seconds", { seconds: 0.6 }));
    expect(itemDetail(items[9]!)).toBe(t("media.library.tempo", { length: seconds(16), bpm: 90 }));
    expect(itemDetail(items[9]!)).toBe("16 s at 90 BPM");
    expect(itemDetail(items[0]!)).toBe("");
    expect([seconds(2.54), seconds(59.6), seconds(60), seconds(125)]).toEqual([
      t("media.library.seconds", { seconds: 2.5 }),
      t("media.library.seconds", { seconds: 60 }),
      t("media.library.minutes", { minutes: 1 }),
      t("media.library.minutesSeconds", { minutes: 2, seconds: 5 }),
    ]);
    setLanguage("hu");
    expect([itemDetail(items[8]!), itemDetail(items[9]!)]).toEqual(["0,6 mp", "16 mp, 90 BPM"]);
  });

  it("adds at the playhead, looks on the selected clip and templates with the chosen media", () => {
    const view = timeline();
    const at = placingOf(view, "clip_0003", 1234, [asset("asset_0001", "video"), asset("asset_0009", "audio")], [
      "asset_0009",
      "asset_0001",
      "asset_gone",
    ]);
    expect(at.selected?.id).toBe("clip_0003");
    expect(at.chosen.map((a) => a.id), "in the order chosen, gone ones left out").toEqual(["asset_0009", "asset_0001"]);
    expect(addPlan(items[0]!, at)).toEqual({
      request: { name: "badge-sale", kind: "sticker", start_flicks: 1234 },
      does: t("media.library.add.does"),
    });
    expect(addPlan(items[8]!, at)).toMatchObject({ request: { name: "whoosh", kind: "sound", start_flicks: 1234 } });
    expect(addPlan(items[6]!, at)).toEqual({
      request: { name: "teal_orange", kind: "look", clip: "clip_0003" },
      does: t("media.library.look.does"),
    });
    expect(addPlan(items[7]!, at), "only video and images fill a template's slots").toEqual({
      request: { name: "vertical-promo", kind: "template", media: ["asset_0001"] },
      does: t("media.library.template.with", { count: 1 }),
    });
    const two = placingOf(view, null, 0, [asset("asset_0001", "video"), asset("asset_0002", "image")], ["asset_0001", "asset_0002"]);
    expect(addPlan(items[7]!, two)).toMatchObject({ does: "Build a new timeline with the 2 chosen videos and images" });

    const nothing = placingOf(view, null, 0, [], []);
    expect(addPlan(items[6]!, nothing)).toEqual({ blocked: t("media.library.look.blocked") });
    expect(addPlan(items[7]!, nothing)).toEqual({
      request: { name: "vertical-promo", kind: "template" },
      does: t("media.library.template.does"),
    });
    const sound = placingOf(
      { ...view, tracks: [track("track_0007", "audio", 0, [clip("clip_0008", "audio", 0, 2)])] },
      "clip_0008",
      0,
      [],
      [],
    );
    expect(addPlan(items[6]!, sound), "a look needs a picture").toHaveProperty("blocked");
    expect(placingOf(null, "clip_0003", 0, [], []).selected).toBeNull();
  });

  it("restyles the selected caption track, or the first, and needs captions", () => {
    const view = timeline();
    expect(addPlan(items[5]!, placingOf(view, null, 0, [], []))).toEqual({
      blocked: t("media.library.captions.blocked"),
    });
    const captioned = {
      ...view,
      tracks: [
        ...view.tracks,
        track("track_0010", "caption", 2, [clip("clip_0011", "caption", 0, 1)]),
        track("track_0012", "caption", 3, [clip("clip_0013", "caption", 0, 1)]),
      ],
    };
    expect(addPlan(items[5]!, placingOf(captioned, null, 0, [], []))).toEqual({
      request: { name: "highlight", kind: "caption_style", track: "track_0010" },
      does: t("media.library.captions.does"),
    });
    expect(addPlan(items[5]!, placingOf(captioned, "clip_0013", 0, [], []))).toMatchObject({
      request: { track: "track_0012" },
    });
    expect(trackOf(captioned, "clip_0013")).toBe("track_0012");
    expect(trackOf(captioned, "clip_9999")).toBeNull();
  });
});
