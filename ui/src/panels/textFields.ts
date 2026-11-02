// A text clip's style as people edit it, with the engine's defaults filled in, and the
// arguments an edit turns into for `text.set` (ADR-0027). Style edits are sparse JSON
// merge patches: a field set back to its default sends null, which resets it, so the
// stored style stays sparse. A look (outline, box, shadow, glow) always sends `enabled`
// with its fields, because the engine needs it to read the look.

import type {
  ClipView,
  RevealEasing,
  RevealMotion,
  RevealOrder,
  RevealUnit,
  TextAlign,
  TextCase,
  TextStyleView,
} from "../engine/types";
import { type MessageKey, t } from "../i18n";
import { FLICKS_PER_SECOND } from "../transport/time";

/** Every style field with a value: a stored style with the defaults filled in. */
export interface TextStyle {
  font: string;
  size: number;
  color: string;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  case: TextCase;
  align: TextAlign;
  letter_spacing: number;
  line_height: number;
  stroke: { enabled: boolean; color: string; width: number };
  box: { enabled: boolean; color: string; padding: number; radius: number };
  shadow: { enabled: boolean; color: string; offset: [number, number]; blur: number };
  glow: { enabled: boolean; color: string; radius: number };
}

export type Look = "stroke" | "box" | "shadow" | "glow";

export const LOOKS: Look[] = ["stroke", "box", "shadow", "glow"];

/** The engine's defaults for a text style with nothing stored. */
export const TEXT_DEFAULTS: TextStyle = {
  font: "Inter",
  size: 0.08,
  color: "#FFFFFF",
  bold: false,
  italic: false,
  underline: false,
  case: "as_written",
  align: "center",
  letter_spacing: 0,
  line_height: 1.2,
  stroke: { enabled: false, color: "#000000", width: 0.12 },
  box: { enabled: false, color: "#000000B3", padding: 0.25, radius: 0.3 },
  shadow: { enabled: false, color: "#000000B3", offset: [0.06, 0.06], blur: 0.08 },
  glow: { enabled: false, color: "#FFFFFF", radius: 0.3 },
};

export const CASES: TextCase[] = ["as_written", "upper", "lower", "title"];
export const ALIGNS: TextAlign[] = ["left", "center", "right"];
export const REVEAL_UNITS: RevealUnit[] = ["letter", "word", "line"];
export const REVEAL_MOTIONS: RevealMotion[] = ["appear", "fade", "rise", "blur", "pop", "zoom", "slide_left", "slide_right", "drop", "collapse"];
/** The easings a reveal offers by name; empty is each motion's own. */
export const REVEAL_EASINGS = ["linear", "ease_in", "ease_out", "ease_in_out", "back", "bounce", "elastic"] as const;
export const REVEAL_ORDERS: RevealOrder[] = ["forward", "backward", "center", "edges", "random"];
/** How much each part's motion overlaps the next one's when the reveal does not say. */
export const REVEAL_OVERLAP = 0.5;

/** How a number field is shown: `scale` times the engine value, within `min` and `max` shown. */
export interface NumberSpec {
  /** The field's name, translated where it shows. */
  label: MessageKey;
  min: number;
  max: number;
  step: number;
  /** Shown value = engine value x scale. */
  scale: number;
  unit: string;
}

/**
 * The number fields by path: top-level fields (`size`) and looks' fields (`shadow.blur`);
 * `shadow.x` and `shadow.y` are the parts of the shadow's offset. Lengths are in em.
 */
export const NUMBERS: Record<string, NumberSpec> = {
  size: { label: "text.number.size", min: 0.5, max: 100, step: 0.5, scale: 100, unit: "%" },
  letter_spacing: { label: "text.number.letter_spacing", min: -0.5, max: 2, step: 0.01, scale: 1, unit: "em" },
  line_height: { label: "text.number.line_height", min: 0.5, max: 4, step: 0.05, scale: 1, unit: "x" },
  "stroke.width": { label: "text.number.stroke.width", min: 0, max: 1, step: 0.01, scale: 1, unit: "em" },
  "box.padding": { label: "text.number.box.padding", min: 0, max: 2, step: 0.01, scale: 1, unit: "em" },
  "box.radius": { label: "text.number.box.radius", min: 0, max: 1, step: 0.01, scale: 1, unit: "" },
  "shadow.x": { label: "text.number.shadow.x", min: -1, max: 1, step: 0.01, scale: 1, unit: "em" },
  "shadow.y": { label: "text.number.shadow.y", min: -1, max: 1, step: 0.01, scale: 1, unit: "em" },
  "shadow.blur": { label: "text.number.shadow.blur", min: 0, max: 1, step: 0.01, scale: 1, unit: "em" },
  "glow.radius": { label: "text.number.glow.radius", min: 0, max: 2, step: 0.01, scale: 1, unit: "em" },
};

