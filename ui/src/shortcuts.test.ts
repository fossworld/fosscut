import { afterEach, describe, expect, it } from "vitest";

import { cleared, draftOf, isDirty, recorded, recording, reset, toSave, withSet, changedActions } from "./app/shortcutDraft";
import { defaultPreferences } from "./engine/preferences";
import { setLanguage } from "./i18n";
import { type Keystroke, pressed } from "./keyboard";
import {
  ACTIONS,
  ACTION_IDS,
  GROUPS,
  SETS,
  actionFor,
  activeKeymap,
  conflicts,
  effectiveKeys,
  hint,
  overridesFor,
  applyShortcuts,
} from "./shortcuts";

const key = (k: string, extra: Partial<Keystroke> = {}): Keystroke => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  typing: false,
  ...extra,
});

afterEach(() => {
  applyShortcuts("fosscut", {});
  setLanguage("en");
});

describe("the shortcut table", () => {
  it("groups every action, and both sets give every action keys without clashes", () => {
    expect(ACTIONS.map((a) => a.id)).toEqual([...ACTION_IDS]);
    for (const group of GROUPS) expect(ACTIONS.some((a) => a.group === group), group).toBe(true);
    expect(ACTIONS.find((a) => a.id === "canvas.nudge_left")).toMatchObject({ group: "other", context: "canvas" });
    for (const mac of [true, false]) {
      for (const set of ["fosscut", "classic"]) {
        const keymap = effectiveKeys(set, {}, mac);
        expect([...conflicts(keymap).keys()], `${set} on ${mac ? "macOS" : "others"}`).toEqual([]);
        for (const id of ACTION_IDS) expect(keymap[id].length, `${set} ${id}`).toBeGreaterThan(0);
      }
    }
    expect(SETS.classic["timeline.split"]).not.toBe(SETS.fosscut["timeline.split"]);
  });

  it("keeps today's keys in the fosscut set", () => {
    const keys = effectiveKeys("fosscut", {}, true);
    expect(keys["timeline.split"]).toEqual(["Mod+B"]);
    expect(keys["basic.redo"]).toEqual(["Mod+Shift+Z", "Ctrl+Y"]);
    expect(keys["timeline.delete"]).toEqual(["Backspace", "Delete"]);
    expect(keys["player.step_back"]).toEqual(["Shift+Left"]);
    expect(keys["basic.settings"]).toEqual(["Mod+Comma"]);
    expect(keys["multicam.angle_3"]).toEqual(["3"]);
    expect(effectiveKeys("fosscut", {}, false)["basic.redo"], "Ctrl is Mod elsewhere").toEqual(["Mod+Shift+Z", "Mod+Y"]);
  });

  it("takes the effective keys from the set and the overrides", () => {
    const keys = effectiveKeys("classic", { "timeline.split": "S", "timeline.marker": "", "player.play": "k, Space" }, true);
    expect(keys["timeline.split"]).toEqual(["S"]);
    expect(keys["timeline.marker"], "an empty override leaves no keys").toEqual([]);
    expect(keys["player.play"]).toEqual(["K", "Space"]);
    expect(keys["timeline.blade_tool"], "the rest from the set").toEqual(["C"]);
    expect(effectiveKeys("mine", {}, true)["timeline.split"], "an unknown set is fosscut").toEqual(["Mod+B"]);
    expect(overridesFor("classic", keys, true)).toEqual({ "timeline.split": "S", "timeline.marker": "", "player.play": "K, Space" });
  });

  it("flags clashes in the same part of the window only", () => {
    const keys = effectiveKeys("fosscut", { "timeline.marker": "N" }, true);
    const found = conflicts(keys);
    expect(found.get("timeline.marker")).toEqual(["timeline.snapping"]);
    expect(found.get("timeline.snapping")).toEqual(["timeline.marker"]);
    expect(found.has("canvas.nudge_left"), "the arrows nudge on the canvas, step elsewhere").toBe(false);
    const canvas = conflicts(effectiveKeys("fosscut", { "canvas.nudge_up": "Left" }, true));
    expect(canvas.get("canvas.nudge_up")).toEqual(["canvas.nudge_left"]);
  });

  it("finds the action of a keystroke, preferring the focused part of the window", () => {
    const keys = effectiveKeys("fosscut", {}, true);
    expect(actionFor(key("b", { metaKey: true }), true, keys)).toBe("timeline.split");
    expect(actionFor(key("ArrowLeft"), true, keys)).toBe("player.frame_back");
    expect(actionFor(key("ArrowLeft"), true, keys, "canvas")).toBe("canvas.nudge_left");
    expect(actionFor(key("ArrowUp"), true, keys), "nudging only on the canvas").toBeNull();
    expect(actionFor(key("2"), true, keys)).toBeNull();
    expect(actionFor(key("2"), true, keys, "timeline")).toBe("multicam.angle_2");
    expect(actionFor(key("ArrowLeft", { shiftKey: true }), true, keys, "canvas")).toBe("player.step_back");
    expect(actionFor(key("z", { metaKey: true, typing: true }), true, keys)).toBeNull();
    expect(actionFor(key("ç", { metaKey: true, altKey: true }), true, keys)).toBe("basic.copy_attributes");
    const classic = effectiveKeys("classic", {}, false);
    expect(actionFor(key("k", { ctrlKey: true }), false, classic)).toBe("timeline.split");
    expect(actionFor(key("c"), false, classic)).toBe("timeline.blade_tool");
  });

  it("uses the keys the preferences choose, in tooltips too", () => {
    applyShortcuts("fosscut", { "basic.undo": "Mod+U" });
    expect(activeKeymap()["basic.undo"]).toEqual(["Mod+U"]);
    expect(actionFor(key("u", { metaKey: true }), true)).toBe("basic.undo");
    expect(actionFor(key("z", { metaKey: true }), true), "the set's key is replaced").toBeNull();
    expect(hint("Undo", "basic.undo")).toMatch(/^Undo \(.+U\)$/);
    applyShortcuts("fosscut", { "basic.undo": "" });
    expect(hint("Undo", "basic.undo"), "no keys, no brackets").toBe("Undo");
    applyShortcuts("fosscut", { "player.play": "Space" });
    setLanguage("hu");
    expect(hint("Lejátszás", "player.play")).toBe("Lejátszás (Szóköz)");
  });
});

