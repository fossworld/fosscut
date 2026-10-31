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
  | "toggle-enabled"
  | "copy"
  | "cut"
  | "paste"
  | "copy-attributes"
  | "paste-attributes"
  | "delete-left"
  | "delete-right"
  | "split-all"
  | "group"
  | "ungroup"
  | "marker"
  | "previous-marker"
  | "next-marker"
  | "magnet"
  | "select-tool"
  | "blade-tool"
  | "snapping"
  | "skimming"
  | "shuttle-back"
  | "shuttle-stop"
  | "shuttle-forward"
  | "zoom-in"
  | "zoom-out"
  | "zoom-fit"
  | "fullscreen"
  | "rulers"
  | "compound";

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
  if (stroke.altKey) {
    // Mod+Alt+C and Mod+Alt+V copy and paste attributes. On macOS Option changes the key
    // the layout reports, so match the physical letters through their usual results too.
    if (!mod || stroke.shiftKey) return null;
    if (key === "c" || key === "ç") return "copy-attributes";
    if (key === "v" || key === "√") return "paste-attributes";
    if (key === "g" || key === "©") return "compound";
    return null;
  }
  if (mod) {
    if (key === "z") return stroke.shiftKey ? "redo" : "undo";
    if (key === "y" && !mac && !stroke.shiftKey) return "redo";
    if (key === "b") return stroke.shiftKey ? "split-all" : "split";
    if (key === "g") return stroke.shiftKey ? "ungroup" : "group";
    if (key === "f" && stroke.shiftKey) return "fullscreen";
    if (key === "r" && stroke.shiftKey) return "rulers";
    if (key === "=" || key === "+") return "zoom-in";
    if (key === "-" || key === "_") return "zoom-out";
    if (stroke.shiftKey) return null;
    if (key === "c") return "copy";
    if (key === "x") return "cut";
    if (key === "v") return "paste";
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
    case "q":
      return stroke.shiftKey ? null : "delete-left";
    case "w":
      return stroke.shiftKey ? null : "delete-right";
    case "m":
      return stroke.shiftKey ? null : "marker";
    case "[":
      return "previous-marker";
    case "]":
      return "next-marker";
    case "p":
      return stroke.shiftKey ? null : "magnet";
    case "a":
      return stroke.shiftKey ? null : "select-tool";
    case "b":
      return stroke.shiftKey ? null : "blade-tool";
    case "n":
      return stroke.shiftKey ? null : "snapping";
    case "s":
      return stroke.shiftKey ? null : "skimming";
    case "j":
      return stroke.shiftKey ? null : "shuttle-back";
    case "k":
      return stroke.shiftKey ? null : "shuttle-stop";
    case "l":
      return stroke.shiftKey ? null : "shuttle-forward";
    case "z":
      return stroke.shiftKey ? "zoom-fit" : null;
    default:
      return null;
  }
}

/**
 * The angle a number key cuts a multicam clip to, 1 to 9, or null: a plain digit key
 * (Shift, Alt and Mod make other shortcuts or characters), never while typing.
 */
export function angleKey(stroke: Keystroke): number | null {
  if (stroke.typing || stroke.metaKey || stroke.ctrlKey || stroke.altKey || stroke.shiftKey) return null;
  return /^[1-9]$/.test(stroke.key) ? Number(stroke.key) : null;
}

/** True when a keyboard event's target takes text. */
export function isTyping(target: EventTarget | null): boolean {
  if (!target || typeof (target as HTMLElement).tagName !== "string") return false;
  const element = target as HTMLElement;
  return element.tagName === "INPUT" || element.tagName === "TEXTAREA" || element.isContentEditable === true;
}