/** The number fields of each look, in the order they show. */
export const LOOK_NUMBERS: Record<Look, string[]> = {
  stroke: ["stroke.width"],
  box: ["box.padding", "box.radius"],
  shadow: ["shadow.x", "shadow.y", "shadow.blur"],
  glow: ["glow.radius"],
};

/** The clip's style with every field at its stored value or its default. */
export function styleOf(clip: ClipView): TextStyle {
  const s: TextStyleView = clip.style ?? {};
  const d = TEXT_DEFAULTS;
  return {
    font: s.font ?? d.font,
    size: s.size ?? d.size,
    color: s.color ?? d.color,
    bold: s.bold ?? d.bold,
    italic: s.italic ?? d.italic,
    underline: s.underline ?? d.underline,
    case: s.case ?? d.case,
    align: s.align ?? d.align,
    letter_spacing: s.letter_spacing ?? d.letter_spacing,
    line_height: s.line_height ?? d.line_height,
    stroke: { ...d.stroke, ...s.stroke },
    box: { ...d.box, ...s.box },
    shadow: { ...d.shadow, ...s.shadow },
    glow: { ...d.glow, ...s.glow },
  };
}

function lookOf(path: string): [Look, string] | null {
  const [head, field] = path.split(".");
  return field !== undefined && LOOKS.includes(head as Look) ? [head as Look, field] : null;
}

/** The value of `path` in a full style. */
export function valueAt(style: TextStyle, path: string): unknown {
  const look = lookOf(path);
  if (!look) return (style as unknown as Record<string, unknown>)[path];
  const [name, field] = look;
  if (name === "shadow" && (field === "x" || field === "y")) return style.shadow.offset[field === "x" ? 0 : 1];
  return (style[name] as Record<string, unknown>)[field];
}

/** The number a field shows for its engine value, rounded to its step. */
export function shownNumber(path: string, value: number): number {
  const spec = NUMBERS[path];
  if (!spec) return value;
  return Math.round(value * spec.scale * 10000) / 10000;
}

const COLOR = /^#([0-9A-F]{6}|[0-9A-F]{8})$/;

/** A colour as the engine stores it (`#RRGGBB` or `#RRGGBBAA`), or null if it is not one. */
export function normalColor(color: string): string | null {
  const upper = color.trim().toUpperCase();
  return COLOR.test(upper) ? upper : null;
}

/** The `#rrggbb` a colour input shows for a stored colour, without its alpha. */
export function rgbOf(color: string): string {
  return (normalColor(color) ?? "#FFFFFF").slice(0, 7).toLowerCase();
}

/** A colour picked as `#rrggbb`, keeping the alpha of the colour it replaces. */
export function picked(current: string, rgb: string): string | null {
  const base = normalColor(rgb);
  if (!base || base.length !== 7) return null;
  const alpha = (normalColor(current) ?? "").slice(7);
  return base + alpha;
}

