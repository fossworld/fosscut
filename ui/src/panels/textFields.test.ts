import { afterEach, describe, expect, it } from "vitest";

import { setLanguage, t } from "../i18n";
import { clip, moment } from "../test/fixtures";
import {
  NUMBERS,
  TEXT_DEFAULTS,
  normalColor,
  picked,
  presetOptions,
  revealArg,
  revealChange,
  revealOf,
  rgbOf,
  shownNumber,
  styleOf,
  stylePatch,
  textChange,
  valueAt,
} from "./textFields";

const plain = clip("clip_0005", "text", 0, 2, { text: "Live edit", style: { size: 0.08, color: "#FFFFFF" } });
const styled = clip("clip_0005", "text", 0, 2, {
  text: "Live edit",
  style: {
    font: "Lora",
    size: 0.1,
    color: "#FFE14D",
    bold: true,
    case: "upper",
    letter_spacing: 0.05,
    box: { enabled: true, color: "#11111180" },
    shadow: { enabled: true, offset: [0.1, 0.06] },
  },
});
const fonts = ["Inter", "Lora"];

describe("text styles", () => {
  it("read stored values with the engine's defaults for the rest", () => {
    expect(styleOf(clip("clip_0005", "text", 0, 2))).toEqual(TEXT_DEFAULTS);
    const style = styleOf(styled);
    expect(style).toMatchObject({ font: "Lora", size: 0.1, bold: true, italic: false, case: "upper", align: "center" });
    expect(style.box).toEqual({ enabled: true, color: "#11111180", padding: 0.25, radius: 0.3 });
    expect(style.stroke, "an outline never stored is off at its defaults").toEqual(TEXT_DEFAULTS.stroke);
    expect([valueAt(style, "shadow.x"), valueAt(style, "shadow.y"), valueAt(style, "box.color")]).toEqual([
      0.1,
      0.06,
      "#11111180",
    ]);
    expect(shownNumber("size", style.size), "size shows as a percentage of the canvas height").toBe(10);
    expect(shownNumber("letter_spacing", 0.05)).toBe(0.05);
  });

  it("turn edits into sparse merge patches, null resetting a field at its default", () => {
    expect(stylePatch(plain, "size", 12)).toEqual({ size: 0.12 });
    expect(stylePatch(styled, "size", 8), "back to the default size").toEqual({ size: null });
    expect(stylePatch(plain, "bold", true)).toEqual({ bold: true });
    expect(stylePatch(styled, "bold", false)).toEqual({ bold: null });
    expect(stylePatch(plain, "color", "#ff0000")).toEqual({ color: "#FF0000" });
    expect(stylePatch(styled, "color", "#ffffff")).toEqual({ color: null });
    expect(stylePatch(plain, "case", "title")).toEqual({ case: "title" });
    expect(stylePatch(plain, "align", "left")).toEqual({ align: "left" });
    expect(stylePatch(styled, "letter_spacing", 0)).toEqual({ letter_spacing: null });
    expect(stylePatch(plain, "line_height", 1.5)).toEqual({ line_height: 1.5 });
    expect(stylePatch(plain, "font", "Lora", fonts)).toEqual({ font: "Lora" });
    expect(stylePatch(styled, "font", "Inter", fonts)).toEqual({ font: null });
  });

  it("send a look's enabled with its fields, so the engine can read it", () => {
    expect(stylePatch(plain, "stroke.enabled", true)).toEqual({ stroke: { enabled: true } });
    expect(stylePatch(styled, "box.enabled", false), "turning off keeps its colour").toEqual({ box: { enabled: false } });
    expect(stylePatch(plain, "stroke.width", 0.2)).toEqual({ stroke: { enabled: false, width: 0.2 } });
    expect(stylePatch(styled, "box.padding", 0.5)).toEqual({ box: { enabled: true, padding: 0.5 } });
    expect(stylePatch(styled, "box.color", "#000000b3")).toEqual({ box: { enabled: true, color: null } });
    expect(stylePatch(plain, "glow.radius", 1)).toEqual({ glow: { enabled: false, radius: 1 } });
    expect(stylePatch(plain, "shadow.blur", 0.5)).toEqual({ shadow: { enabled: false, blur: 0.5 } });
    expect(stylePatch(styled, "shadow.y", 0.2)).toEqual({ shadow: { enabled: true, offset: [0.1, 0.2] } });
    expect(stylePatch(styled, "shadow.x", 0.06), "the offset back at its default").toEqual({
      shadow: { enabled: true, offset: null },
    });
  });

  it("refuse values out of range, of the wrong kind, unknown or unchanged", () => {
    expect(stylePatch(plain, "size", 8), "unchanged").toBeNull();
    expect(stylePatch(plain, "size", 0)).toBeNull();
    expect(stylePatch(plain, "size", 101)).toBeNull();
    expect(stylePatch(plain, "size", Number.NaN)).toBeNull();
    expect(stylePatch(plain, "letter_spacing", 3)).toBeNull();
    expect(stylePatch(plain, "line_height", 0.1)).toBeNull();
    expect(stylePatch(plain, "stroke.width", 1.5)).toBeNull();
    expect(stylePatch(plain, "shadow.x", -2)).toBeNull();
    expect(stylePatch(plain, "color", "red")).toBeNull();
    expect(stylePatch(plain, "color", "#FFF")).toBeNull();
    expect(stylePatch(plain, "case", "small_caps")).toBeNull();
    expect(stylePatch(plain, "bold", "yes")).toBeNull();
    expect(stylePatch(plain, "font", "Comic Sans", fonts), "a font the project lacks").toBeNull();
    expect(stylePatch(plain, "unknown", 1)).toBeNull();
  });

  it("pick colours without losing their alpha", () => {
    expect(normalColor(" #ffe14d ")).toBe("#FFE14D");
    expect(normalColor("#12345")).toBeNull();
    expect(rgbOf("#000000B3")).toBe("#000000");
    expect(rgbOf("not a colour")).toBe("#ffffff");
    expect(picked("#000000B3", "#ff0000")).toBe("#FF0000B3");
    expect(picked("#FFFFFF", "#00ff00")).toBe("#00FF00");
    expect(picked("#FFFFFF", "#00ff0080"), "the picker gives no alpha").toBeNull();
  });
});

