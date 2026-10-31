// The interface's languages (ADR-0031): typed message catalogues (./messages, assembled in
// ./catalogue) and `t(key, params)` in the language the preferences choose, changed live.
// Engine errors are shown as the engine wrote them, in English.

import { useSyncExternalStore } from "react";

import { type Language, type MessageKey, catalogues, english } from "./catalogue";
import type { Message, Params } from "./types";

export type { Language, MessageKey } from "./catalogue";
export type { Params } from "./types";

/** The languages the interface is translated into, English first. */
export const LANGUAGES = Object.keys(catalogues) as Language[];

let current: Language = "en";
const listeners = new Set<() => void>();

/** The language the interface shows. */
export function language(): Language {
  return current;
}

/** True for a code the interface is translated into. */
export function isLanguage(code: string): code is Language {
  return Object.hasOwn(catalogues, code);
}

/** Shows the interface in `code`, or in English when it is not translated into it. */
export function setLanguage(code: string): void {
  const next = isLanguage(code) ? code : "en";
  if (next === current) return;
  current = next;
  for (const listener of listeners) listener();
}

/** Calls `listener` whenever the language changes; returns how to stop. */
export function onLanguage(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The language, re-rendering the component that asks when it changes. */
export function useLanguage(): Language {
  return useSyncExternalStore(onLanguage, language, language);
}

const numbers = new Map<Language, Intl.NumberFormat>();

/** `value` as `lang` writes numbers: 0.5 in English, 0,5 in Hungarian; never grouped. */
export function formatNumber(value: number, lang: Language = current): string {
  let format = numbers.get(lang);
  if (!format) {
    format = new Intl.NumberFormat(lang, { maximumFractionDigits: 6, useGrouping: false });
    numbers.set(lang, format);
  }
  return format.format(value);
}

/** A message with its placeholders filled. */
export function fill(message: Message, params: Params | undefined, lang: Language): string {
  const count = params?.count;
  const text = typeof message === "string" ? message : count === 1 ? message.one : message.other;
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => {
    const value = params[name];
    if (value === undefined) return whole;
    return typeof value === "number" ? formatNumber(value, lang) : value;
  });
}

/** The message `key` in the interface's language, with `params` in its placeholders. */
export function t(key: MessageKey, params?: Params): string {
  const message = catalogues[current][key] ?? english[key];
  return fill(message, params, current);
}

/** `t` in a given language, for tests and for texts that must not follow the interface. */
export function tIn(lang: Language, key: MessageKey, params?: Params): string {
  return fill(catalogues[lang][key] ?? english[key], params, lang);
}
