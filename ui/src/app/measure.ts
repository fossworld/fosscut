// Edits measured from media, as the window offers and reports them: what a tracked
// region can drive, and what stabilizing, splitting at scenes, tracking and making a
// multicam clip did, in a sentence of the interface's language.

import type { ClipView, MotionResult, MulticamResult, SceneSplit, TimelineView } from "../engine/types";
import { type MessageKey, language, t } from "../i18n";
import { FLICKS_PER_SECOND, type Rate, frameAt, timecode } from "../transport/time";

/** Below this, a multicam angle's match by sound is doubtful (the engine's `SYNC_CONFIDENCE`). */
export const SYNC_CONFIDENCE = 0.3;

/** The kinds of clip that can follow a tracked region: they have a place on the canvas. */
const FOLLOWERS = new Set(["text", "shape", "video", "image", "timeline"]);

/** The name of a follower's kind, in the interface's language. */
function kindName(kind: string): string {
  return FOLLOWERS.has(kind) ? t(`motion.target.${kind}` as MessageKey) : kind;
}

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
    label: t("motion.target.mask", { number: i + 1, shape: (mask.shape ?? "rectangle").replaceAll("_", " ") }),
  }));
  const others = timeline.tracks
    .flatMap((t) => t.clips)
    .filter((c) => c.id !== clip.id && FOLLOWERS.has(c.kind))
    .filter((c) => c.start.flicks < clip.end.flicks && c.end.flicks > clip.start.flicks)
    .map((c) => ({
      value: `clip:${c.id}`,
      label: t("motion.target.clip", {
        kind: kindName(c.kind),
        name: c.text ?? c.asset_name ?? c.timeline_name ?? c.shape ?? c.id,
      }),
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

/** A list in words: "a", "a and b", "a, b and c", with the language's "and". */
function listed(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} ${t("motion.and")} ${items[items.length - 1]}`;
}

/** `value` with exactly `digits` decimals, as the interface's language writes them. */
function fixed(value: number, digits: number): string {
  return new Intl.NumberFormat(language(), {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
    useGrouping: false,
  }).format(value);
}

/** What stabilizing did. */
export function stabilized(result: MotionResult): string {
  const zoom = result.motion.zoom ?? 1;
  const count = result.motion.frames;
  if (zoom <= 1.0005) return t("motion.stabilized.still", { count });
  return t("motion.stabilized.zoom", { count, zoom: fixed(zoom, 2) });
}

/** What splitting at scenes did; at most five cuts are named. */
export function splitAtScenes(result: SceneSplit, rate: Rate): string {
  const cuts = result.scenes.cuts;
  if (cuts.length === 0) return t("motion.scenes.found.none");
  const shown = cuts.slice(0, 5).map((c) => at(c, rate));
  if (cuts.length > 5) shown.push(t("motion.scenes.more", { count: cuts.length - 5 }));
  return t("motion.scenes.found", { count: cuts.length + 1, cuts: listed(shown) });
}

/** What tracking did, with the ranges where the region was lost. */
export function tracked(result: MotionResult, rate: Rate): string {
  const lost = result.motion.lost ?? [];
  const done = t("motion.tracked", { count: result.motion.frames });
  if (lost.length === 0) return done;
  const ranges = lost.map((r) => t("motion.tracked.range", { from: at(r.start, rate), to: at(r.end, rate) }));
  return t("motion.tracked.lost", { done, ranges: listed(ranges) });
}

/** What making a multicam clip did, with each angle's offset and doubtful matches. */
export function multicamMade(result: MulticamResult, count: number): string {
  const made = t("motion.multicam.made", { count });
  const sync = result.sync;
  if (!sync) return t("motion.multicam.together", { made });
  const first = sync.offsets[0] ?? 0;
  const offsets = sync.offsets.slice(1).map((offset, i) => {
    const seconds = (offset - first) / FLICKS_PER_SECOND;
    const number = i + 2;
    const amount = t("motion.multicam.seconds", { seconds: fixed(Math.abs(seconds), 2) });
    if (Math.abs(seconds) < 0.005) return t("motion.multicam.with", { number });
    return t(seconds > 0 ? "motion.multicam.after" : "motion.multicam.before", { number, amount });
  });
  const doubtful = sync.confidence
    .map((c, i) => ({ c, angle: i + 1 }))
    .filter(({ c, angle }) => angle > 1 && c < SYNC_CONFIDENCE)
    .map(({ angle }) => t("motion.multicam.angle", { number: angle }));
  const lined = t("motion.multicam.lined", { made, offsets: listed(offsets) });
  if (doubtful.length === 0) return lined;
  return t("motion.multicam.doubtful", { lined, angles: listed(doubtful), count: doubtful.length });
}
