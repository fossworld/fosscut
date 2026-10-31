// Local models in the window (ADR-0033): which models each feature needs, which of those
// to offer to install (from the listing, or from the engine's refusal), voices grouped as
// the text to speech panel lists them, and what translating and dubbing did, in a
// sentence of the interface's language.

import type {
  ApiError,
  DubCaptionsResult,
  MatteSource,
  ModelInfo,
  ModelListing,
  TimelineView,
  TranslateCaptionsResult,
  VoiceInfo,
} from "../engine/types";
import { type MessageKey, formatNumber, t } from "../i18n";

/** The library every model runs on. */
export const RUNTIME = "onnxruntime";

/** The engine's codes for a model, or the runtime, that is not installed. */
export const MODEL_MISSING = new Set(["fosscut.model.not_installed", "fosscut.model.runtime_missing"]);

/** What uses local models, as the window names them. */
export type AssistKind = "background" | "body" | "speech" | "preview" | "translate" | "dub";

/** The effect that removes the background. */
export const SUBJECT_KEY = "key.subject";

/** True for effects that follow a person's pose. */
export function isBodyEffect(type: string): boolean {
  return type.startsWith("body.");
}

/** The models a background removal of `source` needs, the runtime first. */
export function backgroundNeeds(source: MatteSource): string[] {
  return [RUNTIME, source === "subject" ? "birefnet-lite" : "mediapipe-pose-full"];
}

/** The models body effects need. */
export const BODY_NEEDS = [RUNTIME, "mediapipe-pose-full"];

/** The models text to speech and dubbing need. */
export const VOICE_NEEDS = [RUNTIME, "kokoro-82m"];

/** The models a translation into `to` needs. */
export function translateNeeds(to: "hu" | "en"): string[] {
  return [RUNTIME, to === "hu" ? "opus-mt-en-hu" : "opus-mt-hu-en"];
}

/** True when `name` can be used: installed, or for the runtime, found anywhere. */
function present(listing: ModelListing, name: string): boolean {
  if (name === RUNTIME && listing.runtime) return true;
  return listing.models.some((m) => m.name === name && m.installed);
}

/** The models of `needs` that are missing, in that order, as the listing has them. */
export function missingModels(listing: ModelListing, needs: string[]): ModelInfo[] {
  return needs
    .filter((name) => !present(listing, name))
    .map((name) => listing.models.find((m) => m.name === name))
    .filter((m): m is ModelInfo => m !== undefined);
}

/** True for the engine's refusal of a model that is not installed. */
export function isModelMissing(error: ApiError): boolean {
  return MODEL_MISSING.has(error.code);
}

/**
 * The models to install after the engine refused for want of one: those it names
 * (`details.what`, as `birefnet-lite and onnxruntime` or `kokoro-82m/voices/af_heart.bin`),
 * the runtime when it is what is missing, and otherwise the ones `needs` lacks. Only
 * models the listing has and does not show installed; the runtime first.
 */
export function modelsFor(error: ApiError, listing: ModelListing, needs: string[]): ModelInfo[] {
  if (!isModelMissing(error)) return [];
  const what = (error.details as { what?: unknown } | undefined)?.what;
  const named = typeof what === "string" ? what.split(" and ").map((part) => part.trim().split("/")[0] ?? "") : [];
  if (error.code === "fosscut.model.runtime_missing") named.push(RUNTIME);
  const known = named.filter((name) => listing.models.some((m) => m.name === name));
  const wanted = known.length > 0 ? known : needs;
  const ordered = [...new Set(wanted)].sort((a, b) => Number(b === RUNTIME) - Number(a === RUNTIME));
  return missingModels(listing, ordered);
}

/** Bytes in megabytes, as the interface's language writes them: `224`, `11,9`. */
export function megabytes(bytes: number): string {
  const mb = bytes / 1_000_000;
  return formatNumber(mb >= 100 ? Math.round(mb) : Math.round(mb * 10) / 10);
}

/** The total download of `models`. */
export function downloadSize(models: ModelInfo[]): number {
  return models.reduce((sum, m) => sum + m.size_bytes, 0);
}

/** A voice's name as people read it: `af_heart` is Heart. */
export function voiceName(name: string): string {
  const word = name.slice(name.indexOf("_") + 1).replaceAll("_", " ");
  return word.charAt(0).toUpperCase() + word.slice(1);
}

/** A group of voices of one accent and gender. */
export interface VoiceGroup {
  label: string;
  voices: VoiceInfo[];
}

const ACCENTS: VoiceInfo["accent"][] = ["american", "british"];
const GENDERS: VoiceInfo["gender"][] = ["female", "male"];

/** The voices by accent, then gender, each group in the model's order; empty groups left out. */
export function voiceGroups(voices: VoiceInfo[]): VoiceGroup[] {
  return ACCENTS.flatMap((accent) =>
    GENDERS.map((gender) => ({
      label: t("voice.group", { accent: t(`voice.accent.${accent}`), gender: t(`voice.gender.${gender}`) }),
      voices: voices.filter((v) => v.accent === accent && v.gender === gender),
    })),
  ).filter((group) => group.voices.length > 0);
}

/** The voice to start with: the first installed one, or the first. */
export function defaultVoice(voices: VoiceInfo[]): string {
  return (voices.find((v) => v.installed) ?? voices[0])?.name ?? "";
}

/** The track a clip is on. */
export function trackOf(timeline: TimelineView, clip: string): string | undefined {
  return timeline.tracks.find((track) => track.clips.some((c) => c.id === clip))?.id;
}

/** What translating captions did. */
export function translatedReport(result: TranslateCaptionsResult): string {
  const { pages, skipped, mode, pair } = result.translation;
  const to = pair.endsWith("hu") ? "hu" : "en";
  if (pages.length === 0) return t("voice.translated.none");
  const key = `voice.translated.${mode === "replace" ? "replaced" : "bilingual"}.${to}` as MessageKey;
  const done = t(key, { count: pages.length });
  return skipped.length > 0 ? t("voice.translated.skipped", { done, count: skipped.length }) : done;
}

/** What dubbing captions did, and how many pages run past the next. */
export function dubbedReport(result: DubCaptionsResult): string {
  const { pages, skipped } = result.dub;
  if (pages.length === 0) return t("voice.dubbed.none");
  const parts = [t("voice.dubbed", { count: pages.length, voice: voiceName(result.dub.voice) })];
  const long = pages.filter((p) => p.warning).length;
  if (long > 0) parts.push(t("voice.dubbed.long", { count: long }));
  if (skipped.length > 0) parts.push(t("voice.dubbed.skipped", { count: skipped.length }));
  return parts.join(" ");
}
