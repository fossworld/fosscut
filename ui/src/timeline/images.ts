// Decoded pictures for the timeline canvas: the media's frames, which arrive as data URLs,
// decoded once into images the canvas draws, the oldest let go when many are kept.

const KEPT = 900;
const images = new Map<string, HTMLImageElement>();

/**
 * The decoded image of `url`, or null until it is decoded; `onReady` is called once it is,
 * so the canvas draws again.
 */
export function decoded(url: string, onReady: () => void): HTMLImageElement | null {
  const found = images.get(url);
  if (found) {
    images.delete(url);
    images.set(url, found);
    return found.complete && found.naturalWidth > 0 ? found : null;
  }
  if (typeof Image === "undefined") return null;
  const image = new Image();
  image.decoding = "async";
  image.onload = onReady;
  image.src = url;
  images.set(url, image);
  while (images.size > KEPT) {
    const oldest = images.keys().next().value;
    if (oldest === undefined) break;
    images.delete(oldest);
  }
  return null;
}
