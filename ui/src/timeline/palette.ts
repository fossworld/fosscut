// The timeline canvas's colours, read from the design tokens (../theme/tokens.css) so the
// canvas follows the theme like the rest of the page.

/** The colours the timeline draws with. */
export interface Palette {
  background: string;
  row: string;
  rowAlt: string;
  ruler: string;
  tick: string;
  label: string;
  text: string;
  playhead: string;
  selection: string;
  skim: string;
  marker: string;
  drop: string;
  clips: Record<string, { fill: string; tone: string }>;
  clipLabel: string;
}

/** The kinds of clip with colours of their own; others use `other`. */
export const CLIP_KINDS = ["video", "image", "audio", "text", "caption", "shape", "adjustment", "timeline", "other"] as const;

/** The dark theme's colours: what the canvas draws with before the page's styles are read. */
export const DARK: Palette = {
  background: "#131417",
  row: "#18191d",
  rowAlt: "#16171a",
  ruler: "#1b1c20",
  tick: "#3b3c44",
  label: "#a3a4ad",
  text: "#ececef",
  playhead: "#ff7a45",
  selection: "#ffffff",
  skim: "rgba(255, 255, 255, 0.55)",
  marker: "#6fb3ff",
  drop: "rgba(255, 122, 69, 0.18)",
  clips: {
    video: { fill: "#2c62c0", tone: "#9cc0ff" },
    image: { fill: "#1f7891", tone: "#8fd8ec" },
    audio: { fill: "#177a5a", tone: "#6fe0b5" },
    text: { fill: "#9c3f7c", tone: "#f4a8da" },
    caption: { fill: "#8a5a0c", tone: "#ffd07a" },
    shape: { fill: "#ad3459", tone: "#ffa3bd" },
    adjustment: { fill: "#5c43be", tone: "#c7b8ff" },
    timeline: { fill: "#46529e", tone: "#b7c0ff" },
    other: { fill: "#4c4e57", tone: "#c9cad1" },
  },
  clipLabel: "#ffffff",
};

/** The palette from the custom properties of `style`, falling back to the dark theme's. */
export function paletteFrom(read: (name: string) => string): Palette {
  const value = (name: string, fallback: string) => read(name).trim() || fallback;
  const clips: Palette["clips"] = {};
  for (const kind of CLIP_KINDS) {
    const fallback = DARK.clips[kind] ?? DARK.clips.other!;
    clips[kind] = { fill: value(`--clip-${kind}`, fallback.fill), tone: value(`--clip-${kind}-tone`, fallback.tone) };
  }
  return {
    background: value("--timeline-bg", DARK.background),
    row: value("--timeline-row", DARK.row),
    rowAlt: value("--timeline-row-alt", DARK.rowAlt),
    ruler: value("--timeline-ruler", DARK.ruler),
    tick: value("--timeline-tick", DARK.tick),
    label: value("--timeline-label", DARK.label),
    text: value("--text", DARK.text),
    playhead: value("--accent", DARK.playhead),
    selection: value("--timeline-selection", DARK.selection),
    skim: value("--timeline-skim", DARK.skim),
    marker: value("--timeline-marker", DARK.marker),
    drop: value("--timeline-drop", DARK.drop),
    clips,
    clipLabel: value("--clip-label", DARK.clipLabel),
  };
}

const read = new Map<string, Palette>();

/** The palette of the page in `theme`, read from its styles once per theme. */
export function currentPalette(theme: string): Palette {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") return DARK;
  const known = read.get(theme);
  if (known) return known;
  const style = getComputedStyle(document.documentElement);
  const palette = paletteFrom((name) => style.getPropertyValue(name));
  read.set(theme, palette);
  return palette;
}

/** A clip's colours by its kind; a multicam clip is a compound clip. */
export function clipColors(palette: Palette, kind: string): { fill: string; tone: string } {
  return palette.clips[kind] ?? palette.clips.other ?? DARK.clips.other!;
}
