// Media dragged from the browser onto the timeline. The drag carries the asset's id as
// data of its own type; the timeline reads it on drop, and while the drag is over it from
// here, as `dragover` events cannot read a drag's data.

/** The type of data a dragged asset carries. */
export const ASSET_DRAG = "application/x-fosscut-asset";

let dragged: string | null = null;

/** Remembers the asset a drag carries, or forgets it when the drag ends. */
export function dragAsset(asset: string | null): void {
  dragged = asset;
}

/** The asset being dragged, or null. */
export function draggedAsset(): string | null {
  return dragged;
}
