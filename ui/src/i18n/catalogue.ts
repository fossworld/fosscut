// Every message of the interface, by area: each module in ./messages holds one area's
// English messages (`en`) and their Hungarian translation (`hu`), keyed "<area>.<name>".
//
// Hungarian terms used throughout, for consistency:
//   timeline idővonal, track sáv, clip klip, split vágás (kettévágás), crop körülvágás,
//   cut (clipboard) kivágás, copy másolás, paste beillesztés, delete törlés,
//   undo visszavonás, redo mégis, playhead lejátszófej, preview előnézet, frame képkocka,
//   frame rate képkockasebesség, export exportálás, media médiafájl, media bin médiatár,
//   library könyvtár, folder mappa, effect effekt, transition átmenet, keyframe kulcskocka,
//   mask maszk, marker jelölő, project projekt, settings beállítások,
//   shortcuts billentyűparancsok, volume hangerő, opacity átlátszatlanság,
//   blend mode keverési mód, speed sebesség, recording felvétel, caption felirat,
//   title cím, sound hang, music zene, loudness hangosság, normalize normalizálás,
//   group csoportosítás, linked editing csatolt szerkesztés, snapping illesztés,
//   magnet mágnes, skimming átfutás, scrubbing görgetés, blade penge, tool eszköz,
//   zoom in nagyítás, zoom out kicsinyítés, full screen teljes képernyő,
//   rulers vonalzók, guides segédvonalak, canvas vászon, blur elmosás, colour szín,
//   compound clip összetett klip, multicam többkamerás, angle kameranézet,
//   stabilize stabilizálás, motion tracking követés, scene jelenet, template sablon,
//   shape alakzat, sticker matrica, background háttér, look színvilág,
//   adjustment layer korrekciós réteg, cache gyorsítótár, proxy proxy,
//   notification értesítés, update frissítés, trash kuka, agent ügynök,
//   history előzmények, revision változat, cancel mégse, save mentés,
//   reset visszaállítás, apply alkalmazás, dismiss bezárás,
//   model modell, install telepítés, download letöltés, license licenc,
//   remove background háttér eltávolítása, body effects testeffektek,
//   text to speech szövegfelolvasás, voice hang, preview (listen) meghallgatás,
//   translate fordítás, dub szinkron (szinkronizálás), analysis elemzés.

import * as activity from "./messages/activity";
import * as app from "./messages/app";
import * as canvas from "./messages/canvas";
import * as effects from "./messages/effects";
import * as exporting from "./messages/export";
import * as inspector from "./messages/inspector";
import * as masks from "./messages/masks";
import * as media from "./messages/media";
import * as models from "./messages/models";
import * as motion from "./messages/motion";
import * as preview from "./messages/preview";
import * as record from "./messages/record";
import * as settings from "./messages/settings";
import * as shape from "./messages/shape";
import * as text from "./messages/text";
import * as timeline from "./messages/timeline";
import * as transport from "./messages/transport";
import * as voice from "./messages/voice";
import type { Messages, Translation } from "./types";

/** Each area's catalogues, by the prefix of its keys. */
export const areas = {
  activity,
  app,
  canvas,
  effects,
  export: exporting,
  inspector,
  masks,
  media,
  models,
  motion,
  preview,
  record,
  settings,
  shape,
  text,
  timeline,
  transport,
  voice,
} satisfies Record<string, { en: Messages; hu: Messages }>;

/** Every English message. */
export const english = {
  ...activity.en,
  ...app.en,
  ...canvas.en,
  ...effects.en,
  ...exporting.en,
  ...inspector.en,
  ...masks.en,
  ...media.en,
  ...models.en,
  ...motion.en,
  ...preview.en,
  ...record.en,
  ...settings.en,
  ...shape.en,
  ...text.en,
  ...timeline.en,
  ...transport.en,
  ...voice.en,
};

/** The key of a message. */
export type MessageKey = keyof typeof english;

const hungarian: Translation<typeof english> = {
  ...activity.hu,
  ...app.hu,
  ...canvas.hu,
  ...effects.hu,
  ...exporting.hu,
  ...inspector.hu,
  ...masks.hu,
  ...media.hu,
  ...models.hu,
  ...motion.hu,
  ...preview.hu,
  ...record.hu,
  ...settings.hu,
  ...shape.hu,
  ...text.hu,
  ...timeline.hu,
  ...transport.hu,
  ...voice.hu,
};

/** Every language's messages. */
export const catalogues = { en: english, hu: hungarian } satisfies Record<string, Translation<typeof english>>;

/** A language the interface is translated into. */
export type Language = keyof typeof catalogues;
