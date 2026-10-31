import { describe, expect, it } from "vitest";

import { type Keystroke, comboOf, formatCombo, isTyping, normalizeCombo, parseKeys, pressed, writeKeys } from "./keyboard";

const key = (k: string, extra: Partial<Keystroke> = {}): Keystroke => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  typing: false,
  ...extra,
});

describe("keystrokes as combinations", () => {
  it("name keys and modifiers in one order, with Mod as Command on macOS and Control elsewhere", () => {
    expect(comboOf(key("z", { metaKey: true }), true)).toBe("Mod+Z");
    expect(comboOf(key("z", { ctrlKey: true }), false)).toBe("Mod+Z");
    expect(comboOf(key("z", { ctrlKey: true }), true), "Control stays Ctrl on macOS").toBe("Ctrl+Z");
    expect(comboOf(key("Z", { metaKey: true, shiftKey: true }), true)).toBe("Mod+Shift+Z");
    expect(comboOf(key(" "), true)).toBe("Space");
    expect(comboOf(key("ArrowLeft", { shiftKey: true }), false)).toBe("Shift+Left");
    expect(comboOf(key(","), true)).toBe("Comma");
    expect(comboOf(key("Escape"), true)).toBe("Escape");
    expect(comboOf(key("F5"), false)).toBe("F5");
  });

  it("leave Shift out before a printed character, which Shift made", () => {
    expect(comboOf(key("+", { ctrlKey: true, shiftKey: true }), false)).toBe("Mod+Plus");
    expect(comboOf(key("_", { metaKey: true, shiftKey: true }), true)).toBe("Mod+_");
    expect(comboOf(key("!", { shiftKey: true }), true)).toBe("!");
    expect(comboOf(key("S", { shiftKey: true }), true), "a letter keeps it").toBe("Shift+S");
  });

  it("read the physical key under Option, which changes the character on macOS", () => {
    expect(comboOf(key("ç", { metaKey: true, altKey: true }), true)).toBe("Mod+Alt+C");
    expect(comboOf(key("©", { metaKey: true, altKey: true, code: "KeyG" }), true)).toBe("Mod+Alt+G");
    expect(comboOf(key("“", { altKey: true, code: "KeyQ" }), true)).toBe("Alt+Q");
  });

  it("make nothing of a modifier alone, or while typing, except when recording", () => {
    expect(comboOf(key("Shift", { shiftKey: true }), true)).toBeNull();
    expect(comboOf(key("Meta", { metaKey: true }), true)).toBeNull();
    expect(comboOf(key("z", { metaKey: true, typing: true }), true)).toBeNull();
    expect(pressed(key("k", { metaKey: true, typing: true }), true), "recording a shortcut").toBe("Mod+K");
  });

  it("read combinations people write, in any spelling", () => {
    expect(normalizeCombo("cmd+shift+z", true)).toBe("Mod+Shift+Z");
    expect(normalizeCombo("Shift + Ctrl + z", false), "Ctrl is Mod away from macOS").toBe("Mod+Shift+Z");
    expect(normalizeCombo("Ctrl+Y", true)).toBe("Ctrl+Y");
    expect(normalizeCombo("Option+[", true)).toBe("Alt+[");
    expect(normalizeCombo("space", true)).toBe("Space");
    expect(normalizeCombo("Shift+=", true), "a printed character drops Shift").toBe("=");
    expect(normalizeCombo("Hyper+A", true)).toBeNull();
    expect(normalizeCombo("", true)).toBeNull();
    expect(parseKeys("Mod+=, Mod+Plus, mod+=", true), "each combination once").toEqual(["Mod+=", "Mod+Plus"]);
    expect(parseKeys("", true)).toEqual([]);
    expect(writeKeys(["Backspace", "Delete"])).toBe("Backspace, Delete");
  });

  it("show combinations as each system writes them", () => {
    expect(formatCombo("Mod+Shift+Z", true)).toBe("⌘⇧Z");
    expect(formatCombo("Mod+Shift+Z", false)).toBe("Ctrl+Shift+Z");
    expect(formatCombo("Shift+Left", false)).toBe("Shift+←");
    expect(formatCombo("Mod+Comma", true)).toBe("⌘,");
    expect(formatCombo("Space", false, (k) => (k === "Space" ? "Szóköz" : k)), "key names in the language").toBe("Szóköz");
  });

  it("leaves text fields alone", () => {
    expect(isTyping(null)).toBe(false);
    expect(isTyping({ tagName: "INPUT" } as unknown as EventTarget)).toBe(true);
    expect(isTyping({ tagName: "DIV", isContentEditable: true } as unknown as EventTarget)).toBe(true);
    expect(isTyping({ tagName: "CANVAS", isContentEditable: false } as unknown as EventTarget)).toBe(false);
  });
});
