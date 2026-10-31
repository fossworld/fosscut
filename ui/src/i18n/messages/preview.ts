// Messages of the preview and the overlays drawn on it; keys start with "preview.".

import type { Messages, Translation } from "../types";

export const en = {
  "preview.label": "Preview",
  "preview.done": "Done",
  "preview.cancel": "Cancel",
  "preview.region.hint": "Drag over what to follow",
} satisfies Messages;

export const hu: Translation<typeof en> = {
  "preview.label": "Előnézet",
  "preview.done": "Kész",
  "preview.cancel": "Mégse",
  "preview.region.hint": "Húzza át azt, amit követni kell",
};