describe("the shortcut editor's draft", () => {
  it("records a key combination for an action, clears and resets it, and saves only the changes", () => {
    const preferences = defaultPreferences();
    let draft = draftOf(preferences, true);
    expect(isDirty(draft, preferences, true)).toBe(false);
    draft = recording(draft, "timeline.split");
    const combo = pressed(key("K", { metaKey: true, shiftKey: true, typing: true }), true);
    expect(combo).toBe("Mod+Shift+K");
    draft = recorded(draft, combo ?? "");
    expect(draft.recording).toBeNull();
    expect(draft.keymap["timeline.split"]).toEqual(["Mod+Shift+K"]);
    draft = cleared(draft, "timeline.marker");
    expect(changedActions(draft, true)).toEqual(["timeline.split", "timeline.marker"]);
    expect(isDirty(draft, preferences, true)).toBe(true);
    expect(toSave(draft, true)).toEqual({ set: "fosscut", overrides: { "timeline.split": "Mod+Shift+K", "timeline.marker": "" } });
    draft = reset(draft, "timeline.marker", true);
    expect(toSave(draft, true).overrides).toEqual({ "timeline.split": "Mod+Shift+K" });
    draft = reset(draft, null, true);
    expect(toSave(draft, true).overrides).toEqual({});
    expect(recorded(draft, "X"), "nothing is recorded unless an action is chosen").toBe(draft);
  });

  it("starts from the preferences and switches sets with their own keys", () => {
    const preferences = defaultPreferences();
    preferences.shortcuts = { set: "classic", overrides: { "player.play": "P" } };
    const draft = draftOf(preferences, true);
    expect(draft.set).toBe("classic");
    expect(draft.keymap["player.play"]).toEqual(["P"]);
    expect(isDirty(draft, preferences, true)).toBe(false);
    const fosscut = withSet("fosscut", true);
    expect(fosscut.keymap["player.play"]).toEqual(["Space"]);
    expect(toSave(fosscut, true)).toEqual({ set: "fosscut", overrides: {} });
  });
});
