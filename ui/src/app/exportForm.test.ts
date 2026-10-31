import { afterEach, describe, expect, it } from "vitest";

import type { ExportPreset, ExportReport, JobStatus } from "../engine/types";
import { setLanguage, t } from "../i18n";
import { moment } from "../test/fixtures";
import { FLICKS_PER_SECOND, frameAt, framesToFlicks, parseRate } from "../transport/time";
import {
  type ExportForm,
  audioOnly,
  baseName,
  choiceOf,
  coverStart,
  defaultPreset,
  fileName,
  folderOf,
  fpsNumber,
  fpsText,
  lengthText,
  markerRanges,
  parseTime,
  progressOf,
  reportRows,
  showTime,
  sizeFor,
  withExtension,
} from "./exportForm";
import { readableSize } from "./Start";

const S = FLICKS_PER_SECOND;
const fps30 = parseRate("30");

const preset = (name: string, extension: string, available = true): ExportPreset => ({
  name,
  description: name,
  extension,
  available,
});
const presets = [
  preset("mp4-h264", "mp4"),
  preset("mp4-av1", "mp4", false),
  preset("mov-prores", "mov"),
  preset("m4a-aac", "m4a"),
];
const context = { format: { width: 1920, height: 1080, rate: "30" }, duration: 24 * S, presets };
const form: ExportForm = {
  preset: "mp4-h264",
  resolution: "",
  fps: "",
  bitrate: "",
  part: false,
  from: "00:00:00:00",
  to: "00:00:24:00",
  cover: false,
  coverAt: "00:00:00:00",
  subtitles: "",
  output: "/work/Launch.mp4",
  overwrite: false,
};

