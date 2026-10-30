// Keyboard shortcuts. Mod is Command on macOS and Control elsewhere. Typing in a text
// field never triggers one.

export type Action =
  | "toggle-play"
  | "step-back"
  | "step-forward"
  | "second-back"
  | "second-forward"
  | "start"
  | "end"
  | "undo"
  | "redo"
  | "delete"
  | "split"
  | "deselect"
  | "toggle-enabled";

export interface Keystroke {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  /** True when the focus is in an input, a text area or an editable element. */
  typing: boolean;
}

/** The action a keystroke asks for, or null. */
export function actionFor(stroke: Keystroke, mac: boolean): Action | null {
  if (stroke.typing) return null;
  const mod = mac ? stroke.metaKey && !stroke.ctrlKey : stroke.ctrlKey && !stroke.metaKey;
  const key = stroke.key.length === 1 ? stroke.key.toLowerCase() : stroke.key;
  if (stroke.altKey) return null;
  if (mod) {
    if (key === "z") return stroke.shiftKey ? "redo" : "undo";
    if (key === "y" && !mac && !stroke.shiftKey) return "redo";
    if (key === "b" && !stroke.shiftKey) return "split";
    return null;
  }
  if (stroke.metaKey || stroke.ctrlKey) return null;
  switch (key) {
    case " ":
      return "toggle-play";
    case "ArrowLeft":
      return stroke.shiftKey ? "second-back" : "step-back";
    case "ArrowRight":
      return stroke.shiftKey ? "second-forward" : "step-forward";
    case "Home":
      return "start";
    case "End":
      return "end";
    case "Backspace":
    case "Delete":
      return "delete";
    case "Escape":
      return "deselect";
    case "v":
      return stroke.shiftKey ? null : "toggle-enabled";
    default:
      return null;
  }
}

/** True when a keyboard event's target takes text. */
export function isTyping(target: EventTarget | null): boolean {
  if (!target || typeof (target as HTMLElement).tagName !== "string") return false;
  const element = target as HTMLElement;
  return element.tagName === "INPUT" || element.tagName === "TEXTAREA" || element.isContentEditable === true;
}
