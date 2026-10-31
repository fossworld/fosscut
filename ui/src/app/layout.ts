// The editor window's layout: canvas shapes people pick from, and the sizes of the panels
// around the preview, which people drag and the window remembers.

import { useSyncExternalStore } from "react";

/** Canvas shapes offered for new projects and new versions of a timeline. */
export const ASPECTS = [
  { id: "16:9", width: 1920, height: 1080 },
  { id: "9:16", width: 1080, height: 1920 },
  { id: "1:1", width: 1080, height: 1080 },
  { id: "4:5", width: 1080, height: 1350 },
] as const;

export type AspectId = (typeof ASPECTS)[number]["id"];

function gcd(a: number, b: number): number {
  return b === 0 ? a : gcd(b, a % b);
}

/** A canvas's shape as people name it: 16:9, 9:16, or its reduced ratio. */
export function aspectName(width: number, height: number): string {
  if (width <= 0 || height <= 0) return "";
  const known: [number, string][] = [
    [16 / 9, "16:9"],
    [9 / 16, "9:16"],
    [1, "1:1"],
    [4 / 5, "4:5"],
    [4 / 3, "4:3"],
    [3 / 4, "3:4"],
    [21 / 9, "21:9"],
    [2.39, "2.39:1"],
  ];
  const ratio = width / height;
  const match = known.find(([value]) => Math.abs(value - ratio) < 0.01);
  if (match) return match[1];
  const d = gcd(Math.round(width), Math.round(height));
  return `${Math.round(width) / d}:${Math.round(height) / d}`;
}

/** The sizes and visibility of the panels, in CSS pixels. */
export interface Layout {
  browser: number;
  inspector: number;
  timeline: number;
  showBrowser: boolean;
  showInspector: boolean;
}

export const DEFAULT_LAYOUT: Layout = { browser: 380, inspector: 330, timeline: 330, showBrowser: true, showInspector: true };

/** How small and large each panel may be dragged. */
export const LIMITS = {
  browser: [260, 560],
  inspector: [260, 520],
  timeline: [170, 640],
} as const;

/** `layout` with every size inside its limits and inside a window of `width` by `height`. */
export function clampLayout(layout: Layout, width = Infinity, height = Infinity): Layout {
  const clamp = (value: number, [low, high]: readonly [number, number]) => Math.round(Math.min(high, Math.max(low, value)));
  const next = {
    ...layout,
    browser: clamp(layout.browser, LIMITS.browser),
    inspector: clamp(layout.inspector, LIMITS.inspector),
    timeline: clamp(layout.timeline, LIMITS.timeline),
  };
  // The preview keeps at least a third of the window's width and 200 pixels of height.
  const sides = (next.showBrowser ? next.browser : 0) + (next.showInspector ? next.inspector : 0);
  if (Number.isFinite(width) && sides > (width * 2) / 3) {
    const scale = (width * 2) / 3 / sides;
    next.browser = Math.max(LIMITS.browser[0], Math.round(next.browser * scale));
    next.inspector = Math.max(LIMITS.inspector[0], Math.round(next.inspector * scale));
  }
  if (Number.isFinite(height)) next.timeline = Math.max(LIMITS.timeline[0], Math.min(next.timeline, height - 48 - 260));
  return next;
}

/** The layout stored as `text`, or the default for anything unreadable. */
export function parseLayout(text: string | null): Layout {
  if (!text) return DEFAULT_LAYOUT;
  try {
    const value = JSON.parse(text) as Partial<Layout>;
    const number = (v: unknown, fallback: number) => (typeof v === "number" && Number.isFinite(v) ? v : fallback);
    return clampLayout({
      browser: number(value.browser, DEFAULT_LAYOUT.browser),
      inspector: number(value.inspector, DEFAULT_LAYOUT.inspector),
      timeline: number(value.timeline, DEFAULT_LAYOUT.timeline),
      showBrowser: value.showBrowser !== false,
      showInspector: value.showInspector !== false,
    });
  } catch {
    return DEFAULT_LAYOUT;
  }
}

const KEY = "fosscut.layout";

function read(): Layout {
  try {
    return parseLayout(typeof window !== "undefined" ? window.localStorage.getItem(KEY) : null);
  } catch {
    return DEFAULT_LAYOUT;
  }
}

let current = read();
const listeners = new Set<() => void>();

/** Changes the layout and remembers it. */
export function setLayout(change: Partial<Layout>): void {
  current = clampLayout({ ...current, ...change });
  try {
    window.localStorage.setItem(KEY, JSON.stringify(current));
  } catch {
    // Without storage the layout lasts until the window closes.
  }
  for (const listener of listeners) listener();
}

/** The layout, re-rendering when it changes. */
export function useLayout(): Layout {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => current,
    () => current,
  );
}

/** A tab (and a part of it) the inspector is asked to show, as the timeline's crop button asks. */
export interface InspectorRequest {
  tab: string;
  part?: string;
  seq: number;
}

let inspectorAsk: InspectorRequest | null = null;
const askers = new Set<() => void>();

/** Asks the inspector to show `tab`, scrolled to `part` when given. */
export function requestInspector(tab: string, part?: string): void {
  inspectorAsk = { tab, part, seq: (inspectorAsk?.seq ?? 0) + 1 };
  // The inspector shows again if it was hidden.
  if (!current.showInspector) setLayout({ showInspector: true });
  for (const listener of askers) listener();
}

/** The inspector's last request, re-rendering when another comes. */
export function useInspectorRequest(): InspectorRequest | null {
  return useSyncExternalStore(
    (listener) => {
      askers.add(listener);
      return () => askers.delete(listener);
    },
    () => inspectorAsk,
    () => inspectorAsk,
  );
}
