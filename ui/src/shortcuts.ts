// Keyboard shortcuts as data (ADR-0031): every action the keyboard reaches, by id and
// group, two sets of keys for them (`fosscut`, the app's own, and `classic`, a layout in
// the long-standing conventions of editing software), and the person's overrides from
// `shortcuts.overrides`. The keyboard handler looks keys up in the effective table.
// Combinations are written as ./keyboard describes.

import { t } from "./i18n";
import { MAC, type Keystroke, comboOf, formatCombo, parseKeys, writeKeys } from "./keyboard";

/** How the Shortcuts settings group actions. */
export type ShortcutGroup = "timeline" | "player" | "basic" | "other";

export const GROUPS: ShortcutGroup[] = ["timeline", "player", "basic", "other"];

/**
 * Where an action's keys work: everywhere, or only while the preview's canvas or the
 * timeline has the focus. There, an action's keys win over another's that works
 * everywhere, so the arrows nudge on the canvas and step frames elsewhere.
 */
export type ShortcutContext = "global" | "canvas" | "timeline";

export const ACTION_IDS = [
  "timeline.split",
  "timeline.split_all",
  "timeline.delete",
  "timeline.delete_left",
  "timeline.delete_right",
  "timeline.toggle_enabled",
  "timeline.group",
  "timeline.ungroup",
  "timeline.compound",
  "timeline.marker",
  "timeline.magnet",
  "timeline.select_tool",
  "timeline.blade_tool",
  "timeline.snapping",
  "timeline.skimming",
  "timeline.zoom_in",
  "timeline.zoom_out",
  "timeline.zoom_fit",
  "player.play",
  "player.frame_back",
  "player.frame_forward",
  "player.step_back",
  "player.step_forward",
  "player.start",
  "player.end",
  "player.previous_marker",
  "player.next_marker",
  "player.shuttle_back",
  "player.shuttle_stop",
  "player.shuttle_forward",
  "player.fullscreen",
  "player.rulers",
  "basic.undo",
  "basic.redo",
  "basic.copy",
  "basic.cut",
  "basic.paste",
  "basic.copy_attributes",
  "basic.paste_attributes",
  "basic.deselect",
  "basic.settings",
  "canvas.nudge_left",
  "canvas.nudge_right",
  "canvas.nudge_up",
  "canvas.nudge_down",
  "multicam.angle_1",
  "multicam.angle_2",
  "multicam.angle_3",
  "multicam.angle_4",
  "multicam.angle_5",
  "multicam.angle_6",
  "multicam.angle_7",
  "multicam.angle_8",
  "multicam.angle_9",
] as const;

/** An action the keyboard reaches. */
export type ActionId = (typeof ACTION_IDS)[number];

export interface ShortcutAction {
  id: ActionId;
  group: ShortcutGroup;
  context: ShortcutContext;
}

/** The group and context of each action, in the order the settings list them. */
export const ACTIONS: ShortcutAction[] = ACTION_IDS.map((id) => {
  const area = id.split(".")[0];
  const group: ShortcutGroup =
    area === "timeline" || area === "player" || area === "basic" ? area : "other";
  const context: ShortcutContext = area === "canvas" ? "canvas" : area === "multicam" ? "timeline" : "global";
  return { id, group, context };
});

const CONTEXT = new Map(ACTIONS.map((a) => [a.id, a.context]));

/** The shortcut sets, as `shortcuts.set` names them. */
export type ShortcutSetName = "fosscut" | "classic";

export const SET_NAMES: ShortcutSetName[] = ["fosscut", "classic"];

/** Keys by action, as preferences write them. */
export type KeyTable = Record<ActionId, string>;

const ANGLES = Object.fromEntries(
  [1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => [`multicam.angle_${n}`, String(n)]),
) as Record<`multicam.angle_${1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9}`, string>;

const NUDGES = {
  "canvas.nudge_left": "Left",
  "canvas.nudge_right": "Right",
  "canvas.nudge_up": "Up",
  "canvas.nudge_down": "Down",
};

