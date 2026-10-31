// The interface's theme: dark by default, light, or following the system. The choice is a
// preference of this computer's window, kept in the browser storage of the webview, and
// applied as `data-theme` on the root element, where ./tokens.css picks it up.

import { useSyncExternalStore } from "react";

/** What the person chose. */
export type ThemeChoice = "dark" | "light" | "system";
/** What the interface shows. */
export type Theme = "dark" | "light";

export const THEME_CHOICES: ThemeChoice[] = ["dark", "light", "system"];

const KEY = "fosscut.theme";

/** The choice stored as `value`, or dark when nothing valid is stored. */
export function parseChoice(value: string | null | undefined): ThemeChoice {
  return value === "light" || value === "system" || value === "dark" ? value : "dark";
}

/** The theme a choice shows, given whether the system prefers light. */
export function resolveTheme(choice: ThemeChoice, systemPrefersLight: boolean): Theme {
  if (choice === "system") return systemPrefersLight ? "light" : "dark";
  return choice;
}

function storage(): Storage | null {
  try {
    return typeof window !== "undefined" ? window.localStorage : null;
  } catch {
    return null;
  }
}

function prefersLight(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: light)").matches
    : false;
}

let choice: ThemeChoice = parseChoice(storage()?.getItem(KEY));
let shown: Theme = resolveTheme(choice, prefersLight());
const listeners = new Set<() => void>();

function apply(): void {
  shown = resolveTheme(choice, prefersLight());
  if (typeof document !== "undefined") document.documentElement.dataset.theme = shown;
  for (const listener of listeners) listener();
}

/** Applies the stored choice and follows the system's changes; called once at start. */
export function startTheme(): void {
  apply();
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", () => {
      if (choice === "system") apply();
    });
  }
}

/** Shows `next` and remembers it. */
export function setTheme(next: ThemeChoice): void {
  choice = next;
  try {
    storage()?.setItem(KEY, next);
  } catch {
    // A webview without storage still shows the theme until it closes.
  }
  apply();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The person's choice, re-rendering when it changes. */
export function useThemeChoice(): ThemeChoice {
  return useSyncExternalStore(subscribe, () => choice, () => choice);
}

/** The theme shown, re-rendering when it changes. */
export function useTheme(): Theme {
  return useSyncExternalStore(subscribe, () => shown, () => shown);
}
