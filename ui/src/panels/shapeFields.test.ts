import { afterEach, describe, expect, it } from "vitest";

import type { ShapeContentView } from "../engine/types";
import { setLanguage, t } from "../i18n";
import { clip } from "../test/fixtures";
import { SHAPE_DEFAULTS, SHAPES, SHAPE_NUMBERS, isOpen, shapeKinds, shapeOf, shapePatch, shownNumber } from "./shapeFields";

const shape = (content: ShapeContentView) => clip("clip_0041", "shape", 0, 3, { shape: content.shape, content });

const plain = shape({ shape: "ellipse", size: [0.2, 0.2], fill: { color: "#FFFFFF" } });
const badge = shape({
  shape: "rectangle",
  size: [0.25, 0.08],
  roundness: 1,
  fill: { color: "#FF3B5C", gradient: { to: "#FF9A3C", angle: 90 } },
  outline: { enabled: true, width: 0.1 },
  label: { text: "SALE", style: { size: 0.045, color: "#111111", bold: true } },
  label_at: [0, 0.2],
});

describe("shape fields", () => {
  it("read stored values with the engine's defaults for the rest", () => {
    expect(shapeOf(plain)).toEqual({
      shape: "ellipse",
      width: 0.2,
      height: 0.2,
      roundness: 0,
      color: "#FFFFFF",
      gradient: false,
      gradient_to: SHAPE_DEFAULTS.gradient_to,
      angle: 0,
      radial: false,
      outline: false,
      outline_color: "#000000",
      outline_width: 0.04,
      label: "",
      label_color: "#FFFFFF",
      label_size: 0.08,
      label_x: 0,
      label_y: 0,
      no_fill: false,
      closed: true,
      stroke: false,
      stroke_color: "#FFFFFF",
      stroke_width: 0.01,
      cap: "butt",
      join: "miter",
      dashed: false,
      dash: 0.02,
      gap: 0.015,
      trim_start: 0,
      trim_end: 1,
      trim_offset: 0,
    });
    expect(shapeOf(badge)).toMatchObject({
      roundness: 1,
      gradient: true,
      gradient_to: "#FF9A3C",
      angle: 90,
      outline: true,
      outline_color: "#000000",
      outline_width: 0.1,
      label: "SALE",
      label_color: "#111111",
      label_size: 0.045,
      label_y: 0.2,
    });
    expect(shapeOf(clip("clip_0042", "shape", 0, 1, { shape: "star" })).shape, "without its content").toBe("star");
    expect([shownNumber("width", 0.25), shownNumber("outline_width", 0.04), shownNumber("angle", 90)]).toEqual([
      25, 4, 90,
    ]);
  });

  it("turn sizes, roundness and the outline into sparse patches in shown units", () => {
    expect(shapePatch(plain, "width", 30)).toEqual({ size: [0.3, 0.2] });
    expect(shapePatch(plain, "height", 5)).toEqual({ size: [0.2, 0.05] });
    expect(shapePatch(plain, "width", 20), "unchanged").toBeNull();
    expect(shapePatch(plain, "width", 0), "below the smallest size").toBeNull();
    expect(shapePatch(plain, "width", Number.NaN)).toBeNull();
    expect(shapePatch(plain, "roundness", 40)).toEqual({ roundness: 0.4 });
    expect(shapePatch(badge, "roundness", 0), "back to the default").toEqual({ roundness: null });
    expect(shapePatch(plain, "outline", true)).toEqual({ outline: { enabled: true } });
    expect(shapePatch(badge, "outline", false)).toEqual({ outline: { enabled: false } });
    expect(shapePatch(badge, "outline_color", "#ff0000"), "always with enabled").toEqual({
      outline: { enabled: true, color: "#FF0000" },
    });
    expect(shapePatch(badge, "outline_width", 4)).toEqual({ outline: { enabled: true, width: null } });
    expect(shapePatch(badge, "outline_width", 60), "over half the shorter side").toBeNull();
  });

  it("switch outlines among the drawn shapes, and keep a path's points", () => {
    expect(shapePatch(plain, "shape", "star")).toEqual({ shape: "star" });
    expect(shapePatch(plain, "shape", "path"), "a path needs points").toBeNull();
    expect(shapePatch(plain, "shape", "brush")).toBeNull();
    expect(shapeKinds(plain)).toEqual(SHAPES);
    const pen = shape({ shape: "path", size: [0.3, 0.3], points: [{ at: [0, -1] }, { at: [1, 1] }, { at: [-1, 1] }] });
    expect(shapeKinds(pen)).toEqual([...SHAPES, "path"]);
    expect(shapePatch(pen, "shape", "heart")).toEqual({ shape: "heart" });
  });

  it("turn fills and gradients into patches, gradient fields only with a gradient", () => {
    expect(shapePatch(plain, "color", "#00aaff")).toEqual({ fill: { color: "#00AAFF" } });
    expect(shapePatch(plain, "color", "blue")).toBeNull();
    expect(shapePatch(plain, "gradient", true)).toEqual({ fill: { gradient: { to: "#000000" } } });
    expect(shapePatch(badge, "gradient", false)).toEqual({ fill: { gradient: null } });
    expect(shapePatch(plain, "gradient_to", "#123456"), "no gradient to change").toBeNull();
    expect(shapePatch(badge, "gradient_to", "#123456")).toEqual({ fill: { gradient: { to: "#123456" } } });
    expect(shapePatch(badge, "angle", 45)).toEqual({ fill: { gradient: { angle: 45 } } });
    expect(shapePatch(badge, "angle", 0)).toEqual({ fill: { gradient: { angle: null } } });
    expect(shapePatch(badge, "angle", 400)).toBeNull();
    expect(shapePatch(badge, "radial", true)).toEqual({ fill: { gradient: { radial: true } } });
    expect(shapePatch(plain, "radial", true)).toBeNull();
  });

  it("add, change and remove the label, its colour, size and place", () => {
    expect(shapePatch(plain, "label", "  NEW ")).toEqual({ label: { text: "NEW" } });
    expect(shapePatch(plain, "label", "   "), "no label to remove").toBeNull();
    expect(shapePatch(badge, "label", "")).toEqual({ label: null });
    expect(shapePatch(badge, "label", "SALE"), "unchanged").toBeNull();
    expect(shapePatch(badge, "label", "50% OFF")).toEqual({ label: { text: "50% OFF" } });
    expect(shapePatch(badge, "label_color", "#FFFFFF"), "the default").toEqual({ label: { style: { color: null } } });
    expect(shapePatch(badge, "label_color", "#00ff00")).toEqual({ label: { style: { color: "#00FF00" } } });
    expect(shapePatch(badge, "label_size", 6)).toEqual({ label: { style: { size: 0.06 } } });
    expect(shapePatch(plain, "label_size", 6), "no label yet").toBeNull();
    expect(shapePatch(badge, "label_x", 50)).toEqual({ label_at: [0.5, 0.2] });
    expect(shapePatch(badge, "label_y", 0), "the middle").toEqual({ label_at: null });
    expect(shapePatch(badge, "label_y", -150)).toBeNull();
    expect(shapePatch(plain, "label_x", 10)).toBeNull();
  });

  it("turn no fill, strokes, dashes and trims into vector patches (ADR-0036)", () => {
    const line = shape({
      shape: "path",
      size: [0.5, 0.2],
      points: [{ at: [-1, 1] }, { at: [1, -1] }],
      closed: false,
      fill: null,
      stroke: { color: "#FF7A45", width: 0.02, cap: "round", dashes: [0.03, 0.01] },
      trim_end: 0.4,
    });
    expect(shapeOf(line)).toMatchObject({
      no_fill: true,
      closed: false,
      stroke: true,
      stroke_color: "#FF7A45",
      stroke_width: 0.02,
      cap: "round",
      join: "miter",
      dashed: true,
      dash: 0.03,
      gap: 0.01,
      trim_end: 0.4,
    });
    expect(isOpen(line) && !isOpen(plain)).toBe(true);
    expect(shapePatch(plain, "no_fill", true), "null is no fill").toEqual({ fill: null });
    expect(shapePatch(line, "no_fill", false)).toEqual({ fill: { color: "#FFFFFF" } });
    expect(shapePatch(line, "closed", true), "closed is the default").toEqual({ closed: null });
    expect(shapePatch(plain, "closed", false), "only paths open").toBeNull();
    expect(shapePatch(plain, "stroke", true)).toEqual({ stroke: { color: "#FFFFFF" } });
    expect(shapePatch(line, "stroke", false)).toEqual({ stroke: null });
    expect(shapePatch(line, "stroke_color", "#00ff00")).toEqual({ stroke: { color: "#00FF00" } });
    expect(shapePatch(plain, "stroke_color", "#00ff00"), "no stroke yet").toBeNull();
    expect(shapePatch(line, "stroke_width", 0.01), "the default width").toEqual({ stroke: { width: null } });
    expect(shapePatch(line, "stroke_width", 0.8), "over half the canvas").toBeNull();
    expect(shapePatch(line, "cap", "butt")).toEqual({ stroke: { cap: null } });
    expect(shapePatch(line, "join", "bevel")).toEqual({ stroke: { join: "bevel" } });
    expect(shapePatch(line, "join", "sharp")).toBeNull();
    expect(shapePatch(line, "dashed", false)).toEqual({ stroke: { dashes: null } });
    expect(shapePatch(line, "dash", 5)).toEqual({ stroke: { dashes: [0.05, 0.01] } });
    expect(shapePatch(line, "gap", 2)).toEqual({ stroke: { dashes: [0.03, 0.02] } });
    expect(shapePatch(line, "trim_end", 1), "the whole outline").toEqual({ trim_end: null });
    expect(shapePatch(line, "trim_start", 0.25)).toEqual({ trim_start: 0.25 });
    expect(shapePatch(line, "trim_start", 1.5)).toBeNull();
    expect(shapePatch(line, "trim_offset", -0.5), "offsets wrap").toEqual({ trim_offset: -0.5 });
  });

  describe("in Hungarian", () => {
    afterEach(() => setLanguage("en"));

    it("name the number fields in the interface's language", () => {
      expect(t(SHAPE_NUMBERS.label_x!.label)).toBe("Across");
      setLanguage("hu");
      expect(t(SHAPE_NUMBERS.label_x!.label)).toBe("Vízszintesen");
      expect(t(SHAPE_NUMBERS.roundness!.label)).toBe("Kerekítés");
    });
  });
});
