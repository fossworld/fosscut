// The export dialog's choices as people make them, the `export_start` request they turn
// into, and the progress and report of the job it starts, in words.

import type { ExportChoice, ExportPreset, ExportReport, Format, JobStatus, MarkerView } from "../engine/types";
import { t } from "../i18n";
import {
  FLICKS_PER_SECOND,
  type Rate,
  frameAt,
  frameStart,
  framesToFlicks,
  isDropFrame,
  parseRate,
  timecode,
} from "../transport/time";
import { readableSize } from "./Start";

export interface ExportForm {
  preset: string;
  /** A named resolution (`720p`); the timeline's when empty. */
  resolution: string;
  /** A frame rate (`29.97`); the timeline's when empty. */
  fps: string;
  /** A video bit rate (`8M`); the encoder's choice when empty. */
  bitrate: string;
  /** Export only from `from` to `to`, rather than the whole timeline. */
  part: boolean;
  from: string;
  to: string;
  /** Write a cover image of the frame at `coverAt`, a timeline time. */
  cover: boolean;
  coverAt: string;
  subtitles: "" | "srt" | "vtt";
  /** The file to write. */
  output: string;
  /** Replace the file if it exists. */
  overwrite: boolean;
}

/** The frame rates offered besides the timeline's own. */
export const FRAME_RATES = ["23.976", "24", "25", "29.97", "30", "50", "59.94", "60"];

/** Extensions of presets that write sound only. */
const AUDIO_ONLY = ["m4a", "wav", "flac", "mp3"];

/** True for a preset without a picture, where size, rate and bit rate do not apply. */
export function audioOnly(preset: ExportPreset | undefined): boolean {
  return preset !== undefined && AUDIO_ONLY.includes(preset.extension);
}

/** The preset a new export starts with: H.264 where this machine has it, or the first video preset it has. */
export function defaultPreset(presets: ExportPreset[]): ExportPreset | undefined {
  const available = presets.filter((p) => p.available);
  return available.find((p) => p.name === "mp4-h264") ?? available.find((p) => !audioOnly(p)) ?? available[0];
}

