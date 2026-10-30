// What the media bin shows: assets matching a search and a kind, in a folder, in an order.

import type { AssetView } from "../engine/types";

export type SortKey = "imported" | "name" | "duration" | "kind";

export interface MediaFilter {
  /** Words that must all appear in the name, case-insensitively. */
  search: string;
  /** One kind, or every kind when empty. */
  kind: "" | AssetView["kind"];
  /** Only the assets in this folder and its subfolders; null for every folder, "" for the
   * top level only. */
  folder: string | null;
  sort: SortKey;
}

export const ALL: MediaFilter = { search: "", kind: "", folder: null, sort: "imported" };

/** The assets `filter` shows, in its order. */
export function shown(assets: AssetView[], filter: MediaFilter): AssetView[] {
  const words = filter.search.toLowerCase().split(/\s+/).filter(Boolean);
  const inFolder = (asset: AssetView) => {
    if (filter.folder === null) return true;
    const folder = asset.folder ?? "";
    if (filter.folder === "") return folder === "";
    return folder === filter.folder || folder.startsWith(`${filter.folder}/`);
  };
  const kept = assets.filter(
    (asset) =>
      (!filter.kind || asset.kind === filter.kind) &&
      inFolder(asset) &&
      words.every((word) => asset.name.toLowerCase().includes(word)),
  );
  // Names compare as people read them: case aside, "take 2" before "take 10".
  const byName = (a: AssetView, b: AssetView) =>
    a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
  const order: Record<SortKey, (a: AssetView, b: AssetView) => number> = {
    imported: () => 0,
    name: byName,
    duration: (a, b) => (b.duration?.flicks ?? 0) - (a.duration?.flicks ?? 0),
    kind: (a, b) => a.kind.localeCompare(b.kind) || byName(a, b),
  };
  return [...kept].sort(order[filter.sort]);
}
