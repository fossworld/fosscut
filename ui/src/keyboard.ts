// Keystrokes as key combinations. A combination is written as modifiers and a key joined
// by "+", for example "Mod+Shift+Z": Mod is Command on macOS and Control elsewhere, Ctrl
// is Control on macOS (elsewhere it is Mod), then Meta (the Windows key), Alt and Shift.
// Keys are letters in capitals, digits, the characters they print ("[", "=", "-"), or
// names: Space, Left, Right, Up, Down, Home, End, PageUp, PageDown, Backspace, Delete,
// Escape, Enter, Tab, F1 to F12, Comma and Plus. Shift is left out before a printed
// character, since Shift is what printed it. Typing in a text field never makes one.
// Which action a combination does is in ./shortcuts.

/** True on macOS, where Mod is Command. */
export const MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

export interface Keystroke {
  key: string;
  /** The physical key (`KeyC`, `Digit1`), when known: Option on macOS changes `key`. */
  code?: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  /** True when the focus is in an input, a text area or an editable element. */
  typing: boolean;
}

/** The keystroke of a keyboard event. */
export function keystroke(event: KeyboardEvent | React.KeyboardEvent): Keystroke {
  return {
    key: event.key,
    code: event.code,
    metaKey: event.metaKey,
    ctrlKey: event.ctrlKey,
    shiftKey: event.shiftKey,
    altKey: event.altKey,
    typing: isTyping(event.target),
  };
}

const NAMED: Record<string, string> = {
  " ": "Space",
  Spacebar: "Space",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  ArrowUp: "Up",
  ArrowDown: "Down",
  Esc: "Escape",
  Del: "Delete",
  ",": "Comma",
  "+": "Plus",
};

const KEYS = new Set([
  "Space",
  "Left",
  "Right",
  "Up",
  "Down",
  "Home",
  "End",
  "PageUp",
  "PageDown",
  "Backspace",
  "Delete",
  "Escape",
  "Enter",
  "Tab",
  "Insert",
  "Comma",
  "Plus",
]);

/** What Option with a letter types on a US Mac layout, for keystrokes without a code. */
const OPTION_LETTERS: Record<string, string> = { "ç": "C", "√": "V", "©": "G", "å": "A", "∫": "B", "ß": "S" };

/** The key of a keystroke as combinations name it, or null for a modifier alone. */
export function keyName(stroke: Pick<Keystroke, "key" | "code" | "altKey">): string | null {
  const { key } = stroke;
  if (["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock", "OS", "Fn", "Dead", "Unidentified", ""].includes(key)) {
    return null;
  }
  // Option and Alt change the character a letter types; the physical key names it then.
  if (stroke.altKey) {
    const physical = /^Key([A-Z])$/.exec(stroke.code ?? "");
    if (physical?.[1]) return physical[1];
    const digit = /^Digit([0-9])$/.exec(stroke.code ?? "");
    if (digit?.[1]) return digit[1];
    const letter = OPTION_LETTERS[key];
    if (letter) return letter;
  }
  const named = NAMED[key];
  if (named) return named;
  if (KEYS.has(key) || /^F([1-9]|1[0-9]|2[0-4])$/.test(key)) return key;
  if ([...key].length === 1) return key.toUpperCase();
  return null;
}

/** True for a key that prints a character other than a letter, which Shift may have made. */
function printed(key: string): boolean {
  return key === "Comma" || key === "Plus" || ([...key].length === 1 && key.toLowerCase() === key.toUpperCase());
}

const ORDER = ["Mod", "Ctrl", "Meta", "Alt", "Shift"];

/** The combination a keystroke makes, or null for a modifier alone or while typing. */
export function comboOf(stroke: Keystroke, mac: boolean): string | null {
  if (stroke.typing) return null;
  return pressed(stroke, mac);
}

