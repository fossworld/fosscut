// What the library panel shows: the built-in items of a kind that match a search, and
// what adding one does where the editor stands (ADR-0028).

import type { AssetView, ClipView, LibraryAddRequest, LibraryItem, LibraryKind, TimelineView } from "../engine/types";

/** The kinds in the order the panel offers them, with their names in the plural and one. */
export const KINDS: { kind: LibraryKind; plural: string; one: string }[] = [
  { kind: "sticker", plural: "Stickers", one: "Sticker" },
  { kind: "shape", plural: "Shapes", one: "Shape" },
  { kind: "background", plural: "Backgrounds", one: "Background" },
  { kind: "title", plural: "Titles", one: "Title" },
  { kind: "text_style", plural: "Text styles", one: "Text style" },
  { kind: "caption_style", plural: "Caption styles", one: "Caption style" },
  { kind: "look", plural: "Looks", one: "Look" },
  { kind: "template", plural: "Templates", one: "Template" },
  { kind: "sound", plural: "Sound effects", one: "Sound effect" },
  { kind: "music", plural: "Music", one: "Music loop" },
];

export interface LibraryFilter {
  /** One kind, or every kind when empty. */
  kind: "" | LibraryKind;
  /** Words that must all appear in the name, title, description or tags. */
  search: string;
}

export const EVERYTHING: LibraryFilter = { kind: "", search: "" };

/** The items `filter` shows, in the library's order, searched as the engine searches. */
export function shownItems(items: LibraryItem[], filter: LibraryFilter): LibraryItem[] {
  const words = filter.search.toLowerCase().split(/\s+/).filter(Boolean);
  return items.filter((item) => {
    if (filter.kind && item.kind !== filter.kind) return false;
    const text = `${item.name} ${item.title} ${item.description} ${item.tags.join(" ")}`.toLowerCase();
    return words.every((word) => text.includes(word));
  });
}

/** How many items there are of each kind. */
export function counts(items: LibraryItem[]): Record<LibraryKind, number> {
  const all = Object.fromEntries(KINDS.map(({ kind }) => [kind, 0])) as Record<LibraryKind, number>;
  for (const item of items) all[item.kind] = (all[item.kind] ?? 0) + 1;
  return all;
}

/** The name of one item of `kind`. */
export function kindName(kind: LibraryKind): string {
  return KINDS.find((k) => k.kind === kind)?.one ?? kind;
}

/** An item's title; names used as titles (`vertical-promo`) read as words. */
export function titleOf(item: LibraryItem): string {
  if (item.title !== item.name) return item.title;
  const words = item.name.replaceAll("-", " ").replaceAll("_", " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** Seconds as people read them: 0.4 s, 2.5 s, 12 s, 1 min 5 s. */
export function seconds(value: number): string {
  if (value >= 60) {
    const minutes = Math.floor(value / 60);
    const rest = Math.round(value - minutes * 60);
    return rest === 0 ? `${minutes} min` : `${minutes} min ${rest} s`;
  }
  return value < 10 ? `${Math.round(value * 10) / 10} s` : `${Math.round(value)} s`;
}

/** A sound's length, with a music loop's tempo: "1.2 s", "8 s at 120 BPM"; "" for others. */
export function itemDetail(item: LibraryItem): string {
  if (item.seconds === undefined) return "";
  const length = seconds(item.seconds);
  return item.bpm === undefined ? length : `${length} at ${Math.round(item.bpm)} BPM`;
}

/** The track holding `clip`, or null. */
export function trackOf(timeline: TimelineView, clip: string): string | null {
  return timeline.tracks.find((t) => t.clips.some((c) => c.id === clip))?.id ?? null;
}

/** Where the editor stands when an item is added. */
export interface Placing {
  /** The playhead, in flicks. */
  playhead: number;
  /** The selected clip, if any. */
  selected: ClipView | null;
  /** The caption track to style: the selected caption's, or the first; null without one. */
  captionTrack: string | null;
  /** Media chosen in the bin, in the order chosen. */
  chosen: AssetView[];
}

/** What adding an item does: the request and its effect in words, or why it cannot now. */
export type AddPlan = { request: LibraryAddRequest; does: string } | { blocked: string };

/**
 * What adding `item` does where the editor stands: shapes, stickers, backgrounds, titles,
 * text styles, sounds and music go in at the playhead; a look goes on the selected clip;
 * a caption style restyles a caption track; a template builds a new timeline with the
 * video and images chosen in the media bin.
 */
export function addPlan(item: LibraryItem, at: Placing): AddPlan {
  const base: LibraryAddRequest = { name: item.name, kind: item.kind };
  switch (item.kind) {
    case "look": {
      if (!at.selected || at.selected.kind === "audio") return { blocked: "Select a clip with a picture to apply the look to." };
      return { request: { ...base, clip: at.selected.id }, does: "Apply to the selected clip" };
    }
    case "caption_style": {
      if (!at.captionTrack) return { blocked: "Add captions first: caption styles restyle a caption track." };
      return { request: { ...base, track: at.captionTrack }, does: "Restyle the captions" };
    }
    case "template": {
      const media = at.chosen.filter((a) => a.kind === "video" || a.kind === "image").map((a) => a.id);
      if (media.length === 0) return { request: base, does: "Build a new timeline from the template" };
      const what = media.length === 1 ? "the chosen video or image" : `the ${media.length} chosen videos and images`;
      return { request: { ...base, media }, does: `Build a new timeline with ${what}` };
    }
    default:
      return { request: { ...base, start_flicks: at.playhead }, does: "Add at the playhead" };
  }
}

/** Where the editor stands, from the timeline, the selection and the media chosen. */
export function placingOf(
  timeline: TimelineView | null,
  selection: string | null,
  playhead: number,
  media: AssetView[],
  chosen: string[],
): Placing {
  const selected =
    timeline && selection ? (timeline.tracks.flatMap((t) => t.clips).find((c) => c.id === selection) ?? null) : null;
  const selectedTrack = timeline && selected?.kind === "caption" ? trackOf(timeline, selected.id) : null;
  const firstCaptions = timeline?.tracks.find((t) => t.kind === "caption")?.id ?? null;
  return {
    playhead,
    selected,
    captionTrack: selectedTrack ?? firstCaptions,
    chosen: chosen.map((id) => media.find((a) => a.id === id)).filter((a): a is AssetView => a !== undefined),
  };
}
