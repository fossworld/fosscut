import { describe, expect, it } from "vitest";

import { FakeEngine } from "../engine/fake";
import type { ApiError } from "../engine/types";
import { moment } from "../test/fixtures";
import { FLICKS_PER_SECOND } from "../transport/time";
import { BATCH, Pictures, assetKey, bucketsFor, gridStep, onGrid, peakBetween } from "./pictures";

const video = { id: "asset_0001", kind: "video" as const, duration: moment(20), video: { width: 1920, height: 1080 } };
const image = { id: "asset_0002", kind: "image" as const, duration: null, video: { width: 800, height: 600 } };
const sound = { id: "asset_0003", kind: "audio" as const, duration: moment(10) };

describe("pictures of media", () => {
  it("puts filmstrip frames on a grid that zooms share", () => {
    expect(gridStep(0.3)).toBe(0.5);
    expect(gridStep(0.5)).toBe(0.5);
    expect(gridStep(3)).toBe(4);
    expect(gridStep(1e9), "never past the longest step").toBe(4096);
    expect(onGrid(5.3, 2)).toBe(4 * FLICKS_PER_SECOND);
    expect(onGrid(-1, 2), "never before zero").toBe(0);
    // A frame on a coarse grid is on the finer one too.
    expect(onGrid(9, 4) % (2 * FLICKS_PER_SECOND)).toBe(0);
  });

  it("asks for about 120 peaks a second, within the engine's limits", () => {
    expect(bucketsFor(10 * FLICKS_PER_SECOND)).toBe(1200);
    expect(bucketsFor(FLICKS_PER_SECOND), "short sounds still have shape").toBe(256);
    expect(bucketsFor(3600 * FLICKS_PER_SECOND)).toBe(65536);
  });

  it("finds the lowest and highest peak between two fractions", () => {
    const peaks = new Float32Array([-0.1, 0.2, -0.5, 0.4, -0.2, 0.9, -0.3, 0.1]);
    expect(peakBetween(peaks, 0, 0.25)).toEqual([expect.closeTo(-0.1), expect.closeTo(0.2)]);
    expect(peakBetween(peaks, 0.25, 0.75)).toEqual([expect.closeTo(-0.5), expect.closeTo(0.9)]);
    expect(peakBetween(peaks, 0.9, 0.95), "a sliver still reads its bucket").toEqual([expect.closeTo(-0.3), expect.closeTo(0.1)]);
    expect(peakBetween(new Float32Array(), 0, 1)).toEqual([0, 0]);
  });

  it("keys an asset by what changes when its file is replaced", () => {
    expect(assetKey(video)).not.toBe(assetKey({ ...video, duration: moment(21) }));
    expect(assetKey(video)).toBe(assetKey({ ...video }));
  });

  it("asks for frames in batches, once each, and keeps them", async () => {
    const fake = new FakeEngine();
    const pictures = new Pictures(fake, 0);
    let heard = 0;
    pictures.subscribe(() => (heard += 1));
    const times = Array.from({ length: BATCH + 2 }, (_, i) => i * FLICKS_PER_SECOND);
    for (const time of times) expect(pictures.frame(video, time)).toBeNull();
    expect(pictures.frame(video, 0), "asked for already").toBeNull();
    await pictures.flush();
    const asked = fake.callsOf("media.thumbnails").map(([request]) => request as { asset: string; times: number[]; height: number });
    expect(asked.map((r) => r.times.length), "at most a batch per request").toEqual([BATCH, 2]);
    expect(asked[0]?.height).toBe(96);
    expect(pictures.frame(video, 3 * FLICKS_PER_SECOND)).toMatch(/^data:image\/svg\+xml/);
    expect(heard).toBe(2);
    expect(pictures.version()).toBe(2);
  });

  it("shows an image's one frame at every time, and nothing of sound or missing files", async () => {
    const fake = new FakeEngine();
    const pictures = new Pictures(fake, 0);
    pictures.frame(image, 5 * FLICKS_PER_SECOND);
    pictures.frame(image, 0);
    expect(pictures.frame(sound, 0)).toBeNull();
    expect(pictures.frame({ ...video, missing: true }, 0)).toBeNull();
    await pictures.flush();
    expect(fake.callsOf("media.thumbnails")).toEqual([[{ asset: "asset_0002", times: [0], height: 96 }]]);
    expect(pictures.frame(image, 9 * FLICKS_PER_SECOND)).not.toBeNull();
    expect(Pictures.pictured(sound)).toBe(false);
    expect(Pictures.sounding(video)).toBe(true);
  });

  it("does not ask again for an asset the engine could not picture, until cleared", async () => {
    const fake = new FakeEngine();
    fake.pictureFailure = { code: "fosscut.media.no_picture", message: "no picture" } satisfies ApiError;
    const pictures = new Pictures(fake, 0);
    pictures.frame(video, 0);
    await pictures.flush();
    pictures.frame(video, FLICKS_PER_SECOND);
    await pictures.flush();
    expect(fake.callsOf("media.thumbnails")).toHaveLength(1);
    fake.pictureFailure = null;
    pictures.clear();
    pictures.frame(video, FLICKS_PER_SECOND);
    await pictures.flush();
    expect(fake.callsOf("media.thumbnails")).toHaveLength(2);
    expect(pictures.frame(video, FLICKS_PER_SECOND)).not.toBeNull();
  });

  it("reads peaks once and keeps them interleaved, low then high", async () => {
    const fake = new FakeEngine();
    fake.answers["media.list"] = { assets: [{ ...sound, used_by: [] }] };
    const pictures = new Pictures(fake, 0);
    expect(pictures.peaks(sound)).toBeNull();
    expect(pictures.peaks(sound)).toBeNull();
    await new Promise((done) => setTimeout(done, 0));
    const peaks = pictures.peaks(sound);
    expect(fake.callsOf("media.waveform")).toEqual([["asset_0003", 1200]]);
    expect(peaks?.duration).toBe(10 * FLICKS_PER_SECOND);
    expect(peaks?.peaks.length).toBe(2400);
    expect(peaks!.peaks[0]!).toBeLessThanOrEqual(0);
    expect(peaks!.peaks[1]!).toBeGreaterThanOrEqual(0);
    expect(pictures.peaks(image), "an image has no sound").toBeNull();
  });

  it("forgets what arrives for a project that is no longer open", async () => {
    const fake = new FakeEngine();
    const pictures = new Pictures(fake, 0);
    pictures.frame(video, 0);
    const flushing = pictures.flush();
    pictures.clear();
    await flushing;
    expect(pictures.frame(video, 0), "asked for again, not kept from before").toBeNull();
  });
});
