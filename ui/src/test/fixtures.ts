// Engine data for tests, shaped as the engine returns it.

import type { ClipView, LogEntry, Moment, TimelineView, TrackView } from "../engine/types";
import { FLICKS_PER_SECOND } from "../transport/time";

export function moment(seconds: number): Moment {
  const frames = Math.round(seconds * 30);
  return {
    flicks: Math.round(seconds * FLICKS_PER_SECOND),
    frames,
    seconds: String(seconds),
    timecode: `00:00:${String(Math.floor(seconds)).padStart(2, "0")}:${String(frames % 30).padStart(2, "0")}`,
  };
}

export function clip(id: string, kind: string, start: number, end: number, extra: Partial<ClipView> = {}): ClipView {
  return { id, kind, start: moment(start), end: moment(end), duration: moment(end - start), ...extra };
}

export function track(id: string, kind: string, index: number, clips: ClipView[], name?: string): TrackView {
  return name ? { id, kind, index, name, clips } : { id, kind, index, clips };
}

/** 6 s: a video clip on the bottom track, a 2 s title above it. */
export function timeline(revision = 4): TimelineView {
  return {
    timeline: "timeline_0000",
    name: "Timeline 1",
    revision,
    format: { width: 1920, height: 1080, rate: "30" },
    duration: moment(6),
    tracks: [
      track("track_0002", "video", 0, [clip("clip_0003", "video", 0, 6, { asset_name: "clip.mp4" })]),
      track("track_0004", "text", 1, [clip("clip_0005", "text", 0, 2, { text: "Live edit" })]),
    ],
  };
}

export function entry(seq: number, actor: string, commands: string[], kind: LogEntry["kind"] = "commit"): LogEntry {
  return { seq, actor, commands, kind, revision_after: seq, at_ms: 1_000 * seq };
}
