import type { ClipView } from "../engine/types";
import { t } from "../i18n";

/**
 * How a clip's speed reads on the timeline: `2x`, `curve`, `freeze`, with `reversed`, in
 * the interface's language.
 */
export function speedLabel(clip: ClipView): string | null {
  const speed = clip.speed;
  if (!speed) return null;
  const reversed = (shown: string) => (speed.reverse ? t("motion.speed.label.reversed", { speed: shown }) : shown);
  if (speed.curve?.length) return reversed(t("motion.speed.label.curve"));
  const rate = speed.rate ?? 1;
  if (rate === 0) return t("motion.speed.label.freeze");
  return reversed(t("motion.speed.label.rate", { rate: Math.round(rate * 100) / 100 }));
}