describe("text reveals", () => {
  const revealed = clip("clip_0005", "text", 0, 4, {
    text: "Hello there",
    reveal: { unit: "word", motion: "fade", duration: moment(1.5), exit: true },
  });

  it("read the reveal, or a second-long one to start from", () => {
    const defaults = { overlap: 0.5, easing: "", order: "forward", exitMotion: "" };
    expect(revealOf(revealed)).toEqual({ unit: "word", motion: "fade", seconds: 1.5, exit: true, ...defaults });
    expect(revealOf(plain)).toEqual({ unit: "", motion: "appear", seconds: 1, exit: false, ...defaults });
    expect(revealOf(clip("clip_0006", "text", 0, 0.5)).seconds, "half of a short clip").toBe(0.25);
  });

  it("turn edits into the reveal argument, null removing it", () => {
    expect(revealChange(plain, { unit: "letter" })).toEqual({
      reveal: { unit: "letter", motion: "appear", duration: { seconds: "1" }, exit: false },
    });
    expect(revealChange(revealed, { seconds: 0.75 })).toEqual({
      reveal: { unit: "word", motion: "fade", duration: { seconds: "0.75" }, exit: true },
    });
    expect(revealChange(revealed, { motion: "rise", exit: false })?.reveal).toMatchObject({ motion: "rise", exit: false });
    expect(revealChange(revealed, { unit: "" })).toEqual({ reveal: null });
    expect(revealChange(revealed, { unit: "word" }), "unchanged").toBeNull();
    expect(revealChange(plain, { unit: "" }), "no reveal to remove").toBeNull();
    expect(revealChange(plain, { motion: "fade" }), "a motion needs a unit").toBeNull();
    expect(revealChange(revealed, { seconds: 0 })).toBeNull();
    expect(revealChange(revealed, { seconds: Number.NaN })).toBeNull();
    const options = { overlap: 0.5, easing: "", order: "forward", exitMotion: "" } as const;
    expect(revealArg({ unit: "line", motion: "rise", seconds: 1 / 3, exit: false, ...options })?.duration).toEqual({ seconds: "0.333" });
  });

  it("send the motion options only when they differ from their defaults", () => {
    const styled = clip("clip_0007", "text", 0, 4, {
      text: "Pop in",
      reveal: { unit: "letter", motion: "pop", duration: moment(1), exit: true, overlap: 0.8, easing: "bounce", order: "center", exit_motion: "blur" },
    });
    expect(revealOf(styled)).toMatchObject({ motion: "pop", overlap: 0.8, easing: "bounce", order: "center", exitMotion: "blur" });
    expect(revealChange(styled, { overlap: 0.5, easing: "", order: "forward" })).toEqual({
      reveal: { unit: "letter", motion: "pop", duration: { seconds: "1" }, exit: true, exit_motion: "blur" },
    });
    expect(revealChange(styled, { exit: false })?.reveal, "an exit motion needs an exit").not.toHaveProperty("exit_motion");
    expect(revealChange(styled, { easing: { bezier: [0.2, 0, 0.2, 1] } })?.reveal).toMatchObject({ easing: { bezier: [0.2, 0, 0.2, 1] } });
    expect(revealChange(styled, { overlap: 1.2 }), "out of range").toBeNull();
  });
});

describe("text and presets", () => {
  it("change the text only when it is new and not empty", () => {
    expect(textChange(plain, "Live edits\nby agents")).toEqual({ text: "Live edits\nby agents" });
    expect(textChange(plain, "Live edit")).toBeNull();
    expect(textChange(plain, "  \n")).toBeNull();
  });

  it("offer the project's presets first, then built-in ones it has not replaced", () => {
    const builtIn = [
      { name: "outline", description: "Black outline" },
      { name: "neon", description: "Glowing" },
    ];
    expect(presetOptions(["brand", "neon"], builtIn)).toEqual([
      { name: "brand", description: t("text.preset.saved"), saved: true },
      { name: "neon", description: t("text.preset.saved"), saved: true },
      { name: "outline", description: "Black outline", saved: false },
    ]);
    expect(presetOptions([], builtIn).map((p) => p.name)).toEqual(["outline", "neon"]);
  });

  describe("in Hungarian", () => {
    afterEach(() => setLanguage("en"));

    it("name the fields and saved presets in the interface's language, but not the engine's presets", () => {
      setLanguage("hu");
      expect(t(NUMBERS["letter_spacing"]!.label)).toBe("Betűköz");
      expect(presetOptions(["brand"], [{ name: "neon", description: "Glowing" }])).toEqual([
        { name: "brand", description: "Ebben a projektben mentve", saved: true },
        { name: "neon", description: "Glowing", saved: false },
      ]);
    });
  });
});
