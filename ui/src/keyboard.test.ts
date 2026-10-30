import { describe, expect, it } from "vitest";

import { type Keystroke, actionFor, isTyping } from "./keyboard";

const key = (k: string, extra: Partial<Keystroke> = {}): Keystroke => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  typing: false,
  ...extra,
});

describe("keyboard", () => {
  it("uses Command on macOS and Control elsewhere", () => {
    expect(actionFor(key("z", { metaKey: true }), true)).toBe("undo");
    expect(actionFor(key("Z", { metaKey: true, shiftKey: true }), true)).toBe("redo");
    expect(actionFor(key("z", { ctrlKey: true }), true), "Control is not Mod on macOS").toBeNull();
    expect(actionFor(key("z", { ctrlKey: true }), false)).toBe("undo");
    expect(actionFor(key("y", { ctrlKey: true }), false)).toBe("redo");
    expect(actionFor(key("y", { metaKey: true }), true)).toBeNull();
    expect(actionFor(key("b", { metaKey: true }), true)).toBe("split");
    expect(actionFor(key("b", { ctrlKey: true }), false)).toBe("split");
    expect(actionFor(key("q", { metaKey: true }), true)).toBeNull();
  });

  it("maps playback, navigation and editing keys", () => {
    expect(actionFor(key(" "), true)).toBe("toggle-play");
    expect(actionFor(key("ArrowLeft"), true)).toBe("step-back");
    expect(actionFor(key("ArrowRight", { shiftKey: true }), true)).toBe("second-forward");
    expect(actionFor(key("ArrowLeft", { shiftKey: true }), true)).toBe("second-back");
    expect(actionFor(key("ArrowRight"), true)).toBe("step-forward");
    expect(actionFor(key("Home"), true)).toBe("start");
    expect(actionFor(key("End"), true)).toBe("end");
    expect(actionFor(key("Backspace"), true)).toBe("delete");
    expect(actionFor(key("Delete"), false)).toBe("delete");
    expect(actionFor(key("Escape"), false)).toBe("deselect");
    expect(actionFor(key("v"), false)).toBe("toggle-enabled");
    expect(actionFor(key("V", { shiftKey: true }), false)).toBeNull();
    expect(actionFor(key("x"), false)).toBeNull();
    expect(actionFor(key(" ", { altKey: true }), false)).toBeNull();
    expect(actionFor(key(" ", { ctrlKey: true }), true), "a stray modifier is no shortcut").toBeNull();
  });

  it("leaves text fields alone", () => {
    expect(actionFor(key(" ", { typing: true }), true)).toBeNull();
    expect(actionFor(key("z", { metaKey: true, typing: true }), true)).toBeNull();
    expect(isTyping(null)).toBe(false);
    expect(isTyping({ tagName: "INPUT" } as unknown as EventTarget)).toBe(true);
    expect(isTyping({ tagName: "DIV", isContentEditable: true } as unknown as EventTarget)).toBe(true);
    expect(isTyping({ tagName: "CANVAS", isContentEditable: false } as unknown as EventTarget)).toBe(false);
  });
});