/** The combination a keystroke makes, also while typing (for recording a shortcut). */
export function pressed(stroke: Keystroke, mac: boolean): string | null {
  const key = keyName(stroke);
  if (!key) return null;
  const parts: string[] = [];
  if (mac) {
    if (stroke.metaKey) parts.push("Mod");
    if (stroke.ctrlKey) parts.push("Ctrl");
  } else {
    if (stroke.ctrlKey) parts.push("Mod");
    if (stroke.metaKey) parts.push("Meta");
  }
  if (stroke.altKey) parts.push("Alt");
  if (stroke.shiftKey && !printed(key)) parts.push("Shift");
  parts.push(key);
  return parts.join("+");
}

/**
 * A combination as written by a person or a shortcut set, in the one spelling
 * `comboOf` gives: modifiers in order, Cmd and Command as Mod, Option as Alt, and on
 * systems other than macOS Ctrl as Mod. Null when it names no key.
 */
export function normalizeCombo(text: string, mac: boolean): string | null {
  const parts = text
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  const last = parts.pop();
  if (!last) return null;
  const modifiers = new Set<string>();
  for (const part of parts) {
    const lower = part.toLowerCase();
    if (lower === "mod" || lower === "cmd" || lower === "command") modifiers.add("Mod");
    else if (lower === "ctrl" || lower === "control") modifiers.add(mac ? "Ctrl" : "Mod");
    else if (lower === "meta" || lower === "win" || lower === "super") modifiers.add(mac ? "Mod" : "Meta");
    else if (lower === "alt" || lower === "option" || lower === "opt") modifiers.add("Alt");
    else if (lower === "shift") modifiers.add("Shift");
    else return null;
  }
  const key = keyName({ key: last.length > 1 && KEYS.has(capitalized(last)) ? capitalized(last) : last, altKey: false });
  if (!key) return null;
  if (printed(key)) modifiers.delete("Shift");
  return [...ORDER.filter((m) => modifiers.has(m)), key].join("+");
}

function capitalized(word: string): string {
  if (/^page(up|down)$/i.test(word)) return `Page${word.slice(4, 5).toUpperCase()}${word.slice(5).toLowerCase()}`;
  return word.charAt(0).toUpperCase() + word.slice(1).toLowerCase();
}

/** Keys as preferences write them, combinations separated by commas: "Mod+K, Mod+B". */
export function parseKeys(text: string, mac: boolean): string[] {
  const combos: string[] = [];
  for (const part of text.split(",")) {
    const combo = normalizeCombo(part, mac);
    if (combo && !combos.includes(combo)) combos.push(combo);
  }
  return combos;
}

/** Combinations as preferences write them. */
export function writeKeys(combos: string[]): string {
  return combos.join(", ");
}

/** Symbols macOS shows for its modifiers. */
const MAC_MODIFIERS: Record<string, string> = { Mod: "⌘", Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Meta: "⌘" };
const OTHER_MODIFIERS: Record<string, string> = { Mod: "Ctrl", Ctrl: "Ctrl", Alt: "Alt", Shift: "Shift", Meta: "Win" };
const ARROWS: Record<string, string> = { Left: "←", Right: "→", Up: "↑", Down: "↓" };

/**
 * A combination as people read it on this system: symbols on macOS ("⌘⇧Z"),
 * words elsewhere ("Ctrl+Shift+Z"). `names` spells keys out in the interface's language.
 */
export function formatCombo(combo: string, mac: boolean, names: (key: string) => string = (key) => key): string {
  const parts = combo.split("+");
  const key = parts.pop() ?? "";
  const shown = ARROWS[key] ?? (key === "Comma" ? "," : key === "Plus" ? "+" : names(key));
  if (mac) return `${parts.map((m) => MAC_MODIFIERS[m] ?? m).join("")}${shown}`;
  return [...parts.map((m) => OTHER_MODIFIERS[m] ?? m), shown].join("+");
}

/** True when a keyboard event's target takes text. */
export function isTyping(target: EventTarget | null): boolean {
  if (!target || typeof (target as HTMLElement).tagName !== "string") return false;
  const element = target as HTMLElement;
  return element.tagName === "INPUT" || element.tagName === "TEXTAREA" || element.isContentEditable === true;
}