function same(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The engine value of `path` from a shown value, or null when it is not valid there. */
function checked(path: string, value: unknown, fonts: string[]): unknown {
  const field = lookOf(path)?.[1] ?? path;
  const number = NUMBERS[path];
  if (number) {
    if (typeof value !== "number" || !Number.isFinite(value)) return null;
    const engine = value / number.scale;
    if (value < number.min || value > number.max) return null;
    return Math.round(engine * 100000) / 100000;
  }
  if (field === "color") return typeof value === "string" ? normalColor(value) : null;
  if (field === "enabled" || path === "bold" || path === "italic" || path === "underline") {
    return typeof value === "boolean" ? value : null;
  }
  if (path === "case") return CASES.includes(value as TextCase) ? value : null;
  if (path === "align") return ALIGNS.includes(value as TextAlign) ? value : null;
  if (path === "font") return typeof value === "string" && fonts.includes(value) ? value : null;
  return null;
}

/**
 * The `text.set` style patch for a new value of one field, in the field's shown unit (a
 * size of 8 is 8% of the canvas height). Null when the value is not valid or unchanged.
 * A value at its default resets the field with null; `fonts` are the families the
 * project can use.
 */
export function stylePatch(
  clip: ClipView,
  path: string,
  shown: unknown,
  fonts: string[] = [TEXT_DEFAULTS.font],
): Record<string, unknown> | null {
  const value = checked(path, shown, fonts);
  if (value === null) return null;
  const style = styleOf(clip);
  if (same(value, valueAt(style, path))) return null;
  const reset = same(value, valueAt(TEXT_DEFAULTS, path));
  const look = lookOf(path);
  if (!look) return { [path]: reset ? null : value };
  const [name, field] = look;
  if (field === "enabled") return { [name]: { enabled: value } };
  if (name === "shadow" && (field === "x" || field === "y")) {
    const offset: [number, number] = [...style.shadow.offset];
    offset[field === "x" ? 0 : 1] = value as number;
    const back = same(offset, TEXT_DEFAULTS.shadow.offset);
    return { shadow: { enabled: style.shadow.enabled, offset: back ? null : offset } };
  }
  return { [name]: { enabled: style[name].enabled, [field]: reset ? null : value } };
}

/** A reveal as the panel edits it; `unit` is empty when the text shows at once. */
export interface RevealDraft {
  unit: RevealUnit | "";
  motion: RevealMotion;
  /** Time until the whole text shows. */
  seconds: number;
  exit: boolean;
  /** How much each part's motion overlaps the next one's, 0 to 1. */
  overlap: number;
  /** The easing; empty for each motion's own. */
  easing: RevealEasing | "";
  order: RevealOrder;
  /** How it leaves when it exits; empty for its entrance motion. */
  exitMotion: RevealMotion | "";
}

/** The clip's reveal, or none with the values a new one starts from. */
export function revealOf(clip: ClipView): RevealDraft {
  const reveal = clip.reveal;
  const options = { overlap: REVEAL_OVERLAP, easing: "" as const, order: "forward" as const, exitMotion: "" as const };
  if (!reveal) {
    const clipSeconds = clip.duration.flicks / FLICKS_PER_SECOND;
    return { unit: "", motion: "appear", seconds: Math.min(1, Math.round(clipSeconds * 500) / 1000), exit: false, ...options };
  }
  return {
    unit: reveal.unit,
    motion: reveal.motion ?? "appear",
    seconds: Math.round((reveal.duration.flicks / FLICKS_PER_SECOND) * 1000) / 1000,
    exit: reveal.exit ?? false,
    overlap: reveal.overlap ?? options.overlap,
    easing: reveal.easing ?? options.easing,
    order: reveal.order ?? options.order,
    exitMotion: reveal.exit_motion ?? options.exitMotion,
  };
}

/** The `reveal` argument of `text.set` for a draft: null removes the reveal. */
export function revealArg(draft: RevealDraft): Record<string, unknown> | null {
  if (!draft.unit) return null;
  const arg: Record<string, unknown> = {
    unit: draft.unit,
    motion: draft.motion,
    duration: { seconds: String(Math.round(draft.seconds * 1000) / 1000) },
    exit: draft.exit,
  };
  // The motion options stay out at their defaults, so the stored reveal stays sparse.
  if (draft.overlap !== REVEAL_OVERLAP) arg.overlap = Math.round(draft.overlap * 1000) / 1000;
  if (draft.easing) arg.easing = draft.easing;
  if (draft.order !== "forward") arg.order = draft.order;
  if (draft.exit && draft.exitMotion) arg.exit_motion = draft.exitMotion;
  return arg;
}

/**
 * The `text.set` arguments for a change to the clip's reveal, or null when it changes
 * nothing or the time is not positive. Changes other than the unit need a reveal.
 */
export function revealChange(clip: ClipView, change: Partial<RevealDraft>): { reveal: Record<string, unknown> | null } | null {
  const current = revealOf(clip);
  const draft = { ...current, ...change };
  if (!(Number.isFinite(draft.seconds) && draft.seconds > 0)) return null;
  if (!(Number.isFinite(draft.overlap) && draft.overlap >= 0 && draft.overlap <= 1)) return null;
  if (!clip.reveal && !draft.unit) return null;
  if (clip.reveal && same(draft, current)) return null;
  return { reveal: revealArg(draft) };
}

/** The `text.set` arguments for new text, or null when it is empty or unchanged. */
export function textChange(clip: ClipView, text: string): { text: string } | null {
  if (!text.trim() || text === clip.text) return null;
  return { text };
}

/**
 * The text presets to offer: the project's saved ones first, then the built-in ones that
 * the project has not replaced by name. Saved ones say so in the interface's language;
 * built-in ones keep the engine's description.
 */
export function presetOptions(
  saved: string[],
  builtIn: { name: string; description: string }[],
): { name: string; description: string; saved: boolean }[] {
  return [
    ...saved.map((name) => ({ name, description: t("text.preset.saved"), saved: true })),
    ...builtIn.filter((p) => !saved.includes(p.name)).map((p) => ({ ...p, saved: false })),
  ];
}
