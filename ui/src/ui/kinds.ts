// The icons of the kinds of clip and track, shared by the timeline, the inspector and the
// browser, so a kind looks the same everywhere.

import { t } from "../i18n";
import type { ClipView } from "../engine/types";
import { Box, Clapperboard, Film, Image, Layers, type LucideIcon, Music, Shapes, Sparkles, Subtitles, Type, Video } from "lucide-react";

/** The icon of a clip's or track's kind. */
export function kindIcon(kind: string): LucideIcon {
  switch (kind) {
    case "video":
      return Film;
    case "image":
      return Image;
    case "audio":
      return Music;
    case "text":
      return Type;
    case "caption":
      return Subtitles;
    case "shape":
      return Shapes;
    case "adjustment":
      return Layers;
    case "timeline":
      return Clapperboard;
    // A camera (ADR-0037), a particle emitter (ADR-0038) and a 3D object.
    case "camera":
      return Video;
    case "particles":
      return Sparkles;
    case "model":
      return Box;
    default:
      return Sparkles;
  }
}

/**
 * A clip's name as the interface shows it: its words, its media or its compound timeline,
 * else what it is in the interface's language (a camera, particles, a shape's outline).
 */
export function clipName(clip: ClipView): string {
  if (clip.kind === "camera") return t("timeline.clip.camera");
  const own = clip.text ?? clip.asset_name ?? clip.timeline_name;
  if (own) return own;
  if (clip.kind === "particles") return t("timeline.clip.particles");
  if (clip.kind === "shape" && clip.shape) return t(`shape.kind.${clip.shape}`);
  return clip.kind;
}
