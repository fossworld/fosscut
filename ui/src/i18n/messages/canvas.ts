// Messages of the inspector's canvas fill; keys start with "canvas.".

import type { Messages, Translation } from "../types";

export const en = {
  "canvas.title": "Canvas",
  "canvas.fill": "Fill",
  "canvas.fill.help": "What shows around the picture where it does not cover the canvas, such as a wide shot in a vertical video",
  "canvas.none": "None",
  "canvas.blur": "Blur",
  "canvas.color": "Colour",
  "canvas.amount": "Blur amount",
  "canvas.pick": "Fill colour",
  "canvas.several": { one: "Applies to the selected clip", other: "Applies to {count} selected video and image clips" },
} satisfies Messages;

export const hu: Translation<typeof en> = {
  "canvas.title": "Vászon",
  "canvas.fill": "Kitöltés",
  "canvas.fill.help": "Ami a kép körül látszik, ahol a kép nem fedi le a vásznat, például egy széles felvétel körül álló videóban",
  "canvas.none": "Nincs",
  "canvas.blur": "Elmosás",
  "canvas.color": "Szín",
  "canvas.amount": "Elmosás mértéke",
  "canvas.pick": "Kitöltőszín",
  "canvas.several": { one: "A kijelölt klipre érvényes", other: "{count} kijelölt videó- és képklipre érvényes" },
};