describe("export form", () => {
  afterEach(() => setLanguage("en"));

  it("starts from H.264 where this machine has it, and names files after the project", () => {
    expect(defaultPreset(presets)?.name).toBe("mp4-h264");
    expect(defaultPreset([preset("mp4-h264", "mp4", false), preset("m4a-aac", "m4a"), preset("webm-vp9", "webm")])?.name).toBe(
      "webm-vp9",
    );
    expect(defaultPreset([preset("m4a-aac", "m4a")])?.name, "sound only when that is all").toBe("m4a-aac");
    expect(defaultPreset([])).toBeUndefined();
    expect(audioOnly(presets[3])).toBe(true);
    expect(audioOnly(presets[0])).toBe(false);
    expect(fileName("Launch: v2/final", "mp4")).toBe("Launch- v2-final.mp4");
    expect(fileName("  ", "mov")).toBe("Export.mov");
    expect(folderOf("/work/Launch.fosscut")).toBe("/work");
    expect(folderOf("C:\\Videos\\Launch.fosscut")).toBe("C:\\Videos");
    expect(folderOf("Launch.mp4")).toBe("");
    expect(baseName("/work/Launch.mp4")).toBe("Launch.mp4");
    expect(withExtension("/work/Launch.mp4", "mov")).toBe("/work/Launch.mov");
    expect(withExtension("/work.v2/Launch", "mov"), "a dot in a folder is not an extension").toBe("/work.v2/Launch.mov");
    expect(withExtension("/work/.hidden", "mp4")).toBe("/work/.hidden.mp4");
  });

  it("reads times as timecode, minutes and seconds, or seconds, at the start of their frame", () => {
    expect(parseTime("00:00:05:12", fps30)).toBe(framesToFlicks(162, fps30));
    expect(parseTime("1:30.5", fps30)).toBe(framesToFlicks(2715, fps30));
    expect(parseTime("12.5s", fps30)).toBe(12.5 * S);
    expect(parseTime(" 2 ", fps30)).toBe(2 * S);
    expect(parseTime(".5", fps30)).toBe(S / 2);
    expect(parseTime("0.01", fps30), "inside the first frame").toBe(0);
    for (const bad of ["", "soon", "-2", "1:75", "00:61:00:00", "00:00:01:30", "1:2:3"]) {
      expect(parseTime(bad, fps30), bad).toBeNull();
    }
    const ntsc = parseRate("30000/1001");
    for (const frame of [0, 1799, 1800, 17982, 107892]) {
      const at = framesToFlicks(frame, ntsc);
      expect(frameAt(parseTime(showTime(at, ntsc), ntsc)!, ntsc), `drop-frame timecode of frame ${frame}`).toBe(frame);
    }
    const film = parseRate("24000/1001");
    expect(frameAt(parseTime("00:01:00:00", film)!, film), "timecode counts nominal frames").toBe(1440);
  });

  it("offers the parts between markers", () => {
    const markers = [
      { id: "marker_1", at: moment(4), name: "beat" },
      { id: "marker_2", at: moment(15) },
      { id: "marker_3", at: moment(30), name: "after the end" },
    ];
    const range = (from: string, to: string) => t("export.range", { from, to });
    expect(markerRanges(markers, 24 * S)).toEqual([
      { label: range(t("export.range.start"), "beat"), from: 0, to: 4 * S },
      { label: range("beat", moment(15).timecode), from: 4 * S, to: 15 * S },
      { label: range(moment(15).timecode, t("export.range.end")), from: 15 * S, to: 24 * S },
    ]);
    expect(markerRanges(markers, 24 * S)[0]?.label).toBe("Start to beat");
    expect(markerRanges([], 24 * S)).toEqual([]);
    setLanguage("hu");
    expect(markerRanges(markers, 24 * S)[0]?.label).toBe("Eleje és beat között");
  });

  it("makes the request: preset, size, rate and bit rate, part, cover and subtitles", () => {
    expect(choiceOf(form, context)).toEqual({ choice: { output: "/work/Launch.mp4", preset: "mp4-h264" } });
    const full = choiceOf(
      {
        ...form,
        resolution: "720p",
        fps: "29.97",
        bitrate: " 8 M ",
        part: true,
        from: "2s",
        to: "00:00:30:00",
        cover: true,
        coverAt: "5s",
        subtitles: "vtt",
        overwrite: true,
      },
      context,
    );
    expect(full).toEqual({
      choice: {
        output: "/work/Launch.mp4",
        preset: "mp4-h264",
        resolution: "720p",
        fps: "29.97",
        bitrate: "8M",
        range_flicks: [2 * S, 24 * S],
        cover_flicks: 3 * S,
        subtitles: "vtt",
        overwrite: true,
      },
    });
    const sound = choiceOf({ ...form, preset: "m4a-aac", resolution: "4k", fps: "60", bitrate: "lots" }, context);
    expect(sound, "size, rate and bit rate do not apply to sound").toEqual({
      choice: { output: "/work/Launch.mp4", preset: "m4a-aac" },
    });
  });

  it("says what to fix first", () => {
    const problem = (change: Partial<ExportForm>) => {
      const made = choiceOf({ ...form, ...change }, context);
      return "problem" in made ? made.problem : null;
    };
    expect(problem({ output: " " })).toBe(t("export.problem.output"));
    expect(problem({ preset: "gif" })).toBe(t("export.problem.preset"));
    expect(problem({ preset: "mp4-av1" })).toBe(t("export.problem.unavailable", { name: "mp4-av1" }));
    expect(problem({ bitrate: "fast" })).toBe(t("export.problem.bitrate"));
    expect(problem({ part: true, from: "later" })).toBe(t("export.problem.part"));
    expect(problem({ part: true, from: "30s", to: "40s" })).toBe(t("export.problem.partStart"));
    expect(problem({ part: true, from: "5s", to: "5s" })).toBe(t("export.problem.partEnd"));
    expect(problem({ cover: true, coverAt: "x" })).toBe(t("export.problem.coverTime"));
    expect(problem({ part: true, from: "2s", to: "4s", cover: true, coverAt: "4s" })).toBe(t("export.problem.coverInside"));
    expect(problem({ cover: true, coverAt: "24s" })).toBe(t("export.problem.coverInside"));
    setLanguage("hu");
    expect(problem({ output: " " })).toBe("Válassza ki, hová mentse az exportált fájlt.");
  });

  it("starts a cover inside the part exported", () => {
    expect(coverStart({ ...form, coverAt: "20s" }, fps30), "the whole timeline").toBe("20s");
    const part = { ...form, part: true, from: "4s", to: "15s" };
    expect(coverStart({ ...part, coverAt: "0s" }, fps30)).toBe("4s");
    expect(coverStart({ ...part, coverAt: "15s" }, fps30), "the end is not in the part").toBe("4s");
    expect(coverStart({ ...part, coverAt: "6s" }, fps30)).toBe("6s");
    expect(coverStart({ ...part, from: "soon", coverAt: "0s" }, fps30), "nothing better to offer").toBe("0s");
  });

  it("works out sizes as the engine does and names rates and lengths", () => {
    expect(sizeFor({ width: 1920, height: 1080 }, 720)).toEqual([1280, 720]);
    expect(sizeFor({ width: 1080, height: 1920 }, 720), "vertical").toEqual([720, 1280]);
    expect(sizeFor({ width: 1000, height: 750 }, 480)).toEqual([640, 480]);
    expect(sizeFor({ width: 1920, height: 1080 }, null)).toEqual([1920, 1080]);
    expect([fpsText("30"), fpsText("30000/1001"), fpsText("24000/1001")]).toEqual(["30", "29.97", "23.976"]);
    expect(fpsNumber("30000/1001")).toBe(29.97);
    expect([lengthText(1.54), lengthText(42.4), lengthText(120), lengthText(125)]).toEqual([
      t("export.length.seconds", { seconds: 1.5 }),
      t("export.length.seconds", { seconds: 42 }),
      t("export.length.minutes", { minutes: 2 }),
      t("export.length.minutesSeconds", { minutes: 2, seconds: 5 }),
    ]);
    expect(lengthText(125)).toBe("2 min 5 s");
    setLanguage("hu");
    expect([lengthText(1.54), lengthText(125)]).toEqual(["1,5 mp", "2 perc 5 mp"]);
  });

  it("describes a job's progress", () => {
    const job = (state: JobStatus["state"], done: number, total: number, elapsed = 2000): JobStatus => ({
      job: "job_0001",
      kind: "export",
      state,
      progress: { done, total },
      elapsed_ms: elapsed,
    });
    expect(progressOf(job("running", 0, 0))).toEqual({ fraction: null, text: t("export.progress.ready") });
    const counted = t("export.progress.count", { done: 30, total: 90, percent: 33 });
    const left = t("export.progress.left", { left: lengthText(4) });
    expect(progressOf(job("running", 30, 90))).toEqual({ fraction: 1 / 3, text: `${counted}, ${left}.` });
    expect(progressOf(job("running", 30, 90)).text).toBe("30 of 90 (33%), about 4 s left.");
    expect(progressOf(job("running", 90, 90)).text).toBe(
      `${t("export.progress.count", { done: 90, total: 90, percent: 100 })}, ${t("export.progress.checking")}.`,
    );
    expect(progressOf(job("succeeded", 90, 90, 61_000))).toEqual({
      fraction: 1,
      text: t("export.progress.done", { took: lengthText(61) }),
    });
    expect(progressOf(job("cancelled", 10, 90)).text).toBe(t("export.progress.cancelled"));
    expect(progressOf(job("failed", 10, 90))).toEqual({ fraction: null, text: t("export.progress.failed") });
    setLanguage("hu");
    expect(progressOf(job("running", 30, 90)).text).toBe("30 / 90 (33%), még körülbelül 4 mp.");
  });

  it("lists a finished export's file, size, length, streams, cover and subtitles", () => {
    const report: ExportReport = {
      output: "/work/Launch.mp4",
      settings: { container: "mp4" },
      frames: 90,
      duration: 3 * S,
      encoders: { video: "h264_videotoolbox", audio: "aac" },
      verified: {
        container: "mov,mp4",
        size_bytes: 3_400_000,
        duration: 3 * S,
        video: { codec: "h264", width: 1920, height: 1080, rate: "30000/1001" },
        audio: { codec: "aac", sample_rate: 48_000, channels: 2 },
      },
      elapsed_ms: 1500,
      cover: "/work/Launch.cover.png",
      subtitles: "/work/Launch.srt",
    };
    expect(reportRows(report)).toEqual([
      [t("export.report.file"), "/work/Launch.mp4"],
      [t("export.report.size"), readableSize(3_400_000)],
      [t("export.report.length"), lengthText(3)],
      [t("export.report.picture"), t("export.report.pictureAt", { codec: "h264", width: 1920, height: 1080, fps: 29.97 })],
      [t("export.report.sound"), `aac, ${t("export.report.khz", { khz: 48 })} ${t("export.report.stereo")}`],
      [t("export.report.cover"), "/work/Launch.cover.png"],
      [t("export.report.subtitles"), "/work/Launch.srt"],
      [t("export.report.took"), lengthText(1.5)],
    ]);
    expect(reportRows(report)[3]).toEqual(["Picture", "h264, 1920 x 1080 at 29.97 fps"]);
    expect(reportRows(report)[4]).toEqual(["Sound", "aac, 48 kHz stereo"]);
    const sound = { ...report, verified: { container: "mp4", size_bytes: 900, video: null, audio: { codec: "aac", channels: 1 } } };
    expect(reportRows({ ...sound, cover: undefined, subtitles: undefined })).toEqual([
      [t("export.report.file"), "/work/Launch.mp4"],
      [t("export.report.size"), readableSize(900)],
      [t("export.report.length"), lengthText(3)],
      [t("export.report.sound"), `aac ${t("export.report.mono")}`],
      [t("export.report.took"), lengthText(1.5)],
    ]);
    setLanguage("hu");
    expect(reportRows(report)[3]).toEqual(["Kép", "h264, 1920 x 1080, 29,97 fps"]);
  });
});