const SHARED = {
  "player.play": "Space",
  "player.frame_back": "Left",
  "player.frame_forward": "Right",
  "player.step_back": "Shift+Left",
  "player.step_forward": "Shift+Right",
  "player.start": "Home",
  "player.end": "End",
  "player.shuttle_back": "J",
  "player.shuttle_stop": "K",
  "player.shuttle_forward": "L",
  "basic.undo": "Mod+Z",
  "basic.redo": "Mod+Shift+Z, Ctrl+Y",
  "basic.copy": "Mod+C",
  "basic.cut": "Mod+X",
  "basic.paste": "Mod+V",
  "basic.settings": "Mod+Comma",
  "timeline.delete": "Backspace, Delete",
  "timeline.group": "Mod+G",
  "timeline.ungroup": "Mod+Shift+G",
  "timeline.compound": "Mod+Alt+G",
  "timeline.marker": "M",
  "timeline.magnet": "P",
  "timeline.zoom_fit": "Shift+Z",
  ...NUDGES,
  ...ANGLES,
};

/** The two shortcut sets. */
export const SETS: Record<ShortcutSetName, KeyTable> = {
  // The app's own keys: single letters for tools and toggles, Mod for edits.
  fosscut: {
    ...SHARED,
    "timeline.split": "Mod+B",
    "timeline.split_all": "Mod+Shift+B",
    "timeline.delete_left": "Q",
    "timeline.delete_right": "W",
    "timeline.toggle_enabled": "V",
    "timeline.select_tool": "A",
    "timeline.blade_tool": "B",
    "timeline.snapping": "N",
    "timeline.skimming": "S",
    "timeline.zoom_in": "Mod+=, Mod+Plus",
    "timeline.zoom_out": "Mod+-, Mod+_",
    "player.previous_marker": "[",
    "player.next_marker": "]",
    "player.fullscreen": "Mod+Shift+F",
    "player.rulers": "Mod+Shift+R",
    "basic.copy_attributes": "Mod+Alt+C",
    "basic.paste_attributes": "Mod+Alt+V",
    "basic.deselect": "Escape",
  },
  // Conventions many editors share: V selects and C cuts, Mod+K adds an edit, plain
  // = and - zoom, Up and Down jump between markers, Shift with Mod pastes attributes.
  classic: {
    ...SHARED,
    "timeline.split": "Mod+K",
    "timeline.split_all": "Mod+Shift+K",
    "timeline.delete_left": "Alt+[",
    "timeline.delete_right": "Alt+]",
    "timeline.toggle_enabled": "Shift+E",
    "timeline.select_tool": "V",
    "timeline.blade_tool": "C",
    "timeline.snapping": "S",
    "timeline.skimming": "Shift+S",
    "timeline.zoom_in": "=, Plus",
    "timeline.zoom_out": "-, _",
    "player.previous_marker": "Up",
    "player.next_marker": "Down",
    "player.fullscreen": "Mod+Ctrl+F",
    "player.rulers": "Mod+R",
    "basic.copy_attributes": "Mod+Shift+C",
    "basic.paste_attributes": "Mod+Shift+V",
    "basic.deselect": "Escape, Mod+Shift+A",
  },
};

/** The combinations of each action, in this system's spelling. */
export type Keymap = Record<ActionId, string[]>;

/** True for a known shortcut set. */
export function isSetName(name: string): name is ShortcutSetName {
  return (SET_NAMES as string[]).includes(name);
}

/** The keys of `set` with `overrides` (action to keys; "" leaves an action without). */
export function effectiveKeys(set: string, overrides: Record<string, string>, mac: boolean = MAC): Keymap {
  const table = SETS[isSetName(set) ? set : "fosscut"];
  const keymap = {} as Keymap;
  for (const id of ACTION_IDS) {
    const written = Object.hasOwn(overrides, id) ? (overrides[id] ?? "") : table[id];
    keymap[id] = parseKeys(written, mac);
  }
  return keymap;
}

