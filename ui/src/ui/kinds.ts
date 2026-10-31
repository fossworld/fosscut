// The icons of the kinds of clip and track, shared by the timeline, the inspector and the
// browser, so a kind looks the same everywhere.

import { Clapperboard, Film, Image, Layers, type LucideIcon, Music, Shapes, Sparkles, Subtitles, Type } from "lucide-react";

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
    default:
      return Sparkles;
  }
}
