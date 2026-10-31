import { afterEach, describe, expect, it } from "vitest";

import { setLanguage } from "../i18n";
import { track } from "../test/fixtures";
import { MAX_ZOOM, MIN_ZOOM } from "./geometry";
import { DARK, clipColors, paletteFrom } from "./palette";
import { sliderOf, trackName, zoomOf } from "./Timeline";

describe("timeline panel", () => {
  afterEach(() => setLanguage("en"));

  it("names tracks by their own name, or their kind numbered when there are several", () => {
    const video1 = track("track_0001", "video", 0, []);
    const audio = track("track_0002", "audio", 1, []);
    const video2 = track("track_0003", "video", 2, []);
    const tracks = [video1, audio, video2];
    expect(trackName(video1, tracks)).toBe("Video 1");
    expect(trackName(video2, tracks)).toBe("Video 2");
    expect(trackName(audio, tracks), "the only one of its kind").toBe("Audio");
    expect(trackName({ ...audio, name: "Music" }, tracks)).toBe("Music");
    setLanguage("hu");
    expect(trackName(video2, tracks)).toBe("Videó 2");
  });

  it("moves the zoom slider in even steps of ratio, both ways", () => {
    expect(sliderOf(MIN_ZOOM)).toBeCloseTo(0);
    expect(sliderOf(MAX_ZOOM)).toBeCloseTo(100);
    expect(zoomOf(sliderOf(60))).toBeCloseTo(60);
    expect(zoomOf(50)).toBeCloseTo(Math.sqrt(MIN_ZOOM * MAX_ZOOM));
    expect(zoomOf(-5)).toBe(MIN_ZOOM);
  });

  it("draws with the theme's colours, or the dark theme's while none are read", () => {
    const light = paletteFrom((name) => ({ "--timeline-bg": "#f1f1f4", "--clip-video": " #123456 " })[name] ?? "");
    expect(light.background).toBe("#f1f1f4");
    expect(clipColors(light, "video").fill).toBe("#123456");
    expect(clipColors(light, "audio")).toEqual(DARK.clips.audio);
    expect(clipColors(DARK, "something new"), "unknown kinds have a colour too").toEqual(DARK.clips.other);
  });
});
