import { describe, expect, it } from "vitest";

import { type Keystroke, actionFor, angleKey, isTyping } from "./keyboard";

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
  it("maps tools, shuttle, zoom and view keys", () => {
    expect(actionFor(key("a"), true)).toBe("select-tool");
    expect(actionFor(key("b"), true)).toBe("blade-tool");
    expect(actionFor(key("n"), true)).toBe("snapping");
    expect(actionFor(key("s"), true)).toBe("skimming");
    expect([actionFor(key("j"), true), actionFor(key("k"), true), actionFor(key("l"), true)]).toEqual([
      "shuttle-back",
      "shuttle-stop",
      "shuttle-forward",
    ]);
    expect(actionFor(key("=", { metaKey: true }), true)).toBe("zoom-in");
    expect(actionFor(key("-", { ctrlKey: true }), false)).toBe("zoom-out");
    expect(actionFor(key("Z", { shiftKey: true }), true)).toBe("zoom-fit");
    expect(actionFor(key("F", { metaKey: true, shiftKey: true }), true)).toBe("fullscreen");
    expect(actionFor(key("R", { metaKey: true, shiftKey: true }), true)).toBe("rulers");
    expect(actionFor(key("z"), true), "plain z does nothing").toBeNull();
  });

  it("maps clipboard, timeline and marker keys", () => {
    expect(actionFor(key("c", { metaKey: true }), true)).toBe("copy");
    expect(actionFor(key("x", { ctrlKey: true }), false)).toBe("cut");
    expect(actionFor(key("v", { metaKey: true }), true)).toBe("paste");
    expect(actionFor(key("ç", { metaKey: true, altKey: true }), true), "Option+C on macOS").toBe("copy-attributes");
    expect(actionFor(key("v", { ctrlKey: true, altKey: true }), false)).toBe("paste-attributes");
    expect(actionFor(key("c", { altKey: true }), true), "Alt alone does nothing").toBeNull();
    expect(actionFor(key("©", { metaKey: true, altKey: true }), true), "Option+G on macOS").toBe("compound");
    expect(actionFor(key("B", { metaKey: true, shiftKey: true }), true)).toBe("split-all");
    expect(actionFor(key("g", { metaKey: true }), true)).toBe("group");
    expect(actionFor(key("G", { metaKey: true, shiftKey: true }), true)).toBe("ungroup");
    expect(actionFor(key("q"), true)).toBe("delete-left");
    expect(actionFor(key("w"), true)).toBe("delete-right");
    expect(actionFor(key("m"), true)).toBe("marker");
    expect(actionFor(key("["), true)).toBe("previous-marker");
    expect(actionFor(key("]"), true)).toBe("next-marker");
    expect(actionFor(key("p"), true)).toBe("magnet");
    expect(actionFor(key("C", { metaKey: true, shiftKey: true }), true)).toBeNull();
  });

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

  it("cuts to angles 1 to 9 with plain number keys", () => {
    expect(["1", "2", "3", "4", "5", "6", "7", "8", "9"].map((k) => angleKey(key(k)))).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(angleKey(key("0")), "there is no angle 0").toBeNull();
    expect(angleKey(key("10")), "one key at a time").toBeNull();
    expect(angleKey(key("a"))).toBeNull();
    expect(angleKey(key("2", { metaKey: true })), "Mod+2 is not a cut").toBeNull();
    expect(angleKey(key("2", { ctrlKey: true }))).toBeNull();
    expect(angleKey(key("2", { altKey: true }))).toBeNull();
    expect(angleKey(key("2", { shiftKey: true }))).toBeNull();
    expect(angleKey(key("3", { typing: true })), "typing a number in a field").toBeNull();
    expect(actionFor(key("3"), true), "number keys are no other shortcut").toBeNull();
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