/** A file name made from a project's name: characters file systems refuse become dashes. */
export function fileName(name: string, extension: string): string {
  const base = name.replace(/[/\\:*?"<>|]/g, "-").trim() || "Export";
  return `${base}.${extension}`;
}

/** The folder a path is in, without the last separator; empty for a bare name. */
export function folderOf(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut < 0 ? "" : path.slice(0, cut);
}

/** The last part of a path. */
export function baseName(path: string): string {
  return path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1);
}

/** `path` with its extension replaced by `extension`, or given one. */
export function withExtension(path: string, extension: string): string {
  const name = baseName(path);
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? path.slice(0, path.length - name.length + dot) : path;
  return `${stem}.${extension}`;
}

/** A time as the fields show it: timecode at the timeline's rate. */
export function showTime(flicks: number, rate: Rate): string {
  return timecode(frameAt(flicks, rate), rate);
}

/**
 * A time as people type it, in flicks at the start of its frame: timecode
 * (`00:00:05:12`, `;12` at drop-frame rates), minutes and seconds (`1:30.5`), or seconds
 * (`12.5`, `12.5s`). Null when it is none of these.
 */
export function parseTime(text: string, rate: Rate): number | null {
  const typed = text.trim().toLowerCase();
  const seconds = (value: number) => (Number.isFinite(value) && value >= 0 ? value : null);
  let total: number | null = null;
  const code = /^(\d+):(\d{1,2}):(\d{1,2})[:;.](\d{1,3})$/.exec(typed);
  const clock = /^(\d+):(\d{1,2}(?:\.\d+)?)$/.exec(typed);
  const plain = /^(\d+(?:\.\d+)?|\.\d+)\s*s?$/.exec(typed);
  if (code) {
    // The inverse of `timecode`: nominal frames, less the numbers drop frame skips.
    const [hours, minutes, secs, frames] = code.slice(1).map(Number) as [number, number, number, number];
    const fps = Math.ceil(rate.num / rate.den);
    if (minutes > 59 || secs > 59 || frames >= fps) return null;
    let frame = (hours * 3600 + minutes * 60 + secs) * fps + frames;
    if (isDropFrame(rate)) {
      const elapsed = hours * 60 + minutes;
      frame -= Math.floor(fps / 15) * (elapsed - Math.floor(elapsed / 10));
    }
    return framesToFlicks(frame, rate);
  }
  if (clock) {
    const [minutes, secs] = clock.slice(1).map(Number) as [number, number];
    total = secs < 60 ? minutes * 60 + secs : null;
  } else if (plain) {
    total = seconds(Number(plain[1]));
  }
  if (total === null) return null;
  // A little over the exact value, so a time typed at a frame's start stays in that frame.
  return frameStart(Math.round(total * FLICKS_PER_SECOND) + 1, rate);
}

/** A part of the timeline between markers, for the range picker. */
export interface MarkedRange {
  label: string;
  from: number;
  to: number;
}

/** The parts between consecutive markers, from the start to the first and from the last to the end. */
export function markerRanges(markers: MarkerView[], duration: number): MarkedRange[] {
  const points = [
    { at: 0, name: t("export.range.start") },
    ...markers
      .filter((m) => m.at.flicks > 0 && m.at.flicks < duration)
      .map((m) => ({ at: m.at.flicks, name: m.name || m.at.timecode })),
    { at: duration, name: t("export.range.end") },
  ];
  if (points.length < 3) return [];
  return points.slice(1).map((end, index) => {
    const start = points[index]!;
    return { label: t("export.range", { from: start.name, to: end.name }), from: start.at, to: end.at };
  });
}

/** What the request is made against: the timeline and the presets this machine has. */
export interface ExportContext {
  format: Format;
  /** The timeline's length in flicks. */
  duration: number;
  presets: ExportPreset[];
}

const BIT_RATE = /^\d+(\.\d+)?\s*[km]?$/i;

/** The `export_start` request a form makes, or what to fix first, in a sentence. */
export function choiceOf(form: ExportForm, context: ExportContext): { choice: ExportChoice } | { problem: string } {
  const rate = parseRate(context.format.rate);
  const output = form.output.trim();
  if (!output) return { problem: t("export.problem.output") };
  const preset = context.presets.find((p) => p.name === form.preset);
  if (!preset) return { problem: t("export.problem.preset") };
  if (!preset.available) return { problem: t("export.problem.unavailable", { name: preset.name }) };
  const choice: ExportChoice = { output, preset: preset.name };
  if (!audioOnly(preset)) {
    if (form.resolution) choice.resolution = form.resolution;
    if (form.fps) choice.fps = form.fps;
    const bitrate = form.bitrate.trim();
    if (bitrate) {
      if (!BIT_RATE.test(bitrate)) return { problem: t("export.problem.bitrate") };
      choice.bitrate = bitrate.replace(/\s+/g, "");
    }
  }
  let start = 0;
  let end = context.duration;
  if (form.part) {
    const from = parseTime(form.from, rate);
    const to = parseTime(form.to, rate);
    if (from === null || to === null) return { problem: t("export.problem.part") };
    if (from >= context.duration) return { problem: t("export.problem.partStart") };
    if (to <= from) return { problem: t("export.problem.partEnd") };
    start = from;
    end = Math.min(to, context.duration);
    choice.range_flicks = [start, end];
  }
  if (form.cover) {
    const at = parseTime(form.coverAt, rate);
    if (at === null) return { problem: t("export.problem.coverTime") };
    if (at < start || at >= end) return { problem: t("export.problem.coverInside") };
    choice.cover_flicks = at - start;
  }
  if (form.subtitles) choice.subtitles = form.subtitles;
  if (form.overwrite) choice.overwrite = true;
  return { choice };
}

/** Where a cover switched on starts: its time when that is in the part exported, else the part's start. */
export function coverStart(form: ExportForm, rate: Rate): string {
  if (!form.part) return form.coverAt;
  const at = parseTime(form.coverAt, rate);
  const from = parseTime(form.from, rate);
  const to = parseTime(form.to, rate);
  const inside = at !== null && from !== null && to !== null && at >= from && at < to;
  return inside || from === null ? form.coverAt : form.from;
}

/** The size a resolution gives a canvas, as the engine works it out: the shorter side at `pixels`, both even. */
export function sizeFor(format: { width: number; height: number }, pixels: number | null): [number, number] {
  if (pixels === null) return [format.width, format.height];
  const factor = pixels / Math.min(format.width, format.height);
  const even = (value: number) => Math.max(2, Math.round((value * factor) / 2) * 2);
  return [even(format.width), even(format.height)];
}

/** Frames per second of a rate, to three decimals: 30, 29.97. */
export function fpsNumber(rate: string): number {
  const { num, den } = parseRate(rate);
  return Math.round((num / den) * 1000) / 1000;
}

/** A frame rate as people read it: 30, 29.97. */
export function fpsText(rate: string): string {
  return String(fpsNumber(rate));
}

/** A length in seconds as people read it: 1.5 s, 42 s, 2 min 5 s (in the interface's language). */
export function lengthText(seconds: number): string {
  if (seconds < 60) return t("export.length.seconds", { seconds: seconds < 10 ? Math.round(seconds * 10) / 10 : Math.round(seconds) });
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds - minutes * 60);
  return rest === 0 ? t("export.length.minutes", { minutes }) : t("export.length.minutesSeconds", { minutes, seconds: rest });
}

