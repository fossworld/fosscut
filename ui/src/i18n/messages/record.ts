// Messages of the interface's record area; keys start with "record.".

import type { Messages, Translation } from "../types";

export const en = {
  "record.title": "Record",
  "record.close": "Close",
  "record.close.help": "The recording goes on",
  "record.what": "What to record",
  "record.kind.microphone": "Microphone",
  "record.kind.camera": "Camera",
  "record.kind.screen": "Screen",
  "record.devices.looking": "Looking for devices",
  "record.devices.none": "None found",
  "record.sound": "Sound",
  "record.sound.none": "No sound",
  "record.start": "Record",
  "record.stop": "Stop",
  "record.placed": "Recorded {time}; the clip is on the timeline where the playhead was when recording started.",
  "record.binned": "Recorded {time} into the media bin.",
  "record.privacy":
    "The system may not have allowed FossCut to use the microphone, the camera or screen recording: allow it in System Settings, under Privacy and Security, then try again.",
} satisfies Messages;

export const hu: Translation<typeof en> = {
  "record.title": "Felvétel",
  "record.close": "Bezárás",
  "record.close.help": "A felvétel folytatódik",
  "record.what": "Mit rögzít",
  "record.kind.microphone": "Mikrofon",
  "record.kind.camera": "Kamera",
  "record.kind.screen": "Képernyő",
  "record.devices.looking": "Eszközök keresése",
  "record.devices.none": "Nincs ilyen eszköz",
  "record.sound": "Hang",
  "record.sound.none": "Hang nélkül",
  "record.start": "Felvétel",
  "record.stop": "Leállítás",
  "record.placed": "{time} rögzítve; a klip az idővonalon ott van, ahol a felvétel kezdetén a lejátszófej állt.",
  "record.binned": "{time} rögzítve a médiatárba.",
  "record.privacy":
    "Lehet, hogy a rendszer nem engedélyezte a FossCutnak a mikrofon, a kamera vagy a képernyőfelvétel használatát: engedélyezze a Rendszerbeállítások Adatvédelem és biztonság részében, majd próbálja újra.",
};
