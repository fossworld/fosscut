// The Shortcuts section's draft: a set and its keys as the person changes them, saved as
// the set and the overrides that differ from it (`shortcuts` in the preferences).

import type { Preferences } from "../engine/types";
import { MAC } from "../keyboard";
import {
  type ActionId,
  type Keymap,
  type ShortcutSetName,
  conflicts,
  effectiveKeys,
  isSetName,
  overridesFor,
} from "../shortcuts";

export interface ShortcutDraft {
  set: ShortcutSetName;
  keymap: Keymap;
  /** The action whose new keys are being recorded. */
  recording: ActionId | null;
}

/** The draft of the shortcuts the preferences hold. */
export function draftOf(preferences: Preferences, mac: boolean = MAC): ShortcutDraft {
  const set = isSetName(preferences.shortcuts.set) ? preferences.shortcuts.set : "fosscut";
  return { set, keymap: effectiveKeys(set, preferences.shortcuts.overrides, mac), recording: null };
}

/** A draft of another set, with its own keys: changes to the last one are dropped. */
export function withSet(set: ShortcutSetName, mac: boolean = MAC): ShortcutDraft {
  return { set, keymap: effectiveKeys(set, {}, mac), recording: null };
}

/** Starts recording the keys of `id`, or stops when null. */
export function recording(draft: ShortcutDraft, id: ActionId | null): ShortcutDraft {
  return { ...draft, recording: id };
}

/** `combo` as the only keys of the action being recorded. */
export function recorded(draft: ShortcutDraft, combo: string): ShortcutDraft {
  if (!draft.recording) return draft;
  return { ...draft, keymap: { ...draft.keymap, [draft.recording]: [combo] }, recording: null };
}

/** Leaves `id` without keys. */
export function cleared(draft: ShortcutDraft, id: ActionId): ShortcutDraft {
  return { ...draft, keymap: { ...draft.keymap, [id]: [] }, recording: null };
}

/** `id` with its set's keys again, or every action when `id` is null. */
export function reset(draft: ShortcutDraft, id: ActionId | null, mac: boolean = MAC): ShortcutDraft {
  const own = effectiveKeys(draft.set, {}, mac);
  return { ...draft, keymap: id ? { ...draft.keymap, [id]: own[id] } : own, recording: null };
}

/** The actions whose keys differ from their set's. */
export function changedActions(draft: ShortcutDraft, mac: boolean = MAC): ActionId[] {
  return Object.keys(overridesFor(draft.set, draft.keymap, mac)) as ActionId[];
}

/** True when saving would change the preferences. */
export function isDirty(draft: ShortcutDraft, preferences: Preferences, mac: boolean = MAC): boolean {
  const saved = draftOf(preferences, mac);
  return saved.set !== draft.set || JSON.stringify(saved.keymap) !== JSON.stringify(draft.keymap);
}

/** The `shortcuts` preference that holds the draft. */
export function toSave(draft: ShortcutDraft, mac: boolean = MAC): { set: ShortcutSetName; overrides: Record<string, string> } {
  return { set: draft.set, overrides: overridesFor(draft.set, draft.keymap, mac) };
}

/** The actions each action's keys clash with, in the draft. */
export function clashes(draft: ShortcutDraft): Map<ActionId, ActionId[]> {
  return conflicts(draft.keymap);
}
