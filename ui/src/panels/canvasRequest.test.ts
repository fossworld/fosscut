import { afterEach, describe, expect, it } from "vitest";

import { setLanguage, t } from "../i18n";
import { clip } from "../test/fixtures";
import { blurAmount, canvasCommand, fillColor, fillKind, fillOf, fillTargets, hasCanvas } from "./canvasRequest";

afterEach(() => setLanguage("en"));

describe("the canvas fill in the inspector", () => {
  it("applies to the selected video and image clips only, the inspected one first", () => {
    const video = clip("clip_0003", "video", 0, 6);
    const image = clip("clip_0004", "image", 0, 3);
    const title = clip("clip_0005", "text", 0, 2);
    expect([video, image, title].map(hasCanvas)).toEqual([true, true, false]);
    expect(fillTargets(image, [video, image, title])).toEqual(["clip_0004", "clip_0003"]);
    expect(fillTargets(title, [title]), "a title has no canvas").toEqual([]);
  });

  it("builds the clip.canvas request for none, a blur and a colour", () => {
    const blurred = clip("clip_0003", "video", 0, 6, { canvas: { kind: "blur", amount: 0.12 } });
    expect(fillKind(blurred)).toBe("blur");
    expect(fillKind(clip("clip_0004", "image", 0, 3))).toBe("none");
    expect(canvasCommand(["clip_0003", "clip_0004"], fillOf("blur", undefined))).toEqual({
      command: "clip.canvas",
      args: { clips: ["clip_0003", "clip_0004"], fill: { kind: "blur", amount: 0.05 } },
    });
    expect(canvasCommand(["clip_0003"], fillOf("blur", blurred.canvas, 0.3)), "the slider stops at 0.2").toEqual({
      command: "clip.canvas",
      args: { clips: ["clip_0003"], fill: { kind: "blur", amount: 0.2 } },
    });
    expect(fillOf("blur", blurred.canvas), "a blur keeps its amount").toEqual({ kind: "blur", amount: 0.12 });
    expect(canvasCommand(["clip_0003"], fillOf("color", blurred.canvas, "#1a2b3c"))).toEqual({
      command: "clip.canvas",
      args: { clips: ["clip_0003"], fill: { kind: "color", color: "#1A2B3C" } },
    });
    expect(fillOf("color", undefined)).toEqual({ kind: "color", color: "#000000" });
    expect(canvasCommand(["clip_0003"], fillOf("none", blurred.canvas)), "none removes the fill").toEqual({
      command: "clip.canvas",
      args: { clips: ["clip_0003"] },
    });
  });

  it("keeps amounts and colours to what the engine takes", () => {
    expect(blurAmount(-1)).toBe(0);
    expect(blurAmount(0.04449)).toBe(0.044);
    expect(blurAmount(Number.NaN)).toBe(0.05);
    expect(fillColor("ABCDEF")).toBe("#ABCDEF");
    expect(fillColor("red")).toBe("#000000");
  });

  it("names its choices in the interface's language", () => {
    expect(t("canvas.blur")).toBe("Blur");
    setLanguage("hu");
    expect(t("canvas.color")).toBe("Szín");
    expect(t("canvas.several", { count: 2 })).toBe("2 kijelölt videó- és képklipre érvényes");
  });
});

describe("the canvas fill panel", () => {
  it("shows the clip's fill, with the blur slider or the colour picker", async () => {
    const { createElement } = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { CanvasFill } = await import("./CanvasFill");
    const { Editor } = await import("../app/editor");
    const { FakeEngine } = await import("../engine/fake");
    const editor = new Editor(new FakeEngine());
    const render = (canvas?: { kind: "blur"; amount: number } | { kind: "color"; color: string }) =>
      renderToStaticMarkup(createElement(CanvasFill, { editor, clip: clip("clip_0003", "video", 0, 6, canvas ? { canvas } : {}) }));
    const blurred = render({ kind: "blur", amount: 0.1 });
    expect(blurred).toContain('type="range"');
    expect(blurred).toContain(`aria-checked="true" class="active">${t("canvas.blur")}`);
    expect(blurred).toContain("10%");
    expect(render({ kind: "color", color: "#203040" })).toContain('type="color"');
    const none = render();
    expect(none).not.toContain('type="range"');
    expect(none).toContain(`aria-checked="true" class="active">${t("canvas.none")}`);
    setLanguage("hu");
    expect(render()).toContain("Vászon");
  });
});
