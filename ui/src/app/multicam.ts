// Multicam editing in the window (ADR-0029): which chosen media can become a multicam
// clip and the request that makes it, which angle each part of a multicam clip shows, and
// which part a cut at the playhead applies to.

import type { AssetView, ClipView, MulticamRequest, TimelineView } from "../engine/types";

/** True for a multicam clip: a compound clip that shows one of its angles. */
export function isMulticam(clip: ClipView | null | undefined): clip is ClipView & { angles: NonNullable<ClipView["angles"]> } {
  return Boolean(clip && clip.kind === "timeline" && clip.angles && clip.angles.length > 0);
}

/** The number, from 1, of the angle a multicam clip shows; null for other clips. */
export function angleNumber(clip: ClipView): number | null {
  if (!isMulticam(clip) || !clip.angle) return null;
  const index = clip.angles.findIndex((a) => a.id === clip.angle);
  return index >= 0 ? index + 1 : null;
}

/** How an angle is called: its track's name without the number, or the number alone. */
export function angleTitle(clip: ClipView, number: number): string {
  const name = clip.angles?.[number - 1]?.name ?? "";
  const plain = name.replace(/^Angle \d+:\s*/, "");
  return plain ? `Angle ${number}: ${plain}` : `Angle ${number}`;
}

/** The chosen media that can be angles: video whose file is there, in the order chosen. */
export function multicamAngles(media: AssetView[], chosen: string[]): AssetView[] {
  return chosen
    .map((id) => media.find((a) => a.id === id))
    .filter((a): a is AssetView => a !== undefined && a.kind === "video" && !a.missing);
}

/** True when every angle has sound to line them up by. */
export function canSync(angles: AssetView[]): boolean {
  return angles.length > 0 && angles.every((a) => a.audio !== null && a.audio !== undefined);
}

/** What the multicam form holds. */
export interface MulticamForm {
  /** Line the angles up by their sound. */
  sync: boolean;
  /** Whose sound plays, by angle number from 1. */
  sound: number;
  name: string;
}

export const NEW_MULTICAM: MulticamForm = { sync: true, sound: 1, name: "" };

/**
 * The request that makes a multicam clip of `angles` at the playhead: synced by sound
 * when asked and every angle has sound, with the sound of an angle that exists, and a
 * name only when one is typed.
 */
export function multicamRequest(form: MulticamForm, angles: AssetView[], playhead: number): MulticamRequest {
  const request: MulticamRequest = {
    angles: angles.map((a) => a.id),
    sync: form.sync && canSync(angles),
    start: Math.max(0, Math.round(playhead)),
  };
  const sound = Math.min(Math.max(1, Math.round(form.sound)), Math.max(1, angles.length));
  if (sound !== 1) request.sound = sound;
  const name = form.name.trim();
  if (name) request.name = name;
  return request;
}

/**
 * The part of a multicam clip a cut at `at` applies to: the selected clip when it is a
 * multicam clip under `at`, or else the part of the same multicam clip (on its track,
 * showing the same timeline) that is under `at`, as earlier cuts leave the selection on
 * the part before them. Null when there is none.
 */
export function multicamPartAt(timeline: TimelineView, selection: string | null, at: number): ClipView | null {
  if (!selection) return null;
  const inside = (c: ClipView) => c.start.flicks <= at && at < c.end.flicks;
  for (const track of timeline.tracks) {
    const selected = track.clips.find((c) => c.id === selection);
    if (!selected) continue;
    if (!isMulticam(selected)) return null;
    if (inside(selected)) return selected;
    return track.clips.find((c) => isMulticam(c) && c.timeline === selected.timeline && inside(c)) ?? null;
  }
  return null;
}
