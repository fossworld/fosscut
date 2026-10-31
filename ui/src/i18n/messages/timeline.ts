// Messages of the interface's timeline area (the timeline, its tabs and what is drawn on
// it); keys start with "timeline.".

import type { Messages, Translation } from "../types";

export const en = {
  "timeline.label": "Timeline",
  "timeline.tabs": "Timelines",
  "timeline.rename.help": "Double-click to rename",
  "timeline.compound": "{name} (compound)",
  "timeline.remove.help": "Remove this timeline",
  "timeline.add.help": "Add a timeline",
  "timeline.track.video": "Video",
  "timeline.track.audio": "Audio",
  "timeline.track.text": "Titles",
  "timeline.track.caption": "Captions",
  "timeline.clip.off": "{name} (off)",
} satisfies Messages;

export const hu: Translation<typeof en> = {
  "timeline.label": "Idővonal",
  "timeline.tabs": "Idővonalak",
  "timeline.rename.help": "Átnevezés dupla kattintással",
  "timeline.compound": "{name} (összetett)",
  "timeline.remove.help": "Az idővonal eltávolítása",
  "timeline.add.help": "Idővonal hozzáadása",
  "timeline.track.video": "Videó",
  "timeline.track.audio": "Hang",
  "timeline.track.text": "Címek",
  "timeline.track.caption": "Feliratok",
  "timeline.clip.off": "{name} (kikapcsolva)",
};
