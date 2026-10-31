import { afterEach, describe, expect, it } from "vitest";

import { setLanguage, t } from "../i18n";
import { FLICKS_PER_SECOND, formatTime, frameAt, frameStart, framesToFlicks, parseRate, timecode } from "./time";

const fps30 = parseRate("30");
const ntsc = parseRate("30000/1001");

describe("time", () => {
  afterEach(() => setLanguage("en"));

  it("parses the engine's rates and refuses others", () => {
    expect(parseRate("25")).toEqual({ num: 25, den: 1 });
    expect(ntsc).toEqual({ num: 30000, den: 1001 });
    expect(() => parseRate("fast")).toThrow();
    expect(() => parseRate("30/0")).toThrow();
    expect(() => parseRate("29.97")).toThrow();
  });

  it("converts between flicks and frames exactly as fosscut-time does", () => {
    expect(framesToFlicks(1, fps30)).toBe(FLICKS_PER_SECOND / 30);
    expect(framesToFlicks(1, ntsc)).toBe(23_543_520);
    expect(frameAt(framesToFlicks(45, fps30), fps30)).toBe(45);
    expect(frameAt(framesToFlicks(45, fps30) - 1, fps30)).toBe(44);
    expect(frameAt(-1, fps30), "rounds toward negative infinity").toBe(-1);
    expect(frameStart(framesToFlicks(12, ntsc) + 7, ntsc)).toBe(framesToFlicks(12, ntsc));
    // One hour of 29.97 stays exact far beyond 2^53 intermediate products.
    const hour = 107_892;
    expect(frameAt(framesToFlicks(hour, ntsc), ntsc)).toBe(hour);
  });

  it("formats timecode like the engine, drop frame at 29.97", () => {
    expect(timecode(0, fps30)).toBe("00:00:00:00");
    expect(timecode(45, fps30)).toBe("00:00:01:15");
    expect(timecode(30 * 3661 + 7, fps30)).toBe("01:01:01:07");
    expect(timecode(-31, fps30)).toBe("-00:00:01:01");
    expect(timecode(1800, ntsc)).toBe("00:01:00;02");
    expect(timecode(17982, ntsc)).toBe("00:10:00;00");
  });

  it("shows times as the time display preference says, in the interface's language", () => {
    const at = framesToFlicks(375, fps30);
    expect(formatTime(at, fps30)).toBe("00:00:12:15");
    expect(formatTime(at, fps30, "timecode")).toBe("00:00:12:15");
    expect(formatTime(at, fps30, "frames")).toBe("375");
    expect(formatTime(at, fps30, "seconds")).toBe(t("transport.seconds", { seconds: "12.50" }));
    expect(formatTime(at, fps30, "seconds")).toBe("12.50 s");
    setLanguage("hu");
    expect(formatTime(at, fps30, "seconds")).toBe("12,50 mp");
    expect(formatTime(at, ntsc, "frames")).toBe(String(frameAt(at, ntsc)));
  });
});
