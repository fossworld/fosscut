// Edits planned from analyses (P2): reframing, camera tracking, vertical versions, beat
// markers and cuts, and shorts from long clips, said in a sentence of the interface's
// language; and what a missing analysis needs before it can run.

import type {
  ApiError,
  BeatCutsResult,
  BeatMarkersResult,
  MatteSource,
  ReframeResult,
  ShortCandidate,
  ShortCandidates,
  ShortResult,
  TimelineView,
  VerticalResult,
} from "../engine/types";
import { type MessageKey, formatNumber, t } from "../i18n";
import { FLICKS_PER_SECOND } from "../transport/time";
import { BODY_NEEDS, backgroundNeeds } from "./assist";

/** The engine's code for an analysis that must be run first. */
export const ANALYSIS_MISSING = "fosscut.analysis.missing";

/** What a missing-analysis refusal names: the asset, the analyzer and its options. */
export interface MissingAnalysis {
  asset: string | null;
  analyzer: string;
  options: Record<string, unknown> | null;
  /** The local model it runs, when the desktop says so: a speech model for a transcript. */
  model: string | null;
  needsModel: boolean;
}

/** The analysis `error` says must run first, or null for any other refusal. */
export function missingAnalysis(error: ApiError | null | undefined): MissingAnalysis | null {
  if (!error || error.code !== ANALYSIS_MISSING) return null;
  const details = (error.details ?? {}) as { asset?: unknown; analyzer?: unknown; options?: unknown; model?: unknown; needs_model?: unknown };
  const named = typeof details.analyzer === "string" ? details.analyzer : /\b(pose|matte|scenes|beats|transcript|loudness|silences)\b/.exec(error.message)?.[1];
  if (!named) return null;
  const all = details.options && typeof details.options === "object" ? (details.options as Record<string, unknown>) : {};
  const mine = all[named];
  return {
    asset: typeof details.asset === "string" ? details.asset : null,
    analyzer: named,
    options: mine && typeof mine === "object" && Object.keys(mine).length > 0 ? (mine as Record<string, unknown>) : null,
    model: typeof details.model === "string" ? details.model : null,
    needsModel: details.needs_model === true,
  };
}

/**
 * The models an analyzer needs before it runs: a pose or a matte run vision models, a
 * transcript the speech model the desktop names. When it names none, the engine's refusal
 * says which to install.
 */
export function analysisNeeds(missing: MissingAnalysis): string[] {
  if (missing.analyzer === "pose") return BODY_NEEDS;
  if (missing.analyzer === "matte") return backgroundNeeds(matteModel(missing));
  return missing.needsModel && missing.model ? [missing.model] : [];
}

/** Whose matte a missing matte analysis asks for: the subject's unless it says a person's. */
export function matteModel(missing: MissingAnalysis): MatteSource {
  return missing.options?.model === "person" ? "person" : "subject";
}

const ANALYZERS = new Set(["pose", "matte", "scenes", "beats", "transcript"]);

/** An analyzer's name in the interface's language; others as the engine names them. */
export function analyzerName(analyzer: string): string {
  return ANALYZERS.has(analyzer) ? t(`motion.analyzer.${analyzer}` as MessageKey) : analyzer;
}

/** The id of a clip of `timeline` that shows `asset`, to analyze it through. */
export function clipShowing(timeline: TimelineView | null, asset: string | null): string | null {
  if (!timeline || !asset) return null;
  for (const track of timeline.tracks) {
    const clip = track.clips.find((c) => c.asset === asset);
    if (clip) return clip.id;
  }
  return null;
}

const percent = (part: number, whole: number) => (whole > 0 ? Math.round((part / whole) * 100) : 0);

/** What reframing or camera tracking did, in a sentence. */
export function reframed(result: ReframeResult): string {
  const { frames, subject_frames: found, shots } = result.reframe;
  return t("motion.reframe.done", { count: Math.max(1, shots.length), found: percent(found, frames) });
}

/** What making a vertical version did, in a sentence. */
export function verticalMade(result: VerticalResult): string {
  const { clips, centred } = result.vertical;
  const base = t("motion.vertical.done", { count: clips });
  return centred.length > 0 ? `${base} ${t("motion.vertical.centred", { count: centred.length })}` : base;
}

/** What marking a music clip's beats did, in a sentence. */
export function beatsMarked(result: BeatMarkersResult): string {
  const { tempo_bpm: bpm, markers } = result.beats;
  const tempo = bpm ? t("motion.beats.tempo", { bpm: Math.round(bpm) }) : t("motion.beats.noTempo");
  return t("motion.beats.done", { count: markers, tempo });
}

/** What cutting to the beat did, in a sentence. */
export function beatsCut(result: BeatCutsResult): string {
  const { snapped, unsnapped } = result.cuts;
  if (snapped.length === 0) return t("motion.cuts.none");
  const base = t("motion.cuts.done", { count: snapped.length });
  return unsnapped.length > 0 ? `${base} ${t("motion.cuts.left", { count: unsnapped.length })}` : base;
}

/** What looking for shorts found, in a sentence. */
export function shortsFound(result: ShortCandidates): string {
  if (result.candidates.length === 0) return t("motion.shorts.none");
  return t("motion.shorts.found", { count: result.candidates.length, sentences: result.sentences });
}

/** What making a short did, in a sentence. */
export function shortMade(result: ShortResult): string {
  return t("motion.short.done", { name: result.short.name, seconds: formatNumber(Math.round((result.short.duration / FLICKS_PER_SECOND) * 10) / 10) });
}

/** Where a candidate is in its media, as `shorts_make` takes it; its timeline place when the span is missing. */
export function candidateRange(candidate: ShortCandidate): [number, number] {
  const start = candidate.span?.start?.flicks;
  const end = candidate.span?.end?.flicks;
  return typeof start === "number" && typeof end === "number" ? [start, end] : [candidate.timeline_start, candidate.timeline_end];
}

/** A short's length in seconds, rounded for showing. */
export function candidateSeconds(candidate: ShortCandidate): number {
  const seconds = candidate.seconds || (candidate.timeline_end - candidate.timeline_start) / FLICKS_PER_SECOND;
  return Math.round(seconds * 10) / 10;
}

/** The measurements behind a candidate's score that people read at a glance, in words. */
export function candidateReasons(candidate: ShortCandidate): string[] {
  const reasons = candidate.reasons ?? {};
  const number = (name: string) => {
    const value = reasons[name];
    return typeof value === "number" && Number.isFinite(value) ? value : null;
  };
  const shown: string[] = [];
  const pace = number("words_per_second");
  if (pace !== null) shown.push(t("motion.shorts.reason.pace", { value: formatNumber(Math.round(pace * 10) / 10) }));
  const pauses = number("pause_share");
  if (pauses !== null) shown.push(t("motion.shorts.reason.pauses", { value: Math.round(pauses * 100) }));
  const lively = number("loudness_variation_lu");
  if (lively !== null) shown.push(t("motion.shorts.reason.loudness", { value: formatNumber(Math.round(lively * 10) / 10) }));
  return shown;
}
