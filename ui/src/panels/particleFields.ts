// A particle clip's emitter (ADR-0038) as the inspector edits it: the engine's defaults for
// what it leaves out, its main settings as keyed numbers (`particles.<setting>`), and the
// merge patches `particles.set` takes. A value set back to its default sends null, which
// removes it, so the stored emitter stays sparse.

import type { ClipView, CommandCall, ParticleContentView, SpriteKind } from "../engine/types";
import { t } from "../i18n";
import type { KeyedField } from "./keyedFields";
import { patchAt } from "./keyedFields";

/** The engine's defaults for what an emitter leaves out. */
export const PARTICLE_DEFAULTS = {
  rate: 30,
  life: 2,
  speed: 0.25,
  direction: -90,
  spread: 30,
  gravity: 0,
  wind: 0,
  turbulence: 0,
  size: 0.02,
  opacity: 1,
  seed: 0,
  colors: ["#FFFFFF"],
  color_by: "life",
  blend: "add",
  sprite: "dot",
  emitter_shape: "point",
  emitter_position: [0, 0],
} as const;

export const SPRITES: SpriteKind[] = ["dot", "spark", "star", "ring", "square"];
export const EMITTER_SHAPES = ["point", "line", "circle", "rectangle"] as const;
/** The most colours an emitter takes. */
export const MAX_COLORS = 8;

/** The clip's emitter, or the defaults when the clip shows none. */
export function emitterOf(clip: ClipView): ParticleContentView {
  return clip.kind === "particles" ? ((clip.content as ParticleContentView | undefined) ?? {}) : {};
}

/** Every setting the panel shows, at its stored value or its default. */
export function particlesOf(clip: ClipView) {
  const p = emitterOf(clip);
  const d = PARTICLE_DEFAULTS;
  return {
    rate: p.rate ?? d.rate,
    life: p.life ?? d.life,
    speed: p.speed ?? d.speed,
    direction: p.direction ?? d.direction,
    spread: p.spread ?? d.spread,
    gravity: p.gravity ?? d.gravity,
    wind: p.wind ?? d.wind,
    turbulence: p.turbulence ?? d.turbulence,
    size: p.size ?? d.size,
    opacity: p.opacity ?? d.opacity,
    seed: p.seed ?? d.seed,
    colors: p.colors && p.colors.length > 0 ? p.colors : [...d.colors],
    color_by: p.color_by ?? d.color_by,
    blend: p.blend ?? d.blend,
    sprite: p.sprite?.kind ?? d.sprite,
    emitter_shape: p.emitter?.shape ?? d.emitter_shape,
    emitter_position: p.emitter?.position ?? ([...d.emitter_position] as [number, number]),
  };
}

export type ParticleSettings = ReturnType<typeof particlesOf>;

/**
 * The emitter's keyed numbers, by group. Distances are shown in % of the canvas height
 * (speeds per second, gravity per second squared), as the engine measures them in canvas
 * heights.
 */
export function particleFields(p: ParticleSettings): Record<"emission" | "motion" | "look" | "place", KeyedField[]> {
  const field = (path: string, plain: number | [number, number], extra: Omit<KeyedField, "path" | "plain">, part?: 0 | 1): KeyedField => ({
    path,
    plain,
    part,
    ...extra,
  });
  return {
    emission: [
      field("particles.rate", p.rate, { label: t("particles.rate"), unit: t("particles.unit.perSecond"), min: 0, max: 2000, range: [0, 400], help: t("particles.rate.help") }),
      field("particles.life", p.life, { label: t("particles.life"), unit: "s", min: 0.05, max: 30, range: [0.1, 10], step: 0.1, animatable: false }),
    ],
    motion: [
      field("particles.speed", p.speed, { label: t("particles.speed"), unit: "%/s", scale: 100, min: 0, range: [0, 200], help: t("particles.speed.help") }),
      field("particles.direction", p.direction, { label: t("particles.direction"), unit: "°", range: [-180, 180], help: t("particles.direction.help") }),
      field("particles.spread", p.spread, { label: t("particles.spread"), unit: "°", min: 0, max: 360, range: [0, 360] }),
      field("particles.gravity", p.gravity, { label: t("particles.gravity"), unit: "%/s²", scale: 100, range: [-200, 400], help: t("particles.gravity.help") }),
      field("particles.wind", p.wind, { label: t("particles.wind"), unit: "%/s²", scale: 100, range: [-100, 100] }),
      field("particles.turbulence", p.turbulence, { label: t("particles.turbulence"), unit: "%", scale: 100, min: 0, max: 200, range: [0, 50] }),
    ],
    look: [
      field("particles.size", p.size, { label: t("particles.size"), unit: "%", scale: 100, min: 0, max: 200, range: [0, 10], step: 0.1, help: t("particles.size.help") }),
      field("particles.opacity", p.opacity, { label: t("particles.opacity"), unit: "%", scale: 100, min: 0, max: 100, range: [0, 100] }),
    ],
    place: [
      field("particles.emitter.position", p.emitter_position, { label: t("particles.emitter.x"), unit: "%", scale: 100, range: [-100, 100] }, 0),
      field("particles.emitter.position", p.emitter_position, { label: t("particles.emitter.y"), unit: "%", scale: 100, range: [-100, 100] }, 1),
    ],
  };
}

/** The default of a setting by its path under `particles.`, for sparse patches. */
function fallbackOf(path: string): unknown {
  const defaults: Record<string, unknown> = {
    rate: PARTICLE_DEFAULTS.rate,
    life: PARTICLE_DEFAULTS.life,
    speed: PARTICLE_DEFAULTS.speed,
    direction: PARTICLE_DEFAULTS.direction,
    spread: PARTICLE_DEFAULTS.spread,
    gravity: 0,
    wind: 0,
    turbulence: 0,
    size: PARTICLE_DEFAULTS.size,
    opacity: PARTICLE_DEFAULTS.opacity,
    seed: 0,
    colors: PARTICLE_DEFAULTS.colors,
    color_by: PARTICLE_DEFAULTS.color_by,
    blend: PARTICLE_DEFAULTS.blend,
    "sprite.kind": PARTICLE_DEFAULTS.sprite,
    "emitter.shape": PARTICLE_DEFAULTS.emitter_shape,
    "emitter.position": PARTICLE_DEFAULTS.emitter_position,
  };
  return defaults[path];
}

/**
 * The `particles.set` call that gives the setting at `path` (under `particles.`, such as
 * `rate` or `emitter.position`) `value`; a default sends null, which removes it.
 */
export function particlesSet(clip: string, path: string, value: unknown): CommandCall {
  const same = JSON.stringify(value) === JSON.stringify(fallbackOf(path));
  return { command: "particles.set", args: { clip, set: patchAt(path, same ? null : value) } };
}

/** The colours with `index` changed to `color`, removed (null), or one added at the end (index past the end). */
export function recolored(colors: string[], index: number, color: string | null): string[] | null {
  if (color === null) return colors.length > 1 ? colors.filter((_, i) => i !== index) : null;
  if (index >= colors.length) return colors.length < MAX_COLORS ? [...colors, color] : null;
  return colors.map((c, i) => (i === index ? color : c));
}
