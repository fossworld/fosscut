import { describe, expect, it } from "vitest";

import type { ShapeContentView } from "../engine/types";
import { clip } from "../test/fixtures";
import { SHAPE_DEFAULTS, SHAPES, shapeKinds, shapeOf, shapePatch, shownNumber } from "./shapeFields";

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
});
