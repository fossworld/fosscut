import { afterEach, describe, expect, it } from "vitest";

import { setLanguage, t } from "../i18n";
import { clip } from "../test/fixtures";
import { speedLabel } from "./speed";

describe("speed", () => {
  afterEach(() => setLanguage("en"));

  const at = (speed: Parameters<typeof clip>[4]) => speedLabel(clip("clip_0003", "video", 0, 4, speed));

  it("labels rates, curves, reversal and freezes", () => {
    expect(at({})).toBeNull();
    expect(at({ speed: { rate: 2 } })).toBe(t("motion.speed.label.rate", { rate: 2 }));
    expect(at({ speed: { rate: 2 } })).toBe("2x");
    expect(at({ speed: { rate: 0.333 } })).toBe("0.33x");
    expect(at({ speed: { reverse: true } })).toBe(t("motion.speed.label.reversed", { speed: "1x" }));
    expect(at({ speed: { curve: [{ at: 0, rate: 1 }, { at: 1, rate: 2 }] } })).toBe(t("motion.speed.label.curve"));
    expect(at({ speed: { curve: [{ at: 0, rate: 1 }], reverse: true } })).toBe(
      t("motion.speed.label.reversed", { speed: t("motion.speed.label.curve") }),
    );
    expect(at({ speed: { rate: 0 } })).toBe(t("motion.speed.label.freeze"));
  });

  it("labels them in Hungarian, with a decimal comma", () => {
    setLanguage("hu");
    expect(at({ speed: { rate: 0.333 } })).toBe("0,33x");
    expect(at({ speed: { curve: [{ at: 0, rate: 1 }], reverse: true } })).toBe("görbe, visszafelé");
    expect(at({ speed: { rate: 0 } })).toBe("kimerevítve");
  });
});
