// Edits measured from media, as the window offers and reports them: what a tracked
// region can drive, and what stabilizing, splitting at scenes, tracking and making a
// multicam clip did, in a sentence.

import type { ClipView, MotionResult, MulticamResult, SceneSplit, TimelineView } from "../engine/types";
import { FLICKS_PER_SECOND, type Rate, frameAt, timecode } from "../transport/time";

/** Below this, a multicam angle's match by sound is doubtful (the engine's `SYNC_CONFIDENCE`). */
export const SYNC_CONFIDENCE = 0.3;

/** The kinds of clip that can follow a tracked region: they have a place on the canvas. */
const FOLLOWERS = new Set(["text", "shape", "video", "image", "timeline"]);

const NAMES: Record<string, string> = {
  text: "Title",
  shape: "Sticker",
  video: "Video",
  image: "Image",
  timeline: "Compound",
};

/** Something a tracked region can drive: a mask of the clip, or another clip. */
export interface TrackTarget {
  /** `mask:<id>` or `clip:<id>`, as a select's value. */
  value: string;
  label: string;
}

/**
 * What can follow a region tracked in `clip`: each of its masks, then the titles,
 * stickers and other clips with a place on the canvas that show while it does.
 */
export function trackTargets(clip: ClipView, timeline: TimelineView): TrackTarget[] {
  const masks = (clip.masks ?? []).map((mask, i) => ({
    value: `mask:${mask.id}`,
    label: `Mask ${i + 1}: ${(mask.shape ?? "rectangle").replaceAll("_", " ")}`,
  }));
  const others = timeline.tracks
    .flatMap((t) => t.clips)
    .filter((c) => c.id !== clip.id && FOLLOWERS.has(c.kind))
    .filter((c) => c.start.flicks < clip.end.flicks && c.end.flicks > clip.start.flicks)
    .map((c) => ({
      value: `clip:${c.id}`,
      label: `${NAMES[c.kind] ?? c.kind}: ${c.text ?? c.asset_name ?? c.timeline_name ?? c.shape ?? c.id}`,
    }));
  return [...masks, ...others];
}

/** A track target's value as the request's `mask` or `follow`. */
export function targetOf(value: string): { mask: string } | { follow: string } | null {
  const [kind, id] = [value.slice(0, value.indexOf(":")), value.slice(value.indexOf(":") + 1)];
  if (!id || value.indexOf(":") < 0) return null;
  if (kind === "mask") return { mask: id };
  if (kind === "clip") return { follow: id };
  return null;
}

const at = (flicks: number, rate: Rate) => timecode(frameAt(flicks, rate), rate);

/** A list in words: "a", "a and b", "a, b and c". */
function listed(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/** What stabilizing did. */
export function stabilized(result: MotionResult): string {
  const zoom = result.motion.zoom ?? 1;
  const frames = `${result.motion.frames} frames`;
  if (zoom <= 1.0005) return `Stabilized over ${frames}; no zoom was needed.`;
  return `Stabilized over ${frames}, zoomed in ${zoom.toFixed(2)}x to hide the moving edges.`;
}

/** What splitting at scenes did; at most five cuts are named. */
export function splitAtScenes(result: SceneSplit, rate: Rate): string {
  const cuts = result.scenes.cuts;
  if (cuts.length === 0) return "No scene changes found: the clip is one shot.";
  const shown = cuts.slice(0, 5).map((c) => at(c, rate));
  if (cuts.length > 5) shown.push(`${cuts.length - 5} more`);
  return `Split into ${cuts.length + 1} shots at ${listed(shown)}.`;
}

/** What tracking did, with the ranges where the region was lost. */
export function tracked(result: MotionResult, rate: Rate): string {
  const lost = result.motion.lost ?? [];
  const done = `Tracked over ${result.motion.frames} frames.`;
  if (lost.length === 0) return done;
  const ranges = lost.map((r) => `${at(r.start, rate)} to ${at(r.end, rate)}`);
  return `${done} Lost the region from ${listed(ranges)}; it holds its last place there.`;
}

/** What making a multicam clip did, with each angle's offset and doubtful matches. */
export function multicamMade(result: MulticamResult, count: number): string {
  const made = `Made a multicam clip of ${count} angles`;
  const sync = result.sync;
  if (!sync) return `${made}, all starting together.`;
  const first = sync.offsets[0] ?? 0;
  const offsets = sync.offsets.slice(1).map((offset, i) => {
    const seconds = (offset - first) / FLICKS_PER_SECOND;
    const amount = `${Math.abs(seconds).toFixed(2)} s`;
    const when = Math.abs(seconds) < 0.005 ? "with angle 1" : seconds > 0 ? `${amount} after angle 1` : `${amount} before angle 1`;
    return `angle ${i + 2} starts ${when}`;
  });
  const doubtful = sync.confidence
    .map((c, i) => ({ c, angle: i + 1 }))
    .filter(({ c, angle }) => angle > 1 && c < SYNC_CONFIDENCE)
    .map(({ angle }) => `angle ${angle}`);
  const lined = `${made}, lined up by their sound: ${listed(offsets)}.`;
  if (doubtful.length === 0) return lined;
  return `${lined} The match for ${listed(doubtful)} is doubtful; check ${doubtful.length === 1 ? "it" : "them"} by eye.`;
}
