import type { ClipView } from "../engine/types";

/** How a clip's speed reads on the timeline: `2x`, `curve`, `freeze`, with `reversed`. */
export function speedLabel(clip: ClipView): string | null {
  const speed = clip.speed;
  if (!speed) return null;
  if (speed.curve?.length) return speed.reverse ? "curve, reversed" : "curve";
  const rate = speed.rate ?? 1;
  if (rate === 0) return "freeze";
  const shown = `${Math.round(rate * 100) / 100}x`;
  return speed.reverse ? `${shown}, reversed` : shown;
}