/** How far a job is, as a fraction (null before it counts) and in words. */
export function progressOf(status: JobStatus): { fraction: number | null; text: string } {
  const { done, total } = status.progress;
  const took = lengthText(status.elapsed_ms / 1000);
  switch (status.state) {
    case "succeeded":
      return { fraction: 1, text: t("export.progress.done", { took }) };
    case "cancelled":
      return { fraction: null, text: t("export.progress.cancelled") };
    case "failed":
      return { fraction: null, text: t("export.progress.failed") };
    case "running": {
      if (total === 0) return { fraction: null, text: t("export.progress.ready") };
      const fraction = Math.min(1, done / total);
      const parts = [t("export.progress.count", { done, total, percent: Math.floor(fraction * 100) })];
      if (done > 0 && done < total) {
        const left = ((status.elapsed_ms / done) * (total - done)) / 1000;
        parts.push(t("export.progress.left", { left: lengthText(left) }));
      }
      if (done >= total) parts.push(t("export.progress.checking"));
      return { fraction, text: `${parts.join(", ")}.` };
    }
  }
}

/** A finished export's report as label and value rows. */
export function reportRows(report: ExportReport): [string, string][] {
  const rows: [string, string][] = [
    [t("export.report.file"), report.output],
    [t("export.report.size"), readableSize(report.verified.size_bytes)],
    [t("export.report.length"), lengthText((report.verified.duration ?? report.duration) / FLICKS_PER_SECOND)],
  ];
  const video = report.verified.video;
  if (video) {
    const { codec, width, height } = video;
    rows.push([
      t("export.report.picture"),
      video.rate ? t("export.report.pictureAt", { codec, width, height, fps: fpsNumber(video.rate) }) : `${codec}, ${width} x ${height}`,
    ]);
  }
  const audio = report.verified.audio;
  if (audio) {
    const khz = audio.sample_rate ? `, ${t("export.report.khz", { khz: audio.sample_rate / 1000 })}` : "";
    const channels =
      audio.channels === 1 ? ` ${t("export.report.mono")}` : audio.channels === 2 ? ` ${t("export.report.stereo")}` : "";
    rows.push([t("export.report.sound"), `${audio.codec}${khz}${channels}`]);
  }
  if (report.cover) rows.push([t("export.report.cover"), report.cover]);
  if (report.subtitles) rows.push([t("export.report.subtitles"), report.subtitles]);
  rows.push([t("export.report.took"), lengthText(report.elapsed_ms / 1000)]);
  return rows;
}