/** The overrides that make `keymap` from `set`: only the actions whose keys differ. */
export function overridesFor(set: string, keymap: Keymap, mac: boolean = MAC): Record<string, string> {
  const base = effectiveKeys(set, {}, mac);
  const overrides: Record<string, string> = {};
  for (const id of ACTION_IDS) {
    if (writeKeys(keymap[id]) !== writeKeys(base[id])) overrides[id] = writeKeys(keymap[id]);
  }
  return overrides;
}

/**
 * The actions whose keys clash, by action: two actions clash when a combination does
 * both in the same context. An action of the canvas or the timeline does not clash with
 * one that works everywhere; it wins while its part of the window has the focus.
 */
export function conflicts(keymap: Keymap): Map<ActionId, ActionId[]> {
  const byCombo = new Map<string, ActionId[]>();
  for (const id of ACTION_IDS) {
    for (const combo of keymap[id]) {
      const key = `${CONTEXT.get(id)}|${combo}`;
      byCombo.set(key, [...(byCombo.get(key) ?? []), id]);
    }
  }
  const found = new Map<ActionId, ActionId[]>();
  for (const ids of byCombo.values()) {
    if (ids.length < 2) continue;
    for (const id of ids) {
      const others = ids.filter((other) => other !== id);
      found.set(id, [...new Set([...(found.get(id) ?? []), ...others])]);
    }
  }
  return found;
}

/** A keymap turned around: the actions of each combination. */
export function lookup(keymap: Keymap): Map<string, ActionId[]> {
  const byCombo = new Map<string, ActionId[]>();
  for (const id of ACTION_IDS) {
    for (const combo of keymap[id]) byCombo.set(combo, [...(byCombo.get(combo) ?? []), id]);
  }
  return byCombo;
}

/**
 * The action a keystroke asks for with `keymap`, or null: an action of the part of the
 * window that has the focus wins; otherwise one that works everywhere.
 */
export function actionFor(
  stroke: Keystroke,
  mac: boolean,
  keymap: Keymap = active.keymap,
  focus: ShortcutContext = "global",
): ActionId | null {
  const combo = comboOf(stroke, mac);
  if (!combo) return null;
  const ids = (keymap === active.keymap ? active.byCombo : lookup(keymap)).get(combo) ?? [];
  return ids.find((id) => CONTEXT.get(id) === focus && focus !== "global") ?? ids.find((id) => CONTEXT.get(id) === "global") ?? null;
}

/** The context of an action. */
export function contextOf(id: ActionId): ShortcutContext {
  return CONTEXT.get(id) ?? "global";
}

// ---- The keys in use ----

const active: { keymap: Keymap; byCombo: Map<string, ActionId[]> } = {
  keymap: effectiveKeys("fosscut", {}),
  byCombo: new Map(),
};
active.byCombo = lookup(active.keymap);

/** Uses `set` with `overrides` from now on, as the preferences say. */
export function applyShortcuts(set: string, overrides: Record<string, string>): void {
  active.keymap = effectiveKeys(set, overrides);
  active.byCombo = lookup(active.keymap);
}

/** The keymap in use. */
export function activeKeymap(): Keymap {
  return active.keymap;
}

/** A key's name as people read it in the interface's language. */
export function spokenKey(key: string): string {
  if (key === "Space") return t("settings.key.space");
  if (key === "Escape") return "Esc";
  return key;
}

/** Combinations as people read them on this system: "⌘Z, ⌃Y" or "Ctrl+Z, Ctrl+Y". */
export function shownKeys(combos: string[], mac: boolean = MAC): string {
  return combos.map((combo) => formatCombo(combo, mac, spokenKey)).join(", ");
}

/** The keys of an action in use, as people read them on this system; "" without keys. */
export function keysOf(id: ActionId, names: (key: string) => string = spokenKey): string {
  return active.keymap[id].map((combo) => formatCombo(combo, MAC, names)).join(", ");
}

/** `label` with the keys of `id` after it in brackets, when it has keys: "Undo (⌘Z)". */
export function hint(label: string, id: ActionId, names: (key: string) => string = spokenKey): string {
  const keys = keysOf(id, names);
  return keys ? `${label} (${keys})` : label;
}
